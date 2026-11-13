//! The Optimize tab: the print preparation of the model (see
//! `docs/optimize-tab.md`, "Optimize tab layout").
//!
//! - The state of the tab (`pipeline::State`) is stored per model in
//!   `models.json` (see `state.rs`).
//! - The pipeline (`crate::optimize::pipeline::run`) runs in a background
//!   thread when the tab or an export needs it and an input changed: the
//!   parts of the model, the state, the layer heights or the limits.
//! - The 3D view of the tab shows the plate on the bed, or the layers of
//!   the slice preview of the plate. There is one plate: the print area
//!   sets the fit of the parts and the splits. When the parts need more room
//!   than the bed, the slicer arranges them on its plates.
//! - The STL and 3MF exports write the plate. STEP writes the model.

use super::{App, Mode, fmt_duration, rgb, setting, stat};
use crate::assembly::{Assembly, PALETTE, Source};
use crate::export::{self, Format, Meta};
use crate::mesh::{Bounds, Mesh};
use crate::optimize::{self as opt, Layers, Limits, check, fixes, orient, pipeline, split};
use crate::render::{self, PartStyle};
use crate::slicer::Kind;
use crate::ui;
use eframe::egui::{self, RichText, Vec2};
use glam::{Mat3, Mat4, Vec3};
use serde_json::{Map, Value as Json, json};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The pipeline starts when its inputs do not change for this time.
const SETTLE: Duration = Duration::from_millis(300);
/// The renderer revision of the plate view starts here, so that it does not
/// agree with a revision of the model assembly.
pub(super) const PLATE_REV: u64 = 1 << 62;
/// The color of the faces of a check problem.
const WARN: [f32; 3] = [0.86, 0.18, 0.78];
/// The Check card shows at most this many problems.
const MAX_ISSUES: usize = 80;

/// What the 3D view of the Optimize tab shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum PlateView {
    /// The parts on the bed.
    #[default]
    Plate,
    /// The slice preview of the plate.
    Layers,
}

/// The inputs of one pipeline run, taken on the UI thread. `job` makes the
/// `pipeline::Job` in the background thread.
#[derive(Clone)]
pub(super) struct Inputs {
    assembly: Arc<Assembly>,
    assembly_rev: u64,
    mesh: Arc<Mesh>,
    multicolor: bool,
    model_name: String,
    state: pipeline::State,
    layers: Layers,
    limits: Limits,
}

impl Inputs {
    /// A key that changes when an input of the pipeline changes.
    pub fn key(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.assembly_rev.hash(&mut h);
        self.multicolor.hash(&mut h);
        self.model_name.hash(&mut h);
        format!("{:?} {:?} {:?}", self.state, self.layers, self.limits).hash(&mut h);
        h.finish()
    }

    /// The job: one input for each printable part of the assembly. The
    /// reference (`%`) parts are not printed.
    pub fn job(&self) -> pipeline::Job {
        let a = &self.assembly;
        let parts = (a.parts.iter())
            .filter(|p| !p.reference)
            .map(|p| pipeline::Input {
                name: p.name.clone(),
                mesh: Arc::new(opt::sub_mesh(&a.mesh, p.tris.clone())),
            })
            .collect();
        pipeline::Job {
            parts,
            whole_volume: self.mesh.volume().abs(),
            multicolor: self.multicolor,
            model_name: self.model_name.clone(),
            state: self.state.clone(),
            layers: self.layers,
            limits: self.limits,
        }
    }
}

/// Where an export gets the plate from.
pub(super) enum PlateSource {
    /// The outcome of the window is current.
    Ready(Arc<pipeline::Outcome>),
    /// The export thread runs the pipeline first.
    Run(
        Box<Inputs>,
        crate::openscad::Config,
        Arc<Mutex<pipeline::Cache>>,
    ),
}

impl PlateSource {
    /// The outcome. Runs the pipeline when it is not ready (slow).
    pub fn outcome(self) -> Result<Arc<pipeline::Outcome>, String> {
        match self {
            PlateSource::Ready(o) => Ok(o),
            PlateSource::Run(inputs, cfg, cache) => {
                let cancel = Arc::new(AtomicBool::new(false));
                pipeline::run(&cfg, &inputs.job(), &cache, &cancel)
                    .map(Arc::new)
                    .map_err(|e| format!("{e:#}"))
            }
        }
    }
}

/// A pipeline run in the background.
struct Run {
    rx: Receiver<(u64, Result<pipeline::Outcome, String>, Duration)>,
    cancel: Arc<AtomicBool>,
    key: u64,
    started: Instant,
}

/// What made a part of the plate assembly.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Owner {
    /// The index in `Outcome::instances`.
    pub instance: usize,
    /// `Some(j)`: the faces of issue `j` of the print part, in the warning
    /// color.
    pub issue: Option<usize>,
}

/// The display of the plate.
struct PlateMesh {
    /// Outcome revision, all issues shown, marked issue.
    key: (u64, bool, Option<(usize, usize)>),
    assembly: Arc<Assembly>,
    owners: Vec<Owner>,
}

/// The values and the work of the Optimize tab.
pub(super) struct Opt {
    pub state: pipeline::State,
    /// The printer preset that the user selected for `state.area`.
    pub area_preset: String,
    pub limits: Limits,
    pub cache: Arc<Mutex<pipeline::Cache>>,
    run: Option<Run>,
    pub outcome: Option<Arc<pipeline::Outcome>>,
    /// The input key of `outcome` (or of `error`).
    outcome_key: Option<u64>,
    /// Goes up with each new outcome.
    pub outcome_rev: u64,
    pub error: Option<String>,
    pub time: Option<Duration>,
    /// The wanted input key and when it changed.
    want: Option<(u64, Instant)>,
    /// The selected print part, by name.
    pub selected: Option<String>,
    /// The marked check problem: print part and issue.
    pub issue: Option<(usize, usize)>,
    pub view: PlateView,
    plate: Option<PlateMesh>,
    plate_rev: u64,
    /// Split values that the user changed before the split, by part name.
    drafts: HashMap<String, split::Split>,
    /// The state as it was stored last.
    saved: Option<Json>,
    /// The print area that the camera fits now.
    fit_area: Option<opt::Area>,
}

impl Opt {
    pub fn new(area: opt::Area, preset: &str) -> Opt {
        let state = pipeline::State::new(area);
        Opt {
            saved: Some(state.to_json()),
            state,
            area_preset: preset.to_string(),
            limits: Limits::default(),
            cache: Arc::new(Mutex::new(pipeline::Cache::default())),
            run: None,
            outcome: None,
            outcome_key: None,
            outcome_rev: 0,
            error: None,
            time: None,
            want: None,
            selected: None,
            issue: None,
            view: PlateView::Plate,
            plate: None,
            plate_rev: 0,
            drafts: HashMap::new(),
            fit_area: None,
        }
    }

    /// A new model: the state of `area`, an empty cache and no outcome.
    /// The caller then restores the stored values.
    pub fn new_model(&mut self, area: opt::Area, preset: &str) {
        self.cancel();
        let view = self.view;
        *self = Opt::new(area, preset);
        self.view = view;
    }

    /// The state that was just read from `models.json`: it is not a change.
    pub fn restored(&mut self) {
        self.saved = Some(self.state.to_json());
    }

