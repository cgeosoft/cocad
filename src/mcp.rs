//! MCP server, so that Claude Desktop and other MCP clients can drive the app.
//!
//! The client starts `cocad mcp` and speaks JSON-RPC on stdin and stdout.
//! That process holds no model. It passes each tool call to a running window.
//!
//! Each window listens on a free loopback TCP port and writes the port and a
//! random token to `<config>/mcp/<pid>.json`, readable only by the user. A call
//! is one line of JSON and the reply is one line of JSON. TCP keeps one code
//! path on Linux, Windows and macOS.
//!
//! `app/remote.rs` runs the tools inside the window.

use anyhow::{Context, Result};
use serde_json::{Value as Json, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const PROTOCOL_VERSIONS: [&str; 4] = ["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"];
const MAX_LINE: u64 = 1 << 20;
/// Longest time a window has to answer a call. The window itself gives up
/// on waits sooner, see `remote::WAIT`.
const CALL_TIMEOUT: Duration = Duration::from_secs(70);

const INSTRUCTIONS: &str = "CoCAD shows one OpenSCAD model in a desktop window. \
Start with get_state, then get_params to see the Customizer parameters and their limits. \
Use get_source to read the model code. Use view_model to see the model as an image. \
set_params changes values in the window and waits for the render. \
The mesh data in the reply shows the result. If the status is failed, read the error and the log.";

fn endpoint_dir() -> Option<PathBuf> {
    Some(crate::openscad::config_dir()?.join("mcp"))
}

/// One tool call inside a window.
pub struct Request {
    pub tool: String,
    pub args: Json,
    pub reply: Sender<Result<Json, String>>,
}

/// The listener of a window. The endpoint file goes away with it.
pub struct Server {
    pub requests: Receiver<Request>,
    endpoint: PathBuf,
    port: u16,
    stop: Arc<AtomicBool>,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.endpoint);
        // The listener sleeps in accept: one connection wakes it to see the flag.
        self.stop.store(true, Ordering::Relaxed);
        let addr = SocketAddr::from(([127, 0, 0, 1], self.port));
        let _ = TcpStream::connect_timeout(&addr, Duration::from_millis(200));
    }
}

fn write_private(path: &std::path::Path, text: &str) -> std::io::Result<()> {
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    o.open(path)?.write_all(text.as_bytes())
}

fn same_token(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0, |d, (x, y)| d | (x ^ y)) == 0
}

/// Starts the listener of this window. `wake` runs after each request.
pub fn serve(wake: impl Fn() + Send + Clone + 'static) -> Result<Server> {
    let mut raw = [0u8; 32];
    getrandom::fill(&mut raw).map_err(|e| anyhow::anyhow!("no random source: {e}"))?;
    let token: String = raw.iter().map(|b| format!("{b:02x}")).collect();

    let listener = TcpListener::bind(("127.0.0.1", 0)).context("cannot open a local port")?;
    let port = listener.local_addr()?.port();
    let dir = endpoint_dir().context("no config folder")?;
    std::fs::create_dir_all(&dir)?;
    let pid = std::process::id();
    let endpoint = dir.join(format!("{pid}.json"));
    let started = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    write_private(
        &endpoint,
        &json!({ "pid": pid, "port": port, "token": token, "started": started }).to_string(),
    )?;

    let (tx, requests) = channel();
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            if flag.load(Ordering::Relaxed) {
                break;
            }
            let Ok(stream) = stream else { continue };
            let (tx, token, wake) = (tx.clone(), token.clone(), wake.clone());
            std::thread::spawn(move || {
                let _ = connection(stream, &tx, &token, wake);
            });
        }
    });
    Ok(Server {
        requests,
        endpoint,
        port,
        stop,
    })
}

