//! Optional usage analytics and error reports, sent to PostHog (EU cloud).
//!
//! The plan for this code is in `docs/telemetry-consent.md`. Do not add a field
//! here without a change to that plan, the consent text in `src/app.rs`,
//! `README.md` and the Privacy Notice. Raise `NOTICE_VERSION` when the data, the
//! purpose or the recipient changes.
//!
//! - Each option stays off until the user turns it on for the current notice.
//! - A debug build or a build with no `POSTHOG_API_KEY` sends nothing and does
//!   not ask. Only the window sends data: the CLI never turns this on.
//! - Analytics use a random ID in the settings folder. It stays there until the
//!   user resets it, because a deletion request needs it.
//! - An error report has a new random ID and no error text. See `report`.
//! - There is no vendor SDK, so no vendor autocapture. `filter` is the final
//!   allowlist before each send.

mod consent;
mod report;

pub use consent::{Loaded, NOTICE_VERSION, Source};

use report::{Capture, Limit, WINDOW_PANIC};
use serde_json::{Map, Value, json};
use std::panic::PanicHookInfo;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, LazyLock, Mutex, OnceLock};
use std::time::{Duration, SystemTime};

const KEY: Option<&str> = option_env!("POSTHOG_API_KEY");
/// Only for tests against a local server.
const HOST: Option<&str> = option_env!("POSTHOG_HOST");
const EU_HOST: &str = "https://eu.i.posthog.com";
const VERSION: &str = env!("CARGO_PKG_VERSION");
const TIMEOUT: Duration = Duration::from_secs(5);
/// A panic waits this long for its report. The process can end after that.
const HOOK_WAIT: Duration = Duration::from_secs(2);
/// All sends run on threads with this name. A panic there is not reported.
const THREAD: &str = "telemetry";
pub const PRIVACY_URL: &str = "https://cocad.cgeosoft.com/privacy/";

static CLIENT: LazyLock<Client> = LazyLock::new(|| {
    Client::new(
        usable_key(KEY, cfg!(debug_assertions)).map(str::to_string),
        crate::openscad::config_dir(),
        Arc::new(Http),
    )
});

/// A view of the window, for the events.
#[derive(Clone, Copy, Debug)]
pub enum Screen {
    Model,
    Slice,
    Photo,
}

impl Screen {
    fn as_str(self) -> &'static str {
        match self {
            Screen::Model => "model",
            Screen::Slice => "slice",
            Screen::Photo => "photo",
        }
    }
}

/// All analytics events. A property is a count, a boolean or a fixed value.
#[derive(Clone, Copy, Debug)]
pub enum Event {
    AppStarted { view: Screen, first_run: bool },
    ModelOpened { parameters: usize },
    ModelExported { format: crate::export::Format },
    ViewChanged { view: Screen },
    PhotoSaved { samples: u32 },
}

impl Event {
    fn name(&self) -> &'static str {
        match self {
            Event::AppStarted { .. } => "app_started",
            Event::ModelOpened { .. } => "model_opened",
            Event::ModelExported { .. } => "model_exported",
            Event::ViewChanged { .. } => "view_changed",
            Event::PhotoSaved { .. } => "photo_saved",
        }
    }

    fn props(&self) -> Value {
        match *self {
            Event::AppStarted { view, first_run } => {
                json!({ "view": view.as_str(), "first_run": first_run })
            }
            Event::ModelOpened { parameters } => json!({ "parameters": parameters }),
            Event::ModelExported { format } => json!({ "format": format.ext() }),
            Event::ViewChanged { view } => json!({ "view": view.as_str() }),
            Event::PhotoSaved { samples } => json!({ "samples": samples }),
        }
    }
}

/// True when this build can send data, so the app must ask the user.
pub fn available() -> bool {
    CLIENT.key.is_some()
}

/// Reads the consent record and applies it.
pub fn load_consent() -> Loaded {
    CLIENT.load()
}

