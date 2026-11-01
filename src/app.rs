//! The egui application: parameter panel, 3D view, export and slice preview.

use crate::export::{self, Format, Meta};
use crate::mesh::{Bounds, Mesh};
use crate::openscad;
use crate::params::{self, Param, Value, Widget};
use crate::presets;
use crate::render::{self, Renderer, View};
use crate::slicer::{self, Kind, Settings, Sliced};
use crate::ui::{self, ACCENT};
use eframe::egui::{self, Color32, RichText, Vec2};
use glam::{Mat4, Vec3};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

const DEBOUNCE: Duration = Duration::from_millis(350);

#[derive(PartialEq, Clone, Copy)]
enum Mode {
    Model,
    Slice,
}

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

    fn pan(&mut self, d: Vec2, height_px: f32) {
        let eye = self.eye();
        let fwd = (self.target - eye).normalize();
        let right = fwd.cross(Vec3::Z).normalize_or_zero();
        let up = right.cross(fwd);
        let scale = 2.0 * self.dist * (Self::FOV * 0.5).tan() / height_px.max(1.0);
        self.target += (-right * d.x + up * d.y) * scale;
    }
}

struct RenderJob {
    rx: Receiver<Result<openscad::RenderOutput, String>>,
    cancel: Arc<AtomicBool>,
    started: Instant,
}