fn connection(
    stream: TcpStream,
    tx: &Sender<Request>,
    token: &str,
    wake: impl Fn(),
) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut line = String::new();
    BufReader::new((&stream).take(MAX_LINE)).read_line(&mut line)?;
    let msg: Json = serde_json::from_str(&line).unwrap_or_default();
    // Close without a reply: nothing tells a stranger what listens here.
    if !same_token(msg["token"].as_str().unwrap_or(""), token) {
        return Ok(());
    }
    let (reply, rx) = channel();
    let req = Request {
        tool: msg["tool"].as_str().unwrap_or("").to_string(),
        args: msg["args"].clone(),
        reply,
    };
    let result = if tx.send(req).is_ok() {
        wake();
        rx.recv_timeout(CALL_TIMEOUT - Duration::from_secs(2))
            .unwrap_or_else(|_| {
                Err("The window does not respond. It may be minimized or hidden.".into())
            })
    } else {
        Err("The window is closing.".into())
    };
    let out = match result {
        Ok(v) => json!({ "ok": v }),
        Err(e) => json!({ "error": e }),
    };
    let mut w = &stream;
    writeln!(w, "{out}")
}

// ---------------------------------------------------------------- client ----

struct Instance {
    pid: u64,
    addr: SocketAddr,
    token: String,
    started: u64,
}

impl Instance {
    fn call(&self, tool: &str, args: &Json) -> Result<Json, String> {
        let closed = |e: std::io::Error| format!("CoCAD connection failed: {e}");
        let stream = TcpStream::connect_timeout(&self.addr, Duration::from_secs(2))
            .map_err(|e| format!("CoCAD window is gone: {e}"))?;
        stream
            .set_read_timeout(Some(CALL_TIMEOUT))
            .map_err(closed)?;
        let msg = json!({ "token": self.token, "tool": tool, "args": args });
        let mut w = &stream;
        writeln!(w, "{msg}").map_err(closed)?;
        let mut line = String::new();
        BufReader::new(&stream)
            .read_line(&mut line)
            .map_err(closed)?;
        if line.is_empty() {
            return Err("The window closed the connection.".into());
        }
        let reply: Json = serde_json::from_str(&line).map_err(|e| e.to_string())?;
        match reply.get("error") {
            Some(e) => Err(e.as_str().unwrap_or("unknown error").to_string()),
            None => Ok(reply["ok"].clone()),
        }
    }
}

/// The windows that answer, newest first. Files of dead windows are removed.
fn instances() -> Vec<Instance> {
    let Some(entries) = endpoint_dir().and_then(|d| std::fs::read_dir(d).ok()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for path in entries.flatten().map(|e| e.path()) {
        let Some(j) = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str::<Json>(&t).ok())
        else {
            continue;
        };
        let addr = SocketAddr::from(([127, 0, 0, 1], j["port"].as_u64().unwrap_or(0) as u16));
        if TcpStream::connect_timeout(&addr, Duration::from_millis(300)).is_err() {
            let _ = std::fs::remove_file(&path);
            continue;
        }
        out.push(Instance {
            pid: j["pid"].as_u64().unwrap_or(0),
            addr,
            token: j["token"].as_str().unwrap_or("").to_string(),
            started: j["started"].as_u64().unwrap_or(0),
        });
    }
    out.sort_by_key(|i| std::cmp::Reverse(i.started));
    out
}

/// Starts a window and waits until it listens.
fn launch() -> Result<Instance, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    std::process::Command::new(exe)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("Cannot start CoCAD: {e}"))?;
    let until = Instant::now() + Duration::from_secs(15);
    while Instant::now() < until {
        std::thread::sleep(Duration::from_millis(200));
        if let Some(i) = instances().into_iter().next() {
            return Ok(i);
        }
    }
    Err("CoCAD started, but its window does not answer. \
         Check that the MCP option is on in File > Settings."
        .into())
}

// ----------------------------------------------------------------- tools ----

const VIEW: &str =
    "Camera direction: iso, top, front or right. Default: the direction of the window.";

fn tool(name: &str, description: &str, read_only: bool, destructive: bool, input: Json) -> Json {
    json!({
        "name": name,
        "description": description,
        "inputSchema": input,
        "annotations": { "readOnlyHint": read_only, "destructiveHint": destructive },
    })
}

fn no_args() -> Json {
    json!({ "type": "object", "properties": {} })
}

fn photo_props() -> Json {
    json!({
        "width": { "type": "integer", "description": "Image width in pixels" },
        "height": { "type": "integer", "description": "Image height in pixels" },
        "samples": { "type": "integer", "description": "Samples per pixel. More is cleaner and slower." },
        "view": { "type": "string", "enum": ["iso", "top", "front", "right"], "description": VIEW },
    })
}

