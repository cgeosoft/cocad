//! The egui application: parameter panel, 3D view, export and slice preview.

use crate::assembly::Assembly;
use crate::examples;
use crate::export::{self, Format, Meta, Slicer};
use crate::mesh::{Bounds, Mesh};
use crate::openscad;
use crate::params::{self, Param, Value, Widget};
use crate::pathtrace;
use crate::presets;
use crate::recent;
use crate::render::{self, Renderer, View};
use crate::slicer::{self, Kind, Settings, Sliced};
use crate::telemetry::{self, Event, Screen, Source};
use crate::ui;
use crate::update;
use eframe::egui::{self, Color32, RichText, Vec2};
use glam::{Mat4, Vec3};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

mod remote;
mod section;
mod select;
mod tour;
mod view3d;

const DEBOUNCE: Duration = Duration::from_millis(350);
/// Auto save writes the set when the values do not change for this time.
const AUTOSAVE: Duration = Duration::from_millis(600);

#[derive(PartialEq, Clone, Copy)]
enum Mode {
    Model,
    Slice,
    Photo,
}

impl Mode {
    fn screen(self) -> Screen {
        match self {
            Mode::Model => Screen::Model,
            Mode::Slice => Screen::Slice,
            Mode::Photo => Screen::Photo,
        }
    }
}

#[derive(PartialEq, Clone, Copy)]
enum Menu {
    File,
    Model,
    View,
    Help,
}

const MENUS: [(Menu, &str); 4] = [
    (Menu::File, "File"),
    (Menu::Model, "Model"),
    (Menu::View, "View"),
    (Menu::Help, "Help"),
];

/// Named camera views: label, yaw, pitch.
const VIEWS: [(&str, f32, f32); 4] = [
    ("Iso", -0.9, 0.55),
    ("Top", -std::f32::consts::FRAC_PI_2, 1.5697),
    ("Front", -std::f32::consts::FRAC_PI_2, 0.0),
    ("Right", 0.0, 0.0),
];

const REPO: &str = "https://github.com/cgeosoft/cocad";

const SHORTCUTS: [(&str, &str); 16] = [
    ("Ctrl+O", "Open a file"),
    ("Ctrl+,", "Settings"),
    ("Ctrl+Q", "Quit"),
    ("Alt+F, M, V, H", "Open a menu"),
    ("F5", "Render now"),
    ("F, double click", "Fit the model"),
    ("W", "Wireframe"),
    ("E", "Explode the parts"),
    ("S", "Model or Slice tab"),
    ("P", "Model or Photo tab"),
    ("Up, Down, PgUp, PgDn", "Change the slice layer"),
    ("Left drag", "Orbit"),
    ("Right drag, Shift + left drag", "Pan"),
    ("Click", "Select a part"),
    ("Right click", "Part menu: focus, color"),
    ("Esc", "Clear the selection and the focus"),
];

/// What a photo render depends on. A change restarts the render.
#[derive(Clone, Copy, PartialEq)]
struct PhotoKey {
    cam: pathtrace::Camera,
    look: pathtrace::Look,
    mesh_rev: u64,
    /// Part colors and part moves (`App::photo_model_key`).
    model: u64,
    size: (usize, usize),
    samples: u32,
}

/// Pages of the Settings window.
#[derive(Clone, Copy, PartialEq)]
enum SettingsPage {
    Appearance,
    Sets,
    OpenScad,
    Mcp,
    Updates,
    Privacy,
}

/// Photo render settings shown in the Photo panel.
struct PhotoSettings {
    color: [u8; 3],
    finish: &'static str,
    layer_lines: bool,
    studio: pathtrace::Studio,
    samples: u32,
    /// Output image size in pixels.
    width: usize,
    height: usize,
    exposure: f32,
}

const RESOLUTIONS: [(&str, usize, usize); 8] = [
    ("HD 720p", 1280, 720),
    ("Full HD 1080p", 1920, 1080),
    ("QHD 1440p", 2560, 1440),
    ("4K UHD", 3840, 2160),
    ("Site preview 4:3", 1600, 1200),
    ("Square 1:1", 1080, 1080),
    ("Portrait 4:5", 1080, 1350),
    ("Story 9:16", 1080, 1920),
];

const SWATCHES: [(&str, [u8; 3]); 8] = [
    ("Orange", [0xff, 0x93, 0x00]),
    ("CgeoFab", [0xeb, 0x6f, 0x40]),
    ("Grey", [0x8d, 0x93, 0x9c]),
    ("Black", [0x26, 0x27, 0x2a]),
    ("White", [0xee, 0xee, 0xec]),
    ("Red", [0xc8, 0x2a, 0x2a]),
    ("Blue", [0x1f, 0x5f, 0xc8]),
    ("Green", [0x2e, 0x9e, 0x52]),
];

struct Camera {
    yaw: f32,
    pitch: f32,
    dist: f32,
    target: Vec3,
}

impl Camera {
    const FOV: f32 = 0.6;

    fn eye(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        self.target + Vec3::new(cp * cy, cp * sy, sp) * self.dist
    }

    fn matrix(&self, aspect: f32) -> Mat4 {
        let eye = self.eye();
        let proj = Mat4::perspective_rh_gl(Self::FOV, aspect, self.dist * 0.01, self.dist * 50.0);
        proj * Mat4::look_at_rh(eye, self.target, Vec3::Z)
    }

    fn fit(&mut self, b: &Bounds) {
        self.target = b.center();
        let r = (b.size().length() * 0.5).max(1.0);
        self.dist = r / (Self::FOV * 0.5).sin() * 1.1;
    }

    /// Turns around the center of `b`. The distance stays when it is
    /// sufficient to see all of `b`.
    fn pivot_on(&mut self, b: &Bounds) {
        let dist = self.dist;
        self.fit(b);
        self.dist = self.dist.max(dist);
    }

    fn pan(&mut self, d: Vec2, height_px: f32) {
        let eye = self.eye();
        let fwd = (self.target - eye).normalize();
        let right = fwd.cross(Vec3::Z).normalize_or_zero();
        let up = right.cross(fwd);
        let scale = 2.0 * self.dist * (Self::FOV * 0.5).tan() / height_px.max(1.0);
        self.target += (-right * d.x + up * d.y) * scale;
    }
}

/// The result of a render job.
struct Rendered {
    mesh: Arc<Mesh>,
    info: Option<MeshInfo>,
    /// The shells of the mesh.
    assembly: Assembly,
    log: String,
    elapsed: Duration,
}

struct RenderJob {
    rx: Receiver<Result<Rendered, String>>,
    cancel: Arc<AtomicBool>,
    started: Instant,
    /// The parameter values of this render.
    params: Vec<Param>,
}

/// Background render of each part, for the parts of the 3D view.
struct PartsJob {
    /// `None`: the model has no parts, and the shells of the mesh stay.
    rx: Receiver<Result<Option<Assembly>, String>>,
    cancel: Arc<AtomicBool>,
    /// The mesh revision that the parts are for.
    mesh_rev: u64,
    /// The Parts card shows this while the job runs.
    note: &'static str,
}

struct MeshInfo {
    bounds: Bounds,
    volume: f32,
    open_edges: usize,
}

impl MeshInfo {
    fn of(m: &Mesh) -> Option<MeshInfo> {
        m.bounds().map(|bounds| MeshInfo {
            bounds,
            volume: m.volume().abs(),
            open_edges: m.open_edges(),
        })
    }
}

pub struct App {
    file: Option<PathBuf>,
    mtime: Option<SystemTime>,
    last_poll: Instant,
    params: Vec<Param>,
    auto_render: bool,
    dirty_since: Option<Instant>,
    job: Option<RenderJob>,
    log: String,
    error: Option<String>,
    last_time: Option<Duration>,

    mesh: Option<Arc<Mesh>>,
    mesh_rev: u64,
    /// The parts of the mesh for the 3D view and the photo.
    assembly: Option<Arc<Assembly>>,
    assembly_rev: u64,
    parts_job: Option<PartsJob>,
    view3d: view3d::View3d,
    info: Option<MeshInfo>,
    fit_pending: bool,

    mode: Mode,
    settings: Settings,
    sliced: Option<Arc<Sliced>>,
    slice_rev: u64,
    slice_for_rev: u64,
    slice_dirty_since: Option<Instant>,
    slice_job: Option<Receiver<(u64, Sliced, Duration)>>,
    slice_time: Option<Duration>,
    layer: usize,
    ghost: bool,

    cam: Camera,
    wireframe: bool,
    grid: bool,
    show_log: bool,
    photo: PhotoSettings,
    photo_job: Option<pathtrace::Job>,
    photo_key: Option<PhotoKey>,
    /// Wanted key and when it last changed; the render starts when it settles.
    photo_want: Option<(PhotoKey, Instant)>,
    photo_tex: Option<(egui::TextureHandle, u32)>,
    photo_started: Instant,
    themed: bool,
    /// The window title that was sent last. A title command asks for a repaint.
    title: String,
    /// Parameter sets in `<model>.json`.
    sets: Vec<String>,
    sets_file: PathBuf,
    sets_mtime: Option<SystemTime>,
    active_set: Option<String>,
    set_edited: bool,
    /// Name being typed for a new set.
    new_set: Option<String>,
    confirm_delete: bool,
    /// Time of the last edit that auto save has not written yet.
    autosave_at: Option<Instant>,
    /// The menu of the gear button in the sets card, while it is open.
    sets_menu: bool,
    collapsed: std::collections::HashSet<String>,
    cfg: openscad::Config,
    cfg_edit: Option<openscad::Config>,
    settings_page: SettingsPage,
    engine: String,
    renderer: Option<Arc<Mutex<Renderer>>>,
    toast: Option<(String, Instant)>,
    export_rx: Option<Receiver<Result<PathBuf, String>>>,
    menu: Option<Menu>,
    menu_at: egui::Pos2,
    menu_bar_rect: egui::Rect,
    /// The Examples row while its submenu is open.
    examples_row: Option<egui::Rect>,
    recent: Vec<PathBuf>,
    show_about: bool,
    /// Welcome tour, while it is open.
    tour: Option<tour::Tour>,
    /// Privacy question, while it is open.
    consent: Option<ConsentPrompt>,
    /// Privacy card of the Settings window.
    privacy: PrivacyEdit,
    /// Listener for MCP tool calls, when the option is on.
    server: Option<crate::mcp::Server>,
    pending: Vec<remote::Pending>,
    update: update::Checker,
    /// The window with the changes of the new versions, while it is open.
    show_changes: bool,
}

/// The boxes of the privacy question.
struct ConsentPrompt {
    analytics: bool,
    errors: bool,
    source: Source,
}

/// The privacy card of the Settings window. `saved` is the record when the
/// window opened. Consent has its own file, so this is not part of `Config`.
#[derive(Default)]
struct PrivacyEdit {
    analytics: bool,
    errors: bool,
    saved: (bool, bool),
    id: Option<String>,
    confirm_reset: bool,
}

impl PrivacyEdit {
    fn current() -> PrivacyEdit {
        let record = telemetry::load_consent().record;
        let (analytics, errors) = (record.analytics.granted, record.errors.granted);
        PrivacyEdit {
            analytics,
            errors,
            saved: (analytics, errors),
            id: telemetry::install_id(),
            confirm_reset: false,
        }
    }