    pub fn cancel(&mut self) {
        if let Some(r) = self.run.take() {
            r.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// True while a pipeline run works.
    pub fn running(&self) -> bool {
        self.run.is_some()
    }

    /// The seconds of the running pipeline.
    pub fn run_seconds(&self) -> Option<f32> {
        self.run.as_ref().map(|r| r.started.elapsed().as_secs_f32())
    }

    fn part_mut(&mut self, name: &str) -> &mut pipeline::PartState {
        self.state.parts.entry(name.to_string()).or_default()
    }
}

// ---------------------------------------------------------------- helpers ----

/// The direction in the model frame of the normal `n` of a face in the bed
/// frame, for a copy at `at` of a print part with the model to print matrix
/// `matrix`. Uses the inverse of `at * matrix`, rotation part only.
pub(super) fn model_normal(n: Vec3, at: Mat4, matrix: Mat4) -> Vec3 {
    let inv = (at * matrix).inverse();
    (Mat3::from_mat4(inv) * n).normalize_or_zero()
}

/// The triangles `tris` of `mesh` as a mesh of their own.
fn tri_mesh(mesh: &Mesh, tris: &[u32]) -> Mesh {
    let mut map = HashMap::new();
    let mut out = Mesh::default();
    for &t in tris {
        let Some(t) = mesh.triangles.get(t as usize) else {
            continue;
        };
        let t = t.map(|v| {
            *map.entry(v).or_insert_with(|| {
                out.vertices.push(mesh.vertices[v as usize]);
                (out.vertices.len() - 1) as u32
            })
        });
        out.triangles.push(t);
    }
    out
}

/// The print parts on the plate, each one time, in part order.
fn parts_on(o: &pipeline::Outcome) -> Vec<usize> {
    let mut v: Vec<usize> = o.instances.iter().map(|i| i.part).collect();
    v.sort_unstable();
    v.dedup();
    v
}

/// The fix counts of the print parts on the plate.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct FixCounts {
    pub teardrops: usize,
    pub bridges: usize,
    pub brims: usize,
    pub ears: usize,
    pub holes: usize,
    /// Parts with a bed chamfer.
    pub chamfers: usize,
    /// "part: place" for each place that a fix did not change.
    pub skipped: Vec<String>,
}

/// The sums of the fix reports of the print parts on the plate. Each print
/// part counts one time, also with more copies.
pub(super) fn fix_counts(o: &pipeline::Outcome) -> FixCounts {
    let mut c = FixCounts::default();
    for k in parts_on(o) {
        let p = &o.parts[k];
        let r = &p.report;
        c.teardrops += r.teardrops;
        c.bridges += r.bridges;
        c.brims += r.brims;
        c.ears += r.ears;
        c.holes += r.holes;
        c.chamfers += usize::from(r.chamfer);
        c.skipped
            .extend(r.skipped.iter().map(|s| format!("{}: {s}", p.name)));
    }
    c
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The text about the export of the plate of `o`, for example
/// "Plate: 5 parts, 7 copies. Bed chamfer, 4 teardrops." A plate with no
/// part gives "Model as built.": only STEP can export.
pub(super) fn summary(o: &pipeline::Outcome) -> String {
    if o.instances.is_empty() {
        return "Model as built.".to_string();
    }
    let parts = parts_on(o).len();
    let mut s = format!(
        "Plate: {}, {}.",
        plural(parts, "part", "parts"),
        plural(o.instances.len(), "copy", "copies")
    );
    let c = fix_counts(o);
    let mut fixes = Vec::new();
    if c.chamfers > 0 {
        fixes.push("Bed chamfer".to_string());
    }
    if c.teardrops > 0 {
        fixes.push(plural(c.teardrops, "teardrop", "teardrops"));
    }
    if c.bridges > 0 {
        fixes.push(plural(c.bridges, "bridge fix", "bridge fixes"));
    }
    if c.brims > 0 {
        fixes.push(plural(c.brims, "brim", "brims"));
    }
    if c.ears > 0 {
        fixes.push(plural(c.ears, "mouse ear", "mouse ears"));
    }
    if c.holes > 0 {
        fixes.push(plural(c.holes, "hole size fix", "hole size fixes"));
    }
    if !fixes.is_empty() {
        s.push(' ');
        s.push_str(&fixes.join(", "));
        s.push('.');
    }
    s
}

/// The 3MF of the plate: one object for each print part on the plate,
/// named as the print part, and one build item for each copy.
pub(super) fn plate_3mf(o: &pipeline::Outcome, meta: &Meta) -> anyhow::Result<Vec<u8>> {
    let on = o.on_plate();
    let parts = parts_on(o);
    let objects: Vec<(String, &Mesh)> = (parts.iter())
        .map(|&k| (o.parts[k].name.clone(), o.parts[k].mesh.as_ref()))
        .collect();
    let items: Vec<(usize, Mat4)> = (on.iter())
        .map(|(k, at)| (parts.binary_search(k).unwrap_or(0), *at))
        .collect();
    export::plate_3mf_bytes(&objects, &items, meta)
}

/// Writes the plate of `o` to `path` (STL or 3MF, from the extension).
/// `check` can refuse the file before it is written.
pub(super) fn write_plate(
    path: &Path,
    o: &pipeline::Outcome,
    meta: &Meta,
    check: impl Fn(&Path) -> Result<(), String>,
) -> Result<(), String> {
    if o.instances.is_empty() {
        return Err(
            "No part is on the plate. In the Optimize tab, check the parts in the Plate card."
                .into(),
        );
    }
    check(path)?;
    let bytes = match Format::from_path(path) {
        Some(Format::Stl) => export::stl_bytes(&o.plate_mesh()),
        Some(Format::ThreeMf) => plate_3mf(o, meta).map_err(|e| e.to_string())?,
        _ => return Err("A plate is STL or 3MF.".into()),
    };
    std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
}

/// The model mesh of the print part `only` for a STEP export: the part in
/// its model position, without the print fixes. A split part gives its
/// pieces, without the printed pins.
pub(super) fn only_mesh(o: &pipeline::Outcome, only: &str) -> Result<Mesh, String> {
    if let Some(p) = o.parts.iter().find(|p| p.name == only) {
        return Ok((*p.model).clone());
    }
    let mut out = Mesh::default();
    for p in &o.parts {
        if p.piece
            .as_ref()
            .is_some_and(|(parent, l)| parent == only && l != "pin")
        {
            pipeline::append(&mut out, &p.model, Mat4::IDENTITY);
        }
    }
    if out.is_empty() {
        Err(format!("The model has no print part {only}."))
    } else {
        Ok(out)
    }
}

/// The plate assembly: one part for each copy, in print position on the
/// bed. Then the faces of the check problems in the warning color: all
/// problems with `all_issues`, else only `marked`.
fn plate_assembly(
    o: &pipeline::Outcome,
    all_issues: bool,
    marked: Option<(usize, usize)>,
) -> (Assembly, Vec<Owner>) {
    let mut base = Vec::new();
    let mut overlays = Vec::new();
    for (k, inst) in o.instances.iter().enumerate() {
        let p = &o.parts[inst.part];
        base.push((
            p.name.clone(),
            opt::transformed(&p.mesh, inst.at),
            Owner {
                instance: k,
                issue: None,
            },
        ));
        for (j, issue) in p.issues.iter().enumerate() {
            if !(all_issues || marked == Some((inst.part, j))) || issue.tris.is_empty() {
                continue;
            }
            let faces = tri_mesh(&p.mesh, &issue.tris);
            overlays.push((
                format!("{} problem {}", p.name, j + 1),
                opt::transformed(&faces, inst.at),
                Owner {
                    instance: k,
                    issue: Some(j),
                },
            ));
        }
    }
    base.extend(overlays);
    base.retain(|(_, m, _)| !m.is_empty());
    let sources: Vec<Source> = (base.iter())
        .map(|(name, mesh, _)| Source {
            name: name.clone(),
            color: None,
            mesh,
            shells: crate::assembly::Shells::Whole,
            reference: false,
        })
        .collect();
    let a = Assembly::build(&sources);
    let owners = base.into_iter().map(|(_, _, w)| w).collect();
    (a, owners)
}

/// The name of a problem kind in the Check card and in the MCP result.
fn kind_label(k: check::Kind) -> (&'static str, &'static str) {
    match k {
        check::Kind::Overhang => ("Overhang", "overhang"),
        check::Kind::LongBridge => ("Long bridge", "long_bridge"),
        check::Kind::ThinWall => ("Thin wall", "thin_wall"),
        check::Kind::Arc => ("Arc in a bridge", "arc"),
        check::Kind::SmallIsland => ("Small island", "small_island"),
        check::Kind::TooLarge => ("Too large", "too_large"),
    }
}

/// The color of print part `k` on the plate.
fn part_color(k: usize) -> [f32; 3] {
    PALETTE[k % PALETTE.len()].map(|c| c as f32 / 255.0)
}

fn size_text(s: Vec3) -> String {
    format!("{:.1} × {:.1} × {:.1} mm", s.x, s.y, s.z)
}

/// True when a box of `size` fits `area` in one of the axis orders.
fn box_fits(size: Vec3, area: &opt::Area) -> bool {
    let mut s = size.to_array();
    let mut a = [area.w, area.d, area.h];
    s.sort_by(f32::total_cmp);
    a.sort_by(f32::total_cmp);
    s.iter().zip(&a).all(|(s, a)| *s <= *a + 1e-3)
}

/// The top level keys of the Optimize state (`State::to_json`).
const STATE_KEYS: [&str; 5] = ["area", "gap", "only", "fixes", "parts"];

/// Merges `patch` into `target` as a JSON merge patch (RFC 7386): objects
/// merge key by key, `null` removes a key, other values replace.
pub(super) fn merge_patch(target: &mut Json, patch: &Json) {
    let Json::Object(p) = patch else {
        *target = patch.clone();
        return;
    };
    if !target.is_object() {
        *target = Json::Object(Map::new());
    }
    let t = target.as_object_mut().expect("an object");
    for (k, v) in p {
        if v.is_null() {
            t.remove(k);
        } else {
            merge_patch(t.entry(k.clone()).or_insert(Json::Null), v);
        }
    }
}

/// `state` with the partial state `patch` merged in (same keys as
/// `State::to_json`). Unknown keys are an error.
pub(super) fn merged_state(
    state: &pipeline::State,
    patch: &Json,
    area: opt::Area,
) -> Result<pipeline::State, String> {
    let obj = patch.as_object().ok_or("The state must be an object.")?;
    let unknown: Vec<&str> = (obj.keys())
        .map(String::as_str)
        .filter(|k| !STATE_KEYS.contains(k))
        .collect();
    if !unknown.is_empty() {
        return Err(format!(
            "Unknown keys: {}. Use {}.",
            unknown.join(", "),
            STATE_KEYS.join(", ")
        ));
    }
    let mut v = state.to_json();
    merge_patch(&mut v, patch);
    Ok(pipeline::State::from_json(&v, area))
}

// ------------------------------------------------------------- the window ----

impl App {
    /// The layer heights of the print settings.
    pub(super) fn layers(&self) -> Layers {
        Layers {
            first: self.settings.first_layer,
            height: self.settings.layer_height,
            line_width: self.settings.line_width,
        }
    }

    /// The pipeline inputs, when the model has its final parts: no build
    /// and no parts job runs.
    pub(super) fn opt_inputs(&self) -> Option<Inputs> {
        if self.job.is_some()
            || self.parts_job.is_some()
            || (self.auto_render && self.dirty_since.is_some())
        {
            return None;
        }
        let names = crate::parts::names(&self.params);
        Some(Inputs {
            assembly: self.assembly.clone()?,
            assembly_rev: self.assembly_rev,
            mesh: self.mesh.clone()?,
            // The parts of a multicolor part list are the colors of one object
            // when the model shows all of them.
            multicolor: !names.is_empty() && crate::parts::needs_render(&self.params),
            model_name: self.stem(),
            state: self.opt.state.clone(),
            layers: self.layers(),
            limits: self.opt.limits,
        })
    }

    /// True when the outcome (or the error) is for the current inputs.
    pub(super) fn outcome_current(&self) -> bool {
        self.opt_inputs()
            .is_some_and(|i| self.opt.outcome_key == Some(i.key()))
    }

    /// True while a tool call waits for the outcome: the parts or the
    /// pipeline are not ready.
    pub(super) fn optimize_waits(&self) -> bool {
        if self.parts_job.is_some() {
            return true;
        }
        match self.opt_inputs() {
            Some(i) => self.opt.outcome_key != Some(i.key()),
            None => self.job.is_some(),
        }
    }

    /// A key of the plate that the view and the slice show.
    pub(super) fn plate_key(&self) -> u64 {
        self.opt.outcome_rev
    }

    /// The bed of the model print area, as high as the plate, with the
    /// parts that are outside the bed. Fit shows all of it.
    pub(super) fn bed_bounds(&self) -> Bounds {
        let a = self.opt.state.area;
        let plate = (self.opt.plate.as_ref()).and_then(|p| p.assembly.bounds(&[]));
        let top = plate.map_or(10.0, |b| b.max.z.max(10.0));
        let bed = Bounds {
            min: Vec3::ZERO,
            max: Vec3::new(a.w, a.d, top),
        };
        match plate {
            Some(b) => Bounds {
                min: bed.min.min(b.min),
                max: bed.max.max(b.max),
            },
            None => bed,
        }
    }

    /// Takes the result of a pipeline run, and starts a run when the
    /// Optimize tab or a tool call needs the outcome and an input changed.
    /// A change starts the run after `SETTLE`.
    pub(super) fn poll_optimize(&mut self, ctx: &egui::Context) {
        if let Some(run) = &self.opt.run
            && let Ok((key, r, t)) = run.rx.try_recv()
        {
            self.opt.run = None;
            match r {
                Ok(o) => {
                    self.opt.outcome = Some(Arc::new(o));
                    self.opt.outcome_key = Some(key);
                    self.opt.outcome_rev += 1;
                    self.opt.error = None;
                    self.opt.time = Some(t);
                    self.opt.issue = None;
                }
                Err(e) if e == "cancelled" => {}
                Err(e) => {
                    self.opt.outcome = None;
                    self.opt.outcome_key = Some(key);
                    self.opt.outcome_rev += 1;
                    self.opt.error = Some(e);
                    self.opt.time = Some(t);
                }
            }
            ctx.request_repaint();
        }
        if !(self.mode == Mode::Optimize || self.tool_needs_outcome()) {
            return;
        }
        let Some(inputs) = self.opt_inputs() else {
            return;
        };
        let key = inputs.key();
        if self.opt.outcome_key == Some(key) {
            self.opt.cancel();
            self.opt.want = None;
            return;
        }
        if self.opt.run.as_ref().is_some_and(|r| r.key == key) {
            return;
        }
        match self.opt.want {
            Some((k, at)) if k == key => {
                if at.elapsed() >= SETTLE {
                    self.opt.want = None;
                    self.start_pipeline(inputs, key);
                } else {
                    ctx.request_repaint_after(SETTLE.saturating_sub(at.elapsed()));
                }
            }
            _ => {
                self.opt.want = Some((key, Instant::now()));
                ctx.request_repaint_after(SETTLE);
            }
        }
    }

    fn start_pipeline(&mut self, inputs: Inputs, key: u64) {
        self.opt.cancel();
        // The state is stored when it changes, so that the CLI and the next
        // start see it.
        let json = self.opt.state.to_json();
        if self.opt.saved.as_ref() != Some(&json) {
            self.opt.saved = Some(json);
            self.save_state();
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        let (c, cfg, cache) = (cancel.clone(), self.cfg.clone(), self.opt.cache.clone());
        std::thread::spawn(move || {
            let t = Instant::now();
            let job = inputs.job();
            let r = pipeline::run(&cfg, &job, &cache, &c).map_err(|e| format!("{e:#}"));
            let r = if c.load(Ordering::Relaxed) {
                Err("cancelled".to_string())
            } else {
                r
            };
            let _ = tx.send((key, r, t.elapsed()));
        });
        self.opt.run = Some(Run {
            rx,
            cancel,
            key,
            started: Instant::now(),
        });
    }

    /// The plate for an export: the outcome when it is current, else the
    /// inputs to run the pipeline in the export thread.
    pub(super) fn plate_source(&self) -> Result<PlateSource, String> {
        let inputs = self.opt_inputs().ok_or(if self.parts_job.is_some() {
            "CoCAD is still finding the parts. Try again in a moment."
        } else {
            "The model has no current mesh. Build it first."
        })?;
        if self.opt.outcome_key == Some(inputs.key()) {
            if let Some(o) = &self.opt.outcome {
                return Ok(PlateSource::Ready(o.clone()));
            }
            if let Some(e) = &self.opt.error {
                return Err(e.clone());
            }
        }
        Ok(PlateSource::Run(
            Box::new(inputs),
            self.cfg.clone(),
            self.opt.cache.clone(),
        ))
    }

    /// The text about the export: the hover text of Export 3MF.
    pub(crate) fn export_summary(&self) -> String {
        if self.parts_job.is_some() {
            return "Finding the parts…".to_string();
        }
        if self.outcome_current()
            && let Some(o) = &self.opt.outcome
        {
            return summary(o);
        }
        if self.opt.running() {
            return "Preparing the plate…".to_string();
        }
        "STL and 3MF: the print plate. STEP: the model as built.".to_string()
    }

    /// The copies on the plate, when the outcome is current.
    pub(super) fn plate_copies(&self) -> Option<usize> {
        (self.outcome_current())
            .then(|| self.opt.outcome.as_ref().map(|o| o.instances.len()))
            .flatten()
    }

    /// Builds the plate assembly when the outcome or the shown problems
    /// changed.
    pub(super) fn ensure_plate(&mut self) {
        // A new print area: the camera fits the bed again.
        if self.opt.fit_area != Some(self.opt.state.area) {
            self.opt.fit_area = Some(self.opt.state.area);
            self.fit();
        }
        let Some(o) = self.opt.outcome.clone() else {
            if self.opt.plate.take().is_some() {
                self.opt.plate_rev += 1;
            }
            return;
        };
        let all = !self.card_closed("Check");
        let key = (self.opt.outcome_rev, all, self.opt.issue);
        if self.opt.plate.as_ref().is_some_and(|p| p.key == key) {
            return;
        }
        let (assembly, owners) = plate_assembly(&o, all, self.opt.issue);
        self.opt.plate = Some(PlateMesh {
            key,
            assembly: Arc::new(assembly),
            owners,
        });
        self.opt.plate_rev += 1;
    }

    /// The plate assembly for the renderer, and its revision.
    pub(super) fn plate_mesh(&self) -> (Option<Arc<Assembly>>, u64) {
        (
            self.opt.plate.as_ref().map(|p| p.assembly.clone()),
            PLATE_REV + self.opt.plate_rev,
        )
    }

    /// The style of each part of the plate assembly.
    pub(super) fn plate_styles(&self) -> Vec<PartStyle> {
        let (Some(p), Some(o)) = (&self.opt.plate, &self.opt.outcome) else {
            return Vec::new();
        };
        (p.owners.iter())
            .map(|w| {
                let Some(inst) = o.instances.get(w.instance) else {
                    return PartStyle::default();
                };
                let part = inst.part;
                match w.issue {
                    None => PartStyle {
                        color: part_color(part),
                        highlight: self.opt.selected.as_deref()
                            == o.parts.get(part).map(|p| p.name.as_str()),
                        ..PartStyle::default()
                    },
                    Some(j) => PartStyle {
                        color: WARN,
                        highlight: self.opt.issue == Some((part, j)),
                        ..PartStyle::default()
                    },
                }
            })
            .collect()
    }

    /// A click in the 3D view of the Optimize tab. A click on a part selects
    /// its print part. With "Select face" on for the selected part, a click
    /// on that part puts the face under the mouse on the bed.
    pub(super) fn plate_click(&mut self, resp: &egui::Response, rect: egui::Rect) {
        if !resp.clicked() || self.opt.view != PlateView::Plate {
            return;
        }
        let Some(pos) = resp.interact_pointer_pos() else {
            return;
        };
        let (Some(plate), Some(o)) = (&self.opt.plate, self.opt.outcome.clone()) else {
            return;
        };
        let aspect = rect.width() / rect.height().max(1.0);
        let ndc = glam::Vec2::new(
            (pos.x - rect.left()) / rect.width().max(1.0) * 2.0 - 1.0,
            1.0 - (pos.y - rect.top()) / rect.height().max(1.0) * 2.0,
        );
        let (origin, dir) = self.cam.ray(aspect, ndc);
        // A click goes through the side that the Section bar removes.
        let planes = self.view3d.section.planes(super::section::Space::Plate);
        let Some((ap, tri, _)) = plate.assembly.pick_triangle(origin, dir, &[], &planes, &[])
        else {
            self.opt.selected = None;
            self.opt.issue = None;
            return;
        };
        let owner = plate.owners[ap];
        let local = tri - plate.assembly.parts[ap].tris.start;
        let Some(inst) = o.instances.get(owner.instance).copied() else {
            return;
        };
        let pp = &o.parts[inst.part];
        let local = match owner.issue {
            None => local,
            Some(j) => pp.issues[j].tris.get(local).map_or(0, |t| *t as usize),
        };
        let face_mode = self.opt.selected.as_deref() == Some(pp.name.as_str())
            && matches!(
                self.opt.state.part(&pp.name).orientation.choice,
                orient::Choice::Face(_)
            );
        if face_mode {
            let n_print = orient::face_normal(&pp.mesh, local);
            let n_bed = Mat3::from_mat4(inst.at) * n_print;
            let down = model_normal(n_bed, inst.at, pp.matrix);
            if down != Vec3::ZERO {
                let name = pp.name.clone();
                self.opt.part_mut(&name).orientation.choice = orient::Choice::Face(down);
                self.toast = Some((format!("{name}: the face goes on the bed"), Instant::now()));
            }
        } else {
            self.opt.selected = Some(pp.name.clone());
            self.opt.issue = owner.issue.map(|j| (inst.part, j));
        }
    }

    /// The view switch and the state of the plate at the top left of the 3D
    /// view. Returns the rectangle that they use.
    pub(super) fn plate_overlay(&mut self, ui: &mut egui::Ui, rect: egui::Rect) -> egui::Rect {
        let m = 16.0;
        let area = egui::Rect::from_min_size(
            rect.left_top() + Vec2::new(m, m),
            Vec2::new(rect.width().min(420.0) - m, 40.0),
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(area)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                egui::Frame::NONE
                    .fill(ui::glass())
                    .corner_radius(10)
                    .inner_margin(egui::Margin::same(4))
                    .show(ui, |ui| {
                        ui.set_width(150.0);
                        let mut v = self.opt.view;
                        let options = [(PlateView::Plate, "Plate"), (PlateView::Layers, "Layers")];
                        if ui::segmented(ui, &mut v, &options) {
                            self.opt.view = v;
                        }
                    });
                ui.add_space(8.0);
                let text = if let Some(t) = self.opt.run_seconds() {
                    Some(format!("Preparing the plate…  {t:.1} s"))
                } else if self.parts_job.is_some() {
                    Some("Finding the parts…".to_string())
                } else {
                    self.opt.outcome.as_ref().and_then(|o| {
                        if o.instances.is_empty() {
                            Some("No part on the plate".to_string())
                        } else if !o.too_large().is_empty() {
                            Some("A part is too large for the print area".to_string())
                        } else if o.overflow {
                            Some("Larger than the bed. The slicer arranges the parts.".to_string())
                        } else {
                            None
                        }
                    })
                };
                if let Some(text) = text {
                    egui::Frame::NONE
                        .fill(ui::glass())
                        .corner_radius(10)
                        .inner_margin(egui::Margin::symmetric(12, 9))
                        .show(ui, |ui| {
                            ui.label(RichText::new(text).size(13.5));
                        });
                }
            },
        )
        .response
        .rect
    }

    // ------------------------------------------------------------- cards ----

    /// The Optimize tab.
    pub(super) fn optimize_panel(&mut self, ui: &mut egui::Ui) {
        let before = (self.settings, self.opt.limits);
        ui::scroll(ui, ui::PANEL_PAD, |ui| {
            self.plate_card(ui);
            self.orientation_card(ui);
            self.fixes_card(ui);
            self.check_card(ui);
            self.layers_card(ui);
            self.printer_card(ui);
            self.print_settings_card(ui);
            if let Some(t) = self.opt.time {
                ui.add_space(4.0);
                ui.label(ui::dim(format!(
                    "Prepared in {}",
                    if t.as_secs_f32() < 1.0 {
                        format!("{} ms", t.as_millis())
                    } else {
                        format!("{:.1} s", t.as_secs_f32())
                    }
                )));
            }
        });
        if self.settings != before.0 {
            self.slice_dirty_since = Some(Instant::now());
            self.slice_for_rev = u64::MAX;
        }
    }

    fn plate_card(&mut self, ui: &mut egui::Ui) {
        let o = self.opt.outcome.clone();
        let note = match &o {
            Some(o) => plural(o.instances.len(), "copy", "copies"),
            None => String::new(),
        };
        ui::card(ui, |ui| {
            let info = "All print parts go on one plate. The print area sets the bed, the fit \
                        of each part and the splits. When the parts need more room than the \
                        bed, some are outside it: the slicer arranges them on its plates.";
            if !self.card_head_info(ui, "Plate", &note, info) {
                return;
            }
            if self.parts_job.is_some() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Finding the parts…");
                });
            } else if let Some(t) = self.opt.run_seconds() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(format!("Preparing the plate…  {t:.1} s"));
                });
            } else if self.mesh.is_none() {
                ui.label(ui::dim("Build the model first."));
            }
            if let Some(e) = &self.opt.error {
                ui.add(egui::Label::new(RichText::new(e).color(ui::c().danger).size(12.5)).wrap());
            }
            let Some(o) = o else {
                return;
            };
            // All parts or one part.
            let mut names: Vec<String> = Vec::new();
            for p in &o.parts {
                let n = p.piece.as_ref().map_or(&p.name, |(parent, _)| parent);
                if !names.contains(n) {
                    names.push(n.clone());
                }
            }
            let mut only = self.opt.state.only.clone();
            egui::ComboBox::from_id_salt("plate-only")
                .selected_text(only.clone().unwrap_or_else(|| "All parts".into()))
                .width(ui.available_width())
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut only, None, "All parts");
                    for n in &names {
                        ui.selectable_value(&mut only, Some(n.clone()), n);
                    }
                });
            if only != self.opt.state.only {
                self.opt.state.only = only;
            }
            ui.add_space(4.0);
            for (k, p) in o.parts.iter().enumerate() {
                // The name row and the size row of a part close together.
                ui.scope(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    self.part_row(ui, &o, k);
                });
                // After the pieces of a split part: its split.
                let parent = p.piece.as_ref().map(|(parent, _)| parent);
                let next =
                    (o.parts.get(k + 1)).and_then(|q| q.piece.as_ref().map(|(parent, _)| parent));
                if let Some(parent) = parent
                    && Some(parent) != next
                {
                    self.split_done_row(ui, &o, parent);
                }
            }
            // Errors that are not of one part.
            for e in &o.errors {
                if !o
                    .parts
                    .iter()
                    .any(|p| e.starts_with(&format!("{}: ", p.name)))
                {
                    ui.add(
                        egui::Label::new(RichText::new(e).color(ui::c().danger).size(12.5)).wrap(),
                    );
                }
            }
        });
        ui.add_space(4.0);
    }

    /// One print part in the Plate card.
    fn part_row(&mut self, ui: &mut egui::Ui, o: &pipeline::Outcome, k: usize) {
        let p = &o.parts[k];
        let name = p.name.clone();
        let ps = self.opt.state.part(&name);
        let selected = self.opt.selected.as_deref() == Some(name.as_str());
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            let mut include = ps.include;
            if ui
                .checkbox(&mut include, "")
                .on_hover_text("Print this part")
                .changed()
            {
                self.opt.part_mut(&name).include = include;
            }
            let (r, _) = ui.allocate_exact_size(Vec2::splat(10.0), egui::Sense::hover());
            ui.painter()
                .circle_filled(r.center(), 5.0, rgb(part_color(k)));
            let label = if p.count > 1 {
                format!("{name} ×{}", p.count)
            } else {
                name.clone()
            };
            let rtl = egui::Layout::right_to_left(egui::Align::Center);
            ui.with_layout(rtl, |ui| {
                let mut text = ps.copies.map(|c| c.to_string()).unwrap_or_default();
                let edit = egui::TextEdit::singleline(&mut text)
                    .desired_width(34.0)
                    .hint_text(p.count.to_string());
                if ui
                    .add(edit)
                    .on_hover_text("Copies. Empty: the count in the model.")
                    .changed()
                {
                    let t = text.trim();
                    if t.is_empty() {
                        self.opt.part_mut(&name).copies = None;
                    } else if let Ok(n) = t.parse::<u32>() {
                        self.opt.part_mut(&name).copies = Some(n.min(999));
                    }
                }
                let ltr = egui::Layout::left_to_right(egui::Align::Center);
                ui.with_layout(ltr, |ui| {
                    let resp = ui.add(
                        egui::Button::new(label)
                            .selected(selected)
                            .frame(false)
                            .truncate(),
                    );
                    if resp.clicked() {
                        self.opt.selected = if selected { None } else { Some(name.clone()) };
                        self.opt.issue = None;
                    }
                });
            });
        });
        // Size and status.
        let size = opt::bounds_or_zero(&p.mesh).size();
        let (status, color) = if !ps.include {
            ("Not printed".to_string(), ui::c().dim)
        } else if p.copies == 0 {
            ("Not on the plate".to_string(), ui::c().dim)
        } else if !p.fits {
            ("Too large".to_string(), ui::c().danger)
        } else if p.issues.is_empty() {
            ("Fits".to_string(), ui::GOOD)
        } else {
            (plural(p.issues.len(), "problem", "problems"), ui::ACCENT)
        };
        ui.horizontal_wrapped(|ui| {
            ui.add_space(22.0);
            let mut text = size_text(size);
            if let Some((parent, label)) = &p.piece {
                text = format!("Piece {label} of {parent} · {text}");
            }
            ui.label(ui::dim(text));
            let tip =
                (!p.fits && ps.include && p.copies > 0).then_some("Too large for the print area");
            let l = ui.label(RichText::new(status).color(color).size(12.5));
            if let Some(t) = tip {
                l.on_hover_text(t);
            }
            if !p.report.skipped.is_empty() {
                ui.label(ui::dim(format!("{} skipped", p.report.skipped.len())))
                    .on_hover_text(p.report.skipped.join("\n"));
            }
        });
        for e in o
            .errors
            .iter()
            .filter(|e| e.starts_with(&format!("{name}: ")))
        {
            ui.add(egui::Label::new(RichText::new(e).color(ui::c().danger).size(12.5)).wrap());
        }
        if !p.fits && ps.include && p.piece.is_none() {
            self.split_editor(ui, p);
        }
    }

    /// The Split section of a part that does not fit: the proposal, or the
    /// values that the user changed, and the Split button.
    fn split_editor(&mut self, ui: &mut egui::Ui, p: &pipeline::PrintPart) {
        let name = p.name.clone();
        let Some(mut draft) = self.opt.drafts.get(&name).cloned().or(p.proposal.clone()) else {
            ui.add(
                egui::Label::new(ui::dim(
                    "No parallel cuts make it fit. Turn it with a custom orientation, or use a \
                     larger print area.",
                ))
                .wrap(),
            );
            return;
        };
        let before = draft.clone();
        let area = self.opt.state.area;
        egui::Frame::NONE
            .fill(ui::c().input)
            .corner_radius(8)
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("Split").strong());
                split_values(ui, &mut draft, &p.model, &name);
                let sizes = split::piece_sizes(&p.model, &draft);
                for (i, s) in sizes.iter().enumerate() {
                    let fits = box_fits(*s, &area);
                    let text = format!("{}/{}: {}", i + 1, sizes.len(), size_text(*s));
                    let color = if fits { ui::c().dim } else { ui::c().danger };
                    ui.label(RichText::new(text).color(color).size(12.5));
                }
                ui.horizontal(|ui| {
                    if ui
                        .add(ui::primary("Split"))
                        .on_hover_text("Cut the part into pieces with these values")
                        .clicked()
                    {
                        self.opt.part_mut(&name).split = Some(draft.clone());
                        self.opt.drafts.remove(&name);
                        self.opt.selected = None;
                    }
                    if self.opt.drafts.contains_key(&name)
                        && ui.add(ui::ghost("Reset", false)).clicked()
                    {
                        self.opt.drafts.remove(&name);
                    }
                });
            });
        if draft != before {
            // A new joint gets new cut positions.
            if draft.joint != before.joint
                && let Some(again) = split::propose_with(&p.model, &area, draft.joint, draft.values)
            {
                draft = again;
            }
            self.opt.drafts.insert(name, draft);
        }
    }

    /// The split of `parent`, after its pieces: the values and Remove split.
    fn split_done_row(&mut self, ui: &mut egui::Ui, o: &pipeline::Outcome, parent: &str) {
        let Some(mut sp) = self.opt.state.part(parent).split else {
            return;
        };
        // The model of the part is the pieces together.
        let mut model = Mesh::default();
        for p in &o.parts {
            if p.piece
                .as_ref()
                .is_some_and(|(q, l)| q == parent && l != "pin")
            {
                pipeline::append(&mut model, &p.model, Mat4::IDENTITY);
            }
        }
        let open = (self.opt.selected.as_ref())
            .and_then(|s| o.parts.iter().find(|p| &p.name == s))
            .is_some_and(|p| p.piece.as_ref().is_some_and(|(q, _)| q == parent));
        egui::Frame::NONE
            .fill(ui::c().input)
            .corner_radius(8)
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("Split of {parent}")).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(ui::ghost("Remove split", false)).clicked() {
                            self.opt.part_mut(parent).split = None;
                        }
                    });
                });
                if open {
                    let before = sp.clone();
                    split_values(ui, &mut sp, &model, parent);
                    if sp != before {
                        self.opt.part_mut(parent).split = Some(sp);
                    }
                } else {
                    ui.label(ui::dim(format!(
                        "{}, {}. Select a piece to change it.",
                        joint_label(sp.joint),
                        plural(sp.pieces(), "piece", "pieces")
                    )));
                }
            });
    }

    fn orientation_card(&mut self, ui: &mut egui::Ui) {
        let name = self.opt.selected.clone();
        let note = match &name {
            Some(n) => match self.opt.state.part(n).orientation.choice {
                orient::Choice::Model => "Model".to_string(),
                orient::Choice::Auto => "Auto".to_string(),
                orient::Choice::Face(_) => "Face".to_string(),
            },
            None => String::new(),
        };
        ui::card(ui, |ui| {
            let info = "Custom orientation off: the part keeps the rotation of the model.\n\n\
                        Auto: CoCAD puts the part on its best face.\n\n\
                        Select face: click a face of the part in the 3D view. That face goes on \
                        the bed.";
            if !self.card_head_info(ui, "Orientation", &note, info) {
                return;
            }
            let Some(name) = name else {
                ui.label(ui::dim(
                    "Select a part in the Plate card or in the 3D view.",
                ));
                return;
            };
            let pp = (self.opt.outcome.as_ref())
                .and_then(|o| o.parts.iter().find(|p| p.name == name).cloned());
            ui.label(RichText::new(&name).strong());
            let mut o = self.opt.state.part(&name).orientation;
            let before = o;
            let mut custom = o.choice != orient::Choice::Model;
            ui.horizontal(|ui| {
                ui.label("Custom orientation");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui::toggle(ui, &mut custom).changed() {
                        if custom {
                            o.choice = orient::Choice::Auto;
                        } else {
                            o.choice = orient::Choice::Model;
                            o.quarter_turns = 0;
                        }
                    }
                });
            });
            if custom {
                let mut face = matches!(o.choice, orient::Choice::Face(_));
                if ui::segmented(ui, &mut face, &[(false, "Auto"), (true, "Select face")]) {
                    o.choice = if face {
                        // The face that is on the bed now, until a click.
                        let down = pp.as_ref().map_or(Vec3::NEG_Z, |p| {
                            model_normal(Vec3::NEG_Z, Mat4::IDENTITY, p.matrix)
                        });
                        orient::Choice::Face(down)
                    } else {
                        orient::Choice::Auto
                    };
                    self.opt.view = PlateView::Plate;
                }
                if face {
                    ui.add(
                        egui::Label::new(ui::dim(
                            "Click a face of the part in the 3D view. That face goes on the bed.",
                        ))
                        .wrap(),
                    );
                }
                ui.horizontal(|ui| {
                    if ui
                        .add(ui::ghost("Turn 90°", false))
                        .on_hover_text("Turn the part by 90 degrees about z")
                        .clicked()
                    {
                        o.quarter_turns = (o.quarter_turns + 1) % 4;
                    }
                    if !o.quarter_turns.is_multiple_of(4) {
                        ui.label(ui::dim(format!("{}°", 90 * (o.quarter_turns % 4) as u32)));
                    }
                });
            }
            if o != before {
                self.opt.part_mut(&name).orientation = o;
            }
        });
        ui.add_space(4.0);
    }

    fn fixes_card(&mut self, ui: &mut egui::Ui) {
        let counts = (self.opt.outcome.as_ref()).map(|o| fix_counts(o));
        let f = self.opt.state.fixes;
        let on = [
            f.teardrops,
            f.bridges,
            f.chamfer,
            f.brim,
            f.ears,
            f.hole_size,
        ]
        .iter()
        .filter(|b| **b)
        .count();
        let note = if on == 0 {
            "Off".to_string()
        } else {
            format!("{on} on")
        };
        ui::card(ui, |ui| {
            if !self.card_head(ui, "Print fixes", &note) {
                return;
            }
            let c = counts.clone().unwrap_or_default();
            let done = |n: usize, one: &str, many: &str| {
                if counts.is_some() {
                    plural(n, one, many)
                } else {
                    String::new()
                }
            };
            let f = &mut self.opt.state.fixes;
            fix_row(
                ui,
                "Teardrop holes",
                &mut f.teardrops,
                &done(c.teardrops, "hole", "holes"),
                "Round holes with a horizontal axis get a 45 degree point at the top.",
            );
            fix_row(
                ui,
                "Hole size",
                &mut f.hole_size,
                &done(c.holes, "hole", "holes"),
                "Round holes with a vertical axis print too small. They get the diameter of the model, plus a value.",
            );
            if f.hole_size {
                ui.horizontal(|ui| {
                    ui.label(ui::dim("Add to diameter"));
                    ui.add(
                        egui::DragValue::new(&mut f.hole_add)
                            .range(0.0..=1.0)
                            .speed(0.01)
                            .max_decimals(2)
                            .suffix(" mm"),
                    );
                });
                ui.add(
                    egui::Label::new(ui::dim("Set the X-Y hole compensation of the slicer to 0."))
                        .wrap(),
                );
            }
            fix_row(
                ui,
                "Bridges",
                &mut f.bridges,
                &done(c.bridges, "fixed", "fixed"),
                "Bridged ceilings start on straight edges.",
            );
            if f.bridges {
                ui.horizontal(|ui| {
                    ui.label(ui::dim("Hole above"));
                    ui::segmented(
                        ui,
                        &mut f.hole_above,
                        &[
                            (fixes::HoleAbove::Sacrificial, "Sacrificial"),
                            (fixes::HoleAbove::Stepped, "Stepped"),
                        ],
                    );
                });
                if f.hole_above == fixes::HoleAbove::Sacrificial {
                    ui.horizontal(|ui| {
                        ui.label(ui::dim("Sacrificial layers"));
                        ui.add(egui::DragValue::new(&mut f.sacrificial_layers).range(1..=10));
                    });
                }
            }
            fix_row(
                ui,
                "Bed chamfer",
                &mut f.chamfer,
                &done(c.chamfers, "part", "parts"),
                "A 45 degree chamfer of the bed edges, in whole layers.",
            );
            if f.chamfer {
                ui.horizontal(|ui| {
                    ui.label(ui::dim("Size"));
                    ui.add(
                        egui::DragValue::new(&mut f.chamfer_mm)
                            .range(0.1..=2.0)
                            .speed(0.05)
                            .max_decimals(2)
                            .suffix(" mm"),
                    );
                });
                ui.add(
                    egui::Label::new(ui::dim(
                        "Set the elephant foot compensation of the slicer to 0.",
                    ))
                    .wrap(),
                );
            }
            fix_row(
                ui,
                "Brim on small islands",
                &mut f.brim,
                &done(c.brims, "brim", "brims"),
                "A one-layer brim on small first-layer islands.",
            );
            if f.brim {
                ui.horizontal(|ui| {
                    ui.label(ui::dim("Width"));
                    ui.add(
                        egui::DragValue::new(&mut f.brim_w)
                            .range(1.0..=20.0)
                            .speed(0.5)
                            .suffix(" mm"),
                    );
                    ui.label(ui::dim("Gap"));
                    ui.add(
                        egui::DragValue::new(&mut f.brim_gap)
                            .range(0.2..=5.0)
                            .speed(0.1)
                            .max_decimals(1)
                            .suffix(" mm"),
                    );
                });
            }
            fix_row(
                ui,
                "Mouse ears",
                &mut f.ears,
                &done(c.ears, "ear", "ears"),
                "A one-layer disk on each sharp bed corner of a large part. It holds the corner on the bed.",
            );
            if f.ears {
                ui.horizontal(|ui| {
                    ui.label(ui::dim("Diameter"));
                    ui.add(
                        egui::DragValue::new(&mut f.ear_d)
                            .range(5.0..=30.0)
                            .speed(0.5)
                            .suffix(" mm"),
                    );
                });
                // On its own line: the two values do not fit one line.
                ui.horizontal(|ui| {
                    ui.label(ui::dim("Min size"));
                    ui.add(
                        egui::DragValue::new(&mut f.ear_min)
                            .range(20.0..=300.0)
                            .speed(1.0)
                            .suffix(" mm"),
                    );
                });
            }
            if !c.skipped.is_empty() {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(plural(c.skipped.len(), "place skipped", "places skipped"))
                        .color(ui::ACCENT)
                        .size(12.5),
                )
                .on_hover_text(c.skipped.join("\n"));
            }
        });
        ui.add_space(4.0);
    }

    /// The problems of the parts on the plate.
    fn issues(&self) -> Vec<(usize, usize)> {
        let Some(o) = &self.opt.outcome else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for (k, p) in o.parts.iter().enumerate() {
            if p.copies > 0 {
                out.extend((0..p.issues.len()).map(|j| (k, j)));
            }
        }
        out
    }

    fn check_card(&mut self, ui: &mut egui::Ui) {
        let list = self.issues();
        let note = if self.opt.outcome.is_none() {
            String::new()
        } else if list.is_empty() {
            "None".to_string()
        } else {
            plural(list.len(), "problem", "problems")
        };
        ui::card(ui, |ui| {
            let info = "The problems of the plate. Click a problem to show its faces in the 3D \
                        view.";
            if !self.card_head_info(ui, "Check", &note, info) {
                return;
            }
            let Some(o) = self.opt.outcome.clone() else {
                ui.label(ui::dim("The problems of the plate show here."));
                return;
            };
            if list.is_empty() {
                ui.label(
                    RichText::new("No problems found.")
                        .color(ui::GOOD)
                        .size(13.0),
                );
                return;
            }
            for &(k, j) in list.iter().take(MAX_ISSUES) {
                let p = &o.parts[k];
                let is = &p.issues[j];
                let marked = self.opt.issue == Some((k, j));
                let text =
                    RichText::new(format!("{}  ·  {}", kind_label(is.kind).0, p.name)).size(13.0);
                // The title and its text close together.
                let clicked = ui
                    .scope(|ui| {
                        let sp = ui.spacing_mut();
                        sp.item_spacing.y = 0.0;
                        sp.button_padding.y = 2.0;
                        let clicked = ui
                            .add(
                                egui::Button::new(text)
                                    .selected(marked)
                                    .frame(false)
                                    .truncate(),
                            )
                            .clicked();
                        ui.add(egui::Label::new(ui::dim(&is.text).size(12.5)).wrap());
                        clicked
                    })
                    .inner;
                if clicked {
                    self.opt.selected = Some(p.name.clone());
                    self.opt.issue = Some((k, j));
                    self.opt.view = PlateView::Plate;
                }
            }
            if list.len() > MAX_ISSUES {
                ui.label(ui::dim(format!("And {} more.", list.len() - MAX_ISSUES)));
            }
        });
        ui.add_space(4.0);
    }

    fn layers_card(&mut self, ui: &mut egui::Ui) {
        let sl = self
            .sliced
            .clone()
            .filter(|_| self.sliced_for == self.plate_key());
        let note = sl.as_ref().map_or(String::new(), |sl| {
            format!(
                "{}, {:.1} g",
                fmt_duration(sl.stats.seconds),
                sl.stats.grams
            )
        });
        ui::card(ui, |ui| {
            let info = "A rough preview of the layers, for checks. It is not for printing. \
                        Up/Down or PgUp/PgDn change the layer.";
            if !self.card_head_info(ui, "Layers", &note, info) {
                return;
            }
            let mut v = self.opt.view;
            if ui::segmented(
                ui,
                &mut v,
                &[(PlateView::Plate, "Plate"), (PlateView::Layers, "Layers")],
            ) {
                self.opt.view = v;
            }
            ui.add_space(4.0);
            let Some(sl) = sl else {
                ui.horizontal(|ui| {
                    if self.slice_job.is_some() || self.opt.running() {
                        ui.spinner();
                        ui.label("Slicing the plate…");
                    } else {
                        ui.label(ui::dim("The slice of the plate shows here."));
                    }
                });
                return;
            };
            let st = &sl.stats;
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
            ui.add_space(4.0);
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
                    for (i, k) in [Kind::OuterWall, Kind::InnerWall, Kind::Infill, Kind::Solid]
                        .into_iter()
                        .enumerate()
                    {
                        ui.horizontal(|ui| {
                            let (r, _) =
                                ui.allocate_exact_size(Vec2::new(10.0, 10.0), egui::Sense::hover());
                            ui.painter()
                                .circle_filled(r.center(), 5.0, rgb(render::kind_color(k)));
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
        });
        ui.add_space(4.0);
    }

    fn printer_card(&mut self, ui: &mut egui::Ui) {
        let a = self.opt.state.area;
        let note = format!("{:.0} × {:.0} × {:.0}", a.w, a.d, a.h);
        ui::card(ui, |ui| {
            let info = "The print area for this model.";
            if !self.card_head_info(ui, "Printer", &note, info) {
                return;
            }
            crate::printer::area_editor(ui, &mut self.opt.state.area, &mut self.opt.area_preset);
        });
        ui.add_space(4.0);
    }

    fn print_settings_card(&mut self, ui: &mut egui::Ui) {
        let s = self.settings;
        let note = format!("{:.2} mm, {:.0} %", s.layer_height, s.infill * 100.0);
        ui::card(ui, |ui| {
            if !self.card_head(ui, "Print settings", &note) {
                return;
            }
            let s = &mut self.settings;
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
            let l = &mut self.opt.limits;
            setting(
                ui,
                "Overhang limit",
                &mut l.overhang_deg,
                20.0..=80.0,
                1.0,
                "°",
            );
            setting(
                ui,
                "Bridge limit",
                &mut l.bridge_mm,
                1.0..=100.0,
                0.5,
                " mm",
            );
            setting(ui, "Gap", &mut self.opt.state.gap, 0.0..=50.0, 0.5, " mm");
            ui.horizontal(|ui| {
                ui.label("Show model ghost");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui::toggle(ui, &mut self.ghost)
                        .on_hover_text("Show the plate in the Layers view");
                });
            });
        });
    }

    // ---------------------------------------------------------- MCP ----

    /// The Optimize state and a summary of the outcome, for the
    /// `get_optimize` tool.
    pub(super) fn optimize_json(&self) -> Result<Json, String> {
        self.need_model_mesh()?;
        if let Some(e) = &self.opt.error {
            return Err(format!("The print preparation failed: {e}"));
        }
        let o = (self.opt.outcome.as_ref())
            .filter(|_| self.outcome_current())
            .ok_or("The plate is not ready. Call get_optimize again.")?;
        let parts: Vec<Json> = o
            .parts
            .iter()
            .map(|p| {
                let issues: Vec<Json> = (p.issues.iter())
                    .map(|i| json!({ "kind": kind_label(i.kind).1, "text": i.text }))
                    .collect();
                let r = &p.report;
                let mut v = json!({
                    "name": p.name,
                    "count": p.count,
                    "copies": p.copies,
                    "fits": p.fits,
                    "size_mm": opt::bounds_or_zero(&p.mesh).size().to_array(),
                    "orientation": p.state.to_json().get("orient").cloned().unwrap_or(json!("model")),
                    "issues": issues,
                    "fixes": {
                        "teardrops": r.teardrops,
                        "bridges": r.bridges,
                        "brims": r.brims,
                        "ears": r.ears,
                        "holes": r.holes,
                        "chamfer": r.chamfer,
                        "skipped": r.skipped,
                    },
                });
                if let Some((parent, label)) = &p.piece {
                    v["piece"] = json!({ "of": parent, "label": label });
                }
                if let Some(sp) = &p.proposal {
                    let ps = pipeline::PartState {
                        split: Some(sp.clone()),
                        ..pipeline::PartState::default()
                    };
                    v["split_proposal"] = ps.to_json()["split"].clone();
                }
                v
            })
            .collect();
        Ok(json!({
            "state": self.opt.state.to_json(),
            "plate": {
                "copies": o.instances.len(),
                "overflow": o.overflow,
            },
            "summary": summary(o),
            "parts": parts,
            "errors": o.errors,
            "seconds": self.opt.time.map(|t| t.as_secs_f32()),
        }))
    }

    fn need_model_mesh(&self) -> Result<(), String> {
        if self.file.is_none() {
            return Err("No model is open. Call open_model first.".into());
        }
        if self.mesh.is_none() {
            return Err("The model has no mesh yet. Call build first.".into());
        }
        Ok(())
    }

    /// Merges a partial state into the Optimize state (`set_optimize`).
    pub(super) fn apply_optimize(&mut self, patch: &Json) -> Result<(), String> {
        let st = merged_state(&self.opt.state, patch, self.default_area())?;
        self.opt.state = st;
        Ok(())
    }
}

