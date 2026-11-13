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
set_params changes values in the window and waits for the build (the OpenSCAD run). \
The mesh data in the reply shows the result. If the status is failed, read the error and the log. \
get_optimize shows the print preparation: the print parts, the plate, the fixes and the problems. \
set_optimize changes it. export_model writes the plate as STL or 3MF, and the model as STEP.";

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

pub(crate) fn tools() -> Json {
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

    let wait = json!({ "type": "boolean", "description": "Wait for the build. Default: true." });
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
            "Get the loaded model and the app state: file, build status, last error, mesh size, volume and open edges, changed parameters, parameter sets, view mode, parts, part colors, explode, section cut, the copies on the plate of the Optimize tab and the end of the OpenSCAD log.",
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
            "Render the model to an image and return it, so that you can see it. The whole model is in frame, with the part colors, the explode and the section cut of the window. The window does not change. Default 640 x 480, at most 1024 per side.",
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
            "new_model",
            "Write a new .scad file and open it in the window. Starts CoCAD when it is not running. The file must not exist. Without source, the file gets a small model with four parameters. Edit the file later and call build to see the change.",
            false,
            false,
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute path of the new .scad file" },
                    "source": { "type": "string", "description": "OpenSCAD source of the new model" },
                    "wait": wait,
                },
                "required": ["path"],
            })
        ),
        tool(
            "set_params",
            "Change Customizer parameters in the window. The change is all or nothing: one bad value rejects the call. Returns the state after the build.",
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
                    "build": { "type": "boolean", "description": "Build now. Default: true. With false, the app follows its Auto build switch. The old name render also works." },
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
            "build",
            "Build the model: run OpenSCAD again with the current values. Reads the file again when it changed on disk. The old name render also works.",
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
            "Save the current values as a named set in the definition of the model (<model>.yml next to the .scad file).",
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
            "Save the model. The file extension sets the format. .stl and .3mf write the plate of the Optimize tab, as the window does: all print parts in print orientation, with the print fixes that are on, in the plate layout. There is one plate: when the parts need more room than the bed, the slicer arranges them on its plates. A .3mf has one object for each print part and one build item for each copy. .step writes the model as built (with only set in the Optimize state: that part). With slicer, builds each part of the multicolor part list and writes a multicolor .3mf for that slicer. Prepares the plate when it is not ready. Waits for a running build. Fails when the last build failed.",
            false,
            true,
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute path of the output file" },
                    "overwrite": { "type": "boolean", "description": "Replace an existing file" },
                    "slicer": { "type": "string", "enum": ["prusa", "orca"], "description": "Write a multicolor 3MF for this slicer" },
                },
                "required": ["path"],
            })
        ),
        tool(
            "get_optimize",
            "Get the print preparation of the Optimize tab: the state (print area, gap, the part on the plate, the fixes and the values of each print part) and the result: the print parts with count, copies, size, fit, orientation, fix counts, problems and split proposal; the copies on the plate and whether they need more room than the bed; errors. The print area sets the fit and the splits. All print parts are on the one plate. Prepares the plate when it is not ready.",
            true,
            false,
            no_args()
        ),
        tool(
            "set_optimize",
            "Change the print preparation of the Optimize tab. The keys are those of the state of get_optimize. Objects merge key by key, null removes a key (for example \"only\": null prints all parts, \"parts\": {\"Lid\": null} resets the part Lid). All fixes are off until you turn them on. Returns the result as get_optimize does.",
            false,
            false,
            json!({
                "type": "object",
                "properties": {
                    "area": { "type": "array", "items": { "type": "number" }, "description": "Print area [width, depth, height] in mm" },
                    "gap": { "type": "number", "description": "Gap between the parts on the plate, in mm" },
                    "only": { "type": ["string", "null"], "description": "Put only this print part on the plate. null: all parts." },
                    "fixes": {
                        "type": "object",
                        "description": "The print fixes",
                        "properties": {
                            "teardrops": { "type": "boolean", "description": "Teardrop holes: round holes with a horizontal axis get a 45 degree point" },
                            "bridges": { "type": "boolean", "description": "Bridged ceilings start on straight edges" },
                            "hole_above": { "type": "string", "enum": ["sacrificial", "stepped"], "description": "A hole above a bridge: sacrificial layers or a stepped bridge" },
                            "sacrificial_layers": { "type": "integer", "description": "1 to 10" },
                            "chamfer": { "type": "boolean", "description": "45 degree chamfer of the bed edges" },
                            "chamfer_mm": { "type": "number", "description": "Bed chamfer size in mm" },
                            "brim": { "type": "boolean", "description": "A one-layer brim on small bed islands" },
                            "brim_w": { "type": "number", "description": "Brim width in mm" },
                            "brim_gap": { "type": "number", "description": "Gap between the brim and the rest of the part in mm" },
                            "hole_size": { "type": "boolean", "description": "Hole size: round holes with a vertical axis get the diameter of the model plus hole_add" },
                            "hole_add": { "type": "number", "description": "The hole size fix adds this to the diameter in mm, 0 to 1" },
                            "ears": { "type": "boolean", "description": "Mouse ears: a one-layer disk on each sharp bed corner of a large part" },
                            "ear_d": { "type": "number", "description": "Mouse ear diameter in mm, 5 to 30" },
                            "ear_min": { "type": "number", "description": "A part gets mouse ears when its long side on the bed is this or more, in mm, 20 to 300" },
                        },
                    },
                    "parts": {
                        "type": "object",
                        "description": "Values of each print part, by name: {\"include\": bool, \"orient\": \"model\" | \"auto\" | \"face\", \"down\": [x, y, z] (with face: the model direction that goes down), \"turns\": quarter turns about z, \"split\": {\"joint\": \"flat\" | \"pins\" | \"dovetail\", \"cuts\": [[axis 0 to 2, position mm]], \"clearance\": mm}}. A split_proposal of get_optimize is a valid split.",
                        "additionalProperties": { "type": ["object", "null"] },
                    },
                    "wait": { "type": "boolean", "description": "Wait for the plate. Default: true." },
                },
            })
        ),
        tool(
            "save_photo",
            "Render a path traced photo of the model and save it as PNG. The whole model is in frame, with the part colors, the explode and the section cut of the window. The call stops after 40 seconds and saves the samples it has.",
            false,
            true,
            json!({ "type": "object", "properties": photo, "required": ["path"] })
        ),
        tool(
            "slice",
            "Slice the plate of the Optimize tab and return layers, filament, weight and a rough print time. Given values also change the print settings of the Optimize tab.",
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
                    "mode": { "type": "string", "enum": ["model", "optimize", "render"], "description": "The tab of the right panel. The old names slice and photo also work." },
                    "view": { "type": "string", "enum": ["iso", "top", "front", "right"] },
                    "fit": { "type": "boolean", "description": "Fit the model in the view" },
                    "wireframe": { "type": "boolean" },
                    "grid": { "type": "boolean" },
                    "projection": { "type": "string", "enum": ["iso", "persp"], "description": "iso: parallel projection, persp: perspective. The Render tab and the photo use perspective." },
                    "layers": { "type": "boolean", "description": "The 3D view of the Optimize tab: true shows the Layers view (the slice of the plate), false the Plate view. The Layers card has the same switch." },
                    "layer": { "type": "integer", "description": "Slice layer of the Optimize tab, from 1" },
                    "render": { "type": "boolean", "description": "true starts the path-traced render of the Render tab, as the Render button does. The tab does not start it by itself. false stops it. It needs the Render tab. The render stops when the view or a render setting changes. To save an image, use save_photo." },
                    "part_colors": { "type": "boolean", "description": "Show each part in a different color. The 3D view, the photo and the images use the part colors." },
                    "explode": { "type": "number", "description": "Move the parts apart by this distance in mm. The part farthest from the model center moves this far. 0 turns the explode off. The Optimize tab shows no explode." },
                    "explode_axis": { "type": "string", "enum": ["all", "x", "y", "z"], "description": "Explode direction: all moves each part away from the model center, x, y or z moves the parts along that axis only. Turns the explode on." },
                    "section": {
                        "type": "object",
                        "description": "Section cut: cut the model across x, y or z and keep one side, for example {\"x\": {\"at\": 12.5, \"keep\": \"+\"}}. The object replaces all cuts: a missing or null axis has no cut, and {} turns all cuts off. Two or three cuts together keep only the space that all of them keep. The cut moves with each part in the explode. The cuts and the sides to keep stay in all tabs. In the Optimize tab, the cuts go across the plate: at is a position on the bed (default: the center of the plate), and the plate has its own positions. get_state shows the positions of the current tab. The 3D view, the layers, the photo and the images show the cut. The photo and the images show the cut of the model. The export does not use the cut.",
                        "properties": {
                            "x": { "type": ["object", "null"], "properties": { "at": { "type": "number", "description": "Position on the axis in mm. Default: the model center." }, "keep": { "type": "string", "enum": ["+", "-"], "description": "+ keeps the side above the position, - the side below. Default +." } } },
                            "y": { "type": ["object", "null"], "properties": { "at": { "type": "number", "description": "Position on the axis in mm. Default: the model center." }, "keep": { "type": "string", "enum": ["+", "-"], "description": "+ keeps the side above the position, - the side below. Default +." } } },
                            "z": { "type": ["object", "null"], "properties": { "at": { "type": "number", "description": "Position on the axis in mm. Default: the model center." }, "keep": { "type": "string", "enum": ["+", "-"], "description": "+ keeps the side above the position, - the side below. Default +." } } },
                        },
                    },
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

/// The current name of a tool. `render` is the name of `build` before 0.7.
fn new_name(name: &str) -> &str {
    match name {
        "render" => "build",
        other => other,
    }
}

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
        let name = new_name(name);
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
            "open_model" | "new_model" => {
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
    fn render_is_an_alias_of_build() {
        let names: Vec<String> = (tools().as_array().unwrap().iter())
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect();
        assert!(names.contains(&"build".to_string()));
        assert!(!names.contains(&"render".to_string()));
        assert_eq!(new_name("render"), "build");
        assert_eq!(new_name("build"), "build");
        assert_eq!(new_name("slice"), "slice");
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