    fn dirty(&self) -> bool {
        (self.analytics, self.errors) != self.saved
    }
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, file: Option<PathBuf>, mode: &str) -> App {
        ui::fonts(&cc.egui_ctx);
        let renderer = cc.gl.as_ref().and_then(|gl| match Renderer::new(gl) {
            Ok(r) => Some(Arc::new(Mutex::new(r))),
            Err(e) => {
                eprintln!("renderer: {e}");
                None
            }
        });
        let cfg = openscad::Config::load();
        let start_mode = match mode {
            "slice" => Mode::Slice,
            "photo" => Mode::Photo,
            _ => Mode::Model,
        };
        let consent = telemetry::load_consent();
        telemetry::event(Event::AppStarted {
            view: start_mode.screen(),
            first_run: false,
        });
        let mut app = App {
            file: None,
            mtime: None,
            last_poll: Instant::now(),
            params: Vec::new(),
            auto_render: true,
            dirty_since: None,
            job: None,
            log: String::new(),
            error: None,
            last_time: None,
            mesh: None,
            mesh_rev: 0,
            assembly: None,
            assembly_rev: 0,
            parts_job: None,
            view3d: Default::default(),
            info: None,
            fit_pending: true,
            mode: start_mode,
            settings: Settings::default(),
            sliced: None,
            slice_rev: 0,
            slice_for_rev: u64::MAX,
            slice_dirty_since: None,
            slice_job: None,
            slice_time: None,
            layer: 0,
            ghost: false,
            cam: Camera {
                yaw: -0.9,
                pitch: 0.55,
                dist: 150.0,
                target: Vec3::ZERO,
            },
            wireframe: false,
            grid: true,
            show_log: false,
            photo: PhotoSettings {
                color: [0xff, 0x93, 0x00],
                finish: "satin",
                layer_lines: true,
                studio: pathtrace::Studio::Dark,
                samples: 256,
                width: 1920,
                height: 1080,
                exposure: 1.0,
            },
            photo_job: None,
            photo_key: None,
            photo_want: None,
            photo_tex: None,
            photo_started: Instant::now(),
            themed: false,
            title: String::new(),
            sets: Vec::new(),
            sets_file: PathBuf::new(),
            sets_mtime: None,
            active_set: None,
            set_edited: false,
            new_set: None,
            confirm_delete: false,
            autosave_at: None,
            sets_menu: false,
            collapsed: Default::default(),
            consent: consent
                .prompt
                .filter(|_| telemetry::available())
                .map(|source| ConsentPrompt {
                    analytics: consent.record.analytics.granted,
                    errors: consent.record.errors.granted,
                    source,
                }),
            privacy: PrivacyEdit::default(),
            cfg,
            cfg_edit: None,
            settings_page: SettingsPage::Appearance,
            engine: String::new(),
            renderer,
            toast: None,
            export_rx: None,
            menu: None,
            menu_at: egui::Pos2::ZERO,
            menu_bar_rect: egui::Rect::NOTHING,
            examples_row: None,
            recent: recent::load(),
            show_about: false,
            tour: tour::first_start().then(tour::Tour::new),
            server: None,
            pending: Vec::new(),
            update: update::Checker::new(),
            show_changes: false,
        };
        app.sync_server(&cc.egui_ctx);
        if let Some(f) = file {
            app.open(f);
        }
        app
    }

    fn open(&mut self, path: PathBuf) {
        self.flush_autosave();
        match std::fs::read_to_string(&path) {
            Ok(src) => {
                self.params = params::parse(&src);
                self.mtime = mtime(&path);
                self.sets_file = presets::file_for(&path);
                self.sets = presets::names(&self.sets_file);
                self.sets_mtime = mtime(&self.sets_file);
                self.active_set = None;
                self.set_edited = false;
                self.confirm_delete = false;
                self.new_set = None;
                self.autosave_at = None;
                self.autoload_set(&path);
                self.file = Some(path);
                self.mesh = None;
                self.info = None;
                self.sliced = None;
                self.mesh_rev += 1;
                self.cancel_parts();
                self.set_assembly(None);
                self.view3d.new_model();
                self.slice_rev += 1;
                self.fit_pending = true;
                self.error = None;
                self.recent = recent::add(self.file.as_ref().unwrap());
                telemetry::event(Event::ModelOpened {
                    parameters: self.params.len(),
                });
                self.start_render();
            }
            Err(e) => self.error = Some(format!("cannot read {}: {e}", path.display())),
        }
    }

    /// Re-reads the file after an outside change and keeps edited values.
    fn reload(&mut self) {
        let Some(path) = self.file.clone() else {
            return;
        };
        let Ok(src) = std::fs::read_to_string(&path) else {
            return;
        };
        let mut fresh = params::parse(&src);
        for p in &mut fresh {
            if let Some(old) = self.params.iter().find(|o| o.name == p.name && o.changed())
                && std::mem::discriminant(&old.value) == std::mem::discriminant(&p.value)
            {
                p.value = old.value.clone();
            }
        }
        self.params = fresh;
        self.reload_sets();
        self.mtime = mtime(&path);
        self.start_render();
    }

    /// Re-reads the sets file after a change. The active set takes the new
    /// values, unless it has edits. A file that cannot be parsed, for example
    /// during a write, stays unread and the next poll tries it again.
    fn reload_sets(&mut self) {
        let m = mtime(&self.sets_file);
        if m == self.sets_mtime {
            return;
        }
        let Ok(names) = presets::try_names(&self.sets_file) else {
            return;
        };
        self.sets_mtime = m;
        self.sets = names;
        let Some(name) = self.active_set.clone() else {
            return;
        };
        if !self.sets.contains(&name) {
            self.active_set = None;
            self.set_edited = false;
            self.confirm_delete = false;
            self.toast = Some((format!("Set {name} is not in the file"), Instant::now()));
        } else if !self.set_edited {
            let before: Vec<_> = self.params.iter().map(|p| p.value.clone()).collect();
            let skipped = presets::apply(&self.sets_file, &name, &mut self.params);
            if self.params.iter().map(|p| &p.value).ne(before.iter()) {
                let text = match skipped {
                    Ok(s) if !s.is_empty() => format!("Skipped: {}", s.join(", ")),
                    Ok(_) => format!("Set {name} reloaded"),
                    Err(e) => e.to_string(),
                };
                self.toast = Some((text, Instant::now()));
                self.dirty_since = Some(Instant::now());
            }
        }
    }

    /// Loads a set of the sets file when a model opens: the set that was
    /// active last, else the first set. See `presets::autoload`.
    fn autoload_set(&mut self, scad: &Path) {
        let Some(name) = presets::autoload(scad, &self.sets) else {
            return;
        };
        let file = self
            .sets_file
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        let text = match presets::apply(&self.sets_file, &name, &mut self.params) {
            Ok(skipped) if !skipped.is_empty() => {
                format!("Loaded set {name}. Skipped: {}", skipped.join(", "))
            }
            Ok(_) => format!("Loaded set {name} from {file}"),
            Err(e) => {
                self.toast = Some((e.to_string(), Instant::now()));
                return;
            }
        };
        self.toast = Some((text, Instant::now()));
        self.active_set = Some(name);
    }

    /// Makes `name` the active set with no edits, and records it for the
    /// model. `None` is Model defaults.
    fn set_active(&mut self, name: Option<String>) {
        if let Some(f) = &self.file {
            // The record only selects the set at the next start.
            let _ = presets::set_last(f, name.as_deref());
        }
        self.active_set = name;
        self.set_edited = false;
        self.confirm_delete = false;
        self.autosave_at = None;
    }

    /// Call after a value change. The active set then has edits, and auto
    /// save writes them when the values stop changing.
    fn mark_set_edited(&mut self) {
        self.set_edited = self.active_set.is_some();
        self.autosave_at = self.set_edited.then(Instant::now);
    }

    /// Writes the edits to the active set when auto save is on.
    fn autosave(&mut self) {
        self.autosave_at = None;
        if !self.cfg.autosave_sets || !self.set_edited {
            return;
        }
        let Some(name) = self.active_set.clone() else {
            return;
        };
        match presets::save(&self.sets_file, &name, &self.params) {
            Ok(()) => {
                self.set_edited = false;
                self.sets_mtime = mtime(&self.sets_file);
            }
            Err(e) => {
                self.toast = Some((format!("Cannot save set {name}: {e}"), Instant::now()));
            }
        }
    }

    /// Writes the waiting auto save now, before the active set or the model
    /// changes.
    fn flush_autosave(&mut self) {
        if self.autosave_at.is_some() {
            self.autosave();
        }
    }

    fn start_render(&mut self) {
        let Some(file) = self.file.clone() else {
            return;
        };
        if let Some(j) = self.job.take() {
            j.cancel.store(true, Ordering::Relaxed);
        }
        if let Some(j) = self.parts_job.take() {
            j.cancel.store(true, Ordering::Relaxed);
        }
        self.dirty_since = None;
        let defines = params::defines(&self.params);
        let params = self.params.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        let c = cancel.clone();
        let cfg = self.cfg.clone();
        let manifold = cfg.args().iter().any(|a| a == "--backend=manifold");
        self.engine = if manifold {
            "Manifold".into()
        } else {
            "CGAL".into()
        };
        std::thread::spawn(move || {
            let r = openscad::render(&cfg, &file, &defines, &c).map_err(|e| e.to_string());
            if c.load(Ordering::Relaxed) {
                let _ = tx.send(Err("cancelled".into()));
                return;
            }
            // The shells and the checks can take a moment for a large mesh.
            let r = r.map(|out| {
                let mesh = Arc::new(out.mesh);
                let mut assembly = Assembly::shells(&mesh);
                // One shown part of a multicolor model: all shells take its color.
                if let Some(color) = crate::parts::shown_color(&params, &out.log) {
                    assembly.paint_all(color);
                }
                Rendered {
                    info: MeshInfo::of(&mesh),
                    mesh,
                    assembly,
                    log: out.log,
                    elapsed: out.elapsed,
                }
            });
            let _ = tx.send(r);
        });
        self.job = Some(RenderJob {
            rx,
            cancel,
            started: Instant::now(),
            params: self.params.clone(),
        });
    }

    fn cancel_parts(&mut self) {
        if let Some(j) = self.parts_job.take() {
            j.cancel.store(true, Ordering::Relaxed);
        }
    }

    fn set_assembly(&mut self, assembly: Option<Assembly>) {
        self.assembly = assembly.map(Arc::new);
        self.assembly_rev += 1;
        let parts = self.assembly.as_ref().and_then(|a| a.bounds(&[]));
        self.view3d.section.set_bounds(parts);
    }

    /// Finds the parts of the new mesh in the background. The parts then
    /// replace the shells of the mesh. With all parts of a multicolor model
    /// shown, each part of the `cocad_part` list renders. A model without
    /// that list divides at its module instances (see `csg_split.rs`). A
    /// model that shows one part of its list keeps the shells.
    fn color_parts(&mut self, params: Vec<Param>) {
        self.cancel_parts();
        let Some(file) = self.file.clone() else {
            return;
        };
        let cfg = self.cfg.clone();
        if crate::parts::needs_render(&params) {
            self.start_parts_job(false, "Rendering the named parts…", move |cancel| {
                crate::parts::render(&cfg, &file, &params, cancel).map(Some)
            });
        } else if crate::parts::names(&params).is_empty() {
            // The same values as the main render.
            let defines = params::defines(&params);
            self.start_parts_job(true, "Finding the parts…", move |cancel| {
                crate::csg_split::render(&cfg, &file, &defines, cancel)
            });
        }
    }

    /// Starts a background job that renders the parts of the current mesh
    /// one by one. When it is done, its parts replace the shells of the mesh
    /// in the 3D view (see `Assembly::from_parts`). With `split_shells`, each
    /// shell of a part is one part. When `render` gives `None`, the shells
    /// stay. A new render or file cancels the job. The Parts card shows
    /// `note` while the job runs.
    fn start_parts_job(
        &mut self,
        split_shells: bool,
        note: &'static str,
        render: impl FnOnce(&Arc<AtomicBool>) -> anyhow::Result<Option<Vec<crate::parts::Part>>>
        + Send
        + 'static,
    ) {
        self.cancel_parts();
        let Some(mesh) = self.mesh.clone() else {
            return;
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        let c = cancel.clone();
        std::thread::spawn(move || {
            let r = render(&c)
                .map(|parts| parts.map(|p| Assembly::from_parts(&mesh, &p, split_shells)))
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send(r);
        });
        self.parts_job = Some(PartsJob {
            rx,
            cancel,
            mesh_rev: self.mesh_rev,
            note,
        });
    }

    fn start_slice(&mut self) {
        let Some(mesh) = self.mesh.clone() else {
            return;
        };
        self.slice_dirty_since = None;
        let (tx, rx) = channel();
        let s = self.settings;
        let rev = self.mesh_rev;
        std::thread::spawn(move || {
            let t = Instant::now();
            let r = slicer::slice(&mesh, &s);
            let _ = tx.send((rev, r, t.elapsed()));
        });
        self.slice_for_rev = rev;
        self.slice_job = Some(rx);
    }

    fn poll(&mut self, ctx: &egui::Context) {
        if let Some(job) = &self.job
            && let Ok(r) = job.rx.try_recv()
        {
            let params = self.job.take().map(|j| j.params).unwrap_or_default();
            match r {
                Ok(out) => {
                    self.info = out.info;
                    self.mesh = Some(out.mesh);
                    self.mesh_rev += 1;
                    self.set_assembly(Some(out.assembly));
                    self.color_parts(params);
                    self.log = out.log;
                    self.last_time = Some(out.elapsed);
                    self.error = None;
                    if self.fit_pending {
                        self.fit();
                        self.fit_pending = false;
                    }
                    if self.mode == Mode::Slice {
                        self.start_slice();
                    }
                }
                Err(e) if e == "cancelled" => {}
                Err(e) => {
                    self.log = e.clone();
                    self.error = Some(e.lines().next().unwrap_or("").to_string());
                }
            }
        }
        if let Some(job) = &self.parts_job
            && let Ok(r) = job.rx.try_recv()
        {
            let rev = job.mesh_rev;
            self.parts_job = None;
            match r {
                Ok(Some(assembly)) if rev == self.mesh_rev => {
                    self.set_assembly(Some(assembly));
                    ctx.request_repaint();
                }
                // The main render worked, so a failed part render only goes
                // to the log. The shells stay.
                Err(e) if rev == self.mesh_rev && e != "cancelled" => {
                    if !self.log.trim_end().is_empty() {
                        self.log.push_str("\n\n");
                    }
                    self.log.push_str(&format!("The parts failed: {e}\n"));
                }
                // No parts, or an old mesh: the shells stay.
                _ => {}
            }
        }
        if let Some(rx) = &self.slice_job
            && let Ok((rev, sliced, t)) = rx.try_recv()
        {
            self.slice_job = None;
            if rev == self.mesh_rev {
                let first = self.sliced.is_none();
                let n = sliced.layers.len();
                self.sliced = Some(Arc::new(sliced));
                self.slice_rev += 1;
                self.slice_time = Some(t);
                if first || self.layer >= n {
                    self.layer = n.saturating_sub(1);
                }
            }
        }
        if let Some(rx) = &self.export_rx
            && let Ok(r) = rx.try_recv()
        {
            self.export_rx = None;
            self.toast = Some((
                match r {
                    Ok(p) => {
                        if let Some(format) = Format::from_path(&p) {
                            telemetry::event(Event::ModelExported { format });
                        }
                        format!("Saved {}", p.display())
                    }
                    Err(e) => format!("Export failed: {e}"),
                },
                Instant::now(),
            ));
        }

        // Watch the file for outside edits.
        if self.last_poll.elapsed() > Duration::from_millis(700) {
            self.last_poll = Instant::now();
            // Another window can change consent.
            telemetry::reload_consent();
            if let Some(f) = &self.file {
                let m = mtime(f);
                if m.is_some() && m != self.mtime {
                    self.reload();
                }
                self.reload_sets();
            }
        }

        if self.auto_render && self.dirty_since.is_some_and(|t| t.elapsed() > DEBOUNCE) {
            self.start_render();
        }
        if self.autosave_at.is_some_and(|t| t.elapsed() > AUTOSAVE) {
            self.autosave();
        }
        if self
            .slice_dirty_since
            .is_some_and(|t| t.elapsed() > DEBOUNCE)
        {
            self.start_slice();
        }
        self.run_commands();
        if self.job.is_some()
            || self.slice_job.is_some()
            || self.parts_job.is_some()
            || self.export_rx.is_some()
            || !self.pending.is_empty()
        {
            ctx.request_repaint_after(Duration::from_millis(50));
        } else if self.autosave_at.is_some() {
            ctx.request_repaint_after(AUTOSAVE);
        } else {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
    }

    /// Fits the model in the view. While a part has the focus, fits that
    /// part.
    fn fit(&mut self) {
        if let Some(b) = self.focus_bounds().or_else(|| self.view_bounds()) {
            self.cam.fit(&b);
        }
    }

    /// Turns the camera to a standard view. While a part has the focus,
    /// fits that part. Else, with a selected part, the camera turns around
    /// the center of that part and the distance stays (it goes up only
    /// when the part is too large for it). Else it fits the model.
    fn set_view(&mut self, yaw: f32, pitch: f32) {
        self.cam.yaw = yaw;
        self.cam.pitch = pitch;
        if self.focus_bounds().is_some() {
            self.fit();
        } else if let Some(b) = self.pivot_bounds() {
            self.cam.pivot_on(&b);
        } else {
            self.fit();
        }
    }

    fn stem(&self) -> String {
        self.file
            .as_ref()
            .and_then(|f| f.file_stem())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "model".into())
    }

    /// Start folder of the export and photo dialogs: the folder of the last
    /// export, else the folder of the model.
    fn export_dir(&self) -> Option<PathBuf> {
        recent::export_dir().or_else(|| self.file.as_ref()?.parent().map(PathBuf::from))
    }

    fn export(&mut self, fmt: Format) {
        let Some(mesh) = self.mesh.clone() else {
            return;
        };
        let mut dialog = rfd::FileDialog::new()
            .set_file_name(export::suggested_name(
                &self.stem(),
                self.active_set.as_deref(),
                fmt,
            ))
            .add_filter(fmt.label(), &[fmt.ext()]);
        if let Some(dir) = self.export_dir() {
            dialog = dialog.set_directory(dir);
        }
        let Some(path) = dialog.save_file() else {
            return;
        };
        recent::set_export_dir(&path);
        let meta = Meta {
            title: self.stem(),
            params: self
                .params
                .iter()
                .map(|p| (p.name.clone(), p.value.to_scad()))
                .collect(),
        };
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(
                export::write(&path, &mesh, &meta)
                    .map(|_| path)
                    .map_err(|e| e.to_string()),
            );
        });
        self.export_rx = Some(rx);
    }

    /// Renders each part, then writes a multicolor 3MF for `slicer`.
    fn export_parts(&mut self, slicer: Slicer) {
        let Some(file) = self.file.clone() else {
            return;
        };
        let mut dialog = rfd::FileDialog::new()
            .set_file_name(export::suggested_name(
                &self.stem(),
                self.active_set.as_deref(),
                Format::ThreeMf,
            ))
            .add_filter(slicer.label(), &["3mf"]);
        if let Some(dir) = self.export_dir() {
            dialog = dialog.set_directory(dir);
        }
        let Some(path) = dialog.save_file() else {
            return;
        };
        recent::set_export_dir(&path);
        let meta = Meta {
            title: self.stem(),
            params: self
                .params
                .iter()
                .map(|p| (p.name.clone(), p.value.to_scad()))
                .collect(),
        };
        let (cfg, params) = (self.cfg.clone(), self.params.clone());
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let cancel = Arc::new(AtomicBool::new(false));
            let _ = tx.send(
                crate::parts::render(&cfg, &file, &params, &cancel)
                    .and_then(|parts| export::write_parts(&path, &parts, &meta, slicer))
                    .map(|_| path)
                    .map_err(|e| e.to_string()),
            );
        });
        self.export_rx = Some(rx);
    }

    fn pick_file(&mut self) {
        let mut d = rfd::FileDialog::new().add_filter("OpenSCAD", &["scad"]);
        if let Some(dir) = self.file.as_ref().and_then(|f| f.parent()) {
            d = d.set_directory(dir);
        }
        if let Some(p) = d.pick_file() {
            self.open(p);
        }
    }

    fn can_export(&self) -> bool {
        self.mesh.is_some() && self.job.is_none() && self.export_rx.is_none()
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        self.menu_bar_rect = ui.max_rect();
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let (r, _) = ui.allocate_exact_size(Vec2::splat(16.0), egui::Sense::hover());
            ui.painter().rect_filled(r, 4.0, ui::ACCENT);
            ui.painter().rect_filled(r.shrink(4.5), 1.0, ui::c().bg);
            ui.add_space(6.0);
            for (menu, label) in MENUS {
                let open = self.menu == Some(menu);
                let r = ui::menu_trigger(ui, label, open);
                if r.clicked() {
                    self.menu = if open { None } else { Some(menu) };
                } else if r.hovered() && self.menu.is_some() {
                    self.menu = Some(menu);
                }
                if self.menu == Some(menu) {
                    self.menu_at = r.rect.left_bottom();
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(f) = &self.file {
                    let name = f.file_name().unwrap_or_default().to_string_lossy();
                    ui.label(ui::dim(name.into_owned()).monospace().size(12.0))
                        .on_hover_text(f.display().to_string());
                }
            });
        });
    }

    /// The open dropdown. A click on a row or outside it closes the menu.
    fn menu_popup(&mut self, ctx: &egui::Context) {
        let Some(menu) = self.menu else {
            return;
        };
        let area = egui::Area::new(egui::Id::new("menu"))
            .order(egui::Order::Foreground)
            .fixed_pos(self.menu_at + Vec2::new(0.0, 4.0))
            .show(ctx, |ui| {
                ui::dark_scope(ui, |ui| {
                    ui::menu_frame(ui, |ui| match menu {
                        Menu::File => self.file_menu(ui),
                        Menu::Model => self.model_menu(ui),
                        Menu::View => self.view_menu(ui),
                        Menu::Help => self.help_menu(ui),
                    })
                })
            });
        if menu != Menu::File {
            self.examples_row = None;
        }
        let sub_rect = self.examples_row.map(|row| {
            // The submenu touches the menu, so the pointer can move across.
            let at = egui::pos2(area.response.rect.right() - 2.0, row.top() - 7.0);
            self.examples_popup(ctx, at)
        });
        let (pos, pressed, clicked, esc) = ctx.input(|i| {
            (
                i.pointer.interact_pos(),
                i.pointer.any_pressed(),
                i.pointer.any_click(),
                i.key_pressed(egui::Key::Escape),
            )
        });
        let on_menu = pos.is_some_and(|p| area.response.rect.contains(p));
        let on_bar = pos.is_some_and(|p| self.menu_bar_rect.contains(p));
        let on_sub = pos.is_some_and(|p| sub_rect.is_some_and(|r| r.contains(p)));
        // The row spans the menu height only, so its margin keeps the submenu open.
        let on_row = pos.is_some_and(|p| {
            self.examples_row
                .is_some_and(|r| on_menu && r.y_range().contains(p.y))
        });
        if on_menu && !on_row {
            self.examples_row = None;
        }
        let picked = clicked && (on_sub || (on_menu && !on_row));
        if esc || (pressed && !on_menu && !on_bar && !on_sub) || picked {
            self.menu = None;
            self.examples_row = None;
        }
    }

    /// The Examples submenu at `at`. Returns its rectangle.
    fn examples_popup(&mut self, ctx: &egui::Context, at: egui::Pos2) -> egui::Rect {
        egui::Area::new(egui::Id::new("examples_menu"))
            .order(egui::Order::Foreground)
            .fixed_pos(at)
            .show(ctx, |ui| {
                ui::dark_scope(ui, |ui| {
                    ui::menu_frame(ui, |ui| {
                        for (label, file) in examples::MENU {
                            if ui::menu_item(ui, label, "", true, false).clicked() {
                                self.open_example(file);
                            }
                        }
                    })
                })
            })
            .response
            .rect
    }

    fn open_example(&mut self, file: &str) {
        match examples::extract(file) {
            Ok(path) => self.open(path),
            Err(e) => self.error = Some(format!("cannot write the examples: {e}")),
        }
    }

    fn file_menu(&mut self, ui: &mut egui::Ui) {
        if ui::menu_item(ui, "Open…", "Ctrl+O", true, false).clicked() {
            self.pick_file();
        }
        let examples = ui::menu_item(ui, "Examples", "\u{25B6}", true, false);
        if examples.hovered() {
            self.examples_row = Some(examples.rect);
        }
        ui::menu_header(ui, "Recent models");
        if self.recent.is_empty() {
            ui::menu_item(ui, "No recent models", "", false, false);
        }
        // The list is newest first, so the first entry is the open model.
        for (i, path) in self.recent.clone().into_iter().enumerate() {
            if recent_row(ui, &path, i == 0 && self.file.is_some()) {
                self.open(path);
            }
        }
        if !self.recent.is_empty() && ui::menu_item(ui, "Clear list", "", true, false).clicked() {
            recent::clear();
            self.recent.clear();
        }
        ui::menu_separator(ui);
        if ui::menu_item(ui, "Settings", "Ctrl+,", true, false).clicked() {
            self.open_settings();
        }
        ui::menu_separator(ui);
        if ui::menu_item(ui, "Quit", "Ctrl+Q", true, false).clicked() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn model_menu(&mut self, ui: &mut egui::Ui) {
        if ui::menu_item(ui, "Render now", "F5", self.file.is_some(), false).clicked() {
            self.start_render();
        }
        if ui::menu_item(ui, "Auto render", "", true, self.auto_render).clicked() {
            self.auto_render = !self.auto_render;
        }
        ui::menu_separator(ui);
        let can_export = self.can_export();
        for fmt in Format::ALL {
            let label = format!("Export {}", fmt.label());
            if ui::menu_item(ui, &label, "", can_export, false).clicked() {
                self.export(fmt);
            }
        }
        if !crate::parts::names(&self.params).is_empty() {
            for slicer in Slicer::ALL {
                let label = format!("Export multicolor 3MF for {}", slicer.label());
                if ui::menu_item(ui, &label, "", can_export, false).clicked() {
                    self.export_parts(slicer);
                }
            }
        }
    }

    fn view_menu(&mut self, ui: &mut egui::Ui) {
        for (mode, label, key) in [
            (Mode::Model, "Model", ""),
            (Mode::Slice, "Slice", "S"),
            (Mode::Photo, "Photo", "P"),
        ] {
            if ui::menu_item(ui, label, key, true, self.mode == mode).clicked() {
                self.set_mode(mode);
            }
        }
        ui::menu_separator(ui);
        if ui::menu_item(ui, "Fit", "F", self.mesh.is_some(), false).clicked() {
            self.fit();
        }
        for (label, yaw, pitch) in VIEWS {
            if ui::menu_item(ui, label, "", true, false).clicked() {
                self.set_view(yaw, pitch);
            }
        }
        ui::menu_separator(ui);
        if ui::menu_item(ui, "Wireframe", "W", true, self.wireframe).clicked() {
            self.wireframe = !self.wireframe;
        }
        if ui::menu_item(ui, "Grid", "", true, self.grid).clicked() {
            self.grid = !self.grid;
        }
        let parts = self.assembly.as_ref().map_or(0, |a| a.parts.len());
        let explode = self.view3d.explode;
        if ui::menu_item(ui, "Explode", "E", parts >= 2, explode).clicked() {
            self.toggle_explode();
        }
        if ui::menu_item(ui, "OpenSCAD console", "", true, self.show_log).clicked() {
            self.show_log = !self.show_log;
        }
    }

    fn help_menu(&mut self, ui: &mut egui::Ui) {
        if ui::menu_item(ui, "Welcome tour", "", true, false).clicked() {
            self.tour = Some(tour::Tour::new());
        }
        if ui::menu_item(ui, "GitHub page", "", true, false).clicked() {
            ui.ctx().open_url(egui::OpenUrl::new_tab(REPO));
        }
        ui::menu_separator(ui);
        if ui::menu_item(ui, "About and shortcuts", "", true, false).clicked() {
            self.show_about = true;
        }
    }

    fn open_settings(&mut self) {
        self.cfg_edit = Some(self.cfg.clone());
        self.privacy = PrivacyEdit::current();
    }

    /// Asks at the first start and when the notice changes. At the first start
    /// both boxes are clear, and a clear box is a valid answer. The window has
    /// no close button, so that the user makes a choice. Each answer goes through
    /// the same button, so yes and no have the same weight.
    fn consent_window(&mut self, ctx: &egui::Context) {
        let Some(mut prompt) = self.consent.take() else {
            return;
        };
        let mut save = false;
        let frame = egui::Frame::NONE
            .fill(ui::c().panel)
            .corner_radius(14)
            .inner_margin(egui::Margin::same(18))
            .stroke(egui::Stroke::new(1.0_f32, ui::c().line));
        egui::Modal::new(egui::Id::new("consent"))
            .frame(frame)
            .show(ctx, |ui| {
                ui.set_width(460.0);
                ui.heading("Your privacy");
                ui.add_space(6.0);
                ui.label(
                    "CoCAD can send two kinds of data. Both are optional. Each one stays \
                     off until you select it. CoCAD never sends your models, file names or \
                     parameter values.",
                );
                if prompt.source == Source::UpdatePrompt {
                    ui.add_space(6.0);
                    ui.label(UPDATE_NOTE);
                }
                ui.add_space(12.0);
                ui.checkbox(&mut prompt.analytics, "Send usage analytics");
                ui.indent("analytics_note", |ui| {
                    ui.label(ui::dim(ANALYTICS_NOTE).size(12.5))
                });
                ui.add_space(10.0);
                ui.checkbox(&mut prompt.errors, "Send error reports");
                ui.indent("errors_note", |ui| {
                    ui.label(ui::dim(ERRORS_NOTE).size(12.5))
                });
                ui.add_space(12.0);
                ui.label(ui::dim(VENDOR_NOTE).size(12.5));
                ui.label(ui::dim("You can change this at any time in File > Settings.").size(12.5));
                ui.hyperlink_to("Privacy Notice", telemetry::PRIVACY_URL);
                ui.add_space(12.0);
                let row = Vec2::new(ui.available_width(), 34.0);
                let rtl = egui::Layout::right_to_left(egui::Align::Center);
                ui.allocate_ui_with_layout(row, rtl, |ui| {
                    ui.spacing_mut().button_padding = Vec2::new(16.0, 7.0);
                    save = ui.add(ui::primary("Save choice")).clicked();
                });
            });
        if !save {
            self.consent = Some(prompt);
            return;
        }
        if let Err(e) = telemetry::set_consent(prompt.analytics, prompt.errors, prompt.source) {
            self.toast = Some((format!("Cannot save your choice: {e}"), Instant::now()));
        }
        telemetry::event(Event::AppStarted {
            view: self.mode.screen(),
            first_run: prompt.source == Source::Setup,
        });
    }

    /// A small notice at the bottom right. A click opens the list of changes.
    /// It waits while another window covers the app.
    fn update_notice(&mut self, ctx: &egui::Context) {
        let covered = self.consent.is_some()
            || self.tour.is_some()
            || self.cfg_edit.is_some()
            || self.show_changes
            || self.view3d.colors_open;
        if covered || !self.cfg.update_check {
            return;
        }
        let Some(latest) = self.update.offer() else {
            return;
        };
        let mut dismiss = false;
        let frame = egui::Frame::NONE
            .fill(ui::c().panel)
            .corner_radius(10)
            .inner_margin(egui::Margin::symmetric(14, 10))
            .stroke(egui::Stroke::new(1.5_f32, ui::ACCENT))
            .shadow(ctx.style().visuals.popup_shadow);
        let notice = egui::Area::new(egui::Id::new("update_notice"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::RIGHT_BOTTOM, [-14.0, -40.0])
            .sense(egui::Sense::click())
            .show(ctx, |ui| {
                frame.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            let title = format!("CoCAD {latest} is available");
                            ui.add(egui::Label::new(title).selectable(false));
                            let hint = ui::dim("Click to see the changes").size(12.5);
                            ui.add(egui::Label::new(hint).selectable(false));
                        });
                        ui.add_space(10.0);
                        let close = RichText::new("×").size(18.0).color(ui::c().dim);
                        dismiss = ui
                            .add(egui::Button::new(close).frame(false))
                            .on_hover_text("Do not show this version again")
                            .clicked();
                    });
                });
            });
        let notice = notice
            .response
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if dismiss {
            self.update.dismiss();
        } else if notice.clicked() {
            self.show_changes = true;
        }
    }

    /// Lists the new versions. The button opens the download part of the website.
    fn changes_window(&mut self, ctx: &egui::Context) {
        if !self.show_changes {
            return;
        }
        let installed = update::Version::current();
        let mut close = false;
        let mut download = false;
        let frame = egui::Frame::NONE
            .fill(ui::c().panel)
            .corner_radius(14)
            .inner_margin(egui::Margin::same(18))
            .stroke(egui::Stroke::new(1.0_f32, ui::c().line));
        let height = (ctx.screen_rect().height() - 170.0).clamp(120.0, 420.0);
        let modal = egui::Modal::new(egui::Id::new("changes"))
            .frame(frame)
            .show(ctx, |ui| {
                ui.set_width(460.0);
                ui.heading("A new version is available");
                ui.add_space(4.0);
                ui.label(ui::dim(format!("You have version {installed}.")));
                ui.add_space(10.0);
                egui::ScrollArea::vertical()
                    .max_height(height)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for release in self.update.releases() {
                            let mut title = release.version.to_string();
                            if release.version == installed {
                                title.push_str(" (installed)");
                            }
                            ui.label(RichText::new(title).strong());
                            for change in &release.changes {
                                ui.label(format!("•  {change}"));
                            }
                            if release.changes.is_empty() {
                                ui.label(ui::dim("No notes for this version."));
                            }
                            ui.add_space(10.0);
                        }
                    });
                ui.add_space(8.0);
                let row = Vec2::new(ui.available_width(), 34.0);
                let rtl = egui::Layout::right_to_left(egui::Align::Center);
                ui.allocate_ui_with_layout(row, rtl, |ui| {
                    ui.spacing_mut().button_padding = Vec2::new(16.0, 7.0);
                    download = ui.add(ui::primary("Go to downloads")).clicked();
                    close = ui.add(ui::ghost("Close", false)).clicked();
                });
            });
        if download {
            ctx.open_url(egui::OpenUrl::new_tab(update::DOWNLOAD_URL));
        }
        if download || close || modal.should_close() {
            self.show_changes = false;
        }
    }

    fn about_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_about;
        egui::Window::new("About CoCAD")
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(420.0)
            .show(ctx, |ui| {
                ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
                ui.label(ui::dim("MIT license"));
                ui.hyperlink(REPO);
                ui.add_space(8.0);
                ui::section(ui, "Shortcuts");
                egui::Grid::new("shortcuts")
                    .num_columns(2)
                    .spacing([24.0, 6.0])
                    .show(ui, |ui| {
                        for (key, what) in SHORTCUTS {
                            ui.label(RichText::new(key).monospace().color(ui::c().dim));
                            ui.label(what);
                            ui.end_row();
                        }
                    });
            });
        self.show_about = open;
    }

    fn model_panel(&mut self, ui: &mut egui::Ui) {
        ui.label(ui::dim("Render the model and export it."));
        ui.add_space(4.0);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui::card(ui, |ui| {
                    ui::section(ui, "Render");
                    ui.horizontal(|ui| {
                        ui.label("Auto render");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui::toggle(ui, &mut self.auto_render)
                                .on_hover_text("Render when a parameter changes");
                        });
                    });
                    let render =
                        egui::Button::new("Render").min_size(Vec2::new(ui.available_width(), 34.0));
                    if ui
                        .add_enabled(self.file.is_some(), render)
                        .on_hover_text("F5")
                        .clicked()
                    {
                        self.start_render();
                    }
                });
                ui::card(ui, |ui| {
                    ui::section(ui, "Export");
                    let can_export = self.can_export();
                    let w = (ui.available_width() - 2.0 * ui.spacing().item_spacing.x) / 3.0;
                    let mut pick = None;
                    ui.horizontal(|ui| {
                        for fmt in Format::ALL {
                            let b =
                                ui::primary(&fmt.ext().to_uppercase()).min_size(Vec2::new(w, 34.0));
                            if ui
                                .add_enabled(can_export, b)
                                .on_hover_text(fmt.label())
                                .clicked()
                            {
                                pick = Some(fmt);
                            }
                        }
                    });
                    if let Some(fmt) = pick {
                        self.export(fmt);
                    }
                    if !crate::parts::names(&self.params).is_empty() {
                        ui.label(ui::dim("Multicolor 3MF"));
                        let w = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0;
                        let mut pick = None;
                        ui.horizontal(|ui| {
                            for slicer in Slicer::ALL {
                                let b = ui::primary(slicer.label()).min_size(Vec2::new(w, 34.0));
                                if ui.add_enabled(can_export, b).clicked() {
                                    pick = Some(slicer);
                                }
                            }
                        });
                        if let Some(slicer) = pick {
                            self.export_parts(slicer);
                        }
                    }
                });
                self.parts_card(ui);
                self.explode_card(ui);
                self.section_card(ui);
                if let (Some(i), Some(mesh)) = (&self.info, &self.mesh) {
                    let size = i.bounds.size();
                    ui::card(ui, |ui| {
                        ui::section(ui, "Mesh");
                        info_row(
                            ui,
                            "Size",
                            &format!("{:.1} × {:.1} × {:.1} mm", size.x, size.y, size.z),
                        );
                        info_row(ui, "Volume", &format!("{:.2} cm³", i.volume / 1000.0));
                        info_row(ui, "Triangles", &mesh.triangles.len().to_string());
                        info_row(ui, "Open edges", &i.open_edges.to_string());
                    });
                }
            });
    }

    fn set_mode(&mut self, mode: Mode) {
        if mode != self.mode {
            telemetry::event(Event::ViewChanged {
                view: mode.screen(),
            });
        }
        self.mode = mode;
        if mode == Mode::Slice && self.slice_for_rev != self.mesh_rev {
            self.start_slice();
        }
    }

    fn param_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Parameters");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let all_closed = !self.params.is_empty()
                    && self
                        .params
                        .iter()
                        .all(|p| self.collapsed.contains(&p.group));
                let (icon, label, hover) = if all_closed {
                    (ui::Icon::Expand, "Expand", "Open all groups")
                } else {
                    (ui::Icon::Collapse, "Collapse", "Close all groups")
                };
                if ui::icon_text_button(ui, icon, label, 28.0)
                    .on_hover_text(hover)
                    .clicked()
                {
                    if all_closed {
                        self.collapsed.clear();
                    } else {
                        self.collapsed = self.params.iter().map(|p| p.group.clone()).collect();
                    }
                }
                let any = self.params.iter().any(|p| p.changed());
                let reset = ui
                    .add_enabled_ui(any, |ui| {
                        ui::icon_text_button(ui, ui::Icon::Reset, "Reset all", 28.0)
                            .on_hover_text("Set all values back to the model defaults")
                    })
                    .inner;
                if reset.clicked() {
                    for p in &mut self.params {
                        p.value = p.default.clone();
                    }
                    self.dirty_since = Some(Instant::now());
                    self.mark_set_edited();
                }
            });
        });
        ui.add_space(4.0);
        if self.file.is_none() {
            ui::card(ui, |ui| {
                ui.label("Open a .scad file, or drop one on the window.");
                if ui.add(ui::primary("Open file")).clicked() {
                    self.pick_file();
                }
            });
            return;
        }
        if !self.params.is_empty() {
            self.sets_bar(ui);
            ui.add_space(4.0);
        }
        if self.params.is_empty() {
            ui::card(ui, |ui| {
                ui.label(ui::dim("No Customizer parameters in this file."))
            });
        }

        let mut changed = false;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let mut groups: Vec<String> = Vec::new();
                for p in &self.params {
                    if !groups.contains(&p.group) {
                        groups.push(p.group.clone());
                    }
                }
                for g in groups {
                    let open = !self.collapsed.contains(&g);
                    let edited = self
                        .params
                        .iter()
                        .filter(|p| p.group == g && p.changed())
                        .count();
                    ui::card(ui, |ui| {
                        if ui::group_header(ui, &g, open, edited).clicked() {
                            if open {
                                self.collapsed.insert(g.clone());
                            } else {
                                self.collapsed.remove(&g);
                            }
                        }
                        if open {
                            ui.add_space(6.0);
                            for p in self.params.iter_mut().filter(|p| p.group == g) {
                                changed |= param_row(ui, p);
                            }
                        }
                    });
                    ui.add_space(4.0);
                }

                let defs = params::defines(&self.params);
                if !defs.is_empty() && ui.add(ui::ghost("Copy as -D arguments", false)).clicked() {
                    let s: Vec<String> = defs.iter().map(|d| format!("-D '{d}'")).collect();
                    ui.ctx().copy_text(s.join(" "));
                    self.toast = Some(("Copied -D arguments".into(), Instant::now()));
                }
                ui.add_space(12.0);
            });
        if changed {
            self.dirty_since = Some(Instant::now());
            self.mark_set_edited();
        }
    }

    /// The sets file card (name, Open and the gear menu) and below it the
    /// set list with Save. The set list sits directly above the parameter
    /// groups, because the set gives their values.
    fn sets_bar(&mut self, ui: &mut egui::Ui) {
        let Some(file) = self.file.clone() else {
            return;
        };
        let path = self.sets_file.clone();
        let mut msg: Option<String> = None;
        let mut switch_to: Option<PathBuf> = None;
        let dir = path
            .parent()
            .filter(|d| d.is_dir())
            .or(file.parent())
            .map(|d| d.to_path_buf());
        let mut gear_rect = egui::Rect::NOTHING;
        ui::card(ui, |ui| {
            ui.horizontal(|ui| {
                let name = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                let text = if path.exists() {
                    name
                } else {
                    format!("{name} (new)")
                };
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let gear = ui::icon_button(ui, ui::Icon::Gear, 28.0, self.sets_menu)
                        .on_hover_text("New set, new file, delete");
                    if gear.clicked() {
                        self.sets_menu = !self.sets_menu;
                    }
                    gear_rect = gear.rect;
                    if ui::icon_text_button(ui, ui::Icon::Open, "Open", 28.0)
                        .on_hover_text("Open a sets file (.json)")
                        .clicked()
                    {
                        let mut d = rfd::FileDialog::new().add_filter("Parameter sets", &["json"]);
                        if let Some(dir) = &dir {
                            d = d.set_directory(dir);
                        }
                        switch_to = d.pick_file();
                    }
                    // The name gets the space the buttons leave, cut short when needed.
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add(
                            egui::Label::new(RichText::new(text).size(13.0).color(ui::c().dim))
                                .truncate(),
                        )
                        .on_hover_text(path.display().to_string());
                    });
                });
            });
        });
        ui.add_space(4.0);

        let mut pick: Option<Option<String>> = None;
        let mut new_set = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let label = match &self.active_set {
                Some(n) if self.set_edited => format!("{n} (edited)"),
                Some(n) => n.clone(),
                None => "Model defaults".into(),
            };
            // Height of a button row. The Save button is a square of this size.
            let sp = ui.spacing();
            let row = (ui.text_style_height(&egui::TextStyle::Button) + 2.0 * sp.button_padding.y)
                .max(sp.interact_size.y);
            // Show 4.5 rows. The half row tells the user that the list scrolls.
            let list_height = 4.0 * (row + sp.item_spacing.y) + 0.5 * row;
            let width = ui.available_width() - row - sp.item_spacing.x;
            // egui keeps the popup size of the first open. A new id for each
            // number of sets lets the popup take the new size.
            egui::ComboBox::from_id_salt(("sets", self.sets.len()))
                .selected_text(label)
                .width(width)
                .height(list_height)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(self.active_set.is_none(), "Model defaults")
                        .clicked()
                    {
                        pick = Some(None);
                    }
                    for n in &self.sets {
                        if ui
                            .selectable_label(self.active_set.as_ref() == Some(n), n)
                            .clicked()
                        {
                            pick = Some(Some(n.clone()));
                        }
                    }
                });
            let edited = self.set_edited && self.active_set.is_some();
            let save = ui::icon_button(ui, ui::Icon::Save, row, edited).on_hover_text(match &self
                .active_set
            {
                Some(n) => format!("Save the values to {n}"),
                None => "Save the values as a new set".into(),
            });
            if save.clicked() {
                match self.active_set.clone() {
                    Some(n) => match presets::save(&path, &n, &self.params) {
                        Ok(()) => {
                            self.set_edited = false;
                            self.autosave_at = None;
                            msg = Some(format!("Saved {n}"));
                        }
                        Err(e) => msg = Some(e.to_string()),
                    },
                    None => new_set = true,
                }
            }
        });

        // The gear menu, under the gear button, right aligned.
        let mut new_file = false;
        let mut delete = false;
        if self.sets_menu {
            let ctx = ui.ctx().clone();
            let area = egui::Area::new(egui::Id::new("sets_menu"))
                .order(egui::Order::Foreground)
                .pivot(egui::Align2::RIGHT_TOP)
                .fixed_pos(gear_rect.right_bottom() + Vec2::new(0.0, 4.0))
                .show(&ctx, |ui| {
                    ui::menu_frame_width(ui, 200.0, |ui| {
                        new_set |= ui::menu_item(ui, "New set…", "", true, false)
                            .on_hover_text("Save the values as a new set")
                            .clicked();
                        new_file = ui::menu_item(ui, "New file…", "", true, false)
                            .on_hover_text("Choose where to save a new sets file")
                            .clicked();
                        ui::menu_separator(ui);
                        let (text, on) = match &self.active_set {
                            Some(n) => (format!("Delete {n}…"), true),
                            None => ("Delete set…".to_string(), false),
                        };
                        delete = ui::menu_item(ui, &text, "", on, false)
                            .on_disabled_hover_text("Select a set first")
                            .clicked();
                    })
                });
            let outside = ctx.input(|i| {
                i.pointer.any_pressed()
                    && i.pointer
                        .interact_pos()
                        .is_some_and(|p| !area.response.rect.contains(p) && !gear_rect.contains(p))
            });
            let escape = ctx.input(|i| i.key_pressed(egui::Key::Escape));
            if new_set || new_file || delete || outside || escape {
                self.sets_menu = false;
            }
        }

        if let Some(choice) = pick {
            self.flush_autosave();
            match &choice {
                None => {
                    for p in &mut self.params {
                        p.value = p.default.clone();
                    }
                }
                Some(n) => match presets::apply(&path, n, &mut self.params) {
                    Ok(skipped) if !skipped.is_empty() => {
                        msg = Some(format!("Skipped: {}", skipped.join(", ")));
                    }
                    Ok(_) => {}
                    Err(e) => msg = Some(e.to_string()),
                },
            }
            self.set_active(choice);
            self.dirty_since = Some(Instant::now());
        }
        if new_set {
            self.flush_autosave();
            self.new_set = Some(String::new());
            self.confirm_delete = false;
        }
        if new_file {
            let mut d = rfd::FileDialog::new()
                .add_filter("Parameter sets", &["json"])
                .set_file_name(path.file_name().unwrap_or_default().to_string_lossy());
            if let Some(dir) = &dir {
                d = d.set_directory(dir);
            }
            switch_to = d.save_file();
        }
        if delete {
            self.confirm_delete = true;
            self.new_set = None;
        }
        if let Some(p) = switch_to {
            self.flush_autosave();
            if let Err(e) = presets::remember(&file, &p) {
                msg = Some(format!("Cannot remember the sets file: {e}"));
            }
            self.sets_file = p.clone();
            self.sets_mtime = mtime(&p);
            self.active_set = None;
            self.set_edited = false;
            self.confirm_delete = false;
            self.new_set = None;
            let n = presets::names(&p).len();
            msg = msg.or(Some(format!(
                "{n} sets in {}",
                p.file_name().unwrap_or_default().to_string_lossy()
            )));
        }
        if msg.is_some() {
            self.sets = presets::names(&self.sets_file);
        }
        if let Some(m) = msg {
            self.toast = Some((m, Instant::now()));
        }
    }

    /// The frame of a small modal window.
    fn dialog_frame() -> egui::Frame {
        egui::Frame::NONE
            .fill(ui::c().panel)
            .corner_radius(14)
            .inner_margin(egui::Margin::same(18))
            .stroke(egui::Stroke::new(1.0_f32, ui::c().line))
    }

    /// The window that asks for the name of a new set.
    fn new_set_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut name) = self.new_set.take() else {
            return;
        };
        let file = self
            .sets_file
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let mut submit = false;
        let mut cancel = false;
        let modal = egui::Modal::new(egui::Id::new("new_set"))
            .frame(Self::dialog_frame())
            .show(ctx, |ui| {
                ui.set_width(360.0);
                ui.heading("New parameter set");
                ui.add_space(4.0);
                ui.label(ui::dim(format!("Saves the current values in {file}.")));
                ui.add_space(10.0);
                let r = ui.add(
                    egui::TextEdit::singleline(&mut name)
                        .hint_text("Name of the set")
                        .desired_width(f32::INFINITY)
                        .margin(Vec2::new(10.0, 7.0)),
                );
                // Enter takes the focus away. Read that before the focus
                // request, which gives the focus back.
                submit = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if !submit && ui.memory(|m| m.focused().is_none()) {
                    r.request_focus();
                }
                let trimmed = name.trim();
                if self.sets.iter().any(|s| s == trimmed) {
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(format!("A set {trimmed} exists. Save replaces it."))
                            .color(ui::ACCENT)
                            .size(12.5),
                    );
                }
                ui.add_space(12.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().button_padding = Vec2::new(16.0, 7.0);
                    submit |= ui
                        .add_enabled(!trimmed.is_empty(), ui::primary("Save"))
                        .clicked();
                    cancel |= ui.add(ui::ghost("Cancel", false)).clicked();
                });
            });
        cancel |= modal.should_close();
        let name = name.trim().to_string();
        if submit && !name.is_empty() {
            match presets::save(&self.sets_file, &name, &self.params) {
                Ok(()) => {
                    self.toast = Some((format!("Saved {name}"), Instant::now()));
                    self.sets = presets::names(&self.sets_file);
                    self.set_active(Some(name));
                }
                Err(e) => self.toast = Some((e.to_string(), Instant::now())),
            }
        } else if !cancel {
            self.new_set = Some(name);
        }
    }

    /// The window that confirms the delete of the active set.
    fn delete_set_dialog(&mut self, ctx: &egui::Context) {
        if !self.confirm_delete {
            return;
        }
        let Some(n) = self.active_set.clone() else {
            self.confirm_delete = false;
            return;
        };
        let file = self
            .sets_file
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let mut delete = false;
        let mut cancel = false;
        let modal = egui::Modal::new(egui::Id::new("delete_set"))
            .frame(Self::dialog_frame())
            .show(ctx, |ui| {
                ui.set_width(360.0);
                ui.heading("Delete parameter set");
                ui.add_space(4.0);
                ui.label(format!("Delete {n} from {file}? You cannot undo this."));
                ui.add_space(12.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().button_padding = Vec2::new(16.0, 7.0);
                    let b = egui::Button::new(RichText::new("Delete").color(Color32::BLACK))
                        .fill(ui::c().danger)
                        .corner_radius(8);
                    delete = ui.add(b).clicked();
                    cancel = ui.add(ui::ghost("Cancel", false)).clicked();
                });
            });
        cancel |= modal.should_close();
        if delete {
            match presets::delete(&self.sets_file, &n) {
                Ok(()) => {
                    self.toast = Some((format!("Deleted {n}"), Instant::now()));
                    self.sets = presets::names(&self.sets_file);
                    self.set_active(None);
                }
                Err(e) => {
                    self.toast = Some((e.to_string(), Instant::now()));
                    self.confirm_delete = false;
                }
            }
        } else if cancel {
            self.confirm_delete = false;
        }
    }

    /// Thin bar at the bottom: render state, engine, model, mode details.
    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let small = |t: String| RichText::new(t).size(12.5);
        let dim = |t: String| RichText::new(t).size(12.5).color(ui::c().dim);
        let sep = |ui: &mut egui::Ui| {
            let (r, _) = ui.allocate_exact_size(Vec2::new(1.0, 14.0), egui::Sense::hover());
            ui.painter().rect_filled(r, 0.0, ui::c().line);
        };
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            ui.spacing_mut().button_padding = Vec2::new(8.0, 2.0);

            // Render state.
            if let Some(job) = &self.job {
                ui.add(egui::Spinner::new().size(12.0).color(ui::ACCENT));
                ui.label(
                    small(format!(
                        "Rendering {:.1} s",
                        job.started.elapsed().as_secs_f32()
                    ))
                    .color(ui::ACCENT),
                );
            } else if let Some(e) = &self.error {
                let r = ui.add(
                    egui::Label::new(small(format!("Error: {e}")).color(ui::c().danger))
                        .sense(egui::Sense::click()),
                );
                if r.on_hover_text("Show the OpenSCAD output").clicked() {
                    self.show_log = true;
                }
            } else if self.dirty_since.is_some() && !self.auto_render {
                ui.label(small("Parameters changed. Press F5.".into()).color(ui::ACCENT));
            } else if self.file.is_some() {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(8.0), egui::Sense::hover());
                ui.painter().circle_filled(r.center(), 4.0, ui::GOOD);
                let t = self
                    .last_time
                    .map(|t| format!("Ready  ·  {:.2} s", t.as_secs_f32()))
                    .unwrap_or("Ready".into());
                ui.label(small(t));
            } else {
                ui.label(dim("No file".into()));
            }

            // Engine.
            sep(ui);
            let probe = openscad::probe(&self.cfg.binary());
            let version = probe
                .version
                .trim_start_matches("OpenSCAD version ")
                .to_string();
            let backend = if self.engine.is_empty() {
                String::new()
            } else {
                format!("  ·  {}", self.engine)
            };
            ui.label(dim(format!("OpenSCAD {version}{backend}")))
                .on_hover_text(self.cfg.binary());

            // Model.
            if let Some(i) = &self.info {
                sep(ui);
                let s = i.bounds.size();
                let tris = self.mesh.as_ref().map(|m| m.triangles.len()).unwrap_or(0);
                ui.label(small(format!("{:.1} × {:.1} × {:.1} mm", s.x, s.y, s.z)));
                ui.label(dim(format!(
                    "{tris} triangles  ·  {:.2} cm³",
                    i.volume / 1000.0
                )));
                if i.open_edges > 0 {
                    ui.label(small(format!("{} open edges", i.open_edges)).color(ui::ACCENT))
                        .on_hover_text(
                            "The mesh is not closed. Slicing and STEP export may be wrong.",
                        );
                }
            }

            // Mode details.
            match self.mode {
                Mode::Slice => {
                    if let Some(sl) = &self.sliced {
                        sep(ui);
                        ui.label(small(format!(
                            "Layer {} / {}",
                            self.layer + 1,
                            sl.layers.len()
                        )));
                        ui.label(dim(format!(
                            "{}  ·  {:.1} g",
                            fmt_duration(sl.stats.seconds),
                            sl.stats.grams
                        )));
                    }
                }
                Mode::Photo => {
                    sep(ui);
                    ui.label(small(format!(
                        "Photo {} / {}",
                        self.photo_samples(),
                        self.photo.samples
                    )));
                    ui.label(dim(format!("{} × {}", self.photo.width, self.photo.height)));
                }
                Mode::Model => {}
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(dim(format!("v{}", env!("CARGO_PKG_VERSION"))));
                if ui
                    .add(ui::ghost("Console", self.show_log))
                    .on_hover_text("OpenSCAD output")
                    .clicked()
                {
                    self.show_log = !self.show_log;
                }
                if let Some(n) = &self.active_set {
                    ui.label(dim(format!(
                        "Set: {n}{}",
                        if self.set_edited { " (edited)" } else { "" }
                    )));
                }
            });
        });
    }

    fn photo_look(&self) -> pathtrace::Look {
        let p = &self.photo;
        let hex = (p.color[0] as u32) << 16 | (p.color[1] as u32) << 8 | p.color[2] as u32;
        pathtrace::Look {
            color: pathtrace::srgb(hex),
            roughness: pathtrace::finish_roughness(p.finish),
            layer_lines: if p.layer_lines {
                self.settings.layer_height
            } else {
                0.0
            },
            studio: p.studio,
            exposure: p.exposure,
        }
    }

    /// Runs the photo render for the current view and draws it over the viewport.
    fn photo_view(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        if self.mesh.is_none() {
            return;
        }
        let size = (self.photo.width, self.photo.height);
        let frame = photo_frame(rect, size);
        // Same framing as the view: scale the field of view to the frame height.
        let fov = 2.0 * ((Camera::FOV * 0.5).tan() * frame.height() / rect.height()).atan();
        let key = PhotoKey {
            cam: pathtrace::Camera {
                eye: self.cam.eye(),
                target: self.cam.target,
                fov,
            },
            look: self.photo_look(),
            mesh_rev: self.mesh_rev,
            model: self.photo_model_key(),
            size,
            samples: self.photo.samples,
        };
        if self.photo_key != Some(key) {
            // Wait until the camera and settings rest, then start again.
            match self.photo_want {
                Some((k, at)) if k == key => {
                    if at.elapsed() > Duration::from_millis(250)
                        && let Some(model) = self.photo_model()
                    {
                        self.photo_job = Some(pathtrace::Job::start(
                            model,
                            key.cam,
                            key.look,
                            size.0,
                            size.1,
                            key.samples,
                        ));
                        self.photo_key = Some(key);
                        self.photo_tex = None;
                        self.photo_started = Instant::now();
                        self.photo_want = None;
                    }
                }
                _ => {
                    self.photo_job = None;
                    self.photo_want = Some((key, Instant::now()));
                }
            }
            ui.ctx().request_repaint_after(Duration::from_millis(60));
        }
        if let Some(job) = &self.photo_job {
            let f = job.frame.lock().unwrap();
            let have = self.photo_tex.as_ref().map(|t| t.1).unwrap_or(0);
            if f.samples > have && !f.rgba.is_empty() {
                let img = egui::ColorImage::from_rgba_unmultiplied([f.width, f.height], &f.rgba);
                match &mut self.photo_tex {
                    Some((tex, n)) => {
                        tex.set(img, egui::TextureOptions::LINEAR);
                        *n = f.samples;
                    }
                    None => {
                        let tex = ui
                            .ctx()
                            .load_texture("photo", img, egui::TextureOptions::LINEAR);
                        self.photo_tex = Some((tex, f.samples));
                    }
                }
            }
            if f.samples < self.photo.samples {
                ui.ctx().request_repaint_after(Duration::from_millis(120));
            }
        }
        // Dim the view outside the frame, then draw the render into it.
        let shade = if ui::is_dark() {
            Color32::from_black_alpha(170)
        } else {
            Color32::from_black_alpha(90)
        };
        let p = ui.painter();
        for r in [
            egui::Rect::from_min_max(rect.min, egui::pos2(rect.max.x, frame.min.y)),
            egui::Rect::from_min_max(egui::pos2(rect.min.x, frame.max.y), rect.max),
            egui::Rect::from_min_max(
                egui::pos2(rect.min.x, frame.min.y),
                egui::pos2(frame.min.x, frame.max.y),
            ),
            egui::Rect::from_min_max(
                egui::pos2(frame.max.x, frame.min.y),
                egui::pos2(rect.max.x, frame.max.y),
            ),
        ] {
            p.rect_filled(r, 0.0, shade);
        }
        if let (Some((tex, _)), Some(_)) = (&self.photo_tex, &self.photo_job) {
            p.image(
                tex.id(),
                frame,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        p.rect_stroke(
            frame,
            0.0,
            egui::Stroke::new(1.0_f32, ui::ACCENT.gamma_multiply(0.7)),
            egui::StrokeKind::Outside,
        );
        ui::pill(
            p,
            egui::pos2(frame.center().x, frame.top() - 6.0),
            egui::Align2::CENTER_BOTTOM,
            &format!("{} × {}", size.0, size.1),
            egui::FontId::proportional(12.5),
            ui::c().text,
            ui::glass(),
        );
    }

    fn photo_samples(&self) -> u32 {
        self.photo_job
            .as_ref()
            .map(|j| j.frame.lock().unwrap().samples)
            .unwrap_or(0)
    }

    fn save_photo(&mut self) {
        let Some(job) = &self.photo_job else { return };
        let mut d = rfd::FileDialog::new()
            .add_filter("PNG image", &["png"])
            .set_file_name(format!("{}-photo.png", self.stem()));
        if let Some(dir) = self.export_dir() {
            d = d.set_directory(dir);
        }
        let Some(path) = d.save_file() else { return };
        recent::set_export_dir(&path);
        let f = job.frame.lock().unwrap();
        let msg = match pathtrace::save_png(&path, &f) {
            Ok(()) => {
                telemetry::event(Event::PhotoSaved { samples: f.samples });
                format!("Saved {} ({} samples)", path.display(), f.samples)
            }
            Err(e) => format!("Cannot save: {e}"),
        };
        drop(f);
        self.toast = Some((msg, Instant::now()));
    }

    fn photo_panel(&mut self, ui: &mut egui::Ui) {
        ui.label(ui::dim("Path-traced render of the current view."));
        ui.add_space(4.0);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let samples = self.photo_samples();
                ui::card(ui, |ui| {
                    ui::section(ui, "Progress");
                    let target = self.photo.samples;
                    let waiting = self.photo_job.is_none() && self.mesh.is_some();
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("{samples}")).size(22.0));
                        ui.label(ui::dim(format!("of {target} samples")));
                    });
                    ui.add(
                        egui::ProgressBar::new(samples as f32 / target.max(1) as f32)
                            .desired_height(6.0)
                            .fill(ui::ACCENT),
                    );
                    let status = if waiting {
                        "Waiting for the view to rest…".to_string()
                    } else if samples >= target {
                        format!(
                            "Done in {:.0} s",
                            self.photo_started.elapsed().as_secs_f32()
                        )
                    } else {
                        format!("{:.0} s", self.photo_started.elapsed().as_secs_f32())
                    };
                    ui.label(ui::dim(status));
                    ui.add_space(4.0);
                    if ui
                        .add_enabled(samples > 0, ui::primary("Save PNG"))
                        .clicked()
                    {
                        self.save_photo();
                    }
                });
                ui.add_space(4.0);

                let part_colors = self.view3d.part_colors;
                let p = &mut self.photo;
                ui::card(ui, |ui| {
                    ui::section(ui, "Material");
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
                        for (name, c) in SWATCHES {
                            let (r, resp) =
                                ui.allocate_exact_size(Vec2::splat(26.0), egui::Sense::click());
                            let col = Color32::from_rgb(c[0], c[1], c[2]);
                            ui.painter().circle_filled(r.center(), 12.0, col);
                            // Outline, so black and white swatches show on both themes.
                            ui.painter().circle_stroke(
                                r.center(),
                                12.0,
                                egui::Stroke::new(1.0_f32, ui::c().line),
                            );
                            if p.color == c {
                                ui.painter().circle_stroke(
                                    r.center(),
                                    12.5,
                                    egui::Stroke::new(2.0_f32, ui::c().text),
                                );
                            }
                            if resp.on_hover_text(name).clicked() {
                                p.color = c;
                            }
                        }
                        ui.color_edit_button_srgb(&mut p.color);
                    });
                    if part_colors {
                        ui.label(ui::dim("Part colors are on. Each part has its own color."));
                    }
                    ui.add_space(4.0);
                    ui.label("Finish");
                    let mut finish = p.finish;
                    ui::segmented(
                        ui,
                        &mut finish,
                        &[("matte", "Matte"), ("satin", "Satin"), ("gloss", "Gloss")],
                    );
                    p.finish = finish;
                    ui.horizontal(|ui| {
                        ui.label("Print layer lines");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui::toggle(ui, &mut p.layer_lines);
                        });
                    });
                });
                ui.add_space(4.0);
                self.parts_card(ui);
                self.explode_card(ui);
                let p = &mut self.photo;
                ui::card(ui, |ui| {
                    ui::section(ui, "Scene");
                    ui.label("Studio");
                    let mut studio = p.studio;
                    ui::segmented(
                        ui,
                        &mut studio,
                        &[
                            (pathtrace::Studio::Dark, "Dark"),
                            (pathtrace::Studio::Light, "Light"),
                        ],
                    );
                    p.studio = studio;
                    ui.add_space(4.0);
                    setting(ui, "Exposure", &mut p.exposure, 0.3..=3.0, 0.05, "");
                });
                ui.add_space(4.0);
                ui::card(ui, |ui| {
                    ui::section(ui, "Quality");
                    setting(ui, "Samples", &mut p.samples, 16..=2048, 16.0, "");
                    ui.add_space(4.0);
                    ui.label("Resolution");
                    let current = RESOLUTIONS
                        .iter()
                        .find(|r| r.1 == p.width && r.2 == p.height)
                        .map_or("Custom".to_string(), |r| r.0.to_string());
                    egui::ComboBox::from_id_salt("resolution")
                        .selected_text(current)
                        .width(ui.available_width())
                        .show_ui(ui, |ui| {
                            for (name, w, h) in RESOLUTIONS {
                                let sel = p.width == w && p.height == h;
                                if ui
                                    .selectable_label(sel, format!("{name}   {w} × {h}"))
                                    .clicked()
                                {
                                    p.width = w;
                                    p.height = h;
                                }
                            }
                        });
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::DragValue::new(&mut p.width)
                                .range(64..=8192)
                                .speed(8)
                                .suffix(" px"),
                        );
                        ui.label(ui::dim("×"));
                        ui.add(
                            egui::DragValue::new(&mut p.height)
                                .range(64..=8192)
                                .speed(8)
                                .suffix(" px"),
                        );
                        if ui
                            .add(ui::ghost("Swap", false))
                            .on_hover_text("Landscape or portrait")
                            .clicked()
                        {
                            std::mem::swap(&mut p.width, &mut p.height);
                        }
                    });
                    let mp = (p.width * p.height) as f32 / 1e6;
                    ui.label(ui::dim(format!("{mp:.1} megapixels")));
                    ui.label(ui::dim(
                        "Move the view to start again. The render uses all cores.",
                    ));
                });
            });
    }

    fn slice_panel(&mut self, ui: &mut egui::Ui) {
        ui.label(ui::dim("Rough preview for checking. Not for printing."));
        ui.add_space(4.0);
        let before = self.settings;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if let Some(sl) = &self.sliced {
                    let st = &sl.stats;
                    ui::card(ui, |ui| {
                        ui::section(ui, "Estimate");
                        ui.label(
                            RichText::new(fmt_duration(st.seconds))
                                .size(30.0)
                                .color(ui::ACCENT),
                        );
                        ui.columns(3, |c| {
                            stat(&mut c[0], &format!("{:.1} g", st.grams), "weight");
                            stat(&mut c[1], &format!("{:.2} m", st.filament_m), "filament");
                            stat(&mut c[2], &st.layers.to_string(), "layers");
                        });
                    });
                    ui.add_space(4.0);
                    ui::card(ui, |ui| {
                        ui::section(ui, "Layer");
                        if let Some(l) = sl.layers.get(self.layer) {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(format!("{}", self.layer + 1)).size(22.0));
                                ui.label(ui::dim(format!(
                                    "of {}   ·   z = {:.2} mm",
                                    sl.layers.len(),
                                    l.z
                                )));
                            });
                        }
                        ui.add_space(2.0);
                        egui::Grid::new("legend")
                            .num_columns(2)
                            .spacing([16.0, 8.0])
                            .show(ui, |ui| {
                                for (i, k) in
                                    [Kind::OuterWall, Kind::InnerWall, Kind::Infill, Kind::Solid]
                                        .into_iter()
                                        .enumerate()
                                {
                                    ui.horizontal(|ui| {
                                        let (r, _) = ui.allocate_exact_size(
                                            Vec2::new(10.0, 10.0),
                                            egui::Sense::hover(),
                                        );
                                        ui.painter().circle_filled(
                                            r.center(),
                                            5.0,
                                            rgb(render::kind_color(k)),
                                        );
                                        ui.label(
                                            RichText::new(match k {
                                                Kind::OuterWall => "Outer wall",
                                                Kind::InnerWall => "Inner wall",
                                                Kind::Infill => "Sparse infill",
                                                Kind::Solid => "Solid infill",
                                            })
                                            .size(13.0),
                                        );
                                    });
                                    if i % 2 == 1 {
                                        ui.end_row();
                                    }
                                }
                            });
                        ui.label(ui::dim("Up/Down or PgUp/PgDn change the layer."));
                    });
                    ui.add_space(4.0);
                } else if self.slice_job.is_some() {
                    ui::card(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Slicing…");
                        });
                    });
                }

                let s = &mut self.settings;
                ui::card(ui, |ui| {
                    ui::section(ui, "Print settings");
                    setting(
                        ui,
                        "Layer height",
                        &mut s.layer_height,
                        0.05..=0.6,
                        0.01,
                        " mm",
                    );
                    setting(
                        ui,
                        "First layer",
                        &mut s.first_layer,
                        0.05..=0.6,
                        0.01,
                        " mm",
                    );
                    setting(ui, "Line width", &mut s.line_width, 0.2..=1.2, 0.01, " mm");
                    setting(ui, "Walls", &mut s.walls, 0..=6, 1.0, "");
                    let mut pct = s.infill * 100.0;
                    if setting(ui, "Infill", &mut pct, 0.0..=100.0, 1.0, " %") {
                        s.infill = pct / 100.0;
                    }
                    setting(
                        ui,
                        "Top and bottom layers",
                        &mut s.solid_layers,
                        0..=10,
                        1.0,
                        "",
                    );
                    setting(ui, "Speed", &mut s.speed, 10.0..=300.0, 5.0, " mm/s");
                    ui.horizontal(|ui| {
                        ui.label("Show model ghost");
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui::toggle(ui, &mut self.ghost);
                        });
                    });
                });
                if let Some(t) = self.slice_time {
                    ui.add_space(4.0);
                    ui.label(ui::dim(format!("Sliced in {} ms", t.as_millis())));
                }
            });
        if self.settings != before {
            self.slice_dirty_since = Some(Instant::now());
        }
    }

    fn viewport(&mut self, ui: &mut egui::Ui) {
        let rect = ui.available_rect_before_wrap();
        let resp = ui.allocate_rect(rect, egui::Sense::click_and_drag());

        // Background gradient.
        let mut bg = egui::Mesh::default();
        let [top, bottom] = ui::c().view;
        bg.colored_vertex(rect.left_top(), top);
        bg.colored_vertex(rect.right_top(), top);
        bg.colored_vertex(rect.right_bottom(), bottom);
        bg.colored_vertex(rect.left_bottom(), bottom);
        bg.add_triangle(0, 1, 2);
        bg.add_triangle(0, 2, 3);
        ui.painter().add(egui::Shape::mesh(bg));

        // Camera input.
        if resp.dragged_by(egui::PointerButton::Primary) && !ui.input(|i| i.modifiers.shift) {
            let d = resp.drag_delta();
            self.cam.yaw -= d.x * 0.008;
            self.cam.pitch = (self.cam.pitch + d.y * 0.008).clamp(-1.5697, 1.5697);
        } else if resp.dragged_by(egui::PointerButton::Secondary)
            || resp.dragged_by(egui::PointerButton::Middle)
            || (resp.dragged_by(egui::PointerButton::Primary) && ui.input(|i| i.modifiers.shift))
        {
            self.cam.pan(resp.drag_delta(), rect.height());
        }
        if resp.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                self.cam.dist = (self.cam.dist * (-scroll * 0.0015).exp()).clamp(0.5, 20_000.0);
            }
        }
        self.select_input(&resp, rect);
        if resp.double_clicked() {
            self.fit();
        }
        self.update_selection();

        let layer_max = self.sliced.as_ref().map(|s| s.layers.len()).unwrap_or(0);
        let slicing = self.mode == Mode::Slice && layer_max > 0;
        if let Some(r) = &self.renderer {
            let view = View {
                mvp: self.cam.matrix(rect.width() / rect.height().max(1.0)),
                eye: self.cam.eye(),
                show_mesh: !slicing || self.ghost,
                mesh_alpha: if slicing { 0.12 } else { 1.0 },
                parts: self.part_styles(),
                wireframe: self.wireframe,
                grid: self.grid,
                layer: slicing.then_some(self.layer),
                section: self.section_view(),
            };
            let (assembly, assembly_rev) = (self.assembly.clone(), self.assembly_rev);
            let (sliced, slice_rev) = (self.sliced.clone(), self.slice_rev);
            let bounds = self.view_bounds();
            let dark = ui::is_dark();
            let r = r.clone();
            let cb = egui::PaintCallback {
                rect,
                callback: Arc::new(eframe::egui_glow::CallbackFn::new(move |_info, painter| {
                    let gl = painter.gl();
                    let mut r = r.lock().unwrap();
                    r.set_mesh(gl, assembly.as_deref(), assembly_rev);
                    r.set_slice(gl, sliced.as_deref(), slice_rev);
                    let b = bounds.unwrap_or(Bounds {
                        min: Vec3::splat(-50.0),
                        max: Vec3::splat(50.0),
                    });
                    r.set_grid(gl, b.min, b.max, dark);
                    r.paint(gl, &view);
                })),
            };
            ui.painter().add(cb);
        } else {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "OpenGL renderer not available",
                egui::FontId::proportional(16.0),
                ui::c().danger,
            );
        }

        if self.mode == Mode::Photo {
            self.photo_view(ui, rect);
        } else {
            self.photo_job = None;
            self.photo_key = None;
        }

        let m = 16.0;
        // View toolbar, top right.
        let bar = egui::Rect::from_min_size(
            egui::pos2(rect.right() - m - 430.0, rect.bottom() - m - 40.0),
            Vec2::new(430.0, 40.0),
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(bar)
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
            |ui| {
                egui::Frame::NONE
                    .fill(ui::glass())
                    .corner_radius(10)
                    .inner_margin(egui::Margin::same(4))
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        ui.spacing_mut().button_padding = Vec2::new(10.0, 5.0);
                        ui.horizontal(|ui| {
                            if ui.add(ui::ghost("Grid", self.grid)).clicked() {
                                self.grid = !self.grid;
                            }
                            if ui
                                .add(ui::ghost("Wire", self.wireframe))
                                .on_hover_text("W")
                                .clicked()
                            {
                                self.wireframe = !self.wireframe;
                            }
                            if ui
                                .add(ui::ghost("Fit", false))
                                .on_hover_text("F or double click")
                                .clicked()
                            {
                                self.fit();
                            }
                            for (label, yaw, pitch) in VIEWS.into_iter().rev() {
                                if ui.add(ui::ghost(label, false)).clicked() {
                                    self.set_view(yaw, pitch);
                                }
                            }
                        });
                    });
            },
        );

        // Layer slider, right edge.
        if slicing {
            let strip = egui::Rect::from_min_max(
                egui::pos2(rect.right() - m - 36.0, rect.top() + 70.0),
                egui::pos2(rect.right() - m, rect.bottom() - 100.0),
            );
            ui.painter()
                .rect_filled(strip.expand2(Vec2::new(0.0, 14.0)), 18.0, ui::glass());
            self.layer = ui::vertical_slider(ui, strip, self.layer, layer_max);
            ui::pill(
                ui.painter(),
                egui::pos2(strip.center().x, strip.top() - 22.0),
                egui::Align2::CENTER_BOTTOM,
                &format!("{}", self.layer + 1),
                egui::FontId::proportional(14.0),
                Color32::BLACK,
                ui::ACCENT,
            );
        }

        let painter = ui.painter_at(rect);
        let font = egui::FontId::proportional(13.5);

        // Empty view: say why.
        if self.mesh.is_none() && self.job.is_none() {
            let msg = match (&self.file, &self.error) {
                (None, _) if !self.recent.is_empty() => None,
                (None, _) => Some("Open a .scad file, or drop one here.".to_string()),
                (Some(_), Some(e)) => Some(format!("Nothing to show.  {e}")),
                _ => None,
            };
            if let Some(msg) = msg {
                ui::pill(
                    &painter,
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    &msg,
                    egui::FontId::proportional(15.0),
                    ui::c().text,
                    ui::glass(),
                );
            }
        }

        // Start view: the recent models.
        if self.file.is_none() && !self.recent.is_empty() {
            let size = Vec2::new(420.0, 86.0 + 29.0 * self.recent.len() as f32);
            let area = egui::Rect::from_center_size(rect.center(), size);
            let mut pick = None;
            ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
                ui::card(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui::section(ui, "Recent models");
                    ui.add_space(8.0);
                    for path in &self.recent {
                        if recent_row(ui, path, false) {
                            pick = Some(path.clone());
                        }
                    }
                });
            });
            if let Some(path) = pick {
                self.open(path);
            }
        }

        // Toast, bottom centre.
        let toast = if self.export_rx.is_some() {
            Some("Exporting…".to_string())
        } else {
            self.toast
                .as_ref()
                .filter(|(_, at)| at.elapsed() < Duration::from_secs(4))
                .map(|(m, _)| m.clone())
        };
        if let Some(msg) = toast {
            ui::pill(
                &painter,
                rect.center_bottom() + Vec2::new(0.0, -m),
                egui::Align2::CENTER_BOTTOM,
                &msg,
                font,
                ui::c().text,
                ui::glass(),
            );
            ui.ctx().request_repaint_after(Duration::from_millis(200));
        }
        self.part_menu(ui.ctx(), rect);
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if let Some(p) = ctx.input(|i| i.raw.dropped_files.iter().find_map(|f| f.path.clone())) {
            self.open(p);
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Q)) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        // The tour uses the arrow keys, and the app waits behind it.
        if self.tour.is_some() || self.consent.is_some() {
            return;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::O)) {
            self.pick_file();
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Comma)) {
            self.open_settings();
        }
        for (key, menu) in [
            (egui::Key::F, Menu::File),
            (egui::Key::M, Menu::Model),
            (egui::Key::V, Menu::View),
            (egui::Key::H, Menu::Help),
        ] {
            if ctx.input_mut(|i| i.consume_key(egui::Modifiers::ALT, key)) {
                self.menu = if self.menu == Some(menu) {
                    None
                } else {
                    Some(menu)
                };
            }
        }
        if ctx.wants_keyboard_input() {
            return;
        }
        self.select_escape(ctx);
        let (f5, f, w, up, down, pgup, pgdn, tab) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::F5),
                i.key_pressed(egui::Key::F),
                i.key_pressed(egui::Key::W),
                i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::PageUp),
                i.key_pressed(egui::Key::PageDown),
                i.key_pressed(egui::Key::S),
            )
        });
        if ctx.input(|i| i.key_pressed(egui::Key::E)) {
            self.toggle_explode();
        }
        if ctx.input(|i| i.key_pressed(egui::Key::P)) {
            self.set_mode(if self.mode == Mode::Photo {
                Mode::Model
            } else {
                Mode::Photo
            });
        }
        if f5 {
            self.start_render();
        }
        if f {
            self.fit();
        }
        if w {
            self.wireframe = !self.wireframe;
        }
        if tab {
            self.set_mode(if self.mode == Mode::Model {
                Mode::Slice
            } else {
                Mode::Model
            });
        }
        if self.mode == Mode::Slice {
            let max = self
                .sliced
                .as_ref()
                .map(|s| s.layers.len())
                .unwrap_or(1)
                .saturating_sub(1);
            if up {
                self.layer = (self.layer + 1).min(max);
            }
            if down {
                self.layer = self.layer.saturating_sub(1);
            }
            if pgup {
                self.layer = (self.layer + 10).min(max);
            }
            if pgdn {
                self.layer = self.layer.saturating_sub(10);
            }
        }
    }
}