/// The name of a joint in the window.
fn joint_label(j: split::Joint) -> &'static str {
    match j {
        split::Joint::Flat => "Flat",
        split::Joint::Pins => "Pins",
        split::Joint::Dovetail => "Dovetail",
    }
}

/// The joint, the clearance and the cut positions of a split.
fn split_values(ui: &mut egui::Ui, sp: &mut split::Split, model: &Mesh, id: &str) {
    let options = split::Joint::ALL.map(|j| (j, joint_label(j)));
    ui::segmented(ui, &mut sp.joint, &options);
    ui.horizontal(|ui| {
        ui.label(ui::dim("Clearance"));
        ui.add(
            egui::DragValue::new(&mut sp.values.clearance)
                .range(0.0..=2.0)
                .speed(0.01)
                .max_decimals(2)
                .suffix(" mm"),
        )
        .on_hover_text("The total gap of the joint. Each side gets one half.");
    });
    let b = opt::bounds_or_zero(model);
    for (i, c) in sp.cuts.iter_mut().enumerate() {
        let axis = ["x", "y", "z"][c.axis.min(2)];
        ui.push_id((id, i), |ui| {
            ui.horizontal(|ui| {
                ui.label(ui::dim(format!("Cut {} at {axis}", i + 1)));
                ui.add(
                    egui::DragValue::new(&mut c.at)
                        .range(b.min[c.axis.min(2)]..=b.max[c.axis.min(2)])
                        .speed(0.5)
                        .max_decimals(1)
                        .suffix(" mm"),
                );
            });
        });
    }
}

