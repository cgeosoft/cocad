//! The egui application: parameter panel, 3D view, export and slice preview.

use crate::export::{self, Format, Meta};
use crate::mesh::{Bounds, Mesh};
use crate::openscad;
use crate::params::{self, Param, Value, Widget};
use crate::render::{self, Renderer, View};
use crate::slicer::{self, Kind, Settings, Sliced};
use eframe::egui::{self, Color32, RichText, Vec2};
use glam::{Mat4, Vec3};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

const ACCENT: Color32 = Color32::from_rgb(0xff, 0x93, 0x00);
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
    renderer: Option<Arc<Mutex<Renderer>>>,
    toast: Option<(String, Instant)>,
    export_rx: Option<Receiver<Result<PathBuf, String>>>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, file: Option<PathBuf>, slice: bool) -> App {
        style(&cc.egui_ctx);
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
        std::thread::spawn(move || {
            let r = openscad::render(&file, &defines, &c).map_err(|e| e.to_string());
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
        egui::menu::bar(ui, |ui| {
            if ui.button("📂 Open").on_hover_text("Ctrl+O").clicked() {
                self.pick_file();
            }
            let busy = self.job.is_some();
            if ui
                .add_enabled(self.file.is_some(), egui::Button::new("⟳ Render"))
                .on_hover_text("F5")
                .clicked()
            {
                self.start_render();
            }
            ui.checkbox(&mut self.auto_render, "Auto");
            ui.separator();

            let can_export = self.mesh.is_some() && !busy && self.export_rx.is_none();
            ui.add_enabled_ui(can_export, |ui| {
                ui.menu_button("💾 Export", |ui| {
                    for fmt in Format::ALL {
                        if ui.button(fmt.label()).clicked() {
                            ui.close_menu();
                            self.export(fmt);
                        }
                    }
                });
            });
            ui.separator();

            ui.selectable_value(&mut self.mode, Mode::Model, "Model");
            if ui
                .selectable_value(&mut self.mode, Mode::Slice, "Slice")
                .clicked()
                && self.slice_for_rev != self.mesh_rev
            {
                self.start_slice();
            }
            ui.separator();

            ui.menu_button("👁 View", |ui| {
                for (label, yaw, pitch) in [
                    ("Iso", -0.9, 0.55),
                    ("Top", -std::f32::consts::FRAC_PI_2, 1.5697),
                    ("Front", -std::f32::consts::FRAC_PI_2, 0.0),
                    ("Right", 0.0, 0.0),
                    ("Back", std::f32::consts::FRAC_PI_2, 0.0),
                    ("Left", std::f32::consts::PI, 0.0),
                ] {
                    if ui.button(label).clicked() {
                        self.set_view(yaw, pitch);
                        ui.close_menu();
                    }
                }
                ui.separator();
                if ui.button("Fit (F)").clicked() {
                    self.fit();
                    ui.close_menu();
                }
                ui.checkbox(&mut self.wireframe, "Wireframe (W)");
                ui.checkbox(&mut self.grid, "Bed grid");
                ui.checkbox(&mut self.show_log, "Console");
            });

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(f) = &self.file {
                    ui.label(RichText::new(f.display().to_string()).weak());
                }
            });
        });
    }

    fn param_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Parameters");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let any = self.params.iter().any(|p| p.changed());
                if ui
                    .add_enabled(any, egui::Button::new("Reset all"))
                    .clicked()
                {
                    for p in &mut self.params {
                        p.value = p.default.clone();
                    }
                    self.dirty_since = Some(Instant::now());
                }
            });
        });
        ui.separator();
        if self.file.is_none() {
            ui.label("Open a .scad file, or drop one on the window.");
            return;
        }
        if self.params.is_empty() {
            ui.label(RichText::new("No Customizer parameters in this file.").weak());
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
                    egui::CollapsingHeader::new(RichText::new(&g).strong())
                        .default_open(true)
                        .show(ui, |ui| {
                            for p in self.params.iter_mut().filter(|p| p.group == g) {
                                changed |= param_row(ui, p);
                            }
                        });
                }

                let defs = params::defines(&self.params);
                if !defs.is_empty() {
                    ui.add_space(8.0);
                    if ui.small_button("Copy as -D arguments").clicked() {
                        let s: Vec<String> = defs.iter().map(|d| format!("-D '{d}'")).collect();
                        ui.ctx().copy_text(s.join(" "));
                        self.toast = Some(("Copied -D arguments".into(), Instant::now()));
                    }
                }
            });
        if changed {
            self.dirty_since = Some(Instant::now());
        }
    }

    fn slice_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Slice preview");
        ui.label(
            RichText::new("Rough preview. Not for printing.")
                .weak()
                .small(),
        );
        ui.separator();
        let before = self.settings;
        let s = &mut self.settings;
        egui::Grid::new("slice")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                ui.label("Layer height");
                ui.add(
                    egui::Slider::new(&mut s.layer_height, 0.05..=0.6)
                        .step_by(0.01)
                        .suffix(" mm"),
                );
                ui.end_row();
                ui.label("First layer");
                ui.add(
                    egui::Slider::new(&mut s.first_layer, 0.05..=0.6)
                        .step_by(0.01)
                        .suffix(" mm"),
                );
                ui.end_row();
                ui.label("Line width");
                ui.add(
                    egui::Slider::new(&mut s.line_width, 0.2..=1.2)
                        .step_by(0.01)
                        .suffix(" mm"),
                );
                ui.end_row();
                ui.label("Walls");
                ui.add(egui::Slider::new(&mut s.walls, 0..=6));
                ui.end_row();
                ui.label("Infill");
                let mut pct = s.infill * 100.0;
                if ui
                    .add(
                        egui::Slider::new(&mut pct, 0.0..=100.0)
                            .step_by(1.0)
                            .suffix(" %"),
                    )
                    .changed()
                {
                    s.infill = pct / 100.0;
                }
                ui.end_row();
                ui.label("Top/bottom layers");
                ui.add(egui::Slider::new(&mut s.solid_layers, 0..=10));
                ui.end_row();
                ui.label("Speed");
                ui.add(
                    egui::Slider::new(&mut s.speed, 10.0..=300.0)
                        .step_by(5.0)
                        .suffix(" mm/s"),
                );
                ui.end_row();
            });
        if self.settings != before {
            self.slice_dirty_since = Some(Instant::now());
        }
        ui.checkbox(&mut self.ghost, "Show model ghost");
        ui.separator();

        if self.slice_job.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Slicing…");
            });
        }
        if let Some(sl) = &self.sliced {
            let st = &sl.stats;
            egui::Grid::new("stats").num_columns(2).show(ui, |ui| {
                ui.label("Layers");
                ui.label(st.layers.to_string());
                ui.end_row();
                ui.label("Print time");
                ui.label(
                    RichText::new(fmt_duration(st.seconds))
                        .color(ACCENT)
                        .strong(),
                );
                ui.end_row();
                ui.label("Filament");
                ui.label(format!("{:.2} m  ·  {:.1} g", st.filament_m, st.grams));
                ui.end_row();
                if let Some(t) = self.slice_time {
                    ui.label("Sliced in");
                    ui.label(format!("{} ms", t.as_millis()));
                    ui.end_row();
                }
            });
            ui.separator();
            if let Some(l) = sl.layers.get(self.layer) {
                ui.label(format!(
                    "Layer {} / {}  ·  z = {:.2} mm",
                    self.layer + 1,
                    sl.layers.len(),
                    l.z
                ));
            }
            ui.add_space(4.0);
            for k in [Kind::OuterWall, Kind::InnerWall, Kind::Infill, Kind::Solid] {
                let c = render::kind_color(k);
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(Vec2::new(14.0, 4.0), egui::Sense::hover());
                    ui.painter().rect_filled(r, 1.0, rgb(c));
                    ui.label(match k {
                        Kind::OuterWall => "Outer wall",
                        Kind::InnerWall => "Inner wall",
                        Kind::Infill => "Sparse infill",
                        Kind::Solid => "Solid infill",
                    });
                });
            }
            ui.label(
                RichText::new("Up/Down or PgUp/PgDn change the layer.")
                    .weak()
                    .small(),
            );
        }
    }

    fn viewport(&mut self, ui: &mut egui::Ui) {
        let rect = ui.available_rect_before_wrap();
        let resp = ui.allocate_rect(rect, egui::Sense::click_and_drag());

        // Background gradient.
        let mut bg = egui::Mesh::default();
        let top = Color32::from_rgb(0x2a, 0x2d, 0x34);
        let bottom = Color32::from_rgb(0x12, 0x13, 0x16);
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

        if let Some(r) = &self.renderer {
            let layer_max = self.sliced.as_ref().map(|s| s.layers.len()).unwrap_or(0);
            let slicing = self.mode == Mode::Slice && layer_max > 0;
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

            if slicing {
                let strip = egui::Rect::from_min_max(
                    egui::pos2(rect.right() - 44.0, rect.top() + 40.0),
                    egui::pos2(rect.right() - 12.0, rect.bottom() - 40.0),
                );
                let mut layer = self.layer;
                ui.scope_builder(egui::UiBuilder::new().max_rect(strip), |ui| {
                    ui.spacing_mut().slider_width = strip.height();
                    ui.add(
                        egui::Slider::new(&mut layer, 0..=layer_max - 1)
                            .vertical()
                            .show_value(false),
                    );
                });
                self.layer = layer;
                ui.painter().text(
                    egui::pos2(strip.center().x - 8.0, strip.top() - 8.0),
                    egui::Align2::CENTER_BOTTOM,
                    format!("{}", self.layer + 1),
                    egui::FontId::monospace(13.0),
                    ACCENT,
                );
            }
        } else {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "OpenGL renderer not available",
                egui::FontId::proportional(16.0),
                Color32::LIGHT_RED,
            );
        }

        // Overlays.
        let painter = ui.painter_at(rect);
        let mono = egui::FontId::monospace(12.5);
        if let Some(job) = &self.job {
            let t = job.started.elapsed().as_secs_f32();
            let pos = rect.left_top() + Vec2::new(14.0, 14.0);
            painter.text(
                pos,
                egui::Align2::LEFT_TOP,
                format!("Rendering with OpenSCAD… {t:.1} s"),
                mono.clone(),
                ACCENT,
            );
        } else if self.dirty_since.is_some() && !self.auto_render {
            painter.text(
                rect.left_top() + Vec2::new(14.0, 14.0),
                egui::Align2::LEFT_TOP,
                "Parameters changed. Press F5 to render.",
                mono.clone(),
                ACCENT,
            );
        }
        if let Some(e) = &self.error {
            painter.text(
                rect.left_top() + Vec2::new(14.0, 34.0),
                egui::Align2::LEFT_TOP,
                format!("⚠ {e}"),
                mono.clone(),
                Color32::from_rgb(0xff, 0x6b, 0x6b),
            );
        }
        if let Some(i) = &self.info {
            let s = i.bounds.size();
            let tris = self.mesh.as_ref().map(|m| m.triangles.len()).unwrap_or(0);
            let mut lines = vec![
                format!("{:.2} × {:.2} × {:.2} mm", s.x, s.y, s.z),
                format!("{tris} triangles  ·  {:.2} cm³", i.volume / 1000.0),
            ];
            if i.open_edges > 0 {
                lines.push(format!("⚠ {} open edges (not manifold)", i.open_edges));
            }
            if let Some(t) = self.last_time {
                lines.push(format!("rendered in {:.2} s", t.as_secs_f32()));
            }
            painter.text(
                rect.left_bottom() + Vec2::new(14.0, -12.0),
                egui::Align2::LEFT_BOTTOM,
                lines.join("\n"),
                mono.clone(),
                Color32::from_gray(170),
            );
        }
        if let Some((msg, at)) = &self.toast {
            if at.elapsed() < Duration::from_secs(4) {
                painter.text(
                    rect.center_bottom() + Vec2::new(0.0, -16.0),
                    egui::Align2::CENTER_BOTTOM,
                    msg,
                    egui::FontId::proportional(14.0),
                    Color32::WHITE,
                );
                ui.ctx().request_repaint_after(Duration::from_millis(200));
            }
        }
        if self.export_rx.is_some() {
            painter.text(
                rect.center_bottom() + Vec2::new(0.0, -16.0),
                egui::Align2::CENTER_BOTTOM,
                "Exporting…",
                mono,
                ACCENT,
            );
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
            self.mode = if self.mode == Mode::Model {
                Mode::Slice
            } else {
                Mode::Model
            };
            if self.mode == Mode::Slice && self.slice_for_rev != self.mesh_rev {
                self.start_slice();
            }
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

        egui::TopBottomPanel::top("top").show(ctx, |ui| self.top_bar(ui));
        if self.show_log {
            egui::TopBottomPanel::bottom("log")
                .resizable(true)
                .default_height(140.0)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical()
                        .stick_to_bottom(true)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            let text = if self.log.is_empty() {
                                "No OpenSCAD output yet."
                            } else {
                                &self.log
                            };
                            ui.label(RichText::new(text).monospace().size(12.0));
                        });
                });
        }
        egui::SidePanel::left("params")
            .resizable(true)
            .default_width(300.0)
            .show(ctx, |ui| self.param_panel(ui));
        if self.mode == Mode::Slice {
            egui::SidePanel::right("slice")
                .resizable(true)
                .default_width(280.0)
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

/// One parameter row. Returns true when the value changed.
fn param_row(ui: &mut egui::Ui, p: &mut Param) -> bool {
    let before = p.value.clone();
    let label = humanize(&p.name);
    ui.horizontal(|ui| {
        let text = if p.changed() {
            RichText::new(&label).color(ACCENT)
        } else {
            RichText::new(&label)
        };
        let mut r = ui.label(text);
        let hover = if p.description.is_empty() {
            p.name.clone()
        } else {
            format!("{}\n\n{}", p.description, p.name)
        };
        r = r.on_hover_text(hover);
        let _ = r;
        if p.changed() {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .small_button("↺")
                    .on_hover_text(format!("Reset to {}", p.default.to_scad()))
                    .clicked()
                {
                    p.value = p.default.clone();
                }
            });
        }
    });
    if !p.description.is_empty() {
        ui.label(RichText::new(&p.description).weak().small());
    }

    let step = p.step();
    let decimals = decimals(step);
    let full = ui.available_width();
    match (&mut p.value, &p.widget) {
        (Value::Num(n), Widget::Slider { min, max, .. }) => {
            ui.spacing_mut().slider_width = (full - 70.0).max(60.0);
            ui.add(
                egui::Slider::new(n, *min..=*max)
                    .step_by(step)
                    .max_decimals(decimals),
            );
        }
        (Value::Num(n), Widget::Dropdown(opts)) => {
            let cur = opts
                .iter()
                .find(|(v, _)| *v == Value::Num(*n))
                .map(|o| o.1.clone())
                .unwrap_or(params::Value::Num(*n).to_scad());
            egui::ComboBox::from_id_salt(&p.name)
                .selected_text(cur)
                .width(full - 8.0)
                .show_ui(ui, |ui| {
                    for (v, l) in opts {
                        if let Value::Num(x) = v {
                            ui.selectable_value(n, *x, l);
                        }
                    }
                });
        }
        (Value::Num(n), _) => {
            ui.add(egui::DragValue::new(n).speed(step).max_decimals(decimals));
        }
        (Value::Bool(b), _) => {
            ui.checkbox(b, if *b { "on" } else { "off" });
        }
        (Value::Str(s), Widget::Dropdown(opts)) => {
            let cur = opts
                .iter()
                .find(|(v, _)| *v == Value::Str(s.clone()))
                .map(|o| o.1.clone())
                .unwrap_or(s.clone());
            egui::ComboBox::from_id_salt(&p.name)
                .selected_text(cur)
                .width(full - 8.0)
                .show_ui(ui, |ui| {
                    for (v, l) in opts {
                        if let Value::Str(x) = v {
                            ui.selectable_value(s, x.clone(), l);
                        }
                    }
                });
        }
        (Value::Str(s), w) => {
            let mut edit = egui::TextEdit::singleline(s).desired_width(full - 8.0);
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
            ui.horizontal(|ui| {
                for (i, x) in v.iter_mut().enumerate() {
                    let axis = ["x", "y", "z", "w"][i.min(3)];
                    ui.add(
                        egui::DragValue::new(x)
                            .speed(step)
                            .range(range.clone())
                            .max_decimals(decimals)
                            .prefix(format!("{axis} ")),
                    );
                }
            });
        }
    }
    ui.add_space(6.0);
    p.value != before
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

fn style(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = Color32::from_rgb(0x1a, 0x1c, 0x20);
    v.window_fill = Color32::from_rgb(0x1f, 0x21, 0x26);
    v.extreme_bg_color = Color32::from_rgb(0x12, 0x13, 0x16);
    v.selection.bg_fill = Color32::from_rgb(0xb3, 0x5f, 0x00);
    v.selection.stroke.color = ACCENT;
    v.hyperlink_color = ACCENT;
    v.widgets.hovered.bg_stroke.color = ACCENT;
    v.slider_trailing_fill = true;
    ctx.set_visuals(v);
    ctx.style_mut(|s| {
        s.spacing.item_spacing = Vec2::new(8.0, 5.0);
        s.spacing.button_padding = Vec2::new(8.0, 3.0);
    });
}