impl App {
    fn settings_window(&mut self, ctx: &egui::Context) {
        let Some(mut edit) = self.cfg_edit.take() else {
            return;
        };
        let mut close = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let mut save = false;
        let dirty = edit != self.cfg || self.privacy.dirty();
        let running = self.server.is_some();
        let frame = egui::Frame::NONE
            .fill(ui::c().panel)
            .corner_radius(14)
            .inner_margin(egui::Margin::same(18))
            .stroke(egui::Stroke::new(1.0_f32, ui::c().line));
        // A fixed size keeps the buttons in place when the page changes.
        let screen = ctx.screen_rect().size();
        let width = (screen.x - 80.0).clamp(420.0, 604.0);
        let height = (screen.y - 170.0).clamp(160.0, 360.0);
        let probe = settings_probe(ctx, &edit);
        let missing = probe.as_ref().is_some_and(|p| p.version == "not found");
        let mut page = self.settings_page;
        let mut pages = vec![
            (SettingsPage::Appearance, "Appearance", None),
            (SettingsPage::Sets, "Parameter sets", None),
            (
                SettingsPage::OpenScad,
                "OpenSCAD",
                missing.then_some(ui::c().danger),
            ),
            (SettingsPage::Mcp, "MCP server", running.then_some(ui::GOOD)),
            (SettingsPage::Updates, "Updates", None),
        ];
        if crate::telemetry::available() {
            pages.push((SettingsPage::Privacy, "Privacy", None));
        }
        egui::Modal::new(egui::Id::new("settings"))
            .frame(frame)
            .show(ctx, |ui| {
                ui.set_width(width);
                ui.horizontal(|ui| {
                    ui.heading("Settings");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        close |= ui
                            .add(
                                egui::Button::new(RichText::new("×").size(20.0).color(ui::c().dim))
                                    .frame(false),
                            )
                            .on_hover_text("Close (Esc)")
                            .clicked();
                    });
                });
                ui.add_space(10.0);
                let ltr = egui::Layout::left_to_right(egui::Align::Min);
                ui.allocate_ui_with_layout(Vec2::new(width, height), ltr, |ui| {
                    ui.set_min_height(height);
                    let nav = Vec2::new(140.0, height);
                    let down = egui::Layout::top_down(egui::Align::Min);
                    ui.allocate_ui_with_layout(nav, down, |ui| {
                        ui.set_width(nav.x);
                        ui.spacing_mut().item_spacing.y = 2.0;
                        for &(value, label, dot) in &pages {
                            if ui::nav_item(ui, label, page == value, dot).clicked() {
                                page = value;
                            }
                        }
                    });
                    ui.add_space(16.0);
                    let body = Vec2::new(ui.available_width(), height);
                    ui.allocate_ui_with_layout(body, down, |ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("settings-page")
                            .auto_shrink([false, false])
                            .show(ui, |ui| match page {
                                SettingsPage::Appearance => {
                                    ui::card(ui, |ui| theme_row(ui, &mut edit));
                                }
                                SettingsPage::Sets => {
                                    ui::card(ui, |ui| sets_settings(ui, &mut edit));
                                }
                                SettingsPage::OpenScad => {
                                    ui::card(ui, |ui| openscad_settings(ui, &mut edit, &probe));
                                }
                                SettingsPage::Mcp => {
                                    ui::card(ui, |ui| mcp_settings(ui, &mut edit, running));
                                }
                                SettingsPage::Updates => {
                                    ui::card(ui, |ui| update_settings(ui, &mut edit));
                                }
                                SettingsPage::Privacy => {
                                    ui::card(ui, |ui| privacy_settings(ui, &mut self.privacy));
                                }
                            });
                    });
                });
                ui.add_space(12.0);
                let row = Vec2::new(ui.available_width(), 34.0);
                let rtl = egui::Layout::right_to_left(egui::Align::Center);
                ui.allocate_ui_with_layout(row, rtl, |ui| {
                    ui.spacing_mut().button_padding = Vec2::new(16.0, 7.0);
                    save = ui
                        .add_enabled(dirty, ui::primary("Save"))
                        .on_disabled_hover_text("No changes")
                        .clicked();
                    close |= ui.add(ui::ghost("Cancel", false)).clicked();
                    if dirty {
                        let ltr = egui::Layout::left_to_right(egui::Align::Center);
                        ui.with_layout(ltr, |ui| ui.label(ui::dim("Unsaved changes").size(12.5)));
                    }
                });
            });
        self.settings_page = page;
        if save {
            if let Err(e) = edit.save() {
                self.toast = Some((format!("Cannot save settings: {e}"), Instant::now()));
            }
            let rerender = (&edit.binary, &edit.args, edit.manifold)
                != (&self.cfg.binary, &self.cfg.args, self.cfg.manifold);
            self.cfg = edit;
            if self.privacy.dirty() {
                let p = &self.privacy;
                if let Err(e) = telemetry::set_consent(p.analytics, p.errors, Source::Settings) {
                    self.toast = Some((format!("Cannot save your choice: {e}"), Instant::now()));
                }
            }
            self.sync_server(ctx);
            if rerender {
                self.start_render();
            }
        } else if !close {
            self.cfg_edit = Some(edit);
        }
    }
}