struct MeshInfo {
    bounds: Bounds,
    volume: f32,
    open_edges: usize,
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
    /// Parameter sets in `<model>.json`.
    sets: Vec<String>,
    sets_file: PathBuf,
    active_set: Option<String>,
    set_edited: bool,
    /// Name being typed for "Save as".
    new_set: Option<String>,
    confirm_delete: bool,
    collapsed: std::collections::HashSet<String>,
    cfg: openscad::Config,
    cfg_edit: Option<openscad::Config>,
    engine: String,
    renderer: Option<Arc<Mutex<Renderer>>>,
    toast: Option<(String, Instant)>,
    export_rx: Option<Receiver<Result<PathBuf, String>>>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, file: Option<PathBuf>, slice: bool) -> App {
        ui::theme(&cc.egui_ctx);
        let renderer = cc.gl.as_ref().and_then(|gl| match Renderer::new(gl) {
            Ok(r) => Some(Arc::new(Mutex::new(r))),
            Err(e) => {
                eprintln!("renderer: {e}");
                None
            }
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
            info: None,
            fit_pending: true,
            mode: if slice { Mode::Slice } else { Mode::Model },
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
            sets: Vec::new(),
            sets_file: PathBuf::new(),
            active_set: None,
            set_edited: false,
            new_set: None,
            confirm_delete: false,
            collapsed: Default::default(),
            cfg: openscad::Config::load(),
            cfg_edit: None,
            engine: String::new(),
            renderer,
            toast: None,
            export_rx: None,
        };
        if let Some(f) = file {
            app.open(f);
        }
        app
    }

    fn open(&mut self, path: PathBuf) {
        match std::fs::read_to_string(&path) {
            Ok(src) => {
                self.params = params::parse(&src);
                self.mtime = mtime(&path);
                self.sets_file = presets::file_for(&path);
                self.sets = presets::names(&self.sets_file);
                self.active_set = None;
                self.new_set = None;
                self.file = Some(path);
                self.mesh = None;
                self.info = None;
                self.sliced = None;
                self.mesh_rev += 1;
                self.slice_rev += 1;
                self.fit_pending = true;
                self.error = None;
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
            if let Some(old) = self.params.iter().find(|o| o.name == p.name && o.changed()) {
                if std::mem::discriminant(&old.value) == std::mem::discriminant(&p.value) {
                    p.value = old.value.clone();
                }
            }
        }
        self.params = fresh;
        self.sets = presets::names(&self.sets_file);
        self.mtime = mtime(&path);
        self.start_render();
    }

    fn start_render(&mut self) {
        let Some(file) = self.file.clone() else {
            return;
        };
        if let Some(j) = self.job.take() {
            j.cancel.store(true, Ordering::Relaxed);
        }
        self.dirty_since = None;
        let defines = params::defines(&self.params);
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
            let _ = tx.send(r);
        });
        self.job = Some(RenderJob {
            rx,
            cancel,
            started: Instant::now(),
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
        if let Some(job) = &self.job {
            if let Ok(r) = job.rx.try_recv() {
                self.job = None;
                match r {
                    Ok(out) => {
                        let m = out.mesh;
                        self.info = m.bounds().map(|bounds| MeshInfo {
                            bounds,
                            volume: m.volume().abs(),
                            open_edges: m.open_edges(),
                        });
                        self.mesh = Some(Arc::new(m));
                        self.mesh_rev += 1;
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
        }
        if let Some(rx) = &self.slice_job {
            if let Ok((rev, sliced, t)) = rx.try_recv() {
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
        }
        if let Some(rx) = &self.export_rx {
            if let Ok(r) = rx.try_recv() {
                self.export_rx = None;
                self.toast = Some((
                    match r {
                        Ok(p) => format!("Saved {}", p.display()),
                        Err(e) => format!("Export failed: {e}"),
                    },
                    Instant::now(),
                ));
            }
        }

        // Watch the file for outside edits.
        if self.last_poll.elapsed() > Duration::from_millis(700) {
            self.last_poll = Instant::now();
            if let Some(f) = &self.file {
                let m = mtime(f);
                if m.is_some() && m != self.mtime {
                    self.reload();
                }
            }
        }

        if self.auto_render && self.dirty_since.is_some_and(|t| t.elapsed() > DEBOUNCE) {
            self.start_render();
        }
        if self
            .slice_dirty_since
            .is_some_and(|t| t.elapsed() > DEBOUNCE)
        {
            self.start_slice();
        }
        if self.job.is_some() || self.slice_job.is_some() || self.export_rx.is_some() {
            ctx.request_repaint_after(Duration::from_millis(50));
        } else {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
    }

    fn fit(&mut self) {
        if let Some(i) = &self.info {
            self.cam.fit(&i.bounds);
        }
    }

    fn set_view(&mut self, yaw: f32, pitch: f32) {
        self.cam.yaw = yaw;
        self.cam.pitch = pitch;
        self.fit();
    }

    fn stem(&self) -> String {
        self.file
            .as_ref()
            .and_then(|f| f.file_stem())
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "model".into())
    }

    fn export(&mut self, fmt: Format) {
        let Some(mesh) = self.mesh.clone() else {
            return;
        };
        let mut dialog = rfd::FileDialog::new()
            .set_file_name(export::suggested_name(&self.stem(), fmt))
            .add_filter(fmt.label(), &[fmt.ext()]);
        if let Some(dir) = self.file.as_ref().and_then(|f| f.parent()) {
            dialog = dialog.set_directory(dir);
        }
        let Some(path) = dialog.save_file() else {
            return;
        };
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

    fn pick_file(&mut self) {
        let mut d = rfd::FileDialog::new().add_filter("OpenSCAD", &["scad"]);
        if let Some(dir) = self.file.as_ref().and_then(|f| f.parent()) {
            d = d.set_directory(dir);
        }
        if let Some(p) = d.pick_file() {
            self.open(p);
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_centered(|ui| {
            // Brand and file.
            let (r, _) = ui.allocate_exact_size(Vec2::splat(22.0), egui::Sense::hover());
            ui.painter().rect_filled(r, 6.0, ui::ACCENT);
            ui.painter().rect_filled(r.shrink(6.0), 2.0, ui::PANEL);
            ui.label(RichText::new("SCAD Viewer").size(16.0).strong());
            if let Some(f) = &self.file {
                let name = f
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                ui.label(RichText::new(name).color(ui::DIM))
                    .on_hover_text(f.display().to_string());
            }
            ui.add_space(12.0);
            if ui
                .add(ui::ghost("Open", false))
                .on_hover_text("Ctrl+O")
                .clicked()
            {
                self.pick_file();
            }

            // Right side, laid out from the right edge.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let busy = self.job.is_some();
                let can_export = self.mesh.is_some() && !busy && self.export_rx.is_none();
                ui.add_enabled_ui(can_export, |ui| {
                    egui::menu::menu_custom_button(ui, ui::primary("Export"), |ui| {
                        ui.set_min_width(200.0);
                        for fmt in Format::ALL {
                            if ui
                                .add(ui::ghost(fmt.label(), false).min_size(Vec2::new(200.0, 32.0)))
                                .clicked()
                            {
                                ui.close_menu();
                                self.export(fmt);
                            }
                        }
                    });
                });
                if ui
                    .add_enabled(self.file.is_some(), ui::ghost("Render", false))
                    .on_hover_text("F5")
                    .clicked()
                {
                    self.start_render();
                }
                ui.label(RichText::new("Auto").color(ui::DIM));
                ui::toggle(ui, &mut self.auto_render)
                    .on_hover_text("Render when a parameter changes");
                ui.add_space(8.0);
                if ui.add(ui::ghost("Console", self.show_log)).clicked() {
                    self.show_log = !self.show_log;
                }
                if ui
                    .add(ui::ghost("Settings", self.cfg_edit.is_some()))
                    .clicked()
                {
                    self.cfg_edit = Some(self.cfg.clone());
                }

                // Mode switch in the middle of the remaining space.
                let free = ui.available_width();
                ui.add_space(((free - 170.0) / 2.0).max(0.0));
                let mut mode = self.mode;
                if ui::segmented(
                    ui,
                    &mut mode,
                    &[(Mode::Slice, "Slice"), (Mode::Model, "Model")],
                ) {
                    self.set_mode(mode);
                }
            });
        });
    }

    fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
        if mode == Mode::Slice && self.slice_for_rev != self.mesh_rev {
            self.start_slice();
        }
    }

    fn param_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Parameters");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().button_padding = Vec2::new(8.0, 5.0);
                let all_closed = !self.params.is_empty()
                    && self
                        .params
                        .iter()
                        .all(|p| self.collapsed.contains(&p.group));
                let label = if all_closed {
                    "Expand all"
                } else {
                    "Collapse all"
                };
                if ui.add(ui::ghost(label, false)).clicked() {
                    if all_closed {
                        self.collapsed.clear();
                    } else {
                        self.collapsed = self.params.iter().map(|p| p.group.clone()).collect();
                    }
                }
                let any = self.params.iter().any(|p| p.changed());
                if ui.add_enabled(any, ui::ghost("Reset all", false)).clicked() {
                    for p in &mut self.params {
                        p.value = p.default.clone();
                    }
                    self.dirty_since = Some(Instant::now());
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
            self.set_edited = true;
        }
    }

    fn sets_bar(&mut self, ui: &mut egui::Ui) {
        let Some(file) = self.file.clone() else {
            return;
        };
        let mut path = self.sets_file.clone();
        let mut msg: Option<String> = None;
        let mut switch_to: Option<PathBuf> = None;
        ui::card(ui, |ui| {
            ui::section(ui, "Parameter set");
            ui.horizontal(|ui| {
                let name = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                let exists = path.exists();
                let text = if exists {
                    name
                } else {
                    format!("{name} (new)")
                };
                ui.label(ui::dim(text))
                    .on_hover_text(path.display().to_string());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().button_padding = Vec2::new(8.0, 4.0);
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let dir = path
                        .parent()
                        .filter(|d| d.is_dir())
                        .or(file.parent())
                        .map(|d| d.to_path_buf());
                    if ui
                        .add(ui::ghost("New file", false))
                        .on_hover_text("Choose where to save a new sets file")
                        .clicked()
                    {
                        let mut d = rfd::FileDialog::new()
                            .add_filter("Parameter sets", &["json"])
                            .set_file_name(path.file_name().unwrap_or_default().to_string_lossy());
                        if let Some(dir) = &dir {
                            d = d.set_directory(dir);
                        }
                        switch_to = d.save_file();
                    }
                    if ui
                        .add(ui::ghost("Open file", false))
                        .on_hover_text("Load sets from a .json file")
                        .clicked()
                    {
                        let mut d = rfd::FileDialog::new().add_filter("Parameter sets", &["json"]);
                        if let Some(dir) = &dir {
                            d = d.set_directory(dir);
                        }
                        switch_to = d.pick_file();
                    }
                });
            });
            {
                let label = match &self.active_set {
                    Some(n) if self.set_edited => format!("{n} (edited)"),
                    Some(n) => n.clone(),
                    None => "Model defaults".into(),
                };
                let mut pick: Option<Option<String>> = None;
                egui::ComboBox::from_id_salt("sets")
                    .selected_text(label)
                    .width(ui.available_width())
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
                if let Some(choice) = pick {
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
                    self.active_set = choice;
                    self.set_edited = false;
                    self.confirm_delete = false;
                    self.dirty_since = Some(Instant::now());
                }
            }
            ui.horizontal(|ui| {
                ui.spacing_mut().button_padding = Vec2::new(10.0, 5.0);
                ui.spacing_mut().item_spacing.x = 4.0;
                let save = ui
                    .add(ui::ghost("Save", false))
                    .on_hover_text(match &self.active_set {
                        Some(n) => format!("Save the values to '{n}'"),
                        None => "Save the values as a new set".into(),
                    });
                if save.clicked() {
                    match self.active_set.clone() {
                        Some(n) => match presets::save(&path, &n, &self.params) {
                            Ok(()) => {
                                self.set_edited = false;
                                msg = Some(format!("Saved '{n}'"));
                            }
                            Err(e) => msg = Some(e.to_string()),
                        },
                        None => self.new_set = Some(String::new()),
                    }
                }
                if ui
                    .add(ui::ghost("Save as", false))
                    .on_hover_text("Save the values as a new set")
                    .clicked()
                {
                    self.new_set = Some(String::new());
                }
                if let Some(n) = self.active_set.clone() {
                    let text = if self.confirm_delete {
                        "Sure?"
                    } else {
                        "Delete"
                    };
                    let b = egui::Button::new(RichText::new(text).color(ui::DANGER))
                        .fill(Color32::TRANSPARENT)
                        .corner_radius(8);
                    if ui
                        .add(b)
                        .on_hover_text(format!("Delete '{n}' from {}", path.display()))
                        .clicked()
                    {
                        if self.confirm_delete {
                            match presets::delete(&path, &n) {
                                Ok(()) => {
                                    msg = Some(format!("Deleted '{n}'"));
                                    self.active_set = None;
                                }
                                Err(e) => msg = Some(e.to_string()),
                            }
                            self.confirm_delete = false;
                        } else {
                            self.confirm_delete = true;
                        }
                    }
                }
            });

            if let Some(name) = &mut self.new_set {
                ui.add_space(4.0);
                let mut submit = false;
                let mut cancel = false;
                ui.horizontal(|ui| {
                    let r = ui.add(
                        egui::TextEdit::singleline(name)
                            .hint_text("Name of the new set")
                            .desired_width(ui.available_width() - 150.0)
                            .margin(Vec2::new(10.0, 6.0)),
                    );
                    r.request_focus();
                    submit = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    cancel = ui.input(|i| i.key_pressed(egui::Key::Escape));
                    ui.spacing_mut().button_padding = Vec2::new(10.0, 5.0);
                    submit |= ui
                        .add_enabled(!name.trim().is_empty(), ui::primary("Save"))
                        .clicked();
                    cancel |= ui.add(ui::ghost("Cancel", false)).clicked();
                });
                let name = name.trim().to_string();
                if submit && !name.is_empty() {
                    match presets::save(&path, &name, &self.params) {
                        Ok(()) => {
                            msg = Some(format!("Saved '{name}'"));
                            self.active_set = Some(name);
                            self.set_edited = false;
                        }
                        Err(e) => msg = Some(e.to_string()),
                    }
                    self.new_set = None;
                } else if cancel {
                    self.new_set = None;
                }
            }
        });
        if let Some(p) = switch_to {
            if let Err(e) = presets::remember(&file, &p) {
                msg = Some(format!("Cannot remember the sets file: {e}"));
            }
            path = p;
            self.sets_file = path.clone();
            self.active_set = None;
            self.set_edited = false;
            self.confirm_delete = false;
            let n = presets::names(&path).len();
            msg = msg.or(Some(format!(
                "{n} sets in {}",
                path.file_name().unwrap_or_default().to_string_lossy()
            )));
        }
        if msg.is_some() {
            self.sets = presets::names(&path);
        }
        if let Some(m) = msg {
            self.toast = Some((m, Instant::now()));
        }
    }

    fn slice_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Slice preview");
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
        let top = Color32::from_rgb(0x26, 0x29, 0x30);
        let bottom = Color32::from_rgb(0x0e, 0x0f, 0x12);
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
        if resp.double_clicked() {
            self.fit();
        }

        let layer_max = self.sliced.as_ref().map(|s| s.layers.len()).unwrap_or(0);
        let slicing = self.mode == Mode::Slice && layer_max > 0;
        if let Some(r) = &self.renderer {
            let view = View {
                mvp: self.cam.matrix(rect.width() / rect.height().max(1.0)),
                eye: self.cam.eye(),
                show_mesh: !slicing || self.ghost,
                mesh_alpha: if slicing { 0.12 } else { 1.0 },
                wireframe: self.wireframe,
                grid: self.grid,
                layer: slicing.then_some(self.layer),
            };
            let (mesh, mesh_rev) = (self.mesh.clone(), self.mesh_rev);
            let (sliced, slice_rev) = (self.sliced.clone(), self.slice_rev);
            let bounds = self.info.as_ref().map(|i| i.bounds);
            let r = r.clone();
            let cb = egui::PaintCallback {
                rect,
                callback: Arc::new(eframe::egui_glow::CallbackFn::new(move |_info, painter| {
                    let gl = painter.gl();
                    let mut r = r.lock().unwrap();
                    r.set_mesh(gl, mesh.as_deref(), mesh_rev);
                    r.set_slice(gl, sliced.as_deref(), slice_rev);
                    let b = bounds.unwrap_or(Bounds {
                        min: Vec3::splat(-50.0),
                        max: Vec3::splat(50.0),
                    });
                    r.set_grid(gl, b.min, b.max);
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
                ui::DANGER,
            );
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
                            for (label, yaw, pitch) in [
                                ("Right", 0.0, 0.0),
                                ("Front", -std::f32::consts::FRAC_PI_2, 0.0),
                                ("Top", -std::f32::consts::FRAC_PI_2, 1.5697),
                                ("Iso", -0.9, 0.55),
                            ] {
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
            let mut layer = self.layer;
            ui.scope_builder(egui::UiBuilder::new().max_rect(strip), |ui| {
                ui.spacing_mut().slider_width = strip.height();
                ui.spacing_mut().slider_rail_height = 6.0;
                ui.vertical_centered(|ui| {
                    ui.add(
                        egui::Slider::new(&mut layer, 0..=layer_max - 1)
                            .vertical()
                            .show_value(false),
                    );
                });
            });
            self.layer = layer;
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

        // Status, top left.
        let painter = ui.painter_at(rect);
        let font = egui::FontId::proportional(13.5);
        let mut y = rect.top() + m;
        if let Some(job) = &self.job {
            let t = job.started.elapsed().as_secs_f32();
            let r = ui::pill(
                &painter,
                egui::pos2(rect.left() + m, y),
                egui::Align2::LEFT_TOP,
                &format!("      Rendering with OpenSCAD   {t:.1} s"),
                font.clone(),
                ui::TEXT,
                ui::glass(),
            );
            let c = egui::pos2(r.left() + 20.0, r.center().y);
            let a = ui.input(|i| i.time) as f32 * 6.0;
            let pts: Vec<egui::Pos2> = (0..=16)
                .map(|k| {
                    let ang = a + k as f32 / 16.0 * 4.2;
                    c + Vec2::angled(ang) * 6.0
                })
                .collect();
            painter.add(egui::Shape::line(
                pts,
                egui::Stroke::new(2.0_f32, ui::ACCENT),
            ));
            y = r.bottom() + 8.0;
        } else if self.dirty_since.is_some() && !self.auto_render {
            let r = ui::pill(
                &painter,
                egui::pos2(rect.left() + m, y),
                egui::Align2::LEFT_TOP,
                "Parameters changed. Press F5 to render.",
                font.clone(),
                ui::ACCENT,
                ui::glass(),
            );
            y = r.bottom() + 8.0;
        }
        if let Some(e) = &self.error {
            ui::pill(
                &painter,
                egui::pos2(rect.left() + m, y),
                egui::Align2::LEFT_TOP,
                e,
                font.clone(),
                Color32::WHITE,
                Color32::from_rgba_unmultiplied(0x9b, 0x2c, 0x2c, 230),
            );
        }

        // Model info, bottom left.
        if let Some(i) = &self.info {
            let s = i.bounds.size();
            let tris = self.mesh.as_ref().map(|m| m.triangles.len()).unwrap_or(0);
            let mut lines = vec![
                format!("{:.1} × {:.1} × {:.1} mm", s.x, s.y, s.z),
                format!("{tris} triangles   ·   {:.2} cm³", i.volume / 1000.0),
            ];
            if let Some(t) = self.last_time {
                lines.push(format!(
                    "Rendered in {:.2} s with {}",
                    t.as_secs_f32(),
                    self.engine
                ));
            }
            let r = ui::pill(
                &painter,
                rect.left_bottom() + Vec2::new(m, -m),
                egui::Align2::LEFT_BOTTOM,
                &lines.join("\n"),
                font.clone(),
                ui::DIM,
                ui::glass(),
            );
            if i.open_edges > 0 {
                ui::pill(
                    &painter,
                    egui::pos2(r.left(), r.top() - 8.0),
                    egui::Align2::LEFT_BOTTOM,
                    &format!("{} open edges. The mesh is not closed.", i.open_edges),
                    font.clone(),
                    Color32::BLACK,
                    ui::ACCENT,
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
                ui::TEXT,
                ui::glass(),
            );
            ui.ctx().request_repaint_after(Duration::from_millis(200));
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if let Some(p) = ctx.input(|i| i.raw.dropped_files.iter().find_map(|f| f.path.clone())) {
            self.open(p);
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::O)) {
            self.pick_file();
        }
        if ctx.wants_keyboard_input() {
            return;
        }
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
        let mut open = true;
        let mut action = None;
        egui::Window::new("OpenSCAD")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(460.0)
            .show(ctx, |ui| {
                egui::Grid::new("cfg").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                    ui.label("Binary");
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut edit.binary).hint_text("auto: openscad-nightly, else openscad").desired_width(300.0));
                        if ui.button("Browse").clicked() {
                            if let Some(p) = rfd::FileDialog::new().pick_file() {
                                edit.binary = p.display().to_string();
                            }
                        }
                    });
                    ui.end_row();
                    ui.label("Extra arguments");
                    ui.add(egui::TextEdit::singleline(&mut edit.args).hint_text("e.g. --enable=lazy-union").desired_width(300.0));
                    ui.end_row();
                    ui.label("");
                    ui.checkbox(&mut edit.manifold, "Use the Manifold backend when available");
                    ui.end_row();
                    let probe = openscad::probe(&edit.binary());
                    ui.label("Found");
                    ui.label(RichText::new(&probe.version).monospace());
                    ui.end_row();
                    ui.label("Manifold");
                    if probe.manifold {
                        ui.label(RichText::new("supported").color(ACCENT));
                    } else {
                        ui.label(RichText::new("not supported. Use an OpenSCAD development build (2024 or later).").weak());
                    }
                    ui.end_row();
                });
                if std::env::var_os("OPENSCAD").is_some() || std::env::var_os("OPENSCAD_ARGS").is_some() {
                    ui.label(RichText::new("$OPENSCAD / $OPENSCAD_ARGS are set and override these values.").small().weak());
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Save and render").clicked() {
                        action = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        action = Some(false);
                    }
                });
            });
        match (open, action) {
            (_, Some(true)) => {
                if let Err(e) = edit.save() {
                    self.toast = Some((format!("Cannot save settings: {e}"), Instant::now()));
                }
                self.cfg = edit;
                self.start_render();
            }
            (false, _) | (_, Some(false)) => {}
            _ => self.cfg_edit = Some(edit),
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll(ctx);
        self.shortcuts(ctx);
        let title = match &self.file {
            Some(f) => format!(
                "SCAD Viewer · {}",
                f.file_name().unwrap_or_default().to_string_lossy()
            ),
            None => "SCAD Viewer".into(),
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));

        let line = egui::Stroke::new(1.0_f32, ui::LINE);
        egui::TopBottomPanel::top("top")
            .exact_height(60.0)
            .frame(
                egui::Frame::NONE
                    .fill(ui::BG)
                    .inner_margin(egui::Margin::symmetric(18, 0))
                    .stroke(line),
            )
            .show(ctx, |ui| self.top_bar(ui));
        self.settings_window(ctx);
        if self.show_log {
            egui::TopBottomPanel::bottom("log")
                .resizable(true)
                .default_height(160.0)
                .frame(
                    egui::Frame::NONE
                        .fill(ui::BG)
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
                            ui.label(RichText::new(text).monospace().color(ui::DIM));
                        });
                });
        }
        let side = egui::Frame::NONE
            .fill(ui::PANEL)
            .inner_margin(egui::Margin::same(18))
            .stroke(line);
        egui::SidePanel::left("params")
            .resizable(true)
            .default_width(340.0)
            .width_range(280.0..=560.0)
            .frame(side)
            .show(ctx, |ui| self.param_panel(ui));
        if self.mode == Mode::Slice {
            egui::SidePanel::right("slice")
                .resizable(true)
                .default_width(320.0)
                .width_range(280.0..=480.0)
                .frame(side)
                .show(ctx, |ui| self.slice_panel(ui));
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| self.viewport(ui));
    }

    fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        if let Some(j) = &self.job {
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

fn stat(ui: &mut egui::Ui, value: &str, label: &str) {
    ui.label(RichText::new(value).size(16.0));
    ui.label(ui::dim(label));
}

/// One parameter row. Returns true when the value changed.
fn param_row(ui: &mut egui::Ui, p: &mut Param) -> bool {
    let before = p.value.clone();
    let label = humanize(&p.name);
    let changed = p.changed();
    let step = p.step();
    let decimals = decimals(step);

    ui.add_space(2.0);
    ui.horizontal(|ui| {
        if changed {
            let (r, _) = ui.allocate_exact_size(Vec2::new(6.0, 6.0), egui::Sense::hover());
            ui.painter().circle_filled(r.center(), 3.0, ui::ACCENT);
        }
        let hover = if p.description.is_empty() {
            p.name.clone()
        } else {
            format!("{}\n\n{}", p.description, p.name)
        };
        ui.label(RichText::new(&label).size(14.0))
            .on_hover_text(hover);

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
                    .add(egui::Button::new(RichText::new("↺").color(ui::DIM)).frame(false))
                    .on_hover_text(format!("Reset to {}", p.default.to_scad()))
                    .clicked()
            {
                p.value = p.default.clone();
            }
        });
    });
    if !p.description.is_empty() {
        ui.add_space(-6.0);
        ui.label(ui::dim(&p.description));
    }

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