fn tools() -> Json {
    let mut photo = photo_props();
    let m = photo.as_object_mut().unwrap();
    m.insert(
        "path".into(),
        json!({ "type": "string", "description": "Absolute path of the .png file" }),
    );
    m.insert(
        "overwrite".into(),
        json!({ "type": "boolean", "description": "Replace an existing file" }),
    );
    m.insert(
        "color".into(),
        json!({ "type": "string", "description": "Part color as RRGGBB" }),
    );
    m.insert(
        "finish".into(),
        json!({ "type": "string", "enum": ["matte", "satin", "gloss"] }),
    );
    m.insert(
        "studio".into(),
        json!({ "type": "string", "enum": ["dark", "light"] }),
    );
    m.insert(
        "layer_lines".into(),
        json!({ "type": "boolean", "description": "Show print layer lines" }),
    );

    let wait = json!({ "type": "boolean", "description": "Wait for the render. Default: true." });
    json!([
        tool(
            "list_windows",
            "List the open CoCAD windows and their models. Use this when more than one window is open.",
            true,
            false,
            no_args()
        ),
        tool(
            "select_window",
            "Send the next tool calls to this window. Without a selection, the newest window gets them.",
            false,
            false,
            json!({
                "type": "object",
                "properties": { "id": { "type": "integer", "description": "Window id from list_windows" } },
                "required": ["id"],
            })
        ),
        tool(
            "get_state",
            "Get the loaded model and the app state: file, render status, last error, mesh size, volume and open edges, changed parameters, parameter sets, view mode and the end of the OpenSCAD log.",
            true,
            false,
            no_args()
        ),
        tool(
            "get_params",
            "List all Customizer parameters of the loaded model: name, group, description, type, current value, default, and the limits or options.",
            true,
            false,
            no_args()
        ),
        tool(
            "get_source",
            "Read the OpenSCAD source of the loaded model. Files it includes are not part of the result.",
            true,
            false,
            no_args()
        ),
        tool(
            "view_model",
            "Render the model to an image and return it, so that you can see it. The whole model is in frame. The window does not change. Default 640 x 480, at most 1024 per side.",
            true,
            false,
            json!({ "type": "object", "properties": photo_props() })
        ),
        tool(
            "open_model",
            "Open a .scad file in the window. Starts CoCAD when it is not running. Edited values of the previous model are lost.",
            false,
            false,
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute path of the .scad file" },
                    "wait": wait,
                },
                "required": ["path"],
            })
        ),
        tool(
            "set_params",
            "Change Customizer parameters in the window. The change is all or nothing: one bad value rejects the call. Returns the state after the render.",
            false,
            false,
            json!({
                "type": "object",
                "properties": {
                    "values": {
                        "type": "object",
                        "description": "Parameter names and new values. Vectors are arrays of numbers.",
                        "additionalProperties": true,
                    },
                    "render": { "type": "boolean", "description": "Render now. Default: true. With false, the app follows its Auto render switch." },
                    "wait": wait,
                },
                "required": ["values"],
            })
        ),
        tool(
            "reset_params",
            "Set parameters back to the defaults of the model. Without names, resets all of them.",
            false,
            false,
            json!({
                "type": "object",
                "properties": {
                    "names": { "type": "array", "items": { "type": "string" } },
                    "wait": wait,
                },
            })
        ),
        tool(
            "render",
            "Run OpenSCAD again with the current values. Reads the file again when it changed on disk.",
            false,
            false,
            json!({ "type": "object", "properties": { "wait": wait } })
        ),
        tool(
            "load_parameter_set",
            "Load a saved parameter set. Without a name, loads the model defaults.",
            false,
            false,
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "wait": wait,
                },
            })
        ),
        tool(
            "save_parameter_set",
            "Save the current values as a named set in the sets file of the model.",
            false,
            true,
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "overwrite": { "type": "boolean", "description": "Replace a set with this name" },
                },
                "required": ["name"],
            })
        ),
        tool(
            "export_model",
            "Save the current mesh. The file extension sets the format: .stl, .3mf or .step. Waits for a running render. Fails when the last render failed.",
            false,
            true,
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute path of the output file" },
                    "overwrite": { "type": "boolean", "description": "Replace an existing file" },
                },
                "required": ["path"],
            })
        ),
        tool(
            "save_photo",
            "Render a path traced photo of the model and save it as PNG. The whole model is in frame. The call stops after 40 seconds and saves the samples it has.",
            false,
            true,
            json!({ "type": "object", "properties": photo, "required": ["path"] })
        ),
        tool(
            "slice",
            "Slice the model and return layers, filament, weight and a rough print time. Given values also change the Slice panel.",
            false,
            false,
            json!({
                "type": "object",
                "properties": {
                    "layer_height": { "type": "number", "description": "mm" },
                    "first_layer": { "type": "number", "description": "mm" },
                    "line_width": { "type": "number", "description": "mm" },
                    "walls": { "type": "integer" },
                    "infill": { "type": "number", "description": "Percent, 0 to 100" },
                    "solid_layers": { "type": "integer", "description": "Solid layers at the top and at the bottom" },
                    "speed": { "type": "number", "description": "mm/s" },
                },
            })
        ),
        tool(
            "set_view",
            "Change what the window shows. All fields are optional.",
            false,
            false,
            json!({
                "type": "object",
                "properties": {
                    "mode": { "type": "string", "enum": ["model", "slice", "photo"] },
                    "view": { "type": "string", "enum": ["iso", "top", "front", "right"] },
                    "fit": { "type": "boolean", "description": "Fit the model in the view" },
                    "wireframe": { "type": "boolean" },
                    "grid": { "type": "boolean" },
                    "layer": { "type": "integer", "description": "Slice layer, from 1" },
                },
            })
        ),
    ])
}