/// Id of the OpenSCAD binary field.
fn binary_id() -> egui::Id {
    egui::Id::new("settings-binary")
}

/// Checks the OpenSCAD binary, except while its name is being typed.
fn settings_probe(ctx: &egui::Context, edit: &openscad::Config) -> Option<openscad::Probe> {
    // A half typed name must not run as a program.
    let typing = ctx.memory(|m| m.has_focus(binary_id()));
    (!typing).then(|| openscad::probe(&edit.binary()))
}

/// The Appearance page of the Settings window.
fn theme_row(ui: &mut egui::Ui, edit: &mut openscad::Config) {
    let row = Vec2::new(ui.available_width(), 36.0);
    let rtl = egui::Layout::right_to_left(egui::Align::Center);
    ui.allocate_ui_with_layout(row, rtl, |ui| {
        let mut theme: &str = match edit.theme.as_str() {
            "dark" => "dark",
            "light" => "light",
            _ => "system",
        };
        // Right to left: the last item comes first on screen.
        let options = [("light", "Light"), ("dark", "Dark"), ("system", "System")];
        ui::segmented(ui, &mut theme, &options);
        edit.theme = theme.to_string();
        let ltr = egui::Layout::left_to_right(egui::Align::Center);
        ui.with_layout(ltr, |ui| ui.label("Theme"));
    });
}