/// The only way to change consent. It records and applies the choice.
pub fn set_consent(analytics: bool, errors: bool, source: Source) -> Result<(), String> {
    CLIENT.set_consent(analytics, errors, source)
}

/// Picks up a choice that another window made.
pub fn reload_consent() {
    CLIENT.reload();
}

/// Queues an analytics event. Does nothing without consent.
pub fn event(event: Event) {
    CLIENT.event(event);
}

/// The install ID, for Settings. This does not make one.
pub fn install_id() -> Option<String> {
    CLIENT.install_id()
}

/// Removes the install ID. The next event makes a new one.
pub fn reset_install_id() {
    CLIENT.reset_install_id();
}

/// Reports panics, then runs the default hook.
pub fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        CLIENT.on_panic(info);
        default(info);
    }));
}

fn usable_key(key: Option<&'static str>, debug: bool) -> Option<&'static str> {
    key.filter(|k| !debug && k.starts_with("phc_"))
}

/// The way out of the process. Tests use a fake one, so they reach no vendor.
trait Sink: Send + Sync {
    fn send(&self, body: &Value);
}

struct Http;

impl Sink for Http {
    /// Sends one event. It does not retry: a lost event is not important.
    fn send(&self, body: &Value) {
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
}

struct Client {
    key: Option<String>,
    dir: Option<PathBuf>,
    sink: Arc<dyn Sink>,
    analytics: AtomicBool,
    errors: AtomicBool,
    /// The time of the consent file when this process last read it.
    stamp: Mutex<Option<SystemTime>>,
    queue: OnceLock<Sender<Value>>,
    limit: Mutex<Limit>,
}

impl Client {
    fn new(key: Option<String>, dir: Option<PathBuf>, sink: Arc<dyn Sink>) -> Client {
        Client {
            key,
            dir,
            sink,
            analytics: AtomicBool::new(false),
            errors: AtomicBool::new(false),
            stamp: Mutex::new(None),
            queue: OnceLock::new(),
            limit: Mutex::new(Limit::default()),
        }
    }

    fn switch(&self, analytics: bool, errors: bool) {
        let usable = self.key.is_some();
        self.analytics.store(analytics && usable, Ordering::Relaxed);
        self.errors.store(errors && usable, Ordering::Relaxed);
    }

    fn set_stamp(&self, stamp: Option<SystemTime>) {
        *self.stamp.lock().unwrap_or_else(|e| e.into_inner()) = stamp;
    }

    fn load(&self) -> Loaded {
        let Some(dir) = &self.dir else {
            self.switch(false, false);
            return Loaded::default();
        };
        self.set_stamp(consent::mtime(dir));
        let loaded = consent::load(dir);
        self.switch(
            loaded.record.allows_analytics(),
            loaded.record.allows_errors(),
        );
        loaded
    }

    fn reload(&self) {
        let Some(dir) = &self.dir else { return };
        let now = consent::mtime(dir);
        if *self.stamp.lock().unwrap_or_else(|e| e.into_inner()) != now {
            self.load();
        }
    }

    /// Reads the file first, so that this keeps a change from another window.
    /// If the write fails, an "off" still applies, and an "on" does not: the app
    /// cannot show that consent without a record.
    fn set_consent(&self, analytics: bool, errors: bool, source: Source) -> Result<(), String> {
        let Some(dir) = &self.dir else {
            self.switch(false, false);
            return Err("no config folder".into());
        };
        let before = consent::load(dir).record;
        let mut record = before.clone();
        record.choose(analytics, errors, source, &consent::now());
        match consent::write(dir, &record) {
            Ok(()) => {
                self.set_stamp(consent::mtime(dir));
                self.switch(record.allows_analytics(), record.allows_errors());
                Ok(())
            }
            Err(e) => {
                self.switch(
                    before.allows_analytics() && analytics,
                    before.allows_errors() && errors,
                );
                if !analytics || !errors {
                    consent::remove(dir);
                }
                Err(e.to_string())
            }
        }
    }

    fn event(&self, event: Event) {
        if !self.analytics.load(Ordering::Relaxed) {
            return;
        }
        let Some(id) = self.install_id().or_else(|| self.new_install_id()) else {
            return;
        };
        let mut props = common();
        props.insert("source".into(), "desktop".into());
        if let Value::Object(own) = event.props() {
            props.extend(own);
        }
        self.queue(self.body(event.name(), &id, props));
    }

    /// Reads only the place of each frame. The panic message stays here.
    fn on_panic(&self, info: &PanicHookInfo<'_>) {
        if !self.errors.load(Ordering::Relaxed) || std::thread::current().name() == Some(THREAD) {
            return;
        }
        let place = info.location().map(|l| (l.file(), l.line(), l.column()));
        let trace = std::backtrace::Backtrace::force_capture().to_string();
        self.report(WINDOW_PANIC, &report::frames(&trace, place));
    }

    fn report(&self, capture: Capture, frames: &[report::Frame]) {
        if !self.errors.load(Ordering::Relaxed) {
            return;
        }
        let Ok(mut limit) = self.limit.try_lock() else {
            return;
        };
        if !limit.allow(report::fingerprint(capture, frames)) {
            return;
        }
        drop(limit);
        let Some(id) = random_id() else { return };
        let mut props = common();
        props.extend(report::properties(capture, frames));
        let Some(body) = filter(self.body("$exception", &id, props)) else {
            return;
        };
        // On its own thread: a panic inside a panic hook ends the process.
        let (done, wait) = channel();
        let sink = self.sink.clone();
        let thread = std::thread::Builder::new().name(THREAD.into());
        let spawned = thread.spawn(move || {
            sink.send(&body);
            let _ = done.send(());
        });
        if spawned.is_ok() {
            let _ = wait.recv_timeout(HOOK_WAIT);
        }
    }

    fn body(&self, event: &str, distinct_id: &str, properties: Map<String, Value>) -> Value {
        json!({
            "api_key": self.key.as_deref().unwrap_or_default(),
            "event": event,
            "distinct_id": distinct_id,
            "properties": properties,
        })
    }

    fn queue(&self, body: Value) {
        let Some(body) = filter(body) else { return };
        let tx = self.queue.get_or_init(|| {
            let (tx, rx) = channel::<Value>();
            let sink = self.sink.clone();
            let thread = std::thread::Builder::new().name(THREAD.into());
            let _ = thread.spawn(move || {
                for body in rx {
                    sink.send(&body);
                }
            });
            tx
        });
        let _ = tx.send(body);
    }

    fn id_file(&self) -> Option<PathBuf> {
        Some(self.dir.as_ref()?.join("analytics_id"))
    }

    fn install_id(&self) -> Option<String> {
        let id = std::fs::read_to_string(self.id_file()?).ok()?;
        let id = id.trim();
        (!id.is_empty()).then(|| id.to_string())
    }

    fn new_install_id(&self) -> Option<String> {
        let file = self.id_file()?;
        let id = random_id()?;
        std::fs::create_dir_all(file.parent()?).ok()?;
        std::fs::write(&file, &id).ok()?;
        Some(id)
    }

    fn reset_install_id(&self) {
        if let Some(f) = self.id_file() {
            let _ = std::fs::remove_file(f);
        }
    }
}

/// The fields of all events and reports.
fn common() -> Map<String, Value> {
    let props = json!({
        "$lib": "cocad",
        "$lib_version": VERSION,
        "$app_version": VERSION,
        "$os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "consent_version": NOTICE_VERSION,
        // PostHog also has a project setting that discards the address.
        "$ip": "0.0.0.0",
        // That setting does not remove location data, so this stays on.
        "$geoip_disable": true,
    });
    match props {
        Value::Object(m) => m,
        _ => Map::new(),
    }
}

const ROOT_KEYS: [&str; 4] = ["api_key", "event", "distinct_id", "properties"];
const COMMON_KEYS: [&str; 8] = [
    "$lib",
    "$lib_version",
    "$app_version",
    "$os",
    "arch",
    "consent_version",
    "$ip",
    "$geoip_disable",
];
const REPORT_KEYS: [&str; 6] = [
    "$exception_list",
    "$exception_level",
    "capture_code",
    "channel",
    "process",
    "$process_person_profile",
];
const ITEM_KEYS: [&str; 4] = ["type", "value", "mechanism", "stacktrace"];
const MECHANISM_KEYS: [&str; 2] = ["handled", "synthetic"];
const STACK_KEYS: [&str; 2] = ["type", "frames"];
const FRAME_KEYS: [&str; 8] = [
    "platform", "lang", "function", "filename", "lineno", "colno", "in_app", "resolved",
];

/// The own keys of an event, or `None` for a name that is not on the list.
fn event_keys(name: &str) -> Option<&'static [&'static str]> {
    match name {
        "app_started" => Some(&["source", "view", "first_run"]),
        "model_opened" => Some(&["source", "parameters"]),
        "model_exported" => Some(&["source", "format"]),
        "view_changed" => Some(&["source", "view"]),
        "photo_saved" => Some(&["source", "samples"]),
        "$exception" => Some(&REPORT_KEYS),
        _ => None,
    }
}

fn keep(map: &mut Map<String, Value>, keys: &[&str]) {
    map.retain(|k, _| keys.contains(&k.as_str()));
}

/// The last step before a send. Everything that is not on the lists goes.
fn filter(mut body: Value) -> Option<Value> {
    let keys = event_keys(body["event"].as_str()?)?;
    let root = body.as_object_mut()?;
    keep(root, &ROOT_KEYS);
    let props = root.get_mut("properties")?.as_object_mut()?;
    props.retain(|k, _| COMMON_KEYS.contains(&k.as_str()) || keys.contains(&k.as_str()));
    let Some(Value::Array(items)) = props.get_mut("$exception_list") else {
        return Some(body);
    };
    for item in items.iter_mut().filter_map(Value::as_object_mut) {
        keep(item, &ITEM_KEYS);
        if let Some(m) = item.get_mut("mechanism").and_then(Value::as_object_mut) {
            keep(m, &MECHANISM_KEYS);
        }
        let Some(stack) = item.get_mut("stacktrace").and_then(Value::as_object_mut) else {
            continue;
        };
        keep(stack, &STACK_KEYS);
        if let Some(Value::Array(frames)) = stack.get_mut("frames") {
            for f in frames.iter_mut().filter_map(Value::as_object_mut) {
                keep(f, &FRAME_KEYS);
            }
        }
    }
    Some(body)
}

fn random_id() -> Option<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).ok()?;
    Some(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::export::Format;
    use std::collections::BTreeSet;
    use std::path::Path;
    use std::sync::atomic::AtomicU32;

    /// A folder that goes away after the test.
    pub struct TempDir(pub PathBuf);

    impl TempDir {
        pub fn new() -> TempDir {
            static N: AtomicU32 = AtomicU32::new(0);
            let n = N.fetch_add(1, Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("cocad-test-{}-{n}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[derive(Default)]
    struct Capturing {
        sent: Mutex<Vec<Value>>,
    }

    impl Capturing {
        /// Waits for the queue thread.
        fn wait(&self, count: usize) -> Vec<Value> {
            for _ in 0..200 {
                let sent = self.sent.lock().unwrap();
                if sent.len() >= count {
                    return sent.clone();
                }
                drop(sent);
                std::thread::sleep(Duration::from_millis(5));
            }
            self.sent.lock().unwrap().clone()
        }

        fn count(&self) -> usize {
            self.sent.lock().unwrap().len()
        }
    }

    impl Sink for Capturing {
        fn send(&self, body: &Value) {
            self.sent.lock().unwrap().push(body.clone());
        }
    }

    fn client(dir: &Path) -> (Client, Arc<Capturing>) {
        let sink = Arc::new(Capturing::default());
        let key = Some("phc_test".to_string());
        (Client::new(key, Some(dir.into()), sink.clone()), sink)
    }

    const STARTED: Event = Event::AppStarted {
        view: Screen::Model,
        first_run: false,
    };

    fn own(frames: usize) -> Vec<report::Frame> {
        (0..frames)
            .map(|n| report::Frame::Own {
                function: format!("cocad::f{n}"),
                file: None,
                line: None,
                col: None,
            })
            .collect()
    }

    fn keys(v: &Value) -> BTreeSet<String> {
        v.as_object().unwrap().keys().cloned().collect()
    }

    fn set(keys: &[&str]) -> BTreeSet<String> {
        keys.iter().map(|k| k.to_string()).collect()
    }

    /// Runs `f` on a named thread, with a hook that calls `on_panic` as the real
    /// hook does. Other threads do not use this hook.
    fn panic_with_hook(client: Arc<Client>, f: impl FnOnce() + Send + 'static) {
        static LOCK: Mutex<()> = Mutex::new(());
        let _one_at_a_time = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let before = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let name = std::thread::current().name().map(str::to_string);
            if matches!(name.as_deref(), Some("canary") | Some(THREAD)) {
                client.on_panic(info);
            }
        }));
        let t = std::thread::Builder::new().name("canary".into());
        let _ = t.spawn(f).unwrap().join();
        std::panic::set_hook(before);
    }

    #[test]
    fn both_off_sends_nothing_also_after_a_panic() {
        let dir = TempDir::new();
        let (c, sink) = client(&dir.0);
        c.set_consent(false, false, Source::Setup).unwrap();
        let c = Arc::new(c);
        c.event(STARTED);
        panic_with_hook(c.clone(), || panic!("boom"));
        assert!(c.queue.get().is_none());
        assert_eq!(sink.count(), 0);
        assert!(c.install_id().is_none());
    }

    #[test]
    fn nothing_is_sent_before_the_first_answer() {
        let dir = TempDir::new();
        let (c, sink) = client(&dir.0);
        let loaded = c.load();
        assert_eq!(loaded.prompt, Some(Source::Setup));
        c.event(STARTED);
        c.report(WINDOW_PANIC, &own(1));
        assert_eq!(sink.count(), 0);
    }

    #[test]
    fn each_option_works_alone() {
        let dir = TempDir::new();
        let (c, sink) = client(&dir.0);
        c.set_consent(true, false, Source::Setup).unwrap();
        c.event(STARTED);
        c.report(WINDOW_PANIC, &own(1));
        let sent = sink.wait(1);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0]["event"], "app_started");

        let dir = TempDir::new();
        let (c, sink) = client(&dir.0);
        c.set_consent(false, true, Source::Setup).unwrap();
        c.event(STARTED);
        c.report(WINDOW_PANIC, &own(1));
        let sent = sink.wait(1);
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0]["event"], "$exception");
    }

    #[test]
    fn a_toggle_applies_with_no_restart() {
        let dir = TempDir::new();
        let (c, sink) = client(&dir.0);
        c.set_consent(true, false, Source::Setup).unwrap();
        c.event(STARTED);
        assert_eq!(sink.wait(1).len(), 1);
        c.set_consent(false, false, Source::Settings).unwrap();
        c.event(STARTED);
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(sink.count(), 1);
    }

    #[test]
    fn a_second_window_sees_the_change_and_keeps_it() {
        let dir = TempDir::new();
        let (a, _) = client(&dir.0);
        let (b, sink) = client(&dir.0);
        a.set_consent(true, true, Source::Setup).unwrap();
        b.load();
        assert!(b.analytics.load(Ordering::Relaxed));
        std::thread::sleep(Duration::from_millis(20));
        a.set_consent(false, true, Source::Settings).unwrap();
        b.reload();
        assert!(!b.analytics.load(Ordering::Relaxed));
        b.event(STARTED);
        assert_eq!(sink.count(), 0);
    }

    #[test]
    fn config_save_cannot_write_consent() {
        let text = crate::openscad::Config::default().text();
        for key in ["analytics", "error_reports", "privacy_asked"] {
            assert!(!text.contains(key), "{key}");
        }
    }

    #[test]
    fn a_failed_write_applies_off_and_never_on() {
        let dir = TempDir::new();
        let (c, _) = client(&dir.0);
        c.switch(true, true);
        // A folder with the name of the file makes the rename fail.
        std::fs::create_dir(dir.0.join("consent")).unwrap();
        assert!(c.set_consent(false, false, Source::Settings).is_err());
        assert!(!c.analytics.load(Ordering::Relaxed));
        assert!(!c.errors.load(Ordering::Relaxed));
        assert!(c.set_consent(true, true, Source::Settings).is_err());
        assert!(!c.analytics.load(Ordering::Relaxed));
        assert!(!c.errors.load(Ordering::Relaxed));
    }

    #[test]
    fn an_old_on_does_not_send_until_the_prompt_is_answered() {
        let dir = TempDir::new();
        let old = "analytics = true\nerror_reports = true\nprivacy_asked = true\n";
        std::fs::write(dir.0.join("config"), old).unwrap();
        let (c, sink) = client(&dir.0);
        let loaded = c.load();
        assert_eq!(loaded.prompt, Some(Source::UpdatePrompt));
        c.event(STARTED);
        c.report(WINDOW_PANIC, &own(1));
        assert_eq!(sink.count(), 0);
        c.set_consent(true, false, Source::UpdatePrompt).unwrap();
        let r = consent::load(&dir.0).record;
        assert_eq!(r.notice_version, NOTICE_VERSION);
        assert_eq!(r.analytics.source, Source::UpdatePrompt);
        assert!(r.allows_analytics() && !r.allows_errors());
    }

    #[test]
    fn no_mcp_path_reaches_consent() {
        for source in [include_str!("mcp.rs"), include_str!("app/remote.rs")] {
            assert!(!source.contains("set_consent"));
            assert!(!source.contains("reset_install_id"));
        }
        let tools = crate::mcp::tools().to_string().to_lowercase();
        for word in ["consent", "analytics", "telemetry", "privacy"] {
            assert!(!tools.contains(word), "{word}");
        }
    }

    #[test]
    fn the_keys_of_each_event_match_the_list() {
        let dir = TempDir::new();
        let (c, sink) = client(&dir.0);
        c.set_consent(true, false, Source::Setup).unwrap();
        let exported = Event::ModelExported {
            format: Format::ThreeMf,
        };
        let changed = Event::ViewChanged {
            view: Screen::Slice,
        };
        let events = [
            (STARTED, vec!["source", "view", "first_run"]),
            (
                Event::ModelOpened { parameters: 3 },
                vec!["source", "parameters"],
            ),
            (exported, vec!["source", "format"]),
            (changed, vec!["source", "view"]),
            (
                Event::PhotoSaved { samples: 256 },
                vec!["source", "samples"],
            ),
        ];
        for (e, _) in &events {
            c.event(*e);
        }
        let sent = sink.wait(events.len());
        assert_eq!(sent.len(), events.len());
        for (body, (event, own)) in sent.iter().zip(&events) {
            assert_eq!(keys(body), set(&ROOT_KEYS));
            assert_eq!(body["event"], event.name());
            let mut want = set(&COMMON_KEYS);
            want.extend(own.iter().map(|k| k.to_string()));
            assert_eq!(keys(&body["properties"]), want);
            assert_eq!(body["properties"]["consent_version"], NOTICE_VERSION);
            assert_eq!(body["properties"]["$geoip_disable"], true);
            // Analytics have a person profile: the deletion runbook needs one.
            assert!(body["properties"].get("$process_person_profile").is_none());
        }
        assert_eq!(sent[2]["properties"]["format"], "3mf");
    }

    #[test]
    fn the_keys_of_a_report_match_the_list() {
        let dir = TempDir::new();
        let (c, sink) = client(&dir.0);
        c.set_consent(false, true, Source::Setup).unwrap();
        c.report(WINDOW_PANIC, &[report::Frame::External, own(1)[0].clone()]);
        let body = &sink.wait(1)[0];
        assert_eq!(keys(body), set(&ROOT_KEYS));
        let p = &body["properties"];
        let mut want = set(&COMMON_KEYS);
        want.extend(REPORT_KEYS.iter().map(|k| k.to_string()));
        assert_eq!(keys(p), want);
        assert_eq!(p["capture_code"], "window:panic");
        assert_eq!(p["channel"], "stable");
        assert_eq!(p["process"], "window");
        assert_eq!(p["consent_version"], NOTICE_VERSION);
        assert_eq!(p["$process_person_profile"], false);
        let item = &p["$exception_list"][0];
        assert_eq!(keys(item), set(&ITEM_KEYS));
        assert_eq!(item["type"], "panic");
        assert_eq!(item["value"], "window:panic");
        let frames = &item["stacktrace"]["frames"];
        assert_eq!(frames[0]["function"], report::EXTERNAL);
    }

    #[test]
    fn each_report_has_a_new_id_that_is_not_the_install_id() {
        let dir = TempDir::new();
        let (c, sink) = client(&dir.0);
        c.set_consent(true, true, Source::Setup).unwrap();
        c.event(STARTED);
        let install = sink.wait(1)[0]["distinct_id"].clone();
        c.report(WINDOW_PANIC, &own(1));
        c.report(WINDOW_PANIC, &own(2));
        let sent = sink.wait(3);
        let ids: Vec<&Value> = sent.iter().skip(1).map(|b| &b["distinct_id"]).collect();
        assert_eq!(ids.len(), 2);
        assert_ne!(ids[0], ids[1]);
        assert!(ids.iter().all(|id| **id != install));
    }

    #[test]
    fn canary_text_never_reaches_the_sink() {
        const CANARY: [&str; 7] = [
            "sk-secret-canary",
            "/home/ann",
            "https://canary.example",
            "ann@example.com",
            "secret plan.stl",
            "Project Falcon",
            "module canary_model",
        ];
        let dir = TempDir::new();
        let (c, sink) = client(&dir.0);
        c.set_consent(false, true, Source::Setup).unwrap();
        panic_with_hook(Arc::new(c), || {
            panic!(
                "sk-secret-canary /home/ann/secret plan.stl https://canary.example \
                 ann@example.com Project Falcon module canary_model"
            )
        });
        let sent = sink.wait(1);
        assert_eq!(sent.len(), 1, "the panic was reported");
        let text = sent[0].to_string();
        for word in CANARY {
            assert!(!text.contains(word), "{word} in {text}");
        }

        // The same words in a stack.
        let trace = "   0: core::panicking::panic_fmt
   1: cocad::app::load
             at /home/ann/secret plan.stl:1:1
   2: ann@example.com::Project Falcon
             at https://canary.example/x.rs:2:2
   3: sk-secret-canary
             at src/ok.rs:3:3
";
        let frames = report::frames(trace, Some(("/home/ann/secret plan.stl", 4, 4)));
        let dir = TempDir::new();
        let (c, sink) = client(&dir.0);
        c.set_consent(false, true, Source::Setup).unwrap();
        c.report(WINDOW_PANIC, &frames);
        let text = sink.wait(1)[0].to_string();
        for word in CANARY {
            assert!(!text.contains(word), "{word} in {text}");
        }
    }

    #[test]
    fn the_final_filter_removes_unknown_keys_and_events() {
        let body = json!({
            "api_key": "k",
            "event": "$exception",
            "distinct_id": "i",
            "extra": "x",
            "properties": {
                "capture_code": "c",
                "$exception_message": "secret",
                "$exception_list": [{
                    "type": "panic",
                    "value": "c",
                    "module": "secret",
                    "mechanism": { "handled": false, "source": "secret" },
                    "stacktrace": {
                        "type": "raw",
                        "raw": "secret",
                        "frames": [{ "function": "f", "context_line": "secret" }],
                    },
                }],
            },
        });
        let text = filter(body).unwrap().to_string();
        assert!(!text.contains("secret") && !text.contains("extra"));
        assert!(text.contains("capture_code"));
        assert!(filter(json!({ "event": "other", "properties": {} })).is_none());
        let wrong = json!({ "event": "photo_saved", "properties": { "path": "/x", "samples": 1 } });
        let kept = filter(wrong).unwrap();
        assert_eq!(kept["properties"], json!({ "samples": 1 }));
    }

    #[test]
    fn the_rate_limit_stops_repeated_reports() {
        let dir = TempDir::new();
        let (c, sink) = client(&dir.0);
        c.set_consent(false, true, Source::Setup).unwrap();
        for _ in 0..5 {
            c.report(WINDOW_PANIC, &own(1));
        }
        assert_eq!(sink.wait(1).len(), 1);
        for n in 2..report::MAX_PER_RUN + 10 {
            c.report(WINDOW_PANIC, &own(n));
        }
        assert_eq!(sink.wait(report::MAX_PER_RUN).len(), report::MAX_PER_RUN);
    }

    #[test]
    fn a_reporter_failure_makes_no_new_report() {
        #[derive(Default)]
        struct Failing(AtomicU32);
        impl Sink for Failing {
            fn send(&self, _: &Value) {
                self.0.fetch_add(1, Ordering::Relaxed);
                panic!("reporter failed");
            }
        }
        let dir = TempDir::new();
        let sink = Arc::new(Failing::default());
        let c = Client::new(Some("phc_x".into()), Some(dir.0.clone()), sink.clone());
        c.set_consent(false, true, Source::Setup).unwrap();
        panic_with_hook(Arc::new(c), || panic!("first"));
        assert_eq!(sink.0.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn a_debug_build_has_no_key() {
        assert_eq!(usable_key(Some("phc_x"), true), None);
        assert_eq!(usable_key(Some("phc_x"), false), Some("phc_x"));
        assert_eq!(usable_key(Some("\"phc_x\""), false), None);
        assert_eq!(usable_key(None, false), None);
        #[cfg(debug_assertions)]
        assert!(!available());
    }

    #[test]
    fn no_key_means_no_sending() {
        let dir = TempDir::new();
        let sink = Arc::new(Capturing::default());
        let c = Client::new(None, Some(dir.0.clone()), sink.clone());
        c.set_consent(true, true, Source::Setup).unwrap();
        c.event(STARTED);
        c.report(WINDOW_PANIC, &own(1));
        assert_eq!(sink.count(), 0);
    }

    #[test]
    fn the_install_id_stays_after_off_and_reset_makes_a_new_one() {
        let dir = TempDir::new();
        let (c, sink) = client(&dir.0);
        assert_eq!(c.install_id(), None);
        c.set_consent(true, false, Source::Setup).unwrap();
        c.event(STARTED);
        let first = c.install_id().unwrap();
        assert_eq!(first.len(), 32);
        assert_eq!(sink.wait(1)[0]["distinct_id"], first.as_str());
        c.set_consent(false, false, Source::Settings).unwrap();
        assert_eq!(c.install_id().as_deref(), Some(first.as_str()));
        c.reset_install_id();
        assert_eq!(c.install_id(), None);
        c.set_consent(true, false, Source::Settings).unwrap();
        c.event(STARTED);
        assert_ne!(c.install_id().unwrap(), first);
    }
}