/// Text or image content for a tool result.
fn content(v: &Json) -> Json {
    let mut out = Vec::new();
    if let Some(data) = v.get("image") {
        out.push(json!({ "type": "image", "data": data, "mimeType": "image/png" }));
        out.push(json!({ "type": "text", "text": v["info"].to_string() }));
    } else {
        let text = serde_json::to_string_pretty(v).unwrap_or_default();
        out.push(json!({ "type": "text", "text": text }));
    }
    Json::Array(out)
}

// --------------------------------------------------------------- JSON-RPC ----

#[derive(Default)]
struct Bridge {
    selected: Option<u64>,
}

impl Bridge {
    fn window(&self) -> Result<Instance, String> {
        let mut all = instances();
        if all.is_empty() {
            return Err("CoCAD is not running, or its MCP option is off. \
                 Call open_model with a path to start it."
                .into());
        }
        let at = self
            .selected
            .and_then(|s| all.iter().position(|i| i.pid == s));
        Ok(all.remove(at.unwrap_or(0)))
    }

    fn list_windows(&self) -> Result<Json, String> {
        let all = instances();
        let current = self.window().ok().map(|i| i.pid);
        let list: Vec<Json> = all
            .iter()
            .map(|i| {
                let st = i.call("get_state", &json!({})).unwrap_or_default();
                json!({
                    "id": i.pid,
                    "file": st["file"],
                    "status": st["status"],
                    "selected": Some(i.pid) == current,
                })
            })
            .collect();
        Ok(json!({ "windows": list }))
    }

    fn call(&mut self, name: &str, args: &Json) -> Result<Json, String> {
        match name {
            "list_windows" => self.list_windows(),
            "select_window" => {
                let id = args["id"].as_u64().ok_or("id must be a number")?;
                if !instances().iter().any(|i| i.pid == id) {
                    return Err(format!("No window with id {id}."));
                }
                self.selected = Some(id);
                self.list_windows()
            }
            "open_model" => {
                let window = match self.window() {
                    Ok(w) => w,
                    Err(_) => launch()?,
                };
                window.call(name, args)
            }
            _ if tools()
                .as_array()
                .unwrap()
                .iter()
                .any(|t| t["name"] == name) =>
            {
                self.window()?.call(name, args)
            }
            _ => Err(format!("Unknown tool {name}.")),
        }
    }