/// The Parameter sets page of the Settings window.
fn sets_settings(ui: &mut egui::Ui, edit: &mut openscad::Config) {
    ui::section(ui, "Parameter sets");
    ui.add_space(4.0);
    toggle_row(
        ui,
        "Auto save",
        "Saves the values to the selected set when you change them. With Model defaults \
         selected, nothing is saved.",
        &mut edit.autosave_sets,
    );
    ui.add_space(8.0);
    ui.label(ui::dim(
        "When you open a model, CoCAD loads the set that you used last, else the first set \
         of the sets file. The sets file is <model>.json next to the model, or the file that \
         you opened for the model.",
    ));
}

/// The MCP server page of the Settings window.
fn mcp_settings(ui: &mut egui::Ui, edit: &mut openscad::Config, running: bool) {
    let (ok, text) = match (edit.mcp, running) {
        (true, true) => (true, "Running"),
        (false, false) => (false, "Off"),
        (true, false) => (false, "Starts when you save"),
        (false, true) => (true, "Stops when you save"),
    };
    let color = if ok { ui::GOOD } else { ui::c().dim };
    card_header(ui, "MCP server", color, text);
    ui.add_space(4.0);
    toggle_row(
        ui,
        "Allow MCP clients",
        "Clients on this computer, such as Claude Desktop, can control this window.",
        &mut edit.mcp,
    );
}

