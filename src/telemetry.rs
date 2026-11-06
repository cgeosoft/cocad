//! Optional usage analytics and error reports, sent to PostHog (EU cloud).
//!
//! Each one stays off until the user accepts it. A build with no
//! `POSTHOG_API_KEY` sends nothing and does not ask. Only the window sends
//! data: the CLI and `cocad mcp` never turn this on.
//!
//! The two choices are separate, so nothing links them. Analytics use a random
//! ID that is kept in the settings folder and removed when the user turns them
//! off. Error reports use a new random ID at each start.
//!
//! Do not send model contents, file names, parameter values or OpenSCAD output.
//! Put error text through `scrub` first.

use serde_json::{Map, Value, json};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::time::Duration;

const KEY: Option<&str> = option_env!("POSTHOG_API_KEY");
/// Only for tests against a local server.
const HOST: Option<&str> = option_env!("POSTHOG_HOST");
const EU_HOST: &str = "https://eu.i.posthog.com";
const VERSION: &str = env!("CARGO_PKG_VERSION");
const TIMEOUT: Duration = Duration::from_secs(5);
const MAX_TEXT: usize = 300;
const MAX_FRAMES: usize = 50;
pub const PRIVACY_URL: &str = "https://cocad.cgeosoft.com/privacy/";

static ANALYTICS: AtomicBool = AtomicBool::new(false);
static ERRORS: AtomicBool = AtomicBool::new(false);
static QUEUE: OnceLock<Sender<Value>> = OnceLock::new();

/// True when this build can send data, so the app must ask the user.
pub fn available() -> bool {
    key().is_some()
}

fn key() -> Option<&'static str> {
    KEY.filter(|k| k.starts_with("phc_"))
}

/// Applies the choices of the user. Turning analytics off removes their ID.
pub fn apply(analytics: bool, errors: bool) {
    ANALYTICS.store(analytics && available(), Ordering::Relaxed);
    ERRORS.store(errors && available(), Ordering::Relaxed);
    if !analytics && let Some(f) = id_file() {
        let _ = std::fs::remove_file(f);
    }
}

/// Queues an analytics event. Does nothing without consent.
pub fn event(name: &str, props: Value) {
    if !ANALYTICS.load(Ordering::Relaxed) {
        return;
    }
    let Some(id) = analytics_id() else { return };
    queue(payload(name, &id, props));
}

/// Queues a handled error. `kind` is a fixed name, such as `export_failed`.
pub fn error(kind: &str, text: &str) {
    if ERRORS.load(Ordering::Relaxed) {
        let props = exception(kind, &scrub(text), true, Vec::new());
        queue(payload("$exception", run_id(), props));
    }
}

/// Reports panics, then runs the default hook. A panic can end the process, so
/// this sends at once and does not use the queue.
pub fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if ERRORS.load(Ordering::Relaxed) {
            let payload_any = info.payload();
            let text = payload_any
                .downcast_ref::<&str>()
                .copied()
                .or_else(|| payload_any.downcast_ref::<String>().map(String::as_str))
                .unwrap_or("panic");
            let mut frames = Vec::new();
            if let Some(l) = info.location() {
                frames.push(location_frame(l.file(), l.line(), l.column()));
            }
            let trace = std::backtrace::Backtrace::force_capture().to_string();
            frames.splice(0..0, parse_backtrace(&trace));
            let props = exception("panic", &scrub(text), false, frames);
            post(&payload("$exception", run_id(), props));
        }
        default(info);
    }));
}

fn queue(body: Value) {
    let tx = QUEUE.get_or_init(|| {
        let (tx, rx) = channel::<Value>();
        std::thread::spawn(move || {
            for body in rx {
                post(&body);
            }
        });
        tx
    });
    let _ = tx.send(body);
}

/// Sends one event. It does not retry: a lost event is not important.
fn post(body: &Value) {
    let url = format!("{}/i/v0/e/", HOST.unwrap_or(EU_HOST));
    let config = ureq::Agent::config_builder().timeout_global(Some(TIMEOUT));
    // Linux uses rustls, so the app needs no OpenSSL. `cargo check` for Windows
    // and macOS must work on Linux, and ring needs a C toolchain for the target.
    #[cfg(not(target_os = "linux"))]
    let config = config.tls_config(
        ureq::tls::TlsConfig::builder()
            .provider(ureq::tls::TlsProvider::NativeTls)
            .build(),
    );
    let agent: ureq::Agent = config.build().into();
    let _ = agent
        .post(&url)
        .content_type("application/json")
        .send(body.to_string());
}

