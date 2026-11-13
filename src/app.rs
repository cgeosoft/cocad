//! The egui application: parameter panel, 3D view, export and slice preview.

use crate::assembly::Assembly;
use crate::definition;
use crate::examples;
use crate::export::{self, Format, Meta, Slicer};
use crate::mesh::{Bounds, Mesh};
use crate::openscad;
use crate::params::{self, Param, Value, Widget};
use crate::pathtrace;
use crate::presets;
use crate::recent;
use crate::render::{Renderer, View};
use crate::slicer::{self, Settings, Sliced};
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

mod optimize_tab;
mod remote;
mod section;
mod select;
mod start;
mod state;
mod thumb;
mod tour;
mod view3d;

const DEBOUNCE: Duration = Duration::from_millis(350);
/// Auto save writes the set when the values do not change for this time.
const AUTOSAVE: Duration = Duration::from_millis(600);

/// The tabs of the right panel.
#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub(crate) enum Mode {
    Model,
    /// Print preparation and the slice preview. Was the Slice tab.
    Optimize,
    /// The path traced render. Was the Photo tab.
    Render,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Model, Mode::Optimize, Mode::Render];

    fn screen(self) -> Screen {
        match self {
            Mode::Model => Screen::Model,
            Mode::Optimize => Screen::Optimize,
            Mode::Render => Screen::Render,
        }
    }

    /// The name in `models.json`, the CLI and the MCP tools.
    pub fn name(self) -> &'static str {
        match self {
            Mode::Model => "model",
            Mode::Optimize => "optimize",
            Mode::Render => "render",
        }
    }

    /// The mode with this name. The old names "slice" and "photo" (before
    /// 0.7) give Optimize and Render.
    pub fn from_name(name: &str) -> Option<Mode> {
        match name {
            "model" => Some(Mode::Model),
            "optimize" | "slice" => Some(Mode::Optimize),
            "render" | "photo" => Some(Mode::Render),
            _ => None,
        }
    }

    /// The tab label.
    pub fn label(self) -> &'static str {
        match self {
            Mode::Model => "Model",
            Mode::Optimize => "Optimize",
            Mode::Render => "Render",
        }
    }

    /// The key that opens the tab, and the index of its letter in the
    /// label. The same key again goes back to the Model tab.
    fn key(self) -> Option<(egui::Key, usize)> {
        match self {
            Mode::Model => None,
            Mode::Optimize => Some((egui::Key::O, 0)),
            Mode::Render => Some((egui::Key::R, 0)),
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

const SHORTCUTS: [(&str, &str); 17] = [
    ("Ctrl+N", "New model"),
    ("Ctrl+O", "Open a file"),
    ("Ctrl+,", "Settings"),
    ("Ctrl+Q", "Quit"),
    ("Alt+F, M, V, H", "Open a menu"),
    ("F5", "Build now"),
    ("F, double click", "Fit the model"),
    ("W", "Wireframe"),
    ("E", "Explode the parts"),
    ("O", "Model or Optimize tab"),
    ("R", "Model or Render tab"),
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

/// The GPU path tracer of the photo renders.
enum Gpu {
    /// Not made yet: the first photo makes it.
    Untried,
    Ready(pathtrace::gpu::Tracer),
    /// The context cannot run it, or it failed. Photos render on the CPU.
    Off,
}

/// Pages of the Settings window.
#[derive(Clone, Copy, PartialEq)]
enum SettingsPage {
    Appearance,
    Sets,
    OpenScad,
    Printer,
    Mcp,
    Updates,
    Privacy,
}

/// Photo render settings shown in the Render tab.
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
type PartsResult = Result<(Option<Assembly>, Vec<String>), String>;

struct PartsJob {
    /// `None`: the model has no parts, and the shells of the mesh stay.
    /// The notes go to the OpenSCAD output.
    rx: Receiver<PartsResult>,
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

/// The files that an export wrote, and a note (for example the parts that
/// it left out).
type Exported = (Vec<PathBuf>, Option<String>);

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
    /// The plate key (`plate_key`) of `sliced`.
    sliced_for: u64,
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
    /// The user clicked Render. The render starts when the view rests.
    photo_on: bool,
    /// Wanted key and when it last changed; the render starts when it settles.
    photo_want: Option<(PhotoKey, Instant)>,
    photo_tex: Option<(egui::TextureHandle, u32)>,
    photo_started: Instant,
    /// The time of the render when it got all its samples.
    photo_done: Option<Duration>,
    themed: bool,
    /// The window title that was sent last. A title command asks for a repaint.
    title: String,
    /// The model definition `<model>.yml`: text, parts and parameter sets.
    def: definition::Definition,
    def_file: PathBuf,
    /// The time of the definition file that `def` was read from.
    def_mtime: Option<SystemTime>,
    /// The names of the parameter sets of `def`.
    sets: Vec<String>,
    /// Problems of the definition, for the console.
    def_notes: Vec<String>,
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
    /// The OpenSCAD binary and its probe, while it has no Manifold
    /// backend. The window then shows a notice and CoCAD does not build.
    no_manifold: Option<(String, openscad::Probe)>,
    renderer: Option<Arc<Mutex<Renderer>>>,
    /// The OpenGL context, for the GPU path tracer.
    gl: Option<Arc<eframe::glow::Context>>,
    gpu: Gpu,
    toast: Option<(String, Instant)>,
    /// A running export: the written files, or the error.
    export_rx: Option<Receiver<Result<Exported, String>>>,
    /// The Optimize tab: its state, the pipeline and the plate view.
    opt: optimize_tab::Opt,
    menu: Option<Menu>,
    menu_at: egui::Pos2,
    menu_bar_rect: egui::Rect,
    /// The Examples row while its submenu is open.
    examples_row: Option<egui::Rect>,
    /// Height of the start page content in the last frame, to center it.
    start_height: f32,
    /// The tab of the start page card that the user picked.
    start_tab: Option<start::StartTab>,
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
    /// The model opened and needs a thumbnail after its first build.
    thumb_due: bool,
    /// Thumbnails to draw in the next frame.
    thumb_jobs: Vec<thumb::Job>,
    /// Thumbnails and titles of the start page.
    start_cache: thumb::StartCache,
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
        let (cfg_bed, cfg_preset) = (cfg.bed, cfg.bed_preset.clone());
        let start_mode = Mode::from_name(mode).unwrap_or(Mode::Model);
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
            sliced_for: u64::MAX,
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
            photo_on: false,
            photo_want: None,
            photo_tex: None,
            photo_started: Instant::now(),
            photo_done: None,
            themed: false,
            title: String::new(),
            def: definition::Definition::default(),
            def_file: PathBuf::new(),
            def_mtime: None,
            sets: Vec::new(),
            def_notes: Vec::new(),
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
            no_manifold: None,
            renderer,
            gl: cc.gl.clone(),
            gpu: Gpu::Untried,
            toast: None,
            export_rx: None,
            opt: optimize_tab::Opt::new(cfg_bed, &cfg_preset),
            menu: None,
            menu_at: egui::Pos2::ZERO,
            menu_bar_rect: egui::Rect::NOTHING,
            examples_row: None,
            start_height: 0.0,
            start_tab: None,
            recent: recent::load(),
            show_about: false,
            tour: tour::first_start().then(tour::Tour::new),
            server: None,
            pending: Vec::new(),
            update: update::Checker::new(),
            show_changes: false,
            thumb_due: false,
            thumb_jobs: Vec::new(),
            start_cache: thumb::StartCache::default(),
        };
        app.sync_server(&cc.egui_ctx);
        app.check_openscad();
        if let Some(f) = file {
            app.open(f);
        }
        app
    }

    fn open(&mut self, path: PathBuf) {
        // The model that closes gets a new thumbnail.
        self.queue_thumb();
        self.flush_autosave();
        self.save_state();
        match std::fs::read_to_string(&path) {
            Ok(src) => {
                let saved = state::load(&path);
                self.params = params::parse(&src);
                self.mtime = mtime(&path);
                self.def_file = definition::path_for(&path);
                self.load_def();
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
                self.collapsed.clear();
                let (area, preset) = (self.default_area(), self.cfg.bed_preset.clone());
                self.opt.new_model(area, &preset);
                self.sliced_for = u64::MAX;
                self.slice_for_rev = u64::MAX;
                if let Some(st) = &saved {
                    self.restore_panels(st);
                    if self.restore_params(st) {
                        self.set_edited = self.active_set.is_some();
                        let text = match &self.active_set {
                            Some(n) => format!("Restored your last values of set {n}"),
                            None => "Restored your last values".to_string(),
                        };
                        self.toast = Some((text, Instant::now()));
                    }
                }
                self.slice_rev += 1;
                self.fit_pending = true;
                self.thumb_due = true;
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
        self.def.apply_parts(&mut fresh);
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

    /// The time of the file that the definition comes from: `<model>.yml`,
    /// else the `.json` file of the OpenSCAD Customizer.
    fn def_source_mtime(&self) -> Option<SystemTime> {
        definition::source(&self.def_file).and_then(|p| mtime(&p))
    }

    /// Reads the definition of the open model, and gives its parts to the
    /// part parameter. A file that cannot be read gives an empty definition.
    /// The problems go to the console, and a new problem to a message.
    fn load_def(&mut self) {
        self.def_mtime = self.def_source_mtime();
        let name = (definition::source(&self.def_file).as_deref())
            .and_then(Path::file_name)
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let mut notes = Vec::new();
        self.def = match definition::load(&self.def_file) {
            Ok(d) => d,
            Err(e) => {
                notes.push(format!("{e:#}"));
                definition::Definition::default()
            }
        };
        let warnings = self.def.warnings.clone();
        notes.extend(
            (warnings.into_iter())
                .chain(self.def.apply_parts(&mut self.params))
                .map(|w| format!("{name}: {w}")),
        );
        if notes != self.def_notes
            && let Some(first) = notes.first()
        {
            self.toast = Some((first.clone(), Instant::now()));
        }
        self.def_notes = notes;
        self.sets = self.def.set_names();
    }

    /// Re-reads the definition after a change. The parts follow it. The
    /// active set takes the new values, unless it has edits. A file that
    /// cannot be parsed, for example during a write, stays unread and the
    /// next poll tries it again.
    fn reload_sets(&mut self) {
        let m = self.def_source_mtime();
        if m == self.def_mtime {
            return;
        }
        if definition::load(&self.def_file).is_err() {
            return;
        }
        let parts = |params: &[Param]| {
            crate::part_param::selector(params).map(|(p, l)| (p.name.clone(), l.clone()))
        };
        let before = parts(&self.params);
        self.load_def();
        if parts(&self.params) != before {
            self.dirty_since = Some(Instant::now());
        }
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
            let skipped = presets::apply(&self.def_file, &name, &mut self.params);
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

    /// Loads a set of the definition when a model opens: the set that was
    /// active last, else the first set. See `presets::autoload`.
    fn autoload_set(&mut self, scad: &Path) {
        let Some(name) = presets::autoload(scad, &self.sets) else {
            return;
        };
        let text = match presets::apply(&self.def_file, &name, &mut self.params) {
            Ok(skipped) if !skipped.is_empty() => {
                format!("Loaded set {name}. Skipped: {}", skipped.join(", "))
            }
            Ok(_) => format!("Loaded set {name}"),
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
        match presets::save(&self.def_file, &name, &self.params) {
            Ok(()) => {
                self.set_edited = false;
                self.load_def();
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

    /// Probes the OpenSCAD binary again. Without Manifold, the notice
    /// shows. With it, a model without a mesh builds.
    fn check_openscad(&mut self) {
        openscad::forget();
        let binary = self.cfg.binary();
        let probe = openscad::probe(&binary);
        let was_missing = self.no_manifold.is_some();
        self.no_manifold = (!probe.manifold).then_some((binary, probe));
        if was_missing && self.no_manifold.is_none() && self.mesh.is_none() {
            self.start_render();
        }
    }

    fn start_render(&mut self) {
        let Some(file) = self.file.clone() else {
            return;
        };
        // CoCAD needs Manifold. The notice tells how to get it.
        if self.no_manifold.is_some() {
            return;
        }
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
        std::thread::spawn(move || {
            let r = openscad::render(&cfg, &file, &defines, &c).map_err(|e| e.to_string());
            if c.load(Ordering::Relaxed) {
                let _ = tx.send(Err("cancelled".into()));
                return;
            }
            // The shells and the checks can take a moment for a large mesh.
            let r = r.map(|out| {
                let mesh = Arc::new(out.mesh);
                let label = crate::part_param::shown_label(&params);
                let mut assembly = Assembly::shells(&mesh, label.as_deref());
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
        (self.view3d.section).set_bounds(section::Space::Model, parts);
    }

    /// Finds the parts of the new mesh in the background, with
    /// `csg_split::find_parts`: a multicolor part list, a part list or the
    /// module split. The parts then replace the shells of the mesh. A model
    /// that shows one part of its multicolor list keeps the shells,
    /// in the color of that part. Call it after `self.log` has the log of
    /// the build.
    fn color_parts(&mut self, params: Vec<Param>) {
        self.cancel_parts();
        let Some(file) = self.file.clone() else {
            return;
        };
        let cfg = self.cfg.clone();
        // The same values as the main build.
        let defines = params::defines(&params);
        let paint = crate::parts::shown_color(&params, &self.log);
        let note = if crate::parts::needs_render(&params) {
            "Building the named parts…"
        } else if crate::part_param::needs_render(&params) {
            "Building the parts…"
        } else {
            "Finding the parts…"
        };
        self.start_parts_job(paint, note, move |cancel| {
            crate::csg_split::find_parts(&cfg, &file, &params, &defines, cancel)
        });
    }

    /// Starts a background job that renders the parts of the current mesh
    /// one by one. When it is done, its parts replace the shells of the mesh
    /// in the 3D view (see `Assembly::from_parts`). When `render` gives
    /// `None`, the shells stay. `paint` gives all printed parts one color. A
    /// new build or file cancels the job. The Parts card shows `note` while
    /// the job runs.
    fn start_parts_job(
        &mut self,
        paint: Option<[u8; 3]>,
        note: &'static str,
        render: impl FnOnce(&Arc<AtomicBool>) -> anyhow::Result<Option<crate::csg_split::Split>>
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
                .map(|found| match found {
                    Some(s) => {
                        let mut a = Assembly::from_split(&mesh, &s);
                        if let Some(color) = paint {
                            a.paint_all(color);
                        }
                        (Some(a), s.notes)
                    }
                    None => (None, Vec::new()),
                })
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

    /// Slices the plate in the background.
    fn start_slice(&mut self) {
        let Some(o) = self.opt.outcome.clone() else {
            return;
        };
        self.slice_dirty_since = None;
        let (tx, rx) = channel();
        let s = self.settings;
        let rev = self.plate_key();
        std::thread::spawn(move || {
            let t = Instant::now();
            let mesh = o.plate_mesh();
            let r = slicer::slice(&mesh, &s);
            let _ = tx.send((rev, r, t.elapsed()));
        });
        self.slice_for_rev = rev;
        self.slice_job = Some(rx);
    }

    /// True when the slice preview must follow the plate: in the Optimize
    /// tab, or for a tool call.
    fn slice_wanted(&self) -> bool {
        self.mode == Mode::Optimize || self.tool_needs_slice()
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
                    self.log = out.log;
                    self.color_parts(params);
                    self.last_time = Some(out.elapsed);
                    self.error = None;
                    if self.fit_pending {
                        self.fit();
                        self.fit_pending = false;
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
                Ok((assembly, notes)) if rev == self.mesh_rev => {
                    for n in notes {
                        if !self.log.trim_end().is_empty() {
                            self.log.push_str("\n\n");
                        }
                        self.log.push_str(&format!("{n}\n"));
                    }
                    if let Some(assembly) = assembly {
                        self.set_assembly(Some(assembly));
                        ctx.request_repaint();
                    }
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
            if rev == self.plate_key() {
                let first = self.sliced.as_ref().is_none_or(|s| s.layers.is_empty());
                let n = sliced.layers.len();
                self.sliced = Some(Arc::new(sliced));
                self.sliced_for = rev;
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
                    Ok((paths, note)) => {
                        if let Some(format) = paths.first().and_then(|p| Format::from_path(p)) {
                            telemetry::event(Event::ModelExported { format });
                        }
                        let saved = match paths.as_slice() {
                            [] => "Nothing saved".to_string(),
                            [p] => format!("Saved {}", p.display()),
                            [p, ..] => format!("Saved {} files: {} …", paths.len(), p.display()),
                        };
                        match note {
                            Some(n) => format!("{saved}. {n}"),
                            None => saved,
                        }
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
        self.poll_optimize(ctx);
        if self.slice_dirty_since.is_some() && self.slice_wanted() {
            ctx.request_repaint_after(DEBOUNCE);
        }
        // The slice follows the plate, after the print settings settle.
        if self.slice_wanted()
            && self.slice_job.is_none()
            && self
                .slice_dirty_since
                .is_none_or(|t| t.elapsed() > DEBOUNCE)
            && self.opt.outcome.is_some()
            && self.outcome_current()
            && self.slice_for_rev != self.plate_key()
        {
            self.start_slice();
        }
        self.run_commands();
        if self.job.is_some()
            || self.slice_job.is_some()
            || self.parts_job.is_some()
            || self.export_rx.is_some()
            || self.opt.running()
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

    /// The parameter values for the 3MF and STEP headers.
    fn export_meta(&self) -> Meta {
        Meta {
            title: self.stem(),
            params: self
                .params
                .iter()
                .map(|p| (p.name.clone(), p.value.to_scad()))
                .collect(),
        }
    }

    /// Exports STL or 3MF of the plate, or STEP of the model as built. With
    /// one print part selected in the Plate card, STEP writes only that
    /// part. When the plate is not ready, the export thread prepares it
    /// first.
    fn export(&mut self, fmt: Format) {
        let Some(mesh) = self.mesh.clone() else {
            return;
        };
        let only = self.opt.state.only.clone();
        let source = if fmt != Format::Step || only.is_some() {
            match self.plate_source() {
                Ok(s) => Some(s),
                Err(e) => {
                    self.toast = Some((format!("Cannot export: {e}"), Instant::now()));
                    return;
                }
            }
        } else {
            None
        };
        let name = export::suggested_name(&self.stem(), self.active_set.as_deref(), fmt);
        let mut dialog = rfd::FileDialog::new()
            .set_file_name(name)
            .add_filter(fmt.label(), &[fmt.ext()]);
        if let Some(dir) = self.export_dir() {
            dialog = dialog.set_directory(dir);
        }
        let Some(path) = dialog.save_file() else {
            return;
        };
        recent::set_export_dir(&path);
        let meta = self.export_meta();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let r = match (fmt, source) {
                (Format::Step, None) => export::write(&path, &mesh, &meta)
                    .map(|_| (vec![path], None))
                    .map_err(|e| e.to_string()),
                (Format::Step, Some(src)) => src.outcome().and_then(|o| {
                    let m = optimize_tab::only_mesh(&o, only.as_deref().unwrap_or_default())?;
                    export::write(&path, &m, &meta)
                        .map(|_| (vec![path], None))
                        .map_err(|e| e.to_string())
                }),
                (_, Some(src)) => src.outcome().and_then(|o| {
                    optimize_tab::write_plate(&path, &o, &meta, |_| Ok(()))?;
                    Ok((vec![path], o.too_large_note()))
                }),
                (_, None) => Err("no plate".to_string()),
            };
            let _ = tx.send(r);
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
        let meta = self.export_meta();
        let (cfg, params) = (self.cfg.clone(), self.params.clone());
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let cancel = Arc::new(AtomicBool::new(false));
            let _ = tx.send(
                crate::parts::render(&cfg, &file, &params, &cancel)
                    .and_then(|parts| export::write_parts(&path, &parts, &meta, slicer))
                    .map(|_| (vec![path], None))
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
            let (r, _) = ui.allocate_exact_size(Vec2::splat(18.0), egui::Sense::hover());
            ui::paint_app_mark(ui.painter(), r);
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
        if ui::menu_item(ui, "New model…", "Ctrl+N", true, false).clicked() {
            self.new_model();
        }
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
        if ui::menu_item(ui, "Build now", "F5", self.file.is_some(), false).clicked() {
            self.start_render();
        }
        if ui::menu_item(ui, "Auto build", "", true, self.auto_render).clicked() {
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
        for mode in Mode::ALL {
            let key = mode.key().map_or("", |(k, _)| k.name());
            if ui::menu_item(ui, mode.label(), key, true, self.mode == mode).clicked() {
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

    /// The notice when the OpenSCAD binary has no Manifold backend: what
    /// CoCAD found and how to get a development snapshot. CoCAD does not
    /// build while it shows. It waits while the Settings window is open.
    fn openscad_notice(&mut self, ctx: &egui::Context) {
        let Some((binary, probe)) = self.no_manifold.clone() else {
            return;
        };
        if self.cfg_edit.is_some() {
            return;
        }
        let (mut settings, mut again) = (false, false);
        let frame = egui::Frame::NONE
            .fill(ui::c().panel)
            .corner_radius(14)
            .inner_margin(egui::Margin::same(18))
            .stroke(egui::Stroke::new(1.0_f32, ui::c().line));
        egui::Modal::new(egui::Id::new("openscad_notice"))
            .frame(frame)
            .show(ctx, |ui| {
                ui.set_width(520.0);
                ui.heading("CoCAD needs OpenSCAD with Manifold");
                ui.add_space(6.0);
                ui.label(
                    "CoCAD builds the models with the Manifold backend of OpenSCAD. The \
                     development snapshots (nightly builds) of OpenSCAD have it. The release \
                     2021.01 does not. CoCAD does not build a model until it finds a snapshot.",
                );
                ui.add_space(8.0);
                let found = if probe.version == "not found" {
                    "not found".to_string()
                } else {
                    probe.version.clone()
                };
                info_row(ui, "Found", &found);
                info_row(ui, "Binary", &binary);
                ui.add_space(8.0);
                ui::section(ui, "Steps");
                for (i, (step, line)) in openscad::install_steps().into_iter().enumerate() {
                    ui.add(egui::Label::new(format!("{}. {step}", i + 1)).wrap());
                    if let Some(line) = line {
                        ui.horizontal(|ui| {
                            ui.add_space(16.0);
                            ui.add(
                                egui::Label::new(
                                    RichText::new(line)
                                        .monospace()
                                        .size(12.0)
                                        .color(ui::c().dim),
                                )
                                .wrap(),
                            );
                            if ui
                                .add(ui::ghost("Copy", false))
                                .on_hover_text("Copy this line")
                                .clicked()
                            {
                                ui.ctx().copy_text(line.to_string());
                            }
                        });
                    }
                }
                ui.add_space(4.0);
                ui.hyperlink_to(
                    "OpenSCAD downloads: development snapshots",
                    openscad::SNAPSHOTS_URL,
                );
                ui.add_space(12.0);
                let row = Vec2::new(ui.available_width(), 34.0);
                let rtl = egui::Layout::right_to_left(egui::Align::Center);
                ui.allocate_ui_with_layout(row, rtl, |ui| {
                    ui.spacing_mut().button_padding = Vec2::new(16.0, 7.0);
                    again = ui.add(ui::primary("Check again")).clicked();
                    settings = ui
                        .add(ui::ghost("OpenSCAD settings", false))
                        .on_hover_text("Select the OpenSCAD binary")
                        .clicked();
                });
            });
        if settings {
            self.open_settings();
            self.settings_page = SettingsPage::OpenScad;
        }
        if again {
            self.check_openscad();
            if self.no_manifold.is_some() {
                self.toast = Some(("No OpenSCAD with Manifold yet".to_string(), Instant::now()));
            }
        }
    }

    /// A small notice at the bottom right. A click opens the list of changes.
    /// It waits while another window covers the app.
    fn update_notice(&mut self, ctx: &egui::Context) {
        let covered = self.consent.is_some()
            || self.no_manifold.is_some()
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
                        ui::bar_room(ui, |ui| {
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

    /// True when the user closed the right panel card `name`.
    fn card_closed(&self, name: &str) -> bool {
        self.cfg.closed_cards.contains(name)
    }

    /// The header of the right panel card `name`. A click opens or closes
    /// the card, and the config file keeps the choice for all models.
    /// `note` shows in the header while the card is closed. Returns true
    /// when the card is open.
    fn card_head(&mut self, ui: &mut egui::Ui, name: &str, note: &str) -> bool {
        self.card_head_info(ui, name, note, "")
    }

    /// `card_head` with an info icon after the name. The icon shows `info`
    /// on hover.
    fn card_head_info(&mut self, ui: &mut egui::Ui, name: &str, note: &str, info: &str) -> bool {
        let open = !self.card_closed(name);
        let note = if open { "" } else { note };
        if ui::fold_header(ui, name, open, note, info).clicked() {
            self.set_cards_closed(&[name], open);
        }
        let open = !self.card_closed(name);
        if open {
            ui.add_space(6.0);
        }
        open
    }

    /// Closes (`closed`) or opens the cards `names` and saves the choice.
    fn set_cards_closed(&mut self, names: &[&str], closed: bool) {
        for n in names {
            if closed {
                self.cfg.closed_cards.insert(n.to_string());
            } else {
                self.cfg.closed_cards.remove(*n);
            }
        }
        if let Some(edit) = &mut self.cfg_edit {
            edit.closed_cards = self.cfg.closed_cards.clone();
        }
        if let Err(e) = self.cfg.save() {
            self.toast = Some((format!("Cannot save settings: {e}"), Instant::now()));
        }
    }

    fn model_panel(&mut self, ui: &mut egui::Ui) {
        ui::scroll(ui, ui::PANEL_PAD, |ui| {
            ui::card(ui, |ui| {
                let note = if self.job.is_some() {
                    "Building".to_string()
                } else if self.error.is_some() {
                    "Error".to_string()
                } else {
                    self.last_time
                        .map(|t| format!("{:.2} s", t.as_secs_f32()))
                        .unwrap_or_default()
                };
                if !self.card_head(ui, "Build", &note) {
                    return;
                }
                // Auto build and Build on one row.
                ui.horizontal(|ui| {
                    ui.label("Auto build");
                    ui::toggle(ui, &mut self.auto_render)
                        .on_hover_text("Build when a parameter changes");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let build = egui::Button::new("Build").min_size(Vec2::new(110.0, 30.0));
                        if ui
                            .add_enabled(self.file.is_some(), build)
                            .on_hover_text("Build the model now (F5)")
                            .clicked()
                        {
                            self.start_render();
                        }
                    });
                });
            });
            ui.add_space(4.0);
            self.parts_card(ui);
            self.explode_card(ui);
            if let (Some(i), Some(mesh)) = (&self.info, &self.mesh) {
                let size = i.bounds.size();
                let tris = mesh.triangles.len();
                let (volume, open_edges) = (i.volume, i.open_edges);
                let note = format!("{:.1} × {:.1} × {:.1} mm", size.x, size.y, size.z);
                ui::card(ui, |ui| {
                    if !self.card_head(ui, "Mesh", &note) {
                        return;
                    }
                    info_row(
                        ui,
                        "Size",
                        &format!("{:.1} × {:.1} × {:.1} mm", size.x, size.y, size.z),
                    );
                    info_row(ui, "Volume", &format!("{:.2} cm³", volume / 1000.0));
                    info_row(ui, "Triangles", &tris.to_string());
                    info_row(ui, "Open edges", &open_edges.to_string());
                });
            }
        });
    }

    /// The default print area from Settings > Printer, for a model that
    /// has no print area of its own yet.
    pub(crate) fn default_area(&self) -> crate::optimize::Area {
        self.cfg.bed
    }

    /// The bar at the bottom of the Optimize tab. Export 3MF writes the
    /// plate; the arrow opens a menu above it with the other exports.
    fn export_bar(&mut self, ui: &mut egui::Ui) {
        let copies = self.plate_copies();
        let parts_ready = self.parts_job.is_none();
        let can_export = self.can_export();
        let plate_ok = can_export && parts_ready && copies != Some(0);
        let why = if !parts_ready {
            "CoCAD is still finding the parts"
        } else if copies == Some(0) {
            "No part is on the plate"
        } else {
            "Build the model first"
        };
        let multicolor = !crate::parts::names(&self.params).is_empty();
        let summary = self.export_summary();
        let mut pick: Option<Format> = None;
        let mut slicer_pick = None;
        ui.scope(|ui| {
            let (main, arrow) = ui::split_button(ui, "Export 3MF", plate_ok, can_export);
            let main = if plate_ok {
                main.on_hover_text(format!(
                    "The plate as 3MF: one object for each part.\n{summary}"
                ))
            } else {
                main.on_disabled_hover_text(why)
            };
            if main.clicked() {
                pick = Some(Format::ThreeMf);
            }
            let arrow = arrow.on_hover_text("More exports");
            let id = ui.make_persistent_id("export_menu");
            let toggled = arrow.clicked();
            if toggled {
                ui.memory_mut(|m| m.toggle_popup(id));
            }
            if !ui.memory(|m| m.is_popup_open(id)) {
                return;
            }
            // The menu opens above the button group. It is at least 300 px
            // wide and grows to the left, so that the labels fit.
            let group = main.rect.union(arrow.rect);
            let width = group.width().max(300.0);
            let area = egui::Area::new(id.with("area"))
                .kind(egui::UiKind::Popup)
                .order(egui::Order::Foreground)
                .fixed_pos(group.right_top() - Vec2::new(0.0, 4.0))
                .pivot(egui::Align2::RIGHT_BOTTOM)
                .constrain(true)
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_width(width - 16.0);
                        if ui::menu_item(ui, "STL: the plate", "", plate_ok, false).clicked() {
                            pick = Some(Format::Stl);
                        }
                        if ui::menu_item(ui, "STEP: the model as built", "", can_export, false)
                            .clicked()
                        {
                            pick = Some(Format::Step);
                        }
                        if multicolor {
                            for slicer in Slicer::ALL {
                                let label = format!("Multicolor 3MF: {}", slicer.label());
                                if ui::menu_item(ui, &label, "", can_export, false).clicked() {
                                    slicer_pick = Some(slicer);
                                }
                            }
                        }
                    });
                });
            // A choice, Escape or a click outside the menu closes it.
            let outside = !toggled
                && ui.input(|i| i.pointer.any_click())
                && !area.response.contains_pointer();
            let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
            if pick.is_some() || slicer_pick.is_some() || outside || escape {
                ui.memory_mut(|m| m.close_popup());
            }
        });
        if let Some(fmt) = pick {
            self.export(fmt);
        }
        if let Some(slicer) = slicer_pick {
            self.export_parts(slicer);
        }
    }

    fn set_mode(&mut self, mode: Mode) {
        if mode != self.mode {
            telemetry::event(Event::ViewChanged {
                view: mode.screen(),
            });
        }
        let refit = (mode == Mode::Optimize) != (self.mode == Mode::Optimize);
        self.mode = mode;
        // The Optimize tab shows the bed, the other tabs the model.
        if refit {
            self.fit();
        }
    }

    fn param_panel(&mut self, ui: &mut egui::Ui) {
        // The scroll area goes to the edges of the panel, so that its scroll
        // bar is in the side margin and not on the cards. The rows above it
        // get the side margin here.
        let has_model = egui::Frame::NONE
            .inner_margin(egui::Margin::symmetric(ui::PANEL_PAD, 0))
            .show(ui, |ui| self.param_head(ui))
            .inner;
        if !has_model {
            return;
        }
        let mut changed = false;
        // The resize handle of the panel is at its right edge.
        ui.spacing_mut().scroll.bar_outer_margin = ui::BAR_INSET;
        ui::scroll(ui, ui::PANEL_PAD, |ui| {
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

    /// The rows above the parameter groups: the model card and the
    /// parameter sets. False when no model is open.
    fn param_head(&mut self, ui: &mut egui::Ui) -> bool {
        if self.file.is_none() {
            ui::card(ui, |ui| {
                ui.label("Open a .scad file, or drop one on the window.");
                if ui.add(ui::primary("Open file")).clicked() {
                    self.pick_file();
                }
            });
            return false;
        }
        self.model_card(ui);
        ui.add_space(4.0);
        if !self.params.is_empty() {
            self.sets_bar(ui);
            ui.add_space(4.0);
        }
        if self.params.is_empty() {
            ui::card(ui, |ui| {
                ui.label(ui::dim("No Customizer parameters in this file."))
            });
        }
        true
    }

    /// A card with the name and the description of the model, from its
    /// definition. Without a title, the name is the file name. The info
    /// button opens a window with all fields of the definition.
    fn model_card(&mut self, ui: &mut egui::Ui) {
        let Some(file) = &self.file else {
            return;
        };
        let meta = &self.def.meta;
        let stem = file.file_stem().unwrap_or_default().to_string_lossy();
        let name = meta.title.clone().unwrap_or_else(|| stem.into_owned());
        let id = egui::Id::new("model-info");
        let mut show = ui.ctx().data(|d| d.get_temp::<bool>(id).unwrap_or(false));
        egui::Frame::NONE
            .fill(ui::c().card)
            .corner_radius(12)
            .inner_margin(egui::Margin {
                left: 16,
                right: 8,
                top: 8,
                bottom: 8,
            })
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                        if ui::icon_button(ui, ui::Icon::Info, 28.0, show)
                            .on_hover_text("Model info")
                            .clicked()
                        {
                            show = !show;
                        }
                        ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            ui.add_space(4.0);
                            ui.add(
                                egui::Label::new(RichText::new(&name).size(15.0).strong())
                                    .truncate(),
                            );
                            if let Some(d) = &meta.description {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(d).size(12.5).color(ui::c().dim),
                                    )
                                    .wrap(),
                                );
                            }
                            ui.add_space(4.0);
                        });
                    });
                });
            });
        if show {
            let frame = egui::Frame::NONE
                .fill(ui::c().panel)
                .corner_radius(14)
                .inner_margin(egui::Margin::same(20))
                .stroke(egui::Stroke::new(1.0_f32, ui::c().line));
            let modal = egui::Modal::new(id).frame(frame).show(ui.ctx(), |ui| {
                ui.set_width(420.0);
                ui.heading(&name);
                if let Some(d) = &meta.description {
                    ui.add(egui::Label::new(RichText::new(d).color(ui::c().dim)).wrap());
                }
                ui.add_space(4.0);
                egui::Grid::new("model-info-grid")
                    .num_columns(2)
                    .spacing([16.0, 8.0])
                    .show(ui, |ui| {
                        for (k, v) in &meta.other {
                            ui.label(ui::dim(humanize(k)));
                            if v.starts_with("https://") || v.starts_with("http://") {
                                ui.hyperlink_to(v, v);
                            } else {
                                ui.add(egui::Label::new(v).wrap());
                            }
                            ui.end_row();
                        }
                        ui.label(ui::dim("File"));
                        ui.add(egui::Label::new(file.display().to_string()).wrap());
                        ui.end_row();
                        if let Some(src) = definition::source(&self.def_file) {
                            ui.label(ui::dim("Definition"));
                            ui.add(egui::Label::new(src.display().to_string()).wrap());
                            ui.end_row();
                        }
                        ui.label(ui::dim("Parameters"));
                        ui.label(self.params.len().to_string());
                        ui.end_row();
                        ui.label(ui::dim("Sets"));
                        ui.label(self.sets.len().to_string());
                        ui.end_row();
                    });
                ui.add_space(8.0);
                let row = Vec2::new(ui.available_width(), 34.0);
                let rtl = egui::Layout::right_to_left(egui::Align::Center);
                ui.allocate_ui_with_layout(row, rtl, |ui| {
                    ui.spacing_mut().button_padding = Vec2::new(16.0, 7.0);
                    ui.add(ui::primary("Close")).clicked()
                })
                .inner
            });
            if modal.inner || modal.should_close() {
                show = false;
            }
        }
        ui.ctx().data_mut(|d| d.insert_temp(id, show));
    }

    /// Sets all values back to the model defaults.
    fn reset_all_button(&mut self, ui: &mut egui::Ui, size: f32) {
        let any = self.params.iter().any(|p| p.changed());
        let reset = ui
            .add_enabled_ui(any, |ui| {
                ui::icon_button(ui, ui::Icon::Reset, size, false)
                    .on_hover_text("Reset all: set all values back to the model defaults")
            })
            .inner;
        if reset.clicked() {
            for p in &mut self.params {
                p.value = p.default.clone();
            }
            self.dirty_since = Some(Instant::now());
            self.mark_set_edited();
        }
    }

    /// Closes all parameter groups, or opens all when all are closed.
    fn fold_button(&mut self, ui: &mut egui::Ui, size: f32) {
        let all_closed = !self.params.is_empty()
            && self.params.iter().all(|p| self.collapsed.contains(&p.group));
        let (icon, hover) = if all_closed {
            (ui::Icon::Expand, "Open all groups")
        } else {
            (ui::Icon::Collapse, "Close all groups")
        };
        if ui::icon_button(ui, icon, size, false).on_hover_text(hover).clicked() {
            if all_closed {
                self.collapsed.clear();
            } else {
                self.collapsed = self.params.iter().map(|p| p.group.clone()).collect();
            }
        }
    }

    /// The set list with Save and the set menu, and the description of the
    /// active set. The list sits directly above the parameter groups,
    /// because the set gives their values. The sets are in the definition
    /// `<model>.yml` next to the model.
    fn sets_bar(&mut self, ui: &mut egui::Ui) {
        if self.file.is_none() {
            return;
        }
        let path = self.def_file.clone();
        let mut msg: Option<String> = None;
        let mut gear_rect = egui::Rect::NOTHING;
        let mut pick: Option<Option<String>> = None;
        let mut new_set = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let label = match &self.active_set {
                Some(n) if self.set_edited => format!("{n} (edited)"),
                Some(n) => n.clone(),
                None => "Model defaults".into(),
            };
            // Height of a button row. Save and the menu are squares of this size.
            let sp = ui.spacing();
            let row = (ui.text_style_height(&egui::TextStyle::Button) + 2.0 * sp.button_padding.y)
                .max(sp.interact_size.y);
            // Show 4.5 rows. The half row tells the user that the list scrolls.
            let list_height = 4.0 * (row + sp.item_spacing.y) + 0.5 * row;
            // Save, the set menu, Reset all and Collapse are squares.
            let width = ui.available_width() - 4.0 * (row + sp.item_spacing.x);
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
                })
                .response
                .on_hover_text(format!(
                    "Parameter sets in {}",
                    path.file_name().unwrap_or_default().to_string_lossy()
                ));
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
                        Err(e) => msg = Some(format!("{e:#}")),
                    },
                    None => new_set = true,
                }
            }
            let gear = ui::icon_button(ui, ui::Icon::Gear, row, self.sets_menu)
                .on_hover_text("New set, delete set");
            if gear.clicked() {
                self.sets_menu = !self.sets_menu;
            }
            gear_rect = gear.rect;
            self.reset_all_button(ui, row);
            self.fold_button(ui, row);
        });
        if let Some(d) = (self.active_set.as_deref())
            .and_then(|n| self.def.set(n))
            .and_then(|s| s.description.clone())
        {
            ui.add_space(4.0);
            egui::Frame::NONE
                .fill(ui::c().input)
                .corner_radius(8)
                .inner_margin(egui::Margin::symmetric(10, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.add(egui::Label::new(RichText::new(d).size(12.5).color(ui::c().dim)).wrap());
                });
        }

        // The set menu, under the gear button, right aligned.
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
            if new_set || delete || outside || escape {
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
                    Err(e) => msg = Some(format!("{e:#}")),
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
        if delete {
            self.confirm_delete = true;
            self.new_set = None;
        }
        if msg.is_some() {
            self.load_def();
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
            .def_file
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
            match presets::save(&self.def_file, &name, &self.params) {
                Ok(()) => {
                    self.load_def();
                    self.toast = Some((format!("Saved {name}"), Instant::now()));
                    self.set_active(Some(name));
                }
                Err(e) => self.toast = Some((format!("{e:#}"), Instant::now())),
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
            .def_file
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
            match presets::delete(&self.def_file, &n) {
                Ok(()) => {
                    self.load_def();
                    self.toast = Some((format!("Deleted {n}"), Instant::now()));
                    self.set_active(None);
                }
                Err(e) => {
                    self.toast = Some((format!("{e:#}"), Instant::now()));
                    self.confirm_delete = false;
                }
            }
        } else if cancel {
            self.confirm_delete = false;
        }
    }

    /// Thin bar at the bottom: build state, OpenSCAD version, model, mode details.
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
                        "Building {:.1} s",
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
            } else if self.no_manifold.is_some() {
                ui.label(small("Not built: OpenSCAD has no Manifold".into()).color(ui::c().danger));
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
            let label = format!("OpenSCAD {version}");
            if probe.manifold {
                ui.label(dim(label)).on_hover_text(self.cfg.binary());
            } else {
                ui.label(small(label).color(ui::c().danger))
                    .on_hover_text(format!("{}: no Manifold backend", self.cfg.binary()));
            }

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
                Mode::Optimize => {
                    if let Some(sl) = (self.sliced.as_ref())
                        .filter(|s| self.sliced_for == self.plate_key() && !s.layers.is_empty())
                    {
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
                Mode::Render => {
                    sep(ui);
                    let on = match self.photo_job.as_ref().map(|j| j.backend()) {
                        Some(pathtrace::Backend::Gpu) => "  ·  GPU",
                        Some(pathtrace::Backend::Cpu) => "  ·  CPU",
                        None => "",
                    };
                    ui.label(small(format!(
                        "Render {} / {}{on}",
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

    /// The queue of the GPU path tracer. The first call makes the tracer.
    /// `None` when the GPU cannot render: photos then render on the CPU.
    fn gpu_queue(&mut self) -> Option<pathtrace::gpu::Queue> {
        if matches!(self.gpu, Gpu::Untried) {
            self.gpu = match self.gl.as_deref().map(pathtrace::gpu::Tracer::new) {
                Some(Ok(t)) => Gpu::Ready(t),
                Some(Err(e)) => {
                    eprintln!("gpu render: off, {e}");
                    Gpu::Off
                }
                None => Gpu::Off,
            };
        }
        match &self.gpu {
            Gpu::Ready(t) => Some(t.queue()),
            _ => None,
        }
    }

    /// Starts a photo render, on the GPU when it can.
    pub(super) fn photo_job(
        &mut self,
        model: pathtrace::Model,
        cam: pathtrace::Camera,
        look: pathtrace::Look,
        size: (usize, usize),
        samples: u32,
    ) -> pathtrace::Job {
        let queue = self.gpu_queue();
        pathtrace::Job::start_with(queue.as_ref(), model, cam, look, size.0, size.1, samples)
    }

    /// Renders the GPU photo jobs for one frame.
    fn step_gpu(&mut self, ctx: &egui::Context) {
        let (Gpu::Ready(t), Some(gl)) = (&mut self.gpu, &self.gl) else {
            return;
        };
        match t.step(gl) {
            Ok(busy) => {
                if busy {
                    ctx.request_repaint();
                }
            }
            Err(e) => {
                eprintln!("gpu render: off, {e}");
                t.shutdown(gl);
                self.gpu = Gpu::Off;
                ctx.request_repaint();
            }
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
        // A render that no longer matches the view or the settings stops,
        // and the frame shows the 3D view again.
        if self.photo_key.is_some_and(|k| k != key) {
            self.stop_photo();
            self.photo_job = None;
            self.photo_key = None;
            self.photo_tex = None;
        }
        if self.photo_on && self.photo_key.is_none() {
            // Wait until the camera and the settings rest, then start.
            match self.photo_want {
                Some((k, at)) if k == key => {
                    if at.elapsed() > Duration::from_millis(250)
                        && let Some(model) = self.photo_model()
                    {
                        self.photo_job =
                            Some(self.photo_job(model, key.cam, key.look, size, key.samples));
                        self.photo_key = Some(key);
                        self.photo_tex = None;
                        self.photo_started = Instant::now();
                        self.photo_done = None;
                        self.photo_want = None;
                    }
                }
                _ => self.photo_want = Some((key, Instant::now())),
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
                if self.photo_on {
                    ui.ctx().request_repaint_after(Duration::from_millis(120));
                }
            } else if self.photo_done.is_none() {
                self.photo_done = Some(self.photo_started.elapsed());
                self.photo_on = false;
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

    /// Starts a new render of the view in the frame (the Render button).
    fn start_photo(&mut self) {
        self.photo_job = None;
        self.photo_key = None;
        self.photo_tex = None;
        self.photo_want = None;
        self.photo_done = None;
        self.photo_on = true;
    }

    /// Stops the render. The image keeps the samples that are done.
    fn stop_photo(&mut self) {
        self.photo_on = false;
        self.photo_want = None;
        if let Some(job) = &self.photo_job {
            job.stop();
        }
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

    /// The bar at the bottom of the Render tab: the progress, Render and
    /// Save PNG. The tab starts no render by itself.
    fn render_bar(&mut self, ui: &mut egui::Ui) {
        let samples = self.photo_samples();
        let target = self.photo.samples;
        let built = self.mesh.is_some();
        let running = self.photo_on;
        // The samples and the state of the render on one row. The image
        // size is in the header of the Quality card.
        let status = if !built {
            String::new()
        } else if running && self.photo_job.is_none() {
            "Waiting for the view to rest".to_string()
        } else if running {
            format!("{:.0} s", self.photo_started.elapsed().as_secs_f32())
        } else if let Some(t) = self.photo_done.filter(|_| self.photo_job.is_some()) {
            format!("Done in {:.0} s", t.as_secs_f32())
        } else if self.photo_job.is_some() {
            "Stopped".to_string()
        } else {
            String::new()
        };
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{samples}")).size(22.0));
            ui.label(ui::dim(format!("of {target} samples")));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(ui::dim(status));
            });
        });
        ui.add(
            egui::ProgressBar::new(samples as f32 / target.max(1) as f32)
                .desired_height(6.0)
                .fill(ui::ACCENT),
        );
        ui.add_space(4.0);
        let w = ui.available_width();
        let size = Vec2::new(w, 34.0);
        let (label, hover) = if running {
            (
                "Stop",
                "Stop the render. You can save the samples that are done.",
            )
        } else {
            (
                "Render",
                "Path-trace the view in the frame on all CPU cores",
            )
        };
        let go = ui
            .add_enabled_ui(built, |ui| ui.add_sized(size, ui::primary(label)))
            .inner;
        if go
            .on_hover_text(hover)
            .on_disabled_hover_text("Build the model first")
            .clicked()
        {
            if running {
                self.stop_photo();
            } else {
                self.start_photo();
            }
        }
        let save = ui
            .add_enabled_ui(samples > 0, |ui| {
                ui.add_sized(size, egui::Button::new("Save PNG").corner_radius(8))
            })
            .inner;
        if save
            .on_hover_text("Save the image at its full size")
            .clicked()
        {
            self.save_photo();
        }
    }

    /// The "Material" card of the Render tab: the color and the finish. A
    /// model with two or more parts has one color for all parts, or a color
    /// for each part. The second choice is the "Part colors" switch of the
    /// Parts card in the Model tab.
    fn material_card(&mut self, ui: &mut egui::Ui) {
        let parts =
            (self.assembly.clone()).filter(|a| a.parts.iter().filter(|p| !p.reference).count() > 1);
        let each = parts.is_some() && self.view3d.part_colors;
        let finish = match self.photo.finish {
            "matte" => "Matte",
            "gloss" => "Gloss",
            _ => "Satin",
        };
        let note = if each {
            format!("{finish}, part colors")
        } else {
            let name =
                (SWATCHES.iter().find(|s| s.1 == self.photo.color)).map_or("custom color", |s| s.0);
            format!("{finish}, {}", name.to_lowercase())
        };
        ui::card(ui, |ui| {
            let info = if parts.is_some() {
                "One color: all parts have the same color.\n\nEach part: each part has its own \
                 color. Click a row or a part in the view to select it, then click a swatch. With \
                 no part selected, a swatch sets all parts.\n\nThe finish is the same for all \
                 parts. The colors are for the view and the photo. The export does not change."
            } else {
                "The color and the finish of the plastic in the photo. The export does not change."
            };
            if !self.card_head_info(ui, "Material", &note, info) {
                return;
            }
            if let Some(a) = &parts {
                ui.label("Color");
                let mut on = self.view3d.part_colors;
                ui::segmented(ui, &mut on, &[(false, "One color"), (true, "Each part")]);
                self.view3d.part_colors = on;
                ui.add_space(6.0);
                if on {
                    self.part_color_list(ui, a);
                } else if let Some(c) = swatch_row(ui, Some(self.photo.color)) {
                    self.photo.color = c;
                }
            } else if let Some(c) = swatch_row(ui, Some(self.photo.color)) {
                self.photo.color = c;
            }
            ui.add_space(6.0);
            let p = &mut self.photo;
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
    }

    /// The colors of the parts in the Material card: the swatches for the
    /// selected part, or for all parts when no part is selected, and a list
    /// with one row for each part. A click on a row selects the part.
    fn part_color_list(&mut self, ui: &mut egui::Ui, a: &crate::assembly::Assembly) {
        let v = &mut self.view3d;
        let sel = v.sel.part().map(str::to_string);
        let target = sel
            .as_deref()
            .and_then(|n| a.parts.iter().position(|p| p.name == n));
        let current = target.map(|i| v.color_on(i, &a.parts[i]));
        let hint = match target {
            Some(i) => format!("Swatches set: {}", a.parts[i].name),
            None => "Swatches set: all parts".to_string(),
        };
        ui.label(ui::dim(hint));
        if let Some(c) = swatch_row(ui, current) {
            for (i, p) in a.parts.iter().enumerate() {
                if !p.reference && target.is_none_or(|t| t == i) {
                    v.set_color(p, c);
                }
            }
        }
        ui.add_space(6.0);
        const ROW: f32 = 28.0;
        let rows: Vec<usize> = (0..a.parts.len())
            .filter(|&i| !a.parts[i].reference)
            .collect();
        egui::Frame::NONE
            .fill(ui::c().input.gamma_multiply(0.5))
            .corner_radius(8)
            // No margin at the right: the rows leave room for the scroll bar.
            .inner_margin(egui::Margin {
                right: 0,
                ..egui::Margin::same(8)
            })
            .show(ui, |ui| {
                // show_rows counts the item spacing in the row height.
                ui.spacing_mut().item_spacing.y = 0.0;
                egui::ScrollArea::vertical()
                    .id_salt("part_color_list")
                    .max_height(ROW * 8.0)
                    .auto_shrink([false, true])
                    .show_rows(ui, ROW, rows.len(), |ui, range| {
                        ui::bar_room(ui, |ui| {
                            for &i in &rows[range] {
                                let p = &a.parts[i];
                                let size = Vec2::new(ui.available_width(), ROW);
                                let (rect, resp) =
                                    ui.allocate_exact_size(size, egui::Sense::click());
                                let picked = sel.as_deref() == Some(p.name.as_str());
                                if picked {
                                    ui.painter().rect_filled(rect, 6.0, ui::c().selection);
                                } else if resp.hovered() {
                                    ui.painter().rect_filled(rect, 6.0, ui::c().input);
                                }
                                let mut row = ui.new_child(
                                    egui::UiBuilder::new()
                                        .max_rect(rect.shrink2(Vec2::new(8.0, 0.0)))
                                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                                );
                                row.spacing_mut().interact_size = Vec2::new(28.0, 18.0);
                                let mut color = v.color_on(i, p);
                                if row.color_edit_button_srgb(&mut color).changed() {
                                    v.set_color(p, color);
                                }
                                row.add(egui::Label::new(&p.name).truncate().selectable(false));
                                row.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if v.has_custom(p)
                                            && ui
                                                .add(ui::ghost("Reset", false))
                                                .on_hover_text("Use the default color")
                                                .clicked()
                                        {
                                            v.reset_color(p);
                                        }
                                    },
                                );
                                if resp.clicked() {
                                    let name = (!picked).then(|| p.name.clone());
                                    v.sel.select(name);
                                }
                            }
                        });
                    });
            });
        let any = a.parts.iter().any(|p| v.has_custom(p));
        if any && ui.add(ui::ghost("Reset all", false)).clicked() {
            for p in &a.parts {
                v.reset_color(p);
            }
        }
    }

    fn render_panel(&mut self, ui: &mut egui::Ui) {
        ui::scroll(ui, ui::PANEL_PAD, |ui| {
            self.material_card(ui);
            self.explode_card(ui);
            ui::card(ui, |ui| {
                let note = match self.photo.studio {
                    pathtrace::Studio::Light => "Light studio",
                    pathtrace::Studio::Dark => "Dark studio",
                };
                if !self.card_head(ui, "Scene", note) {
                    return;
                }
                let p = &mut self.photo;
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
                let note = format!("{} × {}", self.photo.width, self.photo.height);
                let info = "More samples give a cleaner image and take more time. The render \
                            uses all CPU cores. A change of the view or of a setting stops it.";
                if !self.card_head_info(ui, "Quality", &note, info) {
                    return;
                }
                let p = &mut self.photo;
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
            });
        });
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
        // The Optimize tab shows the plate on the bed.
        let optimize = self.mode == Mode::Optimize;
        if optimize {
            self.plate_click(&resp, rect);
            self.ensure_plate();
            // The Section bar cuts the parts on the plate.
            let plate = self.plate_mesh().0.and_then(|a| a.bounds(&[]));
            (self.view3d.section).set_bounds(section::Space::Plate, plate);
        }
        self.select_input(&resp, rect);
        if resp.double_clicked() {
            self.fit();
        }
        self.update_selection();

        let sliced = (self.sliced.clone()).filter(|_| self.sliced_for == self.plate_key());
        let layer_max = sliced.as_ref().map(|s| s.layers.len()).unwrap_or(0);
        let slicing = optimize && self.opt.view == optimize_tab::PlateView::Layers && layer_max > 0;
        if let Some(r) = &self.renderer {
            let view = View {
                mvp: self.cam.matrix(rect.width() / rect.height().max(1.0)),
                eye: self.cam.eye(),
                show_mesh: !slicing || self.ghost,
                mesh_alpha: if slicing { 0.12 } else { 1.0 },
                parts: if optimize {
                    self.plate_styles()
                } else {
                    self.part_styles()
                },
                wireframe: self.wireframe,
                grid: self.grid,
                layer: slicing.then_some(self.layer),
                section: self.section_view(),
            };
            let (assembly, assembly_rev) = if optimize {
                self.plate_mesh()
            } else {
                (self.assembly.clone(), self.assembly_rev)
            };
            let slice_rev = self.slice_rev;
            let bounds = self.view_bounds();
            let bed = optimize.then_some(self.opt.state.area);
            let dark = ui::is_dark();
            let r = r.clone();
            let cb = egui::PaintCallback {
                rect,
                callback: Arc::new(eframe::egui_glow::CallbackFn::new(move |_info, painter| {
                    let gl = painter.gl();
                    let mut r = r.lock().unwrap();
                    r.set_mesh(gl, assembly.as_deref(), assembly_rev);
                    r.set_slice(gl, sliced.as_deref(), slice_rev);
                    if let Some(a) = bed {
                        r.set_bed(gl, a.w, a.d, dark);
                    } else {
                        let b = bounds.unwrap_or(Bounds {
                            min: Vec3::splat(-50.0),
                            max: Vec3::splat(50.0),
                        });
                        r.set_grid(gl, b.min, b.max, dark);
                    }
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

        if self.mode == Mode::Render {
            self.photo_view(ui, rect);
        } else {
            // The Render tab starts no render by itself: the user clicks Render.
            self.photo_on = false;
            self.photo_want = None;
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

        let taken = (optimize && self.mesh.is_some()).then(|| self.plate_overlay(ui, rect));
        let bar = self.section_bar(ui, rect, taken);

        // Layer slider, right edge, below the Section bar and the number
        // above the slider.
        if slicing {
            let top = (bar.map_or(0.0, |b| b.bottom() + 62.0))
                .max(rect.top() + 70.0)
                .min(rect.bottom() - 160.0);
            let strip = egui::Rect::from_min_max(
                egui::pos2(rect.right() - m - 36.0, top),
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
                (None, _) => None,
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
        if self.tour.is_some() || self.consent.is_some() || self.no_manifold.is_some() {
            return;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::O)) {
            self.pick_file();
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::N)) {
            self.new_model();
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
        let (f5, f, w, up, down, pgup, pgdn) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::F5),
                i.key_pressed(egui::Key::F),
                i.key_pressed(egui::Key::W),
                i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::PageUp),
                i.key_pressed(egui::Key::PageDown),
            )
        });
        if ctx.input(|i| i.key_pressed(egui::Key::E)) {
            self.toggle_explode();
        }
        // O and R open their tab, and the same key goes back to the Model
        // tab. Ctrl+O (open a file) is used above.
        for mode in Mode::ALL {
            let Some((key, _)) = mode.key() else { continue };
            let pressed = ctx.input(|i| {
                let m = i.modifiers;
                i.key_pressed(key) && !m.command && !m.ctrl && !m.alt
            });
            if pressed {
                self.set_mode(if self.mode == mode { Mode::Model } else { mode });
            }
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
        if self.mode == Mode::Optimize {
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
        let line = egui::Stroke::new(1.0_f32, ui::c().line);
        let frame = egui::Frame::NONE
            .fill(ui::c().panel)
            .corner_radius(14)
            .stroke(line);
        // A fixed size keeps the buttons in place when the page changes.
        let screen = ctx.screen_rect().size();
        let width = (screen.x - 44.0).clamp(456.0, 640.0);
        let height = (screen.y - 170.0).clamp(160.0, 360.0);
        let probe = settings_probe(ctx, &edit);
        let missing = probe.as_ref().is_some_and(|p| !p.manifold);
        let mut page = self.settings_page;
        let mut pages = vec![
            (SettingsPage::Appearance, "Appearance", None),
            (SettingsPage::Sets, "Parameter sets", None),
            (
                SettingsPage::OpenScad,
                "OpenSCAD",
                missing.then_some(ui::c().danger),
            ),
            (SettingsPage::Printer, "Printer", None),
            (SettingsPage::Mcp, "MCP server", running.then_some(ui::GOOD)),
            (SettingsPage::Updates, "Updates", None),
        ];
        if crate::telemetry::available() {
            pages.push((SettingsPage::Privacy, "Privacy", None));
        }
        egui::Modal::new(egui::Id::new("settings"))
            .frame(frame)
            .show(ctx, |ui| {
                const PAD: i8 = ui::PANEL_PAD;
                const NAV_W: f32 = 164.0;
                // The page list is a column with its own background, from
                // the top to the bottom of the window. Its size is known
                // only at the end, so the background is set there.
                let nav_bg = ui.painter().add(egui::Shape::Noop);
                let ltr = egui::Layout::left_to_right(egui::Align::Center);
                let rtl = egui::Layout::right_to_left(egui::Align::Center);
                let down = egui::Layout::top_down(egui::Align::Min);
                let gap = ui.spacing().item_spacing.x;
                ui.horizontal_top(|ui| {
                    // No gap between the columns. The columns get it back.
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.x = gap;
                        ui.set_width(NAV_W);
                        egui::Frame::NONE
                            .inner_margin(egui::Margin {
                                left: 12,
                                right: 12,
                                top: PAD,
                                bottom: PAD,
                            })
                            .show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                let row = Vec2::new(ui.available_width(), 28.0);
                                ui.allocate_ui_with_layout(row, ltr, |ui| {
                                    // The same indent as the labels of the list.
                                    ui.add_space(10.0);
                                    ui.heading("Settings");
                                });
                                ui.add_space(10.0);
                                ui.spacing_mut().item_spacing.y = 2.0;
                                for &(value, label, dot) in &pages {
                                    if ui::nav_item(ui, label, page == value, dot).clicked() {
                                        page = value;
                                    }
                                }
                            });
                    });
                    let title = pages.iter().find(|p| p.0 == page).map_or("", |p| p.1);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.x = gap;
                        ui.set_width(width - NAV_W);
                        ui.add_space(f32::from(PAD));
                        egui::Frame::NONE
                            .inner_margin(egui::Margin::symmetric(PAD, 0))
                            .show(ui, |ui| {
                                let row = Vec2::new(ui.available_width(), 28.0);
                                ui.allocate_ui_with_layout(row, ltr, |ui| {
                                    ui.heading(title);
                                    ui.with_layout(rtl, |ui| {
                                        let x = RichText::new("×").size(20.0).color(ui::c().dim);
                                        close |= ui
                                            .add(egui::Button::new(x).frame(false))
                                            .on_hover_text("Close (Esc)")
                                            .clicked();
                                    });
                                });
                            });
                        ui.add_space(10.0);
                        // The page goes to the edge of the window, so that
                        // its scroll bar is in the margin and not on the card.
                        let body = Vec2::new(ui.available_width(), height);
                        ui.allocate_ui_with_layout(body, down, |ui| {
                            ui.set_min_height(height);
                            ui.spacing_mut().scroll.bar_outer_margin = ui::BAR_INSET;
                            ui::scroll(ui, PAD, |ui| match page {
                                SettingsPage::Appearance => {
                                    ui::card(ui, |ui| theme_row(ui, &mut edit));
                                }
                                SettingsPage::Sets => {
                                    ui::card(ui, |ui| sets_settings(ui, &mut edit));
                                }
                                SettingsPage::OpenScad => {
                                    ui::card(ui, |ui| openscad_settings(ui, &mut edit, &probe));
                                }
                                SettingsPage::Printer => {
                                    ui::card(ui, |ui| printer_settings(ui, &mut edit));
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
                        egui::Frame::NONE
                            .inner_margin(egui::Margin {
                                left: PAD,
                                right: PAD,
                                top: 0,
                                bottom: PAD,
                            })
                            .show(ui, |ui| {
                                let row = Vec2::new(ui.available_width(), 34.0);
                                ui.allocate_ui_with_layout(row, rtl, |ui| {
                                    ui.spacing_mut().button_padding = Vec2::new(16.0, 7.0);
                                    save = ui
                                        .add_enabled(dirty, ui::primary("Save"))
                                        .on_disabled_hover_text("No changes")
                                        .clicked();
                                    close |= ui.add(ui::ghost("Cancel", false)).clicked();
                                    if dirty {
                                        ui.with_layout(ltr, |ui| {
                                            ui.label(ui::dim("Unsaved changes").size(12.5))
                                        });
                                    }
                                });
                            });
                    });
                });
                let r = ui.min_rect();
                let nav = egui::Rect::from_min_max(r.min, egui::pos2(r.left() + NAV_W, r.bottom()));
                let round = egui::CornerRadius {
                    nw: 13,
                    sw: 13,
                    ne: 0,
                    se: 0,
                };
                ui.painter()
                    .set(nav_bg, egui::Shape::rect_filled(nav, round, ui::c().bg));
                ui.painter().vline(nav.right(), nav.y_range(), line);
            });
        self.settings_page = page;
        if save {
            if let Err(e) = edit.save() {
                self.toast = Some((format!("Cannot save settings: {e}"), Instant::now()));
            }
            let rerender = (&edit.binary, &edit.args) != (&self.cfg.binary, &self.cfg.args);
            self.cfg = edit;
            if self.privacy.dirty() {
                let p = &self.privacy;
                if let Err(e) = telemetry::set_consent(p.analytics, p.errors, Source::Settings) {
                    self.toast = Some((format!("Cannot save your choice: {e}"), Instant::now()));
                }
            }
            self.sync_server(ctx);
            if rerender {
                self.check_openscad();
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
        "When you open a model, CoCAD loads the set that you used last, else the first set. \
         The sets are in <model>.yml next to the model.",
    ));
}

/// The Printer page of the Settings window: the default print area.
fn printer_settings(ui: &mut egui::Ui, edit: &mut openscad::Config) {
    ui::section(ui, "Print area");
    ui.add_space(4.0);
    ui.add(
        egui::Label::new(ui::dim(
            "The default print area of the printer, for a model that you open for the first \
             time. Select a printer, or type the width, depth and height.",
        ))
        .wrap(),
    );
    ui.add_space(6.0);
    crate::printer::area_editor(ui, &mut edit.bed, &mut edit.bed_preset);
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
        "Asks GitHub for the list of releases when the window opens and each hour after that. \
         GitHub sees your IP address and the app version. CoCAD does not install anything.",
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
            let color = if p.manifold { ui::GOOD } else { ui::c().danger };
            card_header(ui, "OpenSCAD", color, &short);
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
    ui.add_space(4.0);
    if probe.as_ref().is_some_and(|p| !p.manifold) {
        ui.label(
            RichText::new(
                "This binary has no Manifold backend. CoCAD needs an OpenSCAD development \
                 snapshot.",
            )
            .size(12.5)
            .color(ui::c().danger),
        );
        ui.hyperlink_to(
            RichText::new("Development snapshots").size(12.5),
            openscad::SNAPSHOTS_URL,
        );
    } else {
        ui.label(ui::dim(
            "CoCAD builds with the Manifold backend. It adds --backend=manifold.",
        ));
    }
    if openscad::cgal_arg(&edit.args) {
        ui.label(
            RichText::new(
                "--backend=cgal turns off Manifold. Builds are much slower, and CoCAD needs \
                 Manifold.",
            )
            .size(12.5)
            .color(ui::ACCENT),
        );
    }
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
        self.send_thumbs(ctx);
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
        } else if self.no_manifold.is_some() {
            self.openscad_notice(ctx);
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
                            ui::bar_room(ui, |ui| {
                                // Problems of the definition come first.
                                for n in &self.def_notes {
                                    ui.label(RichText::new(format!("WARNING: {n}")).monospace());
                                }
                                let text = if self.log.is_empty() {
                                    "No OpenSCAD output yet."
                                } else {
                                    &self.log
                                };
                                ui.label(RichText::new(text).monospace().color(ui::c().dim));
                            });
                        });
                });
        }
        // The side panels have a border only on the side of the 3D view.
        let side = egui::Frame::NONE
            .fill(ui::c().panel)
            .inner_margin(egui::Margin::same(18));
        let edge = |ctx: &egui::Context, x: f32, rect: egui::Rect| {
            ctx.layer_painter(egui::LayerId::background())
                .vline(x, rect.y_range(), line);
        };
        // Without a model, the start page fills the window.
        if self.file.is_none() {
            egui::CentralPanel::default()
                .frame(egui::Frame::NONE)
                .show(ctx, |ui| self.start_page(ui));
            self.menu_popup(ctx);
            self.update_notice(ctx);
            return;
        }
        let params = egui::SidePanel::left("params")
            .resizable(true)
            .default_width(340.0)
            .width_range(280.0..=560.0)
            .frame(side.inner_margin(egui::Margin {
                top: ui::PANEL_PAD,
                ..egui::Margin::ZERO
            }))
            .show(ctx, |ui| self.param_panel(ui))
            .response
            .rect;
        edge(ctx, params.right() - 0.5, params);
        let side_panel = egui::SidePanel::right("side")
            .resizable(true)
            .default_width(320.0)
            .width_range(280.0..=480.0)
            .frame(side.inner_margin(egui::Margin::ZERO))
            .show(ctx, |ui| {
                const PAD: i8 = ui::PANEL_PAD;
                // The tabs sit in a darker strip. The selected tab has the
                // panel color, so it joins the panel below it.
                egui::TopBottomPanel::top("side_tabs")
                    .frame(
                        egui::Frame::NONE
                            .fill(ui::c().bg)
                            .inner_margin(egui::Margin {
                                left: 10,
                                right: 10,
                                top: 10,
                                bottom: 0,
                            }),
                    )
                    .show_separator_line(false)
                    .show_inside(ui, |ui| {
                        let mut mode = self.mode;
                        let tabs = Mode::ALL.map(|m| (m, m.label(), m.key().map(|(_, at)| at)));
                        if ui::tabs(ui, &mut mode, &tabs) {
                            self.set_mode(mode);
                        }
                    });
                // The bar at the bottom before the tab, so that the tab gets
                // the height that is left above it: Export in the Optimize
                // tab, Render and Save in the Render tab. The Model tab has
                // none. The bar goes to the edges of the panel.
                let bar = |id: &'static str| {
                    egui::TopBottomPanel::bottom(id)
                        .frame(
                            egui::Frame::NONE
                                .fill(ui::c().card)
                                .inner_margin(egui::Margin::symmetric(PAD, 14)),
                        )
                        .show_separator_line(false)
                };
                let shown = match self.mode {
                    Mode::Model => None,
                    Mode::Optimize => Some(
                        bar("export")
                            .show_inside(ui, |ui| self.export_bar(ui))
                            .response,
                    ),
                    Mode::Render => Some(
                        bar("render")
                            .show_inside(ui, |ui| self.render_bar(ui))
                            .response,
                    ),
                };
                if let Some(r) = &shown {
                    ui.painter().hline(r.rect.x_range(), r.rect.top(), line);
                }
                egui::CentralPanel::default()
                    // No side margin: the tabs put it inside their scroll
                    // area, so the scroll bar is at the edge of the panel.
                    .frame(egui::Frame::NONE.inner_margin(egui::Margin {
                        left: 0,
                        right: 0,
                        top: PAD,
                        bottom: 0,
                    }))
                    .show_inside(ui, |ui| match self.mode {
                        Mode::Model => self.model_panel(ui),
                        Mode::Optimize => self.optimize_panel(ui),
                        Mode::Render => self.render_panel(ui),
                    });
            })
            .response
            .rect;
        edge(ctx, side_panel.left() + 0.5, side_panel);
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| self.viewport(ui));
        self.menu_popup(ctx);
        self.update_notice(ctx);
        // Last, so that the jobs of this frame start now.
        self.step_gpu(ctx);
    }

    fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        self.flush_autosave();
        self.save_state();
        if let Some(j) = &self.job {
            j.cancel.store(true, Ordering::Relaxed);
        }
        if let Some(j) = &self.parts_job {
            j.cancel.store(true, Ordering::Relaxed);
        }
        if let Some(gl) = gl {
            self.queue_thumb();
            self.draw_thumbs(gl);
        }
        if let (Some(gl), Some(r)) = (gl, &self.renderer) {
            r.lock().unwrap().destroy(gl);
        }
        if let (Some(gl), Gpu::Ready(t)) = (gl, &mut self.gpu) {
            t.shutdown(gl);
        }
    }
}

/// The color swatches and a color button. `current` has a ring. Returns
/// the color that the user selected in this frame.
fn swatch_row(ui: &mut egui::Ui, current: Option<[u8; 3]>) -> Option<[u8; 3]> {
    let mut out = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(8.0, 8.0);
        for (name, c) in SWATCHES {
            let (r, resp) = ui.allocate_exact_size(Vec2::splat(26.0), egui::Sense::click());
            let col = Color32::from_rgb(c[0], c[1], c[2]);
            ui.painter().circle_filled(r.center(), 12.0, col);
            // Outline, so black and white swatches show on both themes.
            ui.painter()
                .circle_stroke(r.center(), 12.0, egui::Stroke::new(1.0_f32, ui::c().line));
            if current == Some(c) {
                ui.painter().circle_stroke(
                    r.center(),
                    12.5,
                    egui::Stroke::new(2.0_f32, ui::c().text),
                );
            }
            if resp.on_hover_text(name).clicked() {
                out = Some(c);
            }
        }
        let mut color = current.unwrap_or(SWATCHES[0].1);
        if ui
            .color_edit_button_srgb(&mut color)
            .on_hover_text("Any color")
            .changed()
        {
            out = Some(color);
        }
    });
    out
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
    // The label row and the slider sit close together, as the parameters.
    ui.scope(|ui| {
        let sp = ui.spacing_mut();
        sp.item_spacing.y = 2.0;
        sp.interact_size.y = 20.0;
        setting_lines(ui, label, v, range, step, suffix)
    })
    .inner
}

fn setting_lines<N: egui::emath::Numeric>(
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

/// One parameter row. Returns true when the value changed. The label line
/// and the control sit close together, so that more parameters fit.
fn param_row(ui: &mut egui::Ui, p: &mut Param) -> bool {
    ui.scope(|ui| {
        let sp = ui.spacing_mut();
        sp.item_spacing.y = 2.0;
        sp.interact_size.y = 20.0;
        param_lines(ui, p)
    })
    .inner
}

fn param_lines(ui: &mut egui::Ui, p: &mut Param) -> bool {
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