/// The Updates page of the Settings window.
fn update_settings(ui: &mut egui::Ui, edit: &mut openscad::Config) {
    ui::section(ui, "Updates");
    ui.add_space(4.0);
    toggle_row(
        ui,
        "Check for new versions",
        "Asks GitHub for the list of releases when the window opens and once a day. GitHub sees \
         your IP address and the app version. CoCAD does not install anything.",
        &mut edit.update_check,
    );
}

/// The Privacy page of the Settings window.
fn privacy_settings(ui: &mut egui::Ui, p: &mut PrivacyEdit) {
    toggle_row(ui, "Usage analytics", ANALYTICS_NOTE, &mut p.analytics);
    ui.add_space(8.0);
    toggle_row(ui, "Error reports", ERRORS_NOTE, &mut p.errors);
    ui.add_space(8.0);
    ui.label(ui::dim(VENDOR_NOTE).size(12.5));
    ui.add_space(6.0);
    install_id_row(ui, p);
    ui.add_space(6.0);
    ui.hyperlink_to("Privacy Notice", telemetry::PRIVACY_URL);
}

/// The OpenSCAD page of the Settings window.
fn openscad_settings(
    ui: &mut egui::Ui,
    edit: &mut openscad::Config,
    probe: &Option<openscad::Probe>,
) {
    match probe {
        None => card_header(
            ui,
            "OpenSCAD",
            ui::c().dim,
            "Checks when you leave the field",
        ),
        Some(p) if p.version == "not found" => {
            card_header(ui, "OpenSCAD", ui::c().danger, "Not found. Set the binary.");
        }
        Some(p) => {
            let version = p.version.trim_start_matches("OpenSCAD version ");
            let short: String = version.chars().take(32).collect();
            card_header(ui, "OpenSCAD", ui::GOOD, &short);
        }
    }
    ui.add_space(4.0);
    ui.label(ui::dim("Binary"));
    let row = Vec2::new(ui.available_width(), 30.0);
    let rtl = egui::Layout::right_to_left(egui::Align::Center);
    ui.allocate_ui_with_layout(row, rtl, |ui| {
        if ui.add(ui::ghost("Browse", false)).clicked()
            && let Some(p) = rfd::FileDialog::new().pick_file()
        {
            edit.binary = p.display().to_string();
        }
        ui.add(
            egui::TextEdit::singleline(&mut edit.binary)
                .id(binary_id())
                .hint_text(ui::dim("Automatic"))
                .desired_width(f32::INFINITY)
                .margin(Vec2::new(10.0, 6.0)),
        );
    });
    ui.label(ui::dim("Extra arguments"));
    ui.add(
        egui::TextEdit::singleline(&mut edit.args)
            .hint_text(ui::dim("For example --enable=lazy-union"))
            .desired_width(f32::INFINITY)
            .margin(Vec2::new(10.0, 6.0)),
    );
    let manifold = probe.as_ref().is_none_or(|p| p.manifold);
    let note = if manifold {
        "Much faster renders with development builds."
    } else {
        "Needs an OpenSCAD development build, 2024 or later."
    };
    ui.add_space(4.0);
    ui.add_enabled_ui(manifold, |ui| {
        toggle_row(ui, "Manifold backend", note, &mut edit.manifold);
    });
    if std::env::var_os("OPENSCAD").is_some() || std::env::var_os("OPENSCAD_ARGS").is_some() {
        ui.label(ui::dim(
            "$OPENSCAD or $OPENSCAD_ARGS override these values.",
        ));
    }
}