fn payload(event: &str, distinct_id: &str, props: Value) -> Value {
    let mut properties = Map::new();
    if let Value::Object(p) = props {
        properties = p;
    }
    let common = json!({
        "$lib": "cocad",
        "$lib_version": VERSION,
        "$app_version": VERSION,
        "$os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "source": "desktop",
        // PostHog stores the address of the connection unless the event has its
        // own value. An empty value or null does not stop this.
        "$ip": "0.0.0.0",
        "$geoip_disable": true,
        "$process_person_profile": false,
    });
    if let Value::Object(c) = common {
        properties.extend(c);
    }
    json!({
        "api_key": key().unwrap_or_default(),
        "event": event,
        "distinct_id": distinct_id,
        "properties": properties,
    })
}

fn exception(kind: &str, value: &str, handled: bool, frames: Vec<Value>) -> Value {
    let mut item = json!({
        "type": kind,
        "value": value,
        "mechanism": { "handled": handled, "synthetic": false },
    });
    if !frames.is_empty() {
        item["stacktrace"] = json!({ "type": "raw", "frames": frames });
    }
    json!({
        "$exception_list": [item],
        "$exception_level": if handled { "error" } else { "fatal" },
    })
}

fn frame(function: &str, file: Option<&str>, line: Option<u32>, col: Option<u32>) -> Value {
    let file = file.map(short_path);
    let in_app = file.as_deref().is_some_and(|f| f.starts_with("src/"))
        || function.starts_with("cocad::")
        || function.starts_with("<cocad::");
    json!({
        "platform": "custom",
        "lang": "rust",
        "function": function,
        "filename": file,
        "lineno": line,
        "colno": col,
        "in_app": in_app,
        "resolved": true,
    })
}

fn location_frame(file: &str, line: u32, col: u32) -> Value {
    frame("panic", Some(file), Some(line), Some(col))
}

#[derive(Default)]
struct Parsed {
    function: String,
    file: Option<String>,
    line: Option<u32>,
    col: Option<u32>,
}

/// Frames of a `std::backtrace` text, oldest first as PostHog expects. Leaves
/// out the frames of the panic itself. A stripped binary has no names here, so
/// the result can be empty.
fn parse_backtrace(text: &str) -> Vec<Value> {
    let mut parsed: Vec<Parsed> = Vec::new();
    for line in text.lines().map(str::trim) {
        if let Some(place) = line.strip_prefix("at ") {
            let mut parts = place.rsplitn(3, ':');
            let col = parts.next().and_then(|n| n.parse().ok());
            let row = parts.next().and_then(|n| n.parse().ok());
            if let (Some(last), Some(file)) = (parsed.last_mut(), parts.next()) {
                last.file = Some(file.to_string());
                (last.line, last.col) = (row, col);
            }
        } else if let Some((n, function)) = line.split_once(": ")
            && n.chars().all(|c| c.is_ascii_digit())
        {
            parsed.push(Parsed {
                function: function.to_string(),
                ..Default::default()
            });
        }
    }
    let start = parsed
        .iter()
        .rposition(|f| {
            f.function.contains("core::panicking::") || f.function.contains("rust_begin_unwind")
        })
        .map_or(0, |i| i + 1);
    let mut frames: Vec<Value> = parsed
        .iter()
        .skip(start)
        .filter(|f| f.function != "<unknown>")
        .take(MAX_FRAMES)
        .map(|f| frame(&f.function, f.file.as_deref(), f.line, f.col))
        .collect();
    frames.reverse();
    frames
}

/// A source path with no folder of the build computer: `src/app.rs`,
/// `egui-0.31.1/src/ui.rs` or `library/core/src/option.rs`.
fn short_path(path: &str) -> String {
    let path = path.replace('\\', "/");
    for marker in ["/registry/src/", "/rustc/"] {
        if let Some((_, rest)) = path.split_once(marker) {
            return rest.split_once('/').map_or(rest, |r| r.1).to_string();
        }
    }
    if let Some((_, rest)) = path.split_once("/rustlib/src/rust/") {
        return rest.to_string();
    }
    if let Some(rest) = path.strip_prefix("./") {
        return rest.to_string();
    }
    if !path.starts_with('/') && !path.contains(':') {
        return path;
    }
    match path.rfind("/src/") {
        Some(i) => path[i + 1..].to_string(),
        None => path.rsplit('/').next().unwrap_or_default().to_string(),
    }
}