    /// Handles one message. Notifications get no reply.
    fn handle(&mut self, msg: &Json) -> Option<Json> {
        let id = msg.get("id")?.clone();
        let method = msg["method"].as_str().unwrap_or("");
        let result = match method {
            "initialize" => {
                let asked = msg["params"]["protocolVersion"].as_str().unwrap_or("");
                let latest = PROTOCOL_VERSIONS[PROTOCOL_VERSIONS.len() - 1];
                let version = PROTOCOL_VERSIONS
                    .into_iter()
                    .find(|v| *v == asked)
                    .unwrap_or(latest);
                json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "cocad", "version": env!("CARGO_PKG_VERSION") },
                    "instructions": INSTRUCTIONS,
                })
            }
            "ping" => json!({}),
            "tools/list" => json!({ "tools": tools() }),
            "tools/call" => {
                let name = msg["params"]["name"].as_str().unwrap_or("");
                let empty = json!({});
                let args = msg["params"].get("arguments").unwrap_or(&empty);
                match self.call(name, args) {
                    Ok(v) => json!({ "content": content(&v), "isError": false }),
                    Err(e) => json!({
                        "content": [{ "type": "text", "text": e }],
                        "isError": true,
                    }),
                }
            }
            _ => {
                return Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32601, "message": format!("Unknown method {method}") },
                }));
            }
        };
        Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
    }
}

/// `cocad mcp`: serves MCP on stdin and stdout until the client closes them.
pub fn run() -> Result<()> {
    let mut bridge = Bridge::default();
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let out = match serde_json::from_str::<Json>(&line) {
            Ok(msg) => bridge.handle(&msg),
            Err(e) => Some(json!({
                "jsonrpc": "2.0",
                "id": null,
                "error": { "code": -32700, "message": e.to_string() },
            })),
        };
        if let Some(out) = out {
            writeln!(stdout, "{out}")?;
            stdout.flush()?;
        }
    }
    Ok(())
}

/// Standard base64 for image results.
pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_known_values() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
    }

    #[test]
    fn initialize_keeps_a_known_version() {
        let mut b = Bridge::default();
        let r = b
            .handle(&json!({ "id": 1, "method": "initialize",
                "params": { "protocolVersion": "2025-03-26" } }))
            .unwrap();
        assert_eq!(r["id"], 1);
        assert_eq!(r["result"]["protocolVersion"], "2025-03-26");
        assert!(r["result"]["capabilities"]["tools"].is_object());
    }

    #[test]
    fn initialize_answers_an_unknown_version_with_the_latest() {
        let mut b = Bridge::default();
        let r = b
            .handle(&json!({ "id": 1, "method": "initialize",
                "params": { "protocolVersion": "1999-01-01" } }))
            .unwrap();
        assert_eq!(r["result"]["protocolVersion"], "2025-11-25");
    }

    #[test]
    fn notifications_get_no_reply() {
        let mut b = Bridge::default();
        assert!(
            b.handle(&json!({ "method": "notifications/initialized" }))
                .is_none()
        );
    }

    #[test]
    fn unknown_method_is_an_error() {
        let mut b = Bridge::default();
        let r = b.handle(&json!({ "id": 2, "method": "nope" })).unwrap();
        assert_eq!(r["error"]["code"], -32601);
    }

    #[test]
    fn tool_names_are_unique_and_have_schemas() {
        let t = tools();
        let list = t.as_array().unwrap();
        let mut names: Vec<&str> = list.iter().map(|t| t["name"].as_str().unwrap()).collect();
        names.sort();
        let n = names.len();
        names.dedup();
        assert_eq!(n, names.len());
        assert!(list.iter().all(|t| t["inputSchema"]["type"] == "object"));
    }

    #[test]
    fn token_compare() {
        assert!(same_token("abc", "abc"));
        assert!(!same_token("abc", "abd"));
        assert!(!same_token("abc", "ab"));
        assert!(!same_token("", "abc"));
    }

    #[test]
    fn image_results_become_image_content() {
        let c = content(&json!({ "image": "QUJD", "info": { "samples": 4 } }));
        assert_eq!(c[0]["type"], "image");
        assert_eq!(c[0]["mimeType"], "image/png");
        assert_eq!(c[1]["type"], "text");
    }
}