const ANALYTICS_NOTE: &str = "Send feature events, such as app started, with a random install \
    ID. This shows us which features people use.";
const ERRORS_NOTE: &str = "Send a crash report when the app fails. A report holds the error \
    type, the code location, the app version and your operating system. It holds no install ID \
    and no error message.";
const VENDOR_NOTE: &str = "PostHog receives this data for us. It sees your IP address and does \
    not store it.";
const UPDATE_NOTE: &str = "The Privacy Notice changed, so CoCAD asks you again. CoCAD now asks \
    GitHub for new versions. GitHub sees your IP address and the app version. You can turn this \
    off in File > Settings > Updates. Version 0.3.2 also changed the error report. CoCAD sends no \
    analytics and no error reports until you save your choice.";

/// The ID that a deletion request needs, with Copy and Reset.
fn install_id_row(ui: &mut egui::Ui, p: &mut PrivacyEdit) {
    ui.label("Install ID");
    let Some(id) = p.id.clone() else {
        ui.label(ui::dim(
            "None yet. CoCAD makes one when it sends the first event.",
        ));
        return;
    };
    ui.label(RichText::new(&id).monospace().size(12.0));
    ui.label(ui::dim(
        "A request to delete your analytics data needs this ID.",
    ));
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if p.confirm_reset {
            if ui.add(ui::ghost("Keep ID", false)).clicked() {
                p.confirm_reset = false;
            }
            if ui.add(ui::ghost("Reset ID", false)).clicked() {
                telemetry::reset_install_id();
                p.id = None;
                p.confirm_reset = false;
            }
        } else {
            if ui.add(ui::ghost("Copy", false)).clicked() {
                ui.ctx().copy_text(id.clone());
            }
            if ui.add(ui::ghost("Reset", false)).clicked() {
                p.confirm_reset = true;
            }
        }
    });
    if p.confirm_reset {
        ui.label(ui::dim(
            "Copy the ID first. Without the old ID, we cannot find and delete your old data.",
        ));
    }
}