/// One fix: the title, the switch and the count of the changed places.
fn fix_row(ui: &mut egui::Ui, title: &str, on: &mut bool, count: &str, hint: &str) {
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.label(title).on_hover_text(hint);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui::toggle(ui, on).on_hover_text(hint);
            if *on && !count.is_empty() {
                ui.label(ui::dim(count));
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::cube;

    fn print_part(name: &str, mesh: Mesh, report: fixes::Report) -> pipeline::PrintPart {
        let mesh = Arc::new(mesh);
        pipeline::PrintPart {
            name: name.into(),
            sources: vec![0],
            count: 1,
            copies: 1,
            state: pipeline::PartState::default(),
            model: mesh.clone(),
            matrix: Mat4::IDENTITY,
            mesh,
            report,
            issues: Vec::new(),
            fits: true,
            proposal: None,
            piece: None,
        }
    }

    fn outcome() -> pipeline::Outcome {
        let report = fixes::Report {
            teardrops: 3,
            chamfer: true,
            ..fixes::Report::default()
        };
        let a = print_part("Box", cube(10.0), report);
        let b = print_part(
            "Lid",
            cube(5.0),
            fixes::Report {
                teardrops: 1,
                skipped: vec!["Hole d 3.0 mm at z 2.0: wall above too thin".into()],
                ..fixes::Report::default()
            },
        );
        let at = |x: f32| Mat4::from_translation(Vec3::new(x, 10.0, 0.0));
        let inst = |part, x| pipeline::Instance { part, at: at(x) };
        pipeline::Outcome {
            parts: vec![a, b],
            instances: vec![inst(0, 10.0), inst(1, 40.0), inst(1, 60.0)],
            overflow: false,
            errors: Vec::new(),
        }
    }

    #[test]
    fn face_normal_goes_back_to_the_model_frame() {
        // A slab, with the model +x direction down, turned on the bed.
        let slab = opt::transformed(&cube(1.0), Mat4::from_scale(Vec3::new(4.0, 20.0, 30.0)));
        let o = orient::Orientation {
            choice: orient::Choice::Face(Vec3::X),
            quarter_turns: 1,
        };
        let area = opt::Area::default();
        let m = orient::matrix(&slab, &o, &area, &Limits::default());
        let at = Mat4::from_translation(Vec3::new(50.0, 60.0, 0.0))
            * Mat4::from_rotation_z(std::f32::consts::FRAC_PI_2);
        // The bed face points down in the bed frame: it is model +x.
        let down = model_normal(Vec3::NEG_Z, at, m);
        assert!((down - Vec3::X).length() < 1e-4, "{down}");
        // Each face normal of the print mesh on the bed goes back to the
        // normal of the same face in the model.
        let print = opt::transformed(&slab, m);
        for i in 0..slab.triangles.len() {
            let n_bed = Mat3::from_mat4(at) * print.normal(i);
            let back = model_normal(n_bed, at, m);
            assert!((back - slab.normal(i)).length() < 1e-4, "{i}: {back}");
        }
    }

    #[test]
    fn summary_names_the_plate_parts_and_fixes() {
        let o = outcome();
        assert_eq!(
            summary(&o),
            "Plate: 2 parts, 3 copies. Bed chamfer, 4 teardrops."
        );
        let empty = pipeline::Outcome::default();
        assert_eq!(summary(&empty), "Model as built.");
        let mut one = outcome();
        one.instances.truncate(1);
        one.parts[0].report = fixes::Report::default();
        assert_eq!(summary(&one), "Plate: 1 part, 1 copy.");
        let c = fix_counts(&o);
        assert_eq!(
            c.skipped,
            ["Lid: Hole d 3.0 mm at z 2.0: wall above too thin"]
        );
    }

    #[test]
    fn plate_3mf_has_one_object_per_part_and_one_item_per_copy() {
        let o = outcome();
        let bytes = plate_3mf(&o, &Meta::default()).unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut text = String::new();
        std::io::Read::read_to_string(&mut zip.by_name("3D/3dmodel.model").unwrap(), &mut text)
            .unwrap();
        assert_eq!(text.matches("<object ").count(), 2);
        assert_eq!(text.matches("<item ").count(), 3);
        assert_eq!(text.matches("<item objectid=\"2\"").count(), 2);
        assert!(text.contains("name=\"Lid\""));
    }

    #[test]
    fn the_plate_goes_to_one_file() {
        let dir = std::env::temp_dir().join(format!("cocad-plate-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let o = outcome();
        // A refused file stops the export before a write.
        let path = dir.join("box.stl");
        let refused = write_plate(&path, &o, &Meta::default(), |_| Err("no".into()));
        assert!(refused.is_err() && !path.exists());
        write_plate(&path, &o, &Meta::default(), |_| Ok(())).unwrap();
        let m = crate::mesh::read_stl(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(m.triangles.len(), 3 * 12);
        let empty = dir.join("empty.stl");
        let none = pipeline::Outcome::default();
        assert!(write_plate(&empty, &none, &Meta::default(), |_| Ok(())).is_err());
        assert!(!empty.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn plate_assembly_has_one_part_per_copy_and_marks_issues() {
        let mut o = outcome();
        o.parts[1].issues.push(check::Issue {
            kind: check::Kind::Overhang,
            tris: vec![0, 1],
            area: 1.0,
            text: "Overhang".into(),
        });
        let (a, owners) = plate_assembly(&o, false, None);
        assert_eq!(a.parts.len(), 3);
        assert!(owners.iter().all(|w| w.issue.is_none()));
        // The copies are in bed position.
        let b = a.bounds(&[]).unwrap();
        assert!((b.min.x - 10.0).abs() < 1e-4 && (b.max.x - 65.0).abs() < 1e-4);
        let (a, owners) = plate_assembly(&o, true, None);
        assert_eq!(a.parts.len(), 5);
        assert_eq!(
            owners[3],
            Owner {
                instance: 1,
                issue: Some(0)
            }
        );
        assert_eq!(a.parts[3].tris.len(), 2);
        let (a, _) = plate_assembly(&o, false, Some((1, 0)));
        assert_eq!(a.parts.len(), 5);
    }

    #[test]
    fn set_optimize_merges_a_partial_state() {
        let area = opt::Area::default();
        let mut s = pipeline::State::new(area);
        s.fixes.brim = true;
        s.only = Some("Box".into());
        s.parts.insert(
            "Box".into(),
            pipeline::PartState {
                copies: Some(3),
                ..pipeline::PartState::default()
            },
        );
        let patch = json!({
            "fixes": { "chamfer": true, "chamfer_mm": 0.6 },
            "only": null,
            "parts": { "Box": { "orient": "auto" }, "Lid": { "include": false } },
        });
        let m = merged_state(&s, &patch, area).unwrap();
        assert!(m.fixes.brim && m.fixes.chamfer);
        assert!((m.fixes.chamfer_mm - 0.6).abs() < 1e-6);
        assert_eq!(m.only, None);
        let b = m.part("Box");
        assert_eq!(b.copies, Some(3));
        assert_eq!(b.orientation.choice, orient::Choice::Auto);
        assert!(!m.part("Lid").include);
        // null removes a part: it takes the defaults again.
        let m = merged_state(&m, &json!({ "parts": { "Box": null } }), area).unwrap();
        assert_eq!(m.part("Box"), pipeline::PartState::default());
        assert!(merged_state(&m, &json!({ "fix": {} }), area).is_err());
        assert!(merged_state(&m, &json!([1]), area).is_err());
    }

    #[test]
    fn only_mesh_takes_the_model_or_the_pieces() {
        let mut o = outcome();
        assert_eq!(only_mesh(&o, "Lid").unwrap().triangles.len(), 12);
        o.parts[0].piece = Some(("Board".into(), "1/2".into()));
        o.parts[1].piece = Some(("Board".into(), "pin".into()));
        o.parts[0].name = "Board 1/2".into();
        o.parts[1].name = "Board pin".into();
        assert_eq!(only_mesh(&o, "Board").unwrap().triangles.len(), 12);
        assert!(only_mesh(&o, "Other").is_err());
    }
}