/// The first line of `text`, short and with file paths removed. A path runs
/// from the first word with a slash up to the next ": ", because a name can
/// hold spaces.
fn scrub(text: &str) -> String {
    let mut rest = text.lines().next().unwrap_or_default();
    let mut out = String::new();
    while let Some(start) = rest
        .split_whitespace()
        .find(|w| w.contains(['/', '\\']))
        .and_then(|w| rest.find(w))
    {
        out.push_str(&rest[..start]);
        out.push_str("<path>");
        rest = rest[start..]
            .find(": ")
            .map_or("", |end| &rest[start + end..]);
    }
    out.push_str(rest);
    out.chars().take(MAX_TEXT).collect()
}

fn id_file() -> Option<PathBuf> {
    Some(crate::openscad::config_dir()?.join("analytics_id"))
}

fn random_id() -> Option<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).ok()?;
    Some(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn analytics_id() -> Option<String> {
    let file = id_file()?;
    if let Ok(id) = std::fs::read_to_string(&file)
        && !id.trim().is_empty()
    {
        return Some(id.trim().to_string());
    }
    let id = random_id()?;
    std::fs::create_dir_all(file.parent()?).ok()?;
    std::fs::write(&file, &id).ok()?;
    Some(id)
}

fn run_id() -> &'static str {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| random_id().unwrap_or_else(|| "unknown".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrub_removes_paths() {
        assert_eq!(
            scrub("cannot write /home/ann/my box.stl: Permission denied"),
            "cannot write <path>: Permission denied"
        );
        assert_eq!(
            scrub(r"open C:\Users\ann\a b.scad: denied"),
            "open <path>: denied"
        );
        assert_eq!(scrub("failed at /tmp/a.scad"), "failed at <path>");
        assert_eq!(scrub("index out of range"), "index out of range");
        assert_eq!(scrub("one\n/home/ann/two"), "one");
        assert_eq!(scrub("a /x: b /y: c"), "a <path>: b <path>: c");
    }

    #[test]
    fn scrub_limits_length() {
        assert_eq!(scrub(&"é".repeat(MAX_TEXT + 9)).chars().count(), MAX_TEXT);
    }

    #[test]
    fn paths_lose_build_folders() {
        assert_eq!(short_path("src/app.rs"), "src/app.rs");
        assert_eq!(short_path("./src/app.rs"), "src/app.rs");
        assert_eq!(short_path("/home/ann/cocad/src/app.rs"), "src/app.rs");
        assert_eq!(short_path(r"C:\Users\ann\cocad\src\app.rs"), "src/app.rs");
        assert_eq!(
            short_path("/home/ann/.cargo/registry/src/index.crates.io-1/egui-0.31.1/src/ui.rs"),
            "egui-0.31.1/src/ui.rs"
        );
        assert_eq!(
            short_path("/rustc/abc123/library/core/src/option.rs"),
            "library/core/src/option.rs"
        );
        assert_eq!(
            short_path("/home/ann/.rustup/lib/rustlib/src/rust/library/core/src/option.rs"),
            "library/core/src/option.rs"
        );
        assert_eq!(short_path("/home/ann/other.rs"), "other.rs");
    }

    #[test]
    fn backtrace_keeps_frames_after_the_panic_oldest_first() {
        let text = "   0: std::backtrace::Backtrace::force_capture
             at /rustc/abc/library/std/src/backtrace.rs:312:13
   1: core::panicking::panic_fmt
   2: <unknown>
   3: cocad::app::App::open
             at ./src/app.rs:12:5
   4: main
";
        let frames = parse_backtrace(text);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0]["function"], "main");
        assert_eq!(frames[1]["function"], "cocad::app::App::open");
        assert_eq!(frames[1]["filename"], "src/app.rs");
        assert_eq!(frames[1]["lineno"], 12);
        assert_eq!(frames[1]["in_app"], true);
    }

    #[test]
    fn events_have_no_person_profile_and_no_location() {
        let p = payload("x", "id", json!({ "format": "stl" }));
        assert_eq!(p["properties"]["format"], "stl");
        assert_eq!(p["properties"]["$process_person_profile"], false);
        assert_eq!(p["properties"]["$geoip_disable"], true);
        assert_eq!(p["properties"]["$ip"], "0.0.0.0");
    }

    #[test]
    fn nothing_is_queued_without_consent() {
        event("x", json!({}));
        error("x", "y");
        assert!(QUEUE.get().is_none());
    }

    #[test]
    fn handled_error_has_no_stacktrace() {
        let p = exception("export_failed", "text", true, Vec::new());
        assert_eq!(p["$exception_level"], "error");
        assert!(p["$exception_list"][0].get("stacktrace").is_none());
    }
}