/// Title and note on the left, switch on the right.
fn toggle_row(ui: &mut egui::Ui, title: &str, note: &str, on: &mut bool) {
    let row = Vec2::new(ui.available_width(), 38.0);
    let rtl = egui::Layout::right_to_left(egui::Align::Center);
    ui.allocate_ui_with_layout(row, rtl, |ui| {
        ui::toggle(ui, on);
        ui.add_space(12.0);
        ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.label(title);
            ui.add(egui::Label::new(ui::dim(note).size(12.5)).wrap());
        });
    });
}

/// Section title on the left, a colored dot and a short state on the right.
fn card_header(ui: &mut egui::Ui, title: &str, color: Color32, state: &str) {
    let row = Vec2::new(ui.available_width(), 18.0);
    let rtl = egui::Layout::right_to_left(egui::Align::Center);
    ui.allocate_ui_with_layout(row, rtl, |ui| {
        ui.label(ui::dim(state).size(12.5));
        let (r, _) = ui.allocate_exact_size(Vec2::new(8.0, 8.0), egui::Sense::hover());
        ui.painter().circle_filled(r.center(), 4.0, color);
        let ltr = egui::Layout::left_to_right(egui::Align::Center);
        ui.with_layout(ltr, |ui| ui::section(ui, title));
    });
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // The Settings window previews its theme before it is saved.
        let theme = self
            .cfg_edit
            .as_ref()
            .map_or(self.cfg.theme.as_str(), |e| e.theme.as_str());
        let dark = match theme {
            "dark" => true,
            "light" => false,
            _ => ctx.system_theme().is_none_or(|t| t == egui::Theme::Dark),
        };
        if dark != ui::is_dark() || !self.themed {
            ui::apply(ctx, dark);
            self.themed = true;
        }
        self.poll(ctx);
        self.update.poll(ctx, self.cfg.update_check);
        self.shortcuts(ctx);
        let title = match &self.file {
            Some(f) => format!(
                "CoCAD · {}",
                f.file_name().unwrap_or_default().to_string_lossy()
            ),
            None => "CoCAD".into(),
        };
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }

        let line = egui::Stroke::new(1.0_f32, ui::c().line);
        egui::TopBottomPanel::top("top")
            .exact_height(32.0)
            .frame(
                egui::Frame::NONE
                    .fill(ui::dark().bg)
                    .inner_margin(egui::Margin::symmetric(10, 0))
                    .stroke(egui::Stroke::new(1.0_f32, ui::dark().line)),
            )
            .show(ctx, |ui| ui::dark_scope(ui, |ui| self.menu_bar(ui)));
        self.settings_window(ctx);
        self.new_set_dialog(ctx);
        self.delete_set_dialog(ctx);
        self.about_window(ctx);
        if self.consent.is_some() {
            self.consent_window(ctx);
        } else {
            self.tour_window(ctx);
        }
        self.changes_window(ctx);
        self.part_colors_window(ctx);
        self.part_color_window(ctx);
        egui::TopBottomPanel::bottom("status")
            .exact_height(28.0)
            .frame(
                egui::Frame::NONE
                    .fill(ui::c().bg)
                    .inner_margin(egui::Margin::symmetric(14, 0))
                    .stroke(line),
            )
            .show(ctx, |ui| self.status_bar(ui));
        if self.show_log {
            egui::TopBottomPanel::bottom("log")
                .resizable(true)
                .default_height(160.0)
                .frame(
                    egui::Frame::NONE
                        .fill(ui::c().bg)
                        .inner_margin(egui::Margin::same(16))
                        .stroke(line),
                )
                .show(ctx, |ui| {
                    ui::section(ui, "OpenSCAD output");
                    egui::ScrollArea::vertical()
                        .stick_to_bottom(true)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            let text = if self.log.is_empty() {
                                "No OpenSCAD output yet."
                            } else {
                                &self.log
                            };
                            ui.label(RichText::new(text).monospace().color(ui::c().dim));
                        });
                });
        }
        let side = egui::Frame::NONE
            .fill(ui::c().panel)
            .inner_margin(egui::Margin::same(18))
            .stroke(line);
        egui::SidePanel::left("params")
            .resizable(true)
            .default_width(340.0)
            .width_range(280.0..=560.0)
            .frame(side)
            .show(ctx, |ui| self.param_panel(ui));
        egui::SidePanel::right("side")
            .resizable(true)
            .default_width(320.0)
            .width_range(280.0..=480.0)
            .frame(side)
            .show(ctx, |ui| {
                let mut mode = self.mode;
                if ui::tabs(
                    ui,
                    &mut mode,
                    &[
                        (Mode::Model, "Model"),
                        (Mode::Slice, "Slice"),
                        (Mode::Photo, "Photo"),
                    ],
                ) {
                    self.set_mode(mode);
                }
                match self.mode {
                    Mode::Model => self.model_panel(ui),
                    Mode::Slice => self.slice_panel(ui),
                    Mode::Photo => self.photo_panel(ui),
                }
            });
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| self.viewport(ui));
        self.menu_popup(ctx);
        self.update_notice(ctx);
    }

    fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        self.flush_autosave();
        if let Some(j) = &self.job {
            j.cancel.store(true, Ordering::Relaxed);
        }
        if let Some(j) = &self.parts_job {
            j.cancel.store(true, Ordering::Relaxed);
        }
        if let (Some(gl), Some(r)) = (gl, &self.renderer) {
            r.lock().unwrap().destroy(gl);
        }
    }
}

/// Label and value on one row, full-width slider below. Returns true on change.
fn setting<N: egui::emath::Numeric>(
    ui: &mut egui::Ui,
    label: &str,
    v: &mut N,
    range: std::ops::RangeInclusive<N>,
    step: f64,
    suffix: &str,
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).size(14.0));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let d = decimals(step);
            changed |= ui
                .add(
                    egui::DragValue::new(v)
                        .range(range.clone())
                        .speed(step)
                        .max_decimals(d)
                        .min_decimals(d)
                        .suffix(suffix),
                )
                .changed();
        });
    });
    ui.spacing_mut().slider_width = ui.available_width();
    changed |= ui
        .add(egui::Slider::new(v, range).step_by(step).show_value(false))
        .changed();
    ui.add_space(4.0);
    changed
}

/// Dim label on the left, value on the right.
fn info_row(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        ui.label(ui::dim(label));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(value);
        });
    });
}

/// Row for a recent model: file name and folder. A missing file cannot be picked.
fn recent_row(ui: &mut egui::Ui, path: &std::path::Path, current: bool) -> bool {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let dir = path
        .parent()
        .and_then(|d| d.file_name())
        .unwrap_or_default()
        .to_string_lossy();
    ui::menu_item(ui, &name, &dir, path.exists(), current)
        .on_hover_text(path.display().to_string())
        .clicked()
}

fn stat(ui: &mut egui::Ui, value: &str, label: &str) {
    ui.label(RichText::new(value).size(16.0));
    ui.label(ui::dim(label));
}

/// One parameter row. Returns true when the value changed.
fn param_row(ui: &mut egui::Ui, p: &mut Param) -> bool {
    let before = p.value.clone();
    // The description is the label when the file has one. Line 1 holds the
    // label and the inline value, line 2 the slider, list or text field.
    let label = if p.description.is_empty() {
        humanize(&p.name)
    } else {
        p.description.clone()
    };
    let changed = p.changed();
    let step = p.step();
    let decimals = decimals(step);

    ui.add_space(2.0);
    ui.horizontal(|ui| {
        if changed {
            let (r, _) = ui.allocate_exact_size(Vec2::new(6.0, 6.0), egui::Sense::hover());
            ui.painter().circle_filled(r.center(), 3.0, ui::ACCENT);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Inline value widgets for sliders, numbers and switches.
            match (&mut p.value, &p.widget) {
                (Value::Num(n), Widget::Slider { min, max, .. }) => {
                    ui.add(
                        egui::DragValue::new(n)
                            .speed(step)
                            .range(*min..=*max)
                            .max_decimals(decimals)
                            .min_decimals(decimals.min(1)),
                    );
                }
                (Value::Num(n), Widget::Plain) => {
                    ui.add(egui::DragValue::new(n).speed(step).max_decimals(decimals));
                }
                (Value::Bool(b), _) => {
                    ui::toggle(ui, b);
                }
                _ => {}
            }
            if changed
                && ui
                    .add(egui::Button::new(RichText::new("↺").color(ui::c().dim)).frame(false))
                    .on_hover_text(format!("Reset to {}", p.default.to_scad()))
                    .clicked()
            {
                p.value = p.default.clone();
            }
            // The label gets the space the values leave. A long description
            // is cut short and shows in full on hover.
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.add(egui::Label::new(RichText::new(&label).size(14.0)).truncate())
                    .on_hover_text(&p.name);
            });
        });
    });

    let full = ui.available_width();
    match (&mut p.value, &p.widget) {
        (Value::Num(n), Widget::Slider { min, max, .. }) => {
            ui.spacing_mut().slider_width = full;
            ui.add(
                egui::Slider::new(n, *min..=*max)
                    .step_by(step)
                    .show_value(false),
            );
        }
        (Value::Num(n), Widget::Dropdown(opts)) => {
            let cur = opts
                .iter()
                .find(|(v, _)| *v == Value::Num(*n))
                .map(|o| o.1.clone())
                .unwrap_or(Value::Num(*n).to_scad());
            egui::ComboBox::from_id_salt(&p.name)
                .selected_text(cur)
                .width(full)
                .show_ui(ui, |ui| {
                    for (v, l) in opts {
                        if let Value::Num(x) = v {
                            ui.selectable_value(n, *x, l);
                        }
                    }
                });
        }
        (Value::Str(s), Widget::Dropdown(opts)) => {
            let cur = opts
                .iter()
                .find(|(v, _)| *v == Value::Str(s.clone()))
                .map(|o| o.1.clone())
                .unwrap_or(s.clone());
            egui::ComboBox::from_id_salt(&p.name)
                .selected_text(cur)
                .width(full)
                .show_ui(ui, |ui| {
                    for (v, l) in opts {
                        if let Value::Str(x) = v {
                            ui.selectable_value(s, x.clone(), l);
                        }
                    }
                });
        }
        (Value::Str(s), w) => {
            let mut edit = egui::TextEdit::singleline(s)
                .desired_width(full)
                .margin(Vec2::new(10.0, 7.0));
            if let Widget::TextMax(n) = w {
                edit = edit.char_limit(*n);
            }
            ui.add(edit);
        }
        (Value::Vec(v), w) => {
            let range = match w {
                Widget::Slider { min, max, .. } => *min..=*max,
                _ => f64::MIN..=f64::MAX,
            };
            let n = v.len().max(1) as f32;
            let each = (full - (n - 1.0) * 8.0) / n;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                for (i, x) in v.iter_mut().enumerate() {
                    let axis = ["x", "y", "z", "w"][i.min(3)];
                    ui.add_sized(
                        Vec2::new(each, 30.0),
                        egui::DragValue::new(x)
                            .speed(step)
                            .range(range.clone())
                            .max_decimals(decimals)
                            .prefix(format!("{axis}  ")),
                    );
                }
            });
        }
        _ => {}
    }
    ui.add_space(4.0);
    match &mut p.value {
        Value::Num(n) => *n = params::clean(*n),
        Value::Vec(v) => v.iter_mut().for_each(|n| *n = params::clean(*n)),
        _ => {}
    }
    !params::same(&p.value, &before)
}

fn decimals(step: f64) -> usize {
    let mut d = 0;
    let mut s = step.abs();
    while d < 6 && (s - s.round()).abs() > 1e-9 {
        s *= 10.0;
        d += 1;
    }
    d
}

fn humanize(name: &str) -> String {
    let s = name.replace('_', " ");
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => s,
    }
}

fn fmt_duration(secs: f64) -> String {
    let m = (secs / 60.0).round() as u64;
    if m >= 60 {
        format!("{}h {:02}m", m / 60, m % 60)
    } else {
        format!("{m} min")
    }
}

fn rgb(c: [f32; 3]) -> Color32 {
    Color32::from_rgb(
        (c[0] * 255.0) as u8,
        (c[1] * 255.0) as u8,
        (c[2] * 255.0) as u8,
    )
}

fn mtime(p: &std::path::Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// The largest rectangle with the image's aspect ratio inside the view, with a margin.
fn photo_frame(view: egui::Rect, size: (usize, usize)) -> egui::Rect {
    let area = view.shrink(28.0);
    let aspect = size.0 as f32 / size.1.max(1) as f32;
    let (w, h) = if area.width() / area.height() > aspect {
        (area.height() * aspect, area.height())
    } else {
        (area.width(), area.width() / aspect)
    };
    egui::Rect::from_center_size(area.center(), Vec2::new(w, h))
}
