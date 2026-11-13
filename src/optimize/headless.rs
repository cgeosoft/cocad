//! The print preparation without the window, for `cocad export` and
//! `cocad slice`.
//!
//! 1. `inputs` builds the model and finds the parts as the app does: the
//!    multicolor part list, else the part list, else the module split,
//!    else the shells of the mesh. The `%` parts are reference parts: they are not printed.
//! 2. `stored_state` reads the Optimize values that the app stored for the
//!    model in `<config>/models.json`. It does not write the file.
//! 3. `Options` holds the print options of the command line. They change
//!    the state before the pipeline runs.
//! 4. `optimize` runs the pipeline (`pipeline::run`).

use super::check::Kind;
use super::orient::{Choice, Orientation};
use super::pipeline::{self, Cache, Input, Job, Outcome, State};
use super::{Area, Layers, Limits, fixes, split};
use crate::assembly::{self, Assembly};
use crate::csg_split;
use crate::export;
use crate::mesh::Mesh;
use crate::openscad::{self, Config};
use crate::params::{self, Param};
use crate::part_param;
use crate::parts;
use anyhow::{Context, Result, bail};
use serde_json::Value as Json;
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Where the parts of a model come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Found {
    /// A multicolor part list: each part builds on its own.
    List,
    /// A part list: each value builds on its own.
    Selector,
    /// The module split of the CSG tree.
    Modules,
    /// The shells of the built mesh.
    Shells,
}

impl Found {
    pub fn label(self) -> &'static str {
        match self {
            Found::List => "the multicolor part list",
            Found::Selector => "the part parameter",
            Found::Modules => "the module split",
            Found::Shells => "the shells of the mesh",
        }
    }
}

/// The built model and its printable parts.
pub struct Built {
    /// The model as OpenSCAD built it. The `%` parts are not in it.
    pub mesh: Arc<Mesh>,
    /// Time of the main build.
    pub elapsed: Duration,
    /// Time of the parts job. It runs at the same time as the main build.
    pub parts_elapsed: Duration,
    /// The printable parts in the model frame.
    pub parts: Vec<Input>,
    /// The names of the `%` parts. They are not printed.
    pub references: Vec<String>,
    /// Volume of the built mesh, for the overlap test of the pipeline.
    pub whole_volume: f32,
    /// The model has a multicolor part list: its parts are the colors of
    /// one object.
    pub multicolor: bool,
    /// The file name without the extension.
    pub model_name: String,
    pub found: Found,
    /// Problems that did not stop the build, for example a failed parts
    /// job. The shells of the mesh are then the parts, as in the app.
    pub warnings: Vec<String>,
}

impl Built {
    /// The pipeline job for these parts.
    pub fn job(&self, state: State, layers: Layers, limits: Limits) -> Job {
        Job {
            parts: self.parts.clone(),
            whole_volume: self.whole_volume,
            multicolor: self.multicolor,
            model_name: self.model_name.clone(),
            state,
            layers,
            limits,
        }
    }
}

/// Builds `file` with the values of `params` and finds its printable parts
/// with the same functions as the app. The main build and the parts job run
/// at the same time. A failed main build is an error. A failed parts job
/// is a warning: the shells of the mesh are then the parts.
pub fn inputs(
    cfg: &Config,
    file: &Path,
    params: &[Param],
    cancel: &Arc<AtomicBool>,
) -> Result<Built> {
    let defines = params::defines(params);
    // The parts job stops when the main build fails or `cancel` is set.
    let stop = Arc::new(AtomicBool::new(cancel.load(Ordering::Relaxed)));
    let (main, found) = std::thread::scope(|s| {
        let job = s.spawn(|| {
            let t = Instant::now();
            let r = csg_split::find_parts(cfg, file, params, &defines, &stop);
            (r, t.elapsed())
        });
        let main = openscad::render(cfg, file, &defines, cancel);
        if main.is_err() {
            stop.store(true, Ordering::Relaxed);
        }
        while !job.is_finished() {
            if cancel.load(Ordering::Relaxed) {
                stop.store(true, Ordering::Relaxed);
            }
            std::thread::sleep(Duration::from_millis(15));
        }
        let found = job.join().unwrap_or_else(|e| std::panic::resume_unwind(e));
        (main, found)
    });
    let main = main?;
    let (found, parts_elapsed) = found;
    let mut warnings = Vec::new();
    let split = match found {
        Ok(s) => {
            if let Some(s) = &s {
                warnings.extend(s.notes.iter().cloned());
            }
            s
        }
        Err(e) => {
            if cancel.load(Ordering::Relaxed) {
                bail!("cancelled");
            }
            warnings.push(format!(
                "The parts failed. The shells of the mesh are the parts: {e:#}"
            ));
            None
        }
    };
    let mesh = Arc::new(main.mesh);
    let (assembly, found) = match &split {
        Some(s) => {
            let a = Assembly::from_split(&mesh, s);
            // One shown value: its pieces give sources after the first.
            let pieces = a.parts.iter().any(|p| !p.reference && p.source > 0);
            let found = match (&s.kind, s.parts.is_empty()) {
                (assembly::Kind::Shown(_), _) if pieces => Found::Selector,
                (assembly::Kind::Shown(_), _) | (_, true) => Found::Shells,
                (assembly::Kind::Colors, false) => Found::List,
                (assembly::Kind::Labeled, false) => Found::Selector,
                (assembly::Kind::Modules, false) => Found::Modules,
            };
            (a, found)
        }
        None => (
            Assembly::shells(&mesh, part_param::shown_label(params).as_deref()),
            Found::Shells,
        ),
    };
    let parts = (assembly.parts.iter())
        .filter(|p| !p.reference)
        .map(|p| Input {
            name: p.name.clone(),
            mesh: Arc::new(super::sub_mesh(&assembly.mesh, p.tris.clone())),
        })
        .collect();
    let references = (assembly.parts.iter())
        .filter(|p| p.reference)
        .map(|p| p.name.clone())
        .collect();
    Ok(Built {
        whole_volume: mesh.volume().abs(),
        mesh,
        elapsed: main.elapsed,
        parts_elapsed,
        parts,
        references,
        multicolor: !parts::names(params).is_empty(),
        model_name: file
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        found,
        warnings,
    })
}

