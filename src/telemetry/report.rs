//! Error reports. A report holds fixed values and stack frames of our own code.
//! It has no field for an error message, so the text of a panic cannot reach
//! PostHog. `Client::on_panic` does not read the panic payload.

use serde_json::{Map, Value, json};
use std::collections::HashSet;
use std::hash::{DefaultHasher, Hash, Hasher};

const MAX_FRAMES: usize = 50;
pub const MAX_PER_RUN: usize = 20;
pub const EXTERNAL: &str = "[external]";

/// The fixed values of a capture point. Panics are the only uncaught failure
/// in the window. Rust has no class name, so the type is a fixed word too.
#[derive(Clone, Copy, Debug)]
pub struct Capture {
    pub kind: &'static str,
    pub code: &'static str,
    pub process: &'static str,
}

pub const WINDOW_PANIC: Capture = Capture {
    kind: "panic",
    code: "window:panic",
    process: "window",
};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Frame {
    Own {
        function: String,
        file: Option<String>,
        line: Option<u32>,
        col: Option<u32>,
    },
    External,
}

impl Frame {
    fn json(&self) -> Value {
        match self {
            Frame::External => json!({
                "platform": "custom",
                "lang": "rust",
                "function": EXTERNAL,
                "in_app": false,
                "resolved": true,
            }),
            Frame::Own {
                function,
                file,
                line,
                col,
            } => json!({
                "platform": "custom",
                "lang": "rust",
                "function": function,
                "filename": file,
                "lineno": line,
                "colno": col,
                "in_app": true,
                "resolved": true,
            }),
        }
    }
}

/// The properties of a report, apart from the fields that all events share.
pub fn properties(capture: Capture, frames: &[Frame]) -> Map<String, Value> {
    let mut item = json!({
        "type": capture.kind,
        "value": capture.code,
        "mechanism": { "handled": false, "synthetic": false },
    });
    if !frames.is_empty() {
        let frames: Vec<Value> = frames.iter().map(Frame::json).collect();
        item["stacktrace"] = json!({ "type": "raw", "frames": frames });
    }
    let props = json!({
        "$exception_list": [item],
        "$exception_level": "fatal",
        "capture_code": capture.code,
        "channel": "stable",
        "process": capture.process,
        "$process_person_profile": false,
    });
    match props {
        Value::Object(m) => m,
        _ => Map::new(),
    }
}

/// Stops repeats. This stays on the device.
#[derive(Default)]
pub struct Limit {
    seen: HashSet<u64>,
    count: usize,
}

impl Limit {
    pub fn allow(&mut self, fingerprint: u64) -> bool {
        if self.count >= MAX_PER_RUN || !self.seen.insert(fingerprint) {
            return false;
        }
        self.count += 1;
        true
    }
}

/// A hash of the type, the capture code and the top frame.
pub fn fingerprint(capture: Capture, frames: &[Frame]) -> u64 {
    let mut h = DefaultHasher::new();
    capture.kind.hash(&mut h);
    capture.code.hash(&mut h);
    frames.last().hash(&mut h);
    h.finish()
}

/// Only names of our own code pass. Closures keep the name of their function.
fn safe_function(name: &str) -> Option<&str> {
    let mut name = name;
    while let Some(rest) = name.strip_suffix("::{{closure}}") {
        name = rest;
    }
    let ident = |p: &str| {
        p.chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    };
    let mut parts = name.split("::");
    let own = parts.next() == Some("cocad");
    let rest: Vec<&str> = parts.collect();
    (own && !rest.is_empty() && rest.iter().all(|p| ident(p))).then_some(name)
}

/// A relative file of this repo, or nothing.
fn safe_file(path: &str) -> Option<String> {
    let file = short_path(path);
    let plain = file
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '/' | '.'));
    (plain && file.starts_with("src/") && file.ends_with(".rs") && !file.contains(".."))
        .then_some(file)
}

fn safe_frame(function: &str, file: Option<&str>, line: Option<u32>, col: Option<u32>) -> Frame {
    let Some(function) = safe_function(function) else {
        return Frame::External;
    };
    let file = match file.map(safe_file) {
        Some(None) => return Frame::External,
        Some(file) => file,
        // A release build has names but no file and no line.
        None => None,
    };
    Frame::Own {
        function: function.to_string(),
        file,
        line,
        col,
    }
}

fn location_frame(file: &str, line: u32, col: u32) -> Frame {
    match safe_file(file) {
        Some(file) => Frame::Own {
            function: "panic".to_string(),
            file: Some(file),
            line: Some(line),
            col: Some(col),
        },
        None => Frame::External,
    }
}

#[derive(Default)]
struct Parsed {
    function: String,
    file: Option<String>,
    line: Option<u32>,
    col: Option<u32>,
}

/// Safe frames of a `std::backtrace` text and the panic location, oldest first
/// as PostHog expects. Leaves out the frames of the panic itself. A stripped
/// binary has no names, so only the location frame stays.
pub fn frames(trace: &str, location: Option<(&str, u32, u32)>) -> Vec<Frame> {
    let mut parsed: Vec<Parsed> = Vec::new();
    for line in trace.lines().map(str::trim) {
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
    let mut out: Vec<Frame> = Vec::new();
    for f in parsed
        .iter()
        .skip(start)
        .filter(|f| f.function != "<unknown>")
    {
        push(
            &mut out,
            safe_frame(&f.function, f.file.as_deref(), f.line, f.col),
        );
    }
    out.truncate(MAX_FRAMES);
    out.reverse();
    if let Some((file, line, col)) = location {
        push(&mut out, location_frame(file, line, col));
    }
    out
}

/// Adds a frame. Neighbouring external frames become one.
fn push(frames: &mut Vec<Frame>, frame: Frame) {
    if frame != Frame::External || frames.last() != Some(&Frame::External) {
        frames.push(frame);
    }
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn function_names_must_pass_the_pattern() {
        assert_eq!(
            safe_function("cocad::app::App::open"),
            Some("cocad::app::App::open")
        );
        assert_eq!(
            safe_function("cocad::app::App::open::{{closure}}::{{closure}}"),
            Some("cocad::app::App::open")
        );
        for bad in [
            "main",
            "cocad",
            "egui::ui::Ui::label",
            "<cocad::app::App as eframe::App>::update",
            "cocad::load<alloc::string::String>",
            "cocad::a b",
            "/home/ann/secret",
        ] {
            assert_eq!(safe_function(bad), None, "{bad}");
        }
    }

    #[test]
    fn other_frames_become_one_external_frame() {
        let text = "   0: std::backtrace::Backtrace::force_capture
             at /rustc/abc/library/std/src/backtrace.rs:312:13
   1: core::panicking::panic_fmt
   2: <unknown>
   3: cocad::app::App::open
             at ./src/app.rs:12:5
   4: egui::ui::Ui::label
             at /home/ann/.cargo/registry/src/index.crates.io-1/egui-0.31.1/src/ui.rs:9:1
   5: eframe::run_native
             at /home/ann/.cargo/registry/src/index.crates.io-1/eframe-0.31.1/src/lib.rs:9:1
   6: cocad::main
   7: main
";
        let open = Frame::Own {
            function: "cocad::app::App::open".into(),
            file: Some("src/app.rs".into()),
            line: Some(12),
            col: Some(5),
        };
        let main = Frame::Own {
            function: "cocad::main".into(),
            file: None,
            line: None,
            col: None,
        };
        let want = vec![Frame::External, main, Frame::External, open];
        assert_eq!(frames(text, None), want, "oldest first");
    }

    #[test]
    fn own_name_with_a_foreign_file_is_external() {
        let f = safe_frame(
            "cocad::x",
            Some("/home/ann/secret plan.rs"),
            Some(1),
            Some(1),
        );
        assert_eq!(f, Frame::External);
    }

    #[test]
    fn the_location_frame_follows_the_same_rule() {
        let own = frames("", Some(("src/app.rs", 3, 4)));
        assert!(matches!(&own[0], Frame::Own { function, .. } if function == "panic"));
        let path = "/home/ann/.cargo/registry/src/x-1/egui-0.31.1/src/ui.rs";
        assert_eq!(frames("", Some((path, 3, 4))), vec![Frame::External]);
    }

    #[test]
    fn a_stripped_binary_keeps_only_the_location() {
        let text = "   0: <unknown>\n   1: <unknown>\n";
        assert_eq!(frames(text, Some(("src/a.rs", 1, 2))).len(), 1);
    }

    #[test]
    fn limit_allows_one_per_fingerprint_and_a_maximum_per_run() {
        let mut l = Limit::default();
        assert!(l.allow(1));
        assert!(!l.allow(1));
        for n in 2..=MAX_PER_RUN as u64 {
            assert!(l.allow(n));
        }
        assert!(!l.allow(9_999));
    }

    #[test]
    fn fingerprint_uses_the_top_frame() {
        let a = [Frame::External, location_frame("src/a.rs", 1, 1)];
        let b = [Frame::External, location_frame("src/b.rs", 1, 1)];
        let c = [
            location_frame("src/z.rs", 1, 1),
            location_frame("src/a.rs", 1, 1),
        ];
        assert_ne!(fingerprint(WINDOW_PANIC, &a), fingerprint(WINDOW_PANIC, &b));
        assert_eq!(fingerprint(WINDOW_PANIC, &a), fingerprint(WINDOW_PANIC, &c));
    }
}