// ------------------------------------------------------------ stored state

/// The key of a model in `models.json`: its canonical path, as in
/// `app/state.rs`.
fn model_key(file: &Path) -> String {
    file.canonicalize()
        .unwrap_or_else(|_| file.to_path_buf())
        .display()
        .to_string()
}

/// The Optimize state of the model `key` in the content of `models.json`.
fn state_in(all: &Json, key: &str, area: Area) -> Option<State> {
    Some(State::from_json(all.get(key)?.get("optimize")?, area))
}

/// The Optimize state that the app stored for `file` in
/// `<config>/models.json`, with `area` for a missing print area. `None`
/// when the model has no stored state. The file is not changed.
pub fn stored_state(file: &Path, area: Area) -> Option<State> {
    stored_state_in(&openscad::config_dir()?, file, area)
}

/// `stored_state` with the config folder `dir`.
fn stored_state_in(dir: &Path, file: &Path, area: Area) -> Option<State> {
    let text = std::fs::read_to_string(dir.join("models.json")).ok()?;
    let all: Json = serde_json::from_str(&text).ok()?;
    state_in(&all, &model_key(file), area)
}

// ----------------------------------------------------------------- options

/// The plates that a command writes or slices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plates {
    /// One plate (0 is plate 1).
    One(usize),
    All,
}

/// The print options of `cocad export` and `cocad slice`. Each option
/// changes the stored state of the model. A missing option keeps it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Options {
    /// `--plate N` or `--plate all`. `None`: the default of the command.
    pub plate: Option<Plates>,
    /// `--part NAME`: only this print part. "all": all print parts.
    pub part: Option<String>,
    /// `--bed WxDxH`.
    pub bed: Option<Area>,
    /// `--orient model|auto`, for all parts.
    pub orient: Option<Choice>,
    /// `--chamfer[=MM]`. `Some(None)`: on, with the stored size.
    pub chamfer: Option<Option<f32>>,
    pub teardrops: bool,
    /// `--bridges[=sacrificial|stepped]`. `Some(None)`: on, with the stored
    /// choice for a hole above a bridge.
    pub bridges: Option<Option<fixes::HoleAbove>>,
    pub brim: bool,
    pub ears: bool,
    /// `--holes[=MM]`. `Some(None)`: on, with the stored value.
    pub holes: Option<Option<f32>>,
    /// `--no-fixes`: all stored fixes off. The fix options still turn
    /// single fixes on.
    pub no_fixes: bool,
    /// `--as-built`: the model as built, without the print preparation.
    pub as_built: bool,
}

/// The value of an option: the text after `=`, else the next argument.
fn arg(inline: Option<&str>, next: impl FnOnce() -> Result<String>) -> Result<String> {
    match inline {
        Some(v) => Ok(v.to_string()),
        None => next(),
    }
}

fn parse_plate(v: &str) -> Result<Plates> {
    if v.eq_ignore_ascii_case("all") {
        return Ok(Plates::All);
    }
    match v.trim().parse::<usize>() {
        Ok(n) if n >= 1 => Ok(Plates::One(n - 1)),
        _ => bail!("--plate needs a plate number (1 or more) or all, got {v}"),
    }
}

fn parse_bed(v: &str) -> Result<Area> {
    let mm: Vec<f32> = v
        .split(['x', 'X', '×'])
        .map(|s| s.trim().parse::<f32>())
        .collect::<Result<_, _>>()
        .ok()
        .filter(|mm: &Vec<f32>| mm.len() == 3)
        .with_context(|| format!("--bed needs WxDxH in mm, for example 250x210x220, got {v}"))?;
    let (lo, hi) = (crate::printer::MIN_MM, crate::printer::MAX_MM);
    if mm.iter().any(|x| !(lo..=hi).contains(x)) {
        bail!("--bed values must be from {lo} to {hi} mm, got {v}");
    }
    Ok(Area {
        w: mm[0],
        d: mm[1],
        h: mm[2],
    })
}

fn parse_orient(v: &str) -> Result<Choice> {
    match v {
        "model" => Ok(Choice::Model),
        "auto" => Ok(Choice::Auto),
        _ => bail!("--orient must be model or auto, got {v}"),
    }
}

fn parse_chamfer(v: &str) -> Result<f32> {
    match v.trim().parse::<f32>() {
        Ok(mm) if (0.0..=2.0).contains(&mm) => Ok(mm),
        _ => bail!("--chamfer needs a size from 0 to 2 mm, got {v}"),
    }
}

fn parse_hole_add(v: &str) -> Result<f32> {
    match v.trim().parse::<f32>() {
        Ok(mm) if (0.0..=1.0).contains(&mm) => Ok(mm),
        _ => bail!("--holes needs a value from 0 to 1 mm, got {v}"),
    }
}

fn parse_hole_above(v: &str) -> Result<fixes::HoleAbove> {
    fixes::HoleAbove::from_name(v)
        .with_context(|| format!("--bridges must be sacrificial or stepped, got {v}"))
}

impl Options {
    /// Reads the print option `flag`. `next` gives the next argument, for
    /// an option with a value. An option with a value also takes the form
    /// `--plate=2`. Returns false when `flag` is not a print option.
    pub fn parse_flag(
        &mut self,
        flag: &str,
        next: impl FnOnce() -> Result<String>,
    ) -> Result<bool> {
        if !flag.starts_with("--") {
            return Ok(false);
        }
        let (name, inline) = match flag.split_once('=') {
            Some((n, v)) => (n, Some(v)),
            None => (flag, None),
        };
        match (name, inline) {
            ("--plate", v) => self.plate = Some(parse_plate(&arg(v, next)?)?),
            ("--part", v) => {
                let v = arg(v, next)?;
                if v.trim().is_empty() {
                    bail!("--part needs a part name or all");
                }
                self.part = Some(v);
            }
            ("--bed", v) => self.bed = Some(parse_bed(&arg(v, next)?)?),
            ("--orient", v) => self.orient = Some(parse_orient(&arg(v, next)?)?),
            ("--chamfer", v) => self.chamfer = Some(v.map(parse_chamfer).transpose()?),
            ("--bridges", v) => self.bridges = Some(v.map(parse_hole_above).transpose()?),
            ("--teardrops", None) => self.teardrops = true,
            ("--brim", None) => self.brim = true,
            ("--ears", None) => self.ears = true,
            ("--holes", v) => self.holes = Some(v.map(parse_hole_add).transpose()?),
            ("--no-fixes", None) => self.no_fixes = true,
            ("--as-built", None) => self.as_built = true,
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// True when an option changes the print preparation. `--part` and
    /// `--as-built` do not count: they also apply to the model as built.
    pub fn changes_print(&self) -> bool {
        self.plate.is_some()
            || self.bed.is_some()
            || self.orient.is_some()
            || self.chamfer.is_some()
            || self.teardrops
            || self.bridges.is_some()
            || self.brim
            || self.ears
            || self.holes.is_some()
            || self.no_fixes
    }

    /// The part that `--part` selects. `None` for no `--part` and for
    /// `--part all`.
    pub fn only(&self) -> Option<&str> {
        self.part.as_deref().filter(|p| *p != "all")
    }

    /// Changes `st` as the options tell. `--orient` changes the stored
    /// parts here; `optimize` also sets it for the other parts.
    pub fn apply(&self, st: &mut State) {
        if let Some(a) = self.bed {
            st.area = a;
        }
        match self.part.as_deref() {
            Some("all") => st.only = None,
            Some(name) => {
                st.only = Some(name.to_string());
                // The user asks for this part: print it, also when the
                // app turned it off. The pieces of a split part too.
                let piece = format!("{name} ");
                for (k, p) in st.parts.iter_mut() {
                    if k == name || k.starts_with(&piece) {
                        p.include = true;
                    }
                }
            }
            None => {}
        }
        let f = &mut st.fixes;
        if self.no_fixes {
            f.teardrops = false;
            f.bridges = false;
            f.chamfer = false;
            f.brim = false;
            f.ears = false;
            f.hole_size = false;
        }
        match self.chamfer {
            Some(Some(mm)) if mm <= 0.0 => f.chamfer = false,
            Some(Some(mm)) => {
                f.chamfer = true;
                f.chamfer_mm = mm;
            }
            Some(None) => f.chamfer = true,
            None => {}
        }
        if self.teardrops {
            f.teardrops = true;
        }
        if let Some(h) = self.bridges {
            f.bridges = true;
            if let Some(h) = h {
                f.hole_above = h;
            }
        }
        if self.brim {
            f.brim = true;
        }
        if self.ears {
            f.ears = true;
        }
        if let Some(add) = self.holes {
            f.hole_size = true;
            if let Some(mm) = add {
                f.hole_add = mm;
            }
        }
        if let Some(c) = self.orient {
            let names: Vec<String> = st.parts.keys().cloned().collect();
            orient_parts(st, names, c);
        }
    }
}

/// Sets the orientation `choice`, without quarter turns, for each part in
/// `names`.
pub fn orient_parts(st: &mut State, names: impl IntoIterator<Item = String>, choice: Choice) {
    for n in names {
        st.parts.entry(n).or_default().orientation = Orientation {
            choice,
            quarter_turns: 0,
        };
    }
}

/// The names that the print parts of `built` will most likely have: the
/// part names, the model name (multicolor), and the pieces of the stored
/// splits. Joined parts get other names; `optimize` finds them after the
/// first run.
fn likely_names(built: &Built, st: &State) -> Vec<String> {
    let mut out: Vec<String> = built.parts.iter().map(|p| p.name.clone()).collect();
    out.push(built.model_name.clone());
    for (name, p) in &st.parts {
        if let Some(sp) = &p.split {
            let n = sp.pieces();
            out.extend((1..=n).map(|i| format!("{name} {i}/{n}")));
            out.push(format!("{name} pin"));
        }
    }
    out
}

/// The result of `optimize`.
pub struct Prepared {
    /// The state after the options, as the pipeline used it.
    pub state: State,
    /// The model has an Optimize state in `models.json`.
    pub stored: bool,
    pub outcome: Outcome,
    /// Time of the pipeline.
    pub elapsed: Duration,
}

/// Reads the stored state of `file` (else a new state with the print area
/// of the settings), applies `opts` and runs the pipeline on `built`.
pub fn optimize(
    cfg: &Config,
    file: &Path,
    built: &Built,
    opts: &Options,
    layers: Layers,
    cancel: &Arc<AtomicBool>,
) -> Result<Prepared> {
    let stored = stored_state(file, cfg.bed);
    let has_stored = stored.is_some();
    let mut state = stored.unwrap_or_else(|| State::new(cfg.bed));
    opts.apply(&mut state);
    if let Some(c) = opts.orient {
        let names = likely_names(built, &state);
        orient_parts(&mut state, names, c);
    }
    let t = Instant::now();
    let mut job = built.job(state, layers, Limits::default());
    let cache = Mutex::new(Cache::default());
    let mut outcome = pipeline::run(cfg, &job, &cache, cancel)?;
    if let Some(c) = opts.orient {
        // Joined parts have names that only the run knows. Run again for
        // them: the joins and splits come from the cache.
        let want = Orientation {
            choice: c,
            quarter_turns: 0,
        };
        let wrong: Vec<String> = (outcome.parts.iter())
            .filter(|p| p.state.orientation != want)
            .map(|p| p.name.clone())
            .collect();
        if !wrong.is_empty() {
            orient_parts(&mut job.state, wrong, c);
            outcome = pipeline::run(cfg, &job, &cache, cancel)?;
        }
    }
    check_only(&job.state, &outcome, has_stored && opts.only().is_none())?;
    Ok(Prepared {
        state: job.state,
        stored: has_stored,
        outcome,
        elapsed: t.elapsed(),
    })
}

/// An error when the state selects one part (`only`) and no print part has
/// that name. `stored`: the choice comes from the app, not from `--part`.
fn check_only(st: &State, o: &Outcome, stored: bool) -> Result<()> {
    let Some(only) = &st.only else {
        return Ok(());
    };
    let hit = o
        .parts
        .iter()
        .any(|p| p.name == *only || p.piece.as_ref().is_some_and(|(parent, _)| parent == only));
    if hit {
        return Ok(());
    }
    let mut names: Vec<&str> = Vec::new();
    for p in &o.parts {
        let n = p
            .piece
            .as_ref()
            .map_or(p.name.as_str(), |(parent, _)| parent.as_str());
        if !names.contains(&n) {
            names.push(n);
        }
    }
    let from = if stored {
        " The app stored this choice. Use --part all to print all parts."
    } else {
        ""
    };
    bail!(
        "No print part has the name '{only}'.{from}\nThe print parts: {}",
        names.join(", ")
    )
}

// ------------------------------------------------------------------ output

/// The copies of each print part on the plates.
fn placed(o: &Outcome) -> Vec<usize> {
    let mut n = vec![0; o.parts.len()];
    for i in &o.instances {
        n[i.part] += 1;
    }
    n
}

/// `n` and the word for one or more.
fn count(n: usize, one: &str, more: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { more })
}

fn kind_label(k: Kind, n: usize) -> String {
    let (one, more) = match k {
        Kind::Overhang => ("overhang", "overhangs"),
        Kind::LongBridge => ("long bridge", "long bridges"),
        Kind::ThinWall => ("thin wall", "thin walls"),
        Kind::Arc => ("arc in a bridge", "arcs in bridges"),
        Kind::SmallIsland => ("small bed island", "small bed islands"),
        Kind::TooLarge => ("part too large", "parts too large"),
    };
    count(n, one, more)
}

fn size_text(m: &Mesh) -> String {
    let s = super::bounds_or_zero(m).size();
    format!("{:.1} x {:.1} x {:.1} mm", s.x, s.y, s.z)
}

/// The most skipped places that the report lists one by one.
const MAX_SKIPPED: usize = 20;

/// A short report of the print preparation: the print parts with their
/// copies, the plates, the fixes with the number of changed places, the
/// skipped places, the check problems of each kind and the errors. Each
/// line starts with a label of 12 characters, as the slice estimate.
pub fn report(built: &Built, p: &Prepared) -> String {
    let o = &p.outcome;
    let st = &p.state;
    let mut s = String::new();
    let refs = match built.references.len() {
        0 => String::new(),
        1 => ", 1 reference part (not printed)".to_string(),
        n => format!(", {n} reference parts (not printed)"),
    };
    let _ = writeln!(
        s,
        "parts       {} from {}{refs}, found in {:.2} s",
        built.parts.len(),
        built.found.label(),
        built.parts_elapsed.as_secs_f32()
    );
    for w in &built.warnings {
        let _ = writeln!(s, "warning     {w}");
    }
    let _ = writeln!(
        s,
        "state       {}",
        if p.stored {
            "the Optimize values that the app stored for the model"
        } else {
            "the defaults (the app stored no Optimize values for the model)"
        }
    );
    let on = placed(o);
    let width = (o.parts.iter())
        .map(|q| q.name.chars().count())
        .max()
        .unwrap_or(0)
        .min(32);
    let _ = writeln!(s, "print parts {}", o.parts.len());
    for (k, q) in o.parts.iter().enumerate() {
        let what = if !q.fits {
            let why = match &q.proposal {
                Some(sp) => format!(
                    " A split in {} pieces fits: split it in the Optimize tab.",
                    sp.pieces()
                ),
                None if split::fits(&q.model, &st.area) => {
                    " It fits in another orientation: select the bed face in the Optimize tab."
                        .to_string()
                }
                None if q.piece.is_none() => " No split across one axis makes it fit.".to_string(),
                None => String::new(),
            };
            format!(
                "{}: too large for the print area, not on a plate.{why}",
                size_text(&q.mesh)
            )
        } else if q.copies == 0 {
            "not printed".to_string()
        } else {
            let model = if q.copies != q.count {
                format!(" (model: {})", q.count)
            } else {
                String::new()
            };
            format!("x{}{model}  {}", on[k], size_text(&q.mesh))
        };
        let _ = writeln!(s, "  {:<width$}  {what}", q.name);
    }
    let _ = writeln!(
        s,
        "plates      {} (print area {} x {} x {} mm)",
        o.plates, st.area.w, st.area.d, st.area.h
    );
    for plate in 0..o.plates {
        let mut counts: BTreeMap<usize, usize> = BTreeMap::new();
        for (part, _) in o.on_plate(plate) {
            *counts.entry(part).or_default() += 1;
        }
        let list: Vec<String> = (counts.iter())
            .map(|(&k, &n)| {
                if n == 1 {
                    o.parts[k].name.clone()
                } else {
                    format!("{} x{n}", o.parts[k].name)
                }
            })
            .collect();
        let _ = writeln!(s, "  plate {}  {}", plate + 1, list.join(", "));
    }
    s.push_str(&fixes_line(st, o));
    // The same text of one part is one line with a count.
    let mut skipped: Vec<(String, usize)> = Vec::new();
    for q in &o.parts {
        for t in &q.report.skipped {
            let line = format!("{}: {t}", q.name);
            match skipped.iter_mut().find(|(l, _)| *l == line) {
                Some((_, n)) => *n += 1,
                None => skipped.push((line, 1)),
            }
        }
    }
    if !skipped.is_empty() {
        let total = skipped.iter().map(|(_, n)| n).sum();
        let _ = writeln!(s, "skipped     {}", count(total, "place", "places"));
        for (t, n) in skipped.iter().take(MAX_SKIPPED) {
            match n {
                1 => {
                    let _ = writeln!(s, "  {t}");
                }
                n => {
                    let _ = writeln!(s, "  {t} ({n} places)");
                }
            }
        }
        if skipped.len() > MAX_SKIPPED {
            let rest: usize = skipped[MAX_SKIPPED..].iter().map(|(_, n)| n).sum();
            let _ = writeln!(s, "  and {} more", count(rest, "place", "places"));
        }
    }
    let mut kinds: Vec<(Kind, usize)> = Vec::new();
    for i in o.parts.iter().flat_map(|q| &q.issues) {
        match kinds.iter_mut().find(|(k, _)| *k == i.kind) {
            Some((_, n)) => *n += 1,
            None => kinds.push((i.kind, 1)),
        }
    }
    if kinds.is_empty() {
        s.push_str("check       no problems\n");
    } else {
        let list: Vec<String> = kinds.iter().map(|&(k, n)| kind_label(k, n)).collect();
        let _ = writeln!(s, "check       {}", list.join(", "));
    }
    for e in &o.errors {
        let _ = writeln!(s, "error       {}", e.lines().next().unwrap_or(""));
    }
    let _ = writeln!(s, "optimized in {:.2} s", p.elapsed.as_secs_f32());
    s
}

/// The fixes that are on, with the number of places that they changed in
/// the print parts (not counting copies).
fn fixes_line(st: &State, o: &Outcome) -> String {
    let f = &st.fixes;
    if !f.any() {
        return "fixes       off\n".to_string();
    }
    let sum = |g: fn(&fixes::Report) -> usize| o.parts.iter().map(|q| g(&q.report)).sum::<usize>();
    let mut list = Vec::new();
    if f.chamfer {
        let n = o.parts.iter().filter(|q| q.report.chamfer).count();
        list.push(format!(
            "bed chamfer {} mm on {n} of {} parts",
            f.chamfer_mm,
            o.parts.len()
        ));
    }
    if f.teardrops {
        list.push(count(
            sum(|r| r.teardrops),
            "teardrop hole",
            "teardrop holes",
        ));
    }
    if f.bridges {
        list.push(format!(
            "{} (hole above: {})",
            count(sum(|r| r.bridges), "bridge", "bridges"),
            f.hole_above.name()
        ));
    }
    if f.brim {
        list.push(count(sum(|r| r.brims), "brim", "brims"));
    }
    if f.ears {
        list.push(count(sum(|r| r.ears), "mouse ear", "mouse ears"));
    }
    if f.hole_size {
        list.push(format!(
            "hole size +{} mm on {}",
            f.hole_add,
            count(sum(|r| r.holes), "hole", "holes")
        ));
    }
    format!("fixes       {}\n", list.join(", "))
}

/// A 3MF file of `plate`: one object for each print part on the plate, one
/// build item for each copy.
pub fn plate_3mf(o: &Outcome, plate: usize, meta: &export::Meta) -> Result<Vec<u8>> {
    let mut index: HashMap<usize, usize> = HashMap::new();
    let mut objects: Vec<(String, &Mesh)> = Vec::new();
    let mut items = Vec::new();
    for (part, at) in o.on_plate(plate) {
        let k = *index.entry(part).or_insert_with(|| {
            objects.push((o.parts[part].name.clone(), o.parts[part].mesh.as_ref()));
            objects.len() - 1
        });
        items.push((k, at));
    }
    export::plate_3mf_bytes(&objects, &items, meta)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(args: &[&str]) -> Result<Options> {
        let mut o = Options::default();
        let mut it = args.iter();
        while let Some(a) = it.next() {
            let next = || it.next().map(|s| s.to_string()).context("needs a value");
            if !o.parse_flag(a, next)? {
                bail!("not a print option: {a}");
            }
        }
        Ok(o)
    }

    #[test]
    fn flags_parse() {
        let o = parse(&[
            "--plate",
            "2",
            "--part",
            "Part 1",
            "--bed",
            "250x210x220",
            "--orient",
            "auto",
            "--chamfer",
            "--teardrops",
            "--bridges=stepped",
            "--brim",
            "--no-fixes",
        ])
        .unwrap();
        assert_eq!(o.plate, Some(Plates::One(1)));
        assert_eq!(o.part.as_deref(), Some("Part 1"));
        assert_eq!(
            o.bed,
            Some(Area {
                w: 250.0,
                d: 210.0,
                h: 220.0
            })
        );
        assert_eq!(o.orient, Some(Choice::Auto));
        assert_eq!(o.chamfer, Some(None));
        assert!(o.teardrops && o.brim && o.no_fixes && !o.as_built);
        assert_eq!(o.bridges, Some(Some(fixes::HoleAbove::Stepped)));
        assert!(o.changes_print());

        let o = parse(&["--plate=all", "--chamfer=0.6", "--bridges", "--as-built"]).unwrap();
        assert_eq!(o.plate, Some(Plates::All));
        assert_eq!(o.chamfer, Some(Some(0.6)));
        assert_eq!(o.bridges, Some(None));
        assert!(o.as_built);
        assert!(
            !parse(&["--as-built", "--part", "all"])
                .unwrap()
                .changes_print()
        );
        assert_eq!(parse(&["--part", "all"]).unwrap().only(), None);
        assert_eq!(parse(&["--bed", "100X50×60"]).unwrap().bed.unwrap().d, 50.0);
    }

    #[test]
    fn bad_flags_fail() {
        for bad in [
            &["--plate", "0"][..],
            &["--plate", "x"],
            &["--plate"],
            &["--bed", "100x100"],
            &["--bed", "5x100x100"],
            &["--bed", "axbxc"],
            &["--orient", "face"],
            &["--chamfer=3"],
            &["--chamfer=-1"],
            &["--bridges=round"],
            &["--part", " "],
        ] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
        let mut o = Options::default();
        let none = || -> Result<String> { panic!("no value wanted") };
        assert!(!o.parse_flag("-D", none).unwrap());
        assert!(!o.parse_flag("--teardrops=1", none).unwrap());
        assert!(!o.parse_flag("--layer-height", none).unwrap());
        assert!(!o.parse_flag("model.scad", none).unwrap());
        assert_eq!(o, Options::default());
    }

    fn stored() -> State {
        let mut st = State::new(Area::default());
        st.fixes.chamfer = true;
        st.fixes.chamfer_mm = 0.6;
        st.fixes.brim = true;
        st.only = Some("Part 2".into());
        st.parts.insert(
            "Part 1".into(),
            pipeline::PartState {
                include: false,
                orientation: Orientation {
                    choice: Choice::Face(glam::Vec3::X),
                    quarter_turns: 1,
                },
                ..Default::default()
            },
        );
        st
    }

    #[test]
    fn no_options_keep_the_state() {
        let mut st = stored();
        Options::default().apply(&mut st);
        assert_eq!(st, stored());
    }

    #[test]
    fn fix_options_change_the_fixes() {
        let mut st = stored();
        parse(&["--no-fixes", "--teardrops"])
            .unwrap()
            .apply(&mut st);
        assert!(st.fixes.teardrops && !st.fixes.chamfer && !st.fixes.brim && !st.fixes.bridges);
        // The values stay for the next time.
        assert_eq!(st.fixes.chamfer_mm, 0.6);

        let mut st = stored();
        parse(&["--chamfer", "--bridges=stepped"])
            .unwrap()
            .apply(&mut st);
        assert!(st.fixes.chamfer && st.fixes.bridges && st.fixes.brim);
        assert_eq!(st.fixes.chamfer_mm, 0.6);
        assert_eq!(st.fixes.hole_above, fixes::HoleAbove::Stepped);

        let mut st = stored();
        parse(&["--chamfer=0.4"]).unwrap().apply(&mut st);
        assert!(st.fixes.chamfer && st.fixes.chamfer_mm == 0.4);
        parse(&["--chamfer=0"]).unwrap().apply(&mut st);
        assert!(!st.fixes.chamfer);
    }

    #[test]
    fn part_bed_and_orient_options_change_the_state() {
        let mut st = stored();
        parse(&["--part", "all", "--bed", "180x180x180"])
            .unwrap()
            .apply(&mut st);
        assert_eq!(st.only, None);
        assert_eq!(st.area.w, 180.0);

        let mut st = stored();
        parse(&["--part", "Part 1"]).unwrap().apply(&mut st);
        assert_eq!(st.only.as_deref(), Some("Part 1"));
        assert!(st.parts["Part 1"].include);

        let mut st = stored();
        parse(&["--orient", "model"]).unwrap().apply(&mut st);
        assert_eq!(st.parts["Part 1"].orientation.choice, Choice::Model);
        // The other values of the part stay.
        assert!(!st.parts["Part 1"].include);

        orient_parts(&mut st, ["Part 3".to_string()], Choice::Auto);
        assert_eq!(st.parts["Part 3"].orientation.choice, Choice::Auto);
        assert!(st.parts["Part 3"].include);
    }

    #[test]
    fn stored_state_comes_from_the_model_key() {
        let area = Area {
            w: 180.0,
            d: 180.0,
            h: 180.0,
        };
        let all = json!({
            "/m/box.scad": { "mode": "optimize", "optimize": {
                "area": [250, 210, 220], "only": "Lid",
                "fixes": { "teardrops": true } } },
            "/m/other.scad": { "mode": "model" },
        });
        let st = state_in(&all, "/m/box.scad", area).unwrap();
        assert_eq!(st.area.w, 250.0);
        assert_eq!(st.only.as_deref(), Some("Lid"));
        assert!(st.fixes.teardrops && !st.fixes.chamfer);
        assert!(state_in(&all, "/m/other.scad", area).is_none());
        assert!(state_in(&all, "/m/none.scad", area).is_none());
    }

    #[test]
    fn stored_state_reads_models_json_by_the_canonical_path() {
        let dir = std::env::temp_dir().join(format!("cocad-headless-state-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let scad = dir.join("box.scad");
        std::fs::write(&scad, "cube(1);").unwrap();
        let key = scad.canonicalize().unwrap().display().to_string();
        let all = json!({ key: { "optimize": { "only": "Lid", "fixes": { "brim": true } } } });
        std::fs::write(dir.join("models.json"), all.to_string()).unwrap();
        // Another spelling of the same path finds the same entry.
        let other = dir.join("sub").join("..").join("box.scad");
        let st = stored_state_in(&dir, &other, Area::default()).unwrap();
        assert_eq!(st.only.as_deref(), Some("Lid"));
        assert!(st.fixes.brim);
        assert_eq!(st.area, Area::default());
        assert!(stored_state_in(&dir, &dir.join("none.scad"), Area::default()).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn likely_names_have_the_pieces_of_a_split() {
        let built = Built {
            mesh: Arc::new(Mesh::default()),
            elapsed: Duration::ZERO,
            parts_elapsed: Duration::ZERO,
            parts: vec![Input {
                name: "Part 1".into(),
                mesh: Arc::new(crate::mesh::cube(1.0)),
            }],
            references: Vec::new(),
            whole_volume: 1.0,
            multicolor: false,
            model_name: "box".into(),
            found: Found::Shells,
            warnings: Vec::new(),
        };
        let mut st = State::new(Area::default());
        st.parts.insert(
            "Part 1".into(),
            pipeline::PartState {
                split: Some(split::Split {
                    cuts: vec![split::Cut { axis: 0, at: 1.0 }],
                    joint: split::Joint::Pins,
                    values: split::JointValues::default(),
                }),
                ..Default::default()
            },
        );
        let names = likely_names(&built, &st);
        for n in ["Part 1", "box", "Part 1 1/2", "Part 1 2/2", "Part 1 pin"] {
            assert!(names.iter().any(|x| x == n), "{n} in {names:?}");
        }
    }

    fn cubes_outcome(area: Area) -> (Built, Prepared) {
        let at = |x: f32| {
            Arc::new(super::super::transformed(
                &crate::mesh::cube(10.0),
                glam::Mat4::from_translation(glam::Vec3::new(x, 0.0, 0.0)),
            ))
        };
        let parts = vec![
            Input {
                name: "Part 1".into(),
                mesh: at(0.0),
            },
            Input {
                name: "Part 2".into(),
                mesh: at(30.0),
            },
            Input {
                name: "Part 3".into(),
                mesh: at(60.0),
            },
        ];
        let built = Built {
            mesh: Arc::new(Mesh::default()),
            elapsed: Duration::ZERO,
            parts_elapsed: Duration::ZERO,
            whole_volume: 3000.0,
            parts,
            references: vec!["Reference".into()],
            multicolor: false,
            model_name: "cubes".into(),
            found: Found::Modules,
            warnings: Vec::new(),
        };
        let job = built.job(State::new(area), Layers::default(), Limits::default());
        let outcome = pipeline::run(
            &Config::default(),
            &job,
            &Mutex::new(Cache::default()),
            &Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        let p = Prepared {
            state: job.state,
            stored: false,
            outcome,
            elapsed: Duration::ZERO,
        };
        (built, p)
    }

    #[test]
    fn plate_3mf_has_one_object_per_part_and_one_item_per_copy() {
        let (_, p) = cubes_outcome(Area::default());
        // Three same cubes: one print part with three copies.
        assert_eq!(p.outcome.parts.len(), 1);
        assert_eq!(p.outcome.plates, 1);
        let bytes = plate_3mf(&p.outcome, 0, &export::Meta::default()).unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let text = std::io::read_to_string(zip.by_name("3D/3dmodel.model").unwrap()).unwrap();
        assert_eq!(text.matches("<object ").count(), 1);
        assert_eq!(text.matches("<item ").count(), 3);
        assert!(plate_3mf(&p.outcome, 1, &export::Meta::default()).is_err());
    }

    #[test]
    fn report_lists_parts_plates_and_fixes() {
        // A bed for one cube with its gap: three plates.
        let (built, p) = cubes_outcome(Area {
            w: 15.0,
            d: 15.0,
            h: 15.0,
        });
        assert_eq!(p.outcome.plates, 3);
        let r = report(&built, &p);
        assert!(
            r.contains("parts       3 from the module split, 1 reference part"),
            "{r}"
        );
        assert!(
            r.contains("print parts 1\n  Part 1  x3  10.0 x 10.0 x 10.0 mm\n"),
            "{r}"
        );
        assert!(
            r.contains("plates      3 (print area 15 x 15 x 15 mm)"),
            "{r}"
        );
        assert!(r.contains("  plate 3  Part 1\n"), "{r}");
        assert!(r.contains("fixes       off\n"), "{r}");
        assert!(r.contains("check       no problems\n"), "{r}");
    }

    /// Writes `src` to a temp `.scad` file and builds its parts.
    fn built(name: &str, src: &str) -> Built {
        let dir =
            std::env::temp_dir().join(format!("cocad-headless-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{name}.scad"));
        std::fs::write(&path, src).unwrap();
        let params = params::parse(src);
        let b = inputs(
            &Config::default(),
            &path,
            &params,
            &Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        b
    }

    #[test]
    #[ignore = "needs openscad-nightly"]
    fn inputs_find_two_parts_and_a_reference() {
        let b = built(
            "two",
            "module a() cube(10);\nmodule b() translate([20, 0, 0]) cube(5);\n\
             a();\nb();\n%translate([0, 30, 0]) cube(3);\n",
        );
        assert!(b.warnings.is_empty(), "{:?}", b.warnings);
        assert_eq!(b.found, Found::Modules);
        let names: Vec<&str> = b.parts.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Part 1", "Part 2"]);
        assert_eq!(b.references, ["Reference"]);
        assert!(!b.multicolor);
        assert_eq!(b.model_name, "two");
        // The reference part is not in the built mesh.
        assert!((b.whole_volume - 1125.0).abs() < 0.5, "{}", b.whole_volume);
        let vols: Vec<f32> = b.parts.iter().map(|p| p.mesh.volume()).collect();
        assert!(
            (vols[0] - 1000.0).abs() < 0.5 && (vols[1] - 125.0).abs() < 0.5,
            "{vols:?}"
        );
    }

    #[test]
    #[ignore = "needs openscad-nightly"]
    fn inputs_without_modules_take_the_shells() {
        let b = built("shells", "cube(10);\ntranslate([20, 0, 0]) cube(5);\n");
        assert_eq!(b.found, Found::Shells);
        assert_eq!(b.parts.len(), 2);
        assert!(b.references.is_empty());
    }

    #[test]
    #[ignore = "needs openscad-nightly"]
    fn inputs_of_a_part_list_are_multicolor() {
        let b = built(
            "list",
            "cocad_part = \"all\"; // [all, base, top]\n\
             if (cocad_part == \"all\" || cocad_part == \"base\") cube([20, 20, 2]);\n\
             if (cocad_part == \"all\" || cocad_part == \"top\") translate([5, 5, 2]) cube([10, 10, 1]);\n",
        );
        assert_eq!(b.found, Found::List);
        assert!(b.multicolor);
        let names: Vec<&str> = b.parts.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["base", "top"]);
    }

    #[test]
    #[ignore = "needs openscad-nightly"]
    fn inputs_of_a_part_selector_take_the_labels() {
        let b = built(
            "selector",
            "part = \"all\"; // [all:All, base:Base plate, pins:Pin]\n\
             if (part == \"all\" || part == \"base\") cube([20, 20, 2]);\n\
             if (part == \"all\" || part == \"pins\") for (x = [30, 40]) translate([x, 0, 0]) cube(x / 10);\n\
             if (part == \"all\") translate([0, 40, 0]) cube(1);\n\
             if (part == \"all\") %translate([0, 60, 0]) cube(3);\n",
        );
        assert!(b.warnings.is_empty(), "{:?}", b.warnings);
        assert_eq!(b.found, Found::Selector);
        assert!(!b.multicolor);
        let names: Vec<&str> = b.parts.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Base plate", "Pin 1", "Pin 2", "Other"]);
        assert_eq!(b.references, ["Reference"]);
    }

    #[test]
    #[ignore = "needs openscad-nightly"]
    fn a_failed_part_build_falls_back_to_the_module_split() {
        let b = built(
            "fallback",
            "part = \"all\"; // [all, a, b]\n\
             assert(part == \"all\", \"one part fails\");\n\
             module a() cube(10);\nmodule b() translate([20, 0, 0]) cube(5);\na();\nb();\n",
        );
        assert_eq!(b.found, Found::Modules);
        assert_eq!(b.warnings.len(), 1, "{:?}", b.warnings);
        assert!(b.warnings[0].contains("module split"), "{:?}", b.warnings);
    }

    #[test]
    #[ignore = "needs openscad-nightly"]
    fn touching_pieces_of_one_value_divide_at_their_modules() {
        let b = built(
            "segments",
            "part = \"all\"; // [all, segments:Segment, lid:Lid]\n\
             module show(name) if (part == \"all\" || part == name) children();\n\
             module segment(i) translate([i * 10, 0, 0]) cube(10);\n\
             show(\"segments\") for (i = [0 : 2]) segment(i);\n\
             show(\"lid\") translate([0, 0, 10]) cube([30, 10, 2]);\n",
        );
        assert!(b.warnings.is_empty(), "{:?}", b.warnings);
        assert_eq!(b.found, Found::Selector);
        let names: Vec<&str> = b.parts.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Segment 1", "Segment 2", "Segment 3", "Lid"]);
    }

    #[test]
    #[ignore = "needs openscad-nightly"]
    fn one_selected_value_divides_at_its_modules() {
        let b = built(
            "one-value",
            "part = \"segments\"; // [all, segments:Segment, lid:Lid]\n\
             module show(name) if (part == \"all\" || part == name) children();\n\
             module segment(i) translate([i * 10, 0, 0]) cube(10);\n\
             show(\"segments\") for (i = [0 : 2]) segment(i);\n\
             show(\"lid\") translate([0, 0, 10]) cube([30, 10, 2]);\n",
        );
        assert!(b.warnings.is_empty(), "{:?}", b.warnings);
        assert_eq!(b.found, Found::Selector);
        let names: Vec<&str> = b.parts.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Segment 1", "Segment 2", "Segment 3"]);
    }
}
