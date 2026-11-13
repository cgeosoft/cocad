//! The Optimize pipeline: from the printable parts of the built model to
//! the plates. The app runs `run` in a background thread when the model or
//! an Optimize value changes. The slow steps (joins, splits, fixes) keep
//! their results in a `Cache`, so a change runs only what it needs.

use super::{Area, Layers, Limits, check, fixes, orient, plate, scad, split};
use crate::mesh::Mesh;
use crate::openscad::Config;
use anyhow::Result;
use glam::{Mat4, Vec2, Vec3};
use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// The Optimize values of one print part, stored per model and part name.
#[derive(Clone, Debug, PartialEq)]
pub struct PartState {
    /// False: the part is not on the plates.
    pub include: bool,
    /// Copies on the plates. `None`: the count in the model.
    pub copies: Option<u32>,
    pub orientation: orient::Orientation,
    /// The split that the user accepted.
    pub split: Option<split::Split>,
}

impl Default for PartState {
    fn default() -> Self {
        PartState {
            include: true,
            copies: None,
            orientation: orient::Orientation::default(),
            split: None,
        }
    }
}

/// The Optimize values of one model.
#[derive(Clone, Debug, PartialEq)]
pub struct State {
    pub area: Area,
    pub fixes: fixes::Settings,
    /// Gap between the parts on a plate, in mm.
    pub gap: f32,
    /// `Some(name)`: only this print part is on the plates.
    pub only: Option<String>,
    /// The plate that the view and the export show.
    pub plate: usize,
    pub parts: BTreeMap<String, PartState>,
}

impl State {
    pub fn new(area: Area) -> State {
        State {
            area,
            fixes: fixes::Settings::default(),
            gap: 10.0,
            only: None,
            plate: 0,
            parts: BTreeMap::new(),
        }
    }

    pub fn part(&self, name: &str) -> PartState {
        self.parts.get(name).cloned().unwrap_or_default()
    }
}

/// One printable part of the built model, in the model frame.
#[derive(Clone)]
pub struct Input {
    pub name: String,
    pub mesh: Arc<Mesh>,
}

/// Everything that `run` needs.
#[derive(Clone)]
pub struct Job {
    /// The printable parts. Reference (`%`) parts are not in it.
    pub parts: Vec<Input>,
    /// Volume of the built mesh, for the overlap test.
    pub whole_volume: f32,
    /// The parts are the colors of one object (a multicolor part list).
    pub multicolor: bool,
    /// The model name, for a multicolor object.
    pub model_name: String,
    pub state: State,
    pub layers: Layers,
    pub limits: Limits,
}

/// One print part: a part, joined parts or a piece of a split part.
#[derive(Clone)]
pub struct PrintPart {
    pub name: String,
    /// The `Input` parts in it.
    pub sources: Vec<usize>,
    /// The number of this part in the model.
    pub count: u32,
    /// Copies on the plates (0 when it is not included).
    pub copies: u32,
    pub state: PartState,
    /// The part in the model frame, for the face pick.
    pub model: Arc<Mesh>,
    /// Model frame to print frame.
    pub matrix: Mat4,
    /// The part in print frame, with the fixes.
    pub mesh: Arc<Mesh>,
    pub report: fixes::Report,
    pub issues: Vec<check::Issue>,
    /// The part fits the print area in its print orientation.
    pub fits: bool,
    /// A split that makes the part fit, when it does not fit.
    pub proposal: Option<split::Split>,
    /// For a piece of a split part: the name of the part and the label.
    pub piece: Option<(String, String)>,
}

/// One copy of a print part on a plate.
#[derive(Clone, Copy, Debug)]
pub struct Instance {
    pub part: usize,
    pub plate: usize,
    /// Print frame to bed. The bed goes from (0, 0) to (area.w, area.d).
    pub at: Mat4,
}

/// The result of `run`.
#[derive(Clone, Default)]
pub struct Outcome {
    pub parts: Vec<PrintPart>,
    pub instances: Vec<Instance>,
    pub plates: usize,
    /// Problems of single parts. The other parts are still on the plates.
    pub errors: Vec<String>,
}

impl Outcome {
    /// All copies on `plate` as one mesh, in bed coordinates.
    pub fn plate_mesh(&self, plate: usize) -> Mesh {
        let mut out = Mesh::default();
        for i in self.instances.iter().filter(|i| i.plate == plate) {
            append(&mut out, &self.parts[i.part].mesh, i.at);
        }
        out
    }

    /// The print parts that should be on the plates but are not, because
    /// they are too large for the print area.
    pub fn left_out(&self) -> Vec<&str> {
        (self.parts.iter())
            .filter(|p| p.copies > 0 && !p.fits)
            .map(|p| p.name.as_str())
            .collect()
    }

    /// A note for an export that leaves parts out, or `None`.
    pub fn left_out_note(&self) -> Option<String> {
        let names = self.left_out();
        (!names.is_empty()).then(|| {
            format!(
                "Not on a plate, too large for the print area: {}. Split or turn it in the Optimize tab.",
                names.join(", ")
            )
        })
    }

    /// The copies on `plate`: (print part, bed matrix).
    pub fn on_plate(&self, plate: usize) -> Vec<(usize, Mat4)> {
        self.instances
            .iter()
            .filter(|i| i.plate == plate)
            .map(|i| (i.part, i.at))
            .collect()
    }
}

/// Appends `m` moved by `at` to `out`.
pub fn append(out: &mut Mesh, m: &Mesh, at: Mat4) {
    let base = out.vertices.len() as u32;
    out.vertices
        .extend(m.vertices.iter().map(|v| at.transform_point3(*v)));
    out.triangles
        .extend(m.triangles.iter().map(|t| t.map(|v| v + base)));
}

/// Results of the slow steps, kept between runs.
#[derive(Default)]
pub struct Cache {
    joined: HashMap<u64, Arc<Mesh>>,
    pieces: HashMap<u64, Arc<Vec<split::Piece>>>,
    fixed: HashMap<u64, Arc<(Mesh, fixes::Report, Vec<check::Issue>)>>,
    auto: HashMap<u64, Vec3>,
}

impl Cache {
    /// Keeps the cache small: a full map starts again.
    fn trim(&mut self) {
        const MAX: usize = 256;
        if self.fixed.len() > MAX {
            self.fixed.clear();
        }
        if self.pieces.len() > MAX {
            self.pieces.clear();
        }
        if self.joined.len() > MAX {
            self.joined.clear();
        }
        if self.auto.len() > MAX {
            self.auto.clear();
        }
    }
}

/// A hash of the mesh content.
pub fn mesh_hash(m: &Mesh) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    m.vertices.len().hash(&mut h);
    for v in &m.vertices {
        v.x.to_bits().hash(&mut h);
        v.y.to_bits().hash(&mut h);
        v.z.to_bits().hash(&mut h);
    }
    m.triangles.hash(&mut h);
    h.finish()
}

fn hash_of(parts: &[&dyn std::fmt::Debug]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for p in parts {
        format!("{p:?}").hash(&mut h);
    }
    h.finish()
}

/// A part before the orientation: its name, sources, count and mesh.
struct Pre {
    name: String,
    sources: Vec<usize>,
    count: u32,
    mesh: Arc<Mesh>,
    piece: Option<(String, String)>,
    state: PartState,
    /// For a piece of a split part: how the whole part lies. A piece on
    /// Auto lies the same way, so that all pieces have the same face on the
    /// bed.
    lie_as: Option<orient::Choice>,
}

/// The bed face that Auto finds for `mesh`, as the direction that points
/// down. The cache keeps it, so the slow search runs one time.
fn auto_down(mesh: &Mesh, limits: &Limits, cache: &Mutex<Cache>) -> Vec3 {
    let key = hash_of(&[&mesh_hash(mesh), limits]);
    let hit = cache.lock().unwrap().auto.get(&key).copied();
    match hit {
        Some(d) => d,
        None => {
            let d = orient::auto_down(mesh, limits);
            cache.lock().unwrap().auto.insert(key, d);
            d
        }
    }
}

/// Runs the pipeline. A failed fix or split of one part goes to
/// `Outcome::errors`, and the part goes on without it.
pub fn run(
    cfg: &Config,
    job: &Job,
    cache: &Mutex<Cache>,
    cancel: &Arc<AtomicBool>,
) -> Result<Outcome> {
    let mut errors = Vec::new();
    let st = &job.state;

    // 1. Print parts: join the colors of one object, and parts that overlap.
    let meshes: Vec<&Mesh> = job.parts.iter().map(|p| p.mesh.as_ref()).collect();
    let sum: f32 = meshes.iter().map(|m| m.volume().abs()).sum();
    let groups: Vec<Vec<usize>> = if job.parts.is_empty() {
        Vec::new()
    } else if job.multicolor {
        vec![(0..job.parts.len()).collect()]
    } else if job.whole_volume < sum * 0.998 {
        plate::overlap_groups(&meshes)
    } else {
        (0..job.parts.len()).map(|i| vec![i]).collect()
    };
    let mut joined = Vec::with_capacity(groups.len());
    for g in &groups {
        if g.len() == 1 {
            joined.push((job.parts[g[0]].name.clone(), job.parts[g[0]].mesh.clone()));
            continue;
        }
        let name = if job.multicolor {
            job.model_name.clone()
        } else {
            g.iter()
                .map(|&i| job.parts[i].name.as_str())
                .collect::<Vec<_>>()
                .join(" + ")
        };
        let key = hash_of(&[&g.iter().map(|&i| mesh_hash(meshes[i])).collect::<Vec<_>>()]);
        let hit = cache.lock().unwrap().joined.get(&key).cloned();
        let mesh = match hit {
            Some(m) => m,
            None => {
                let parts: Vec<&Mesh> = g.iter().map(|&i| meshes[i]).collect();
                match scad::run(
                    cfg,
                    &parts,
                    |imports| format!("union() {{\n{}\n}}\n", imports.join("\n")),
                    cancel,
                ) {
                    Ok(m) => {
                        let m = Arc::new(m);
                        cache.lock().unwrap().joined.insert(key, m.clone());
                        m
                    }
                    Err(e) => {
                        errors.push(format!("{name}: cannot join the parts: {e:#}"));
                        // The slicer joins overlapping meshes itself.
                        let mut m = Mesh::default();
                        for p in &parts {
                            append(&mut m, p, Mat4::IDENTITY);
                        }
                        Arc::new(m)
                    }
                }
            }
        };
        joined.push((name, mesh));
    }

    // 2. Parts with the same shape: one row with a count.
    let sigs: Vec<plate::Signature> = joined.iter().map(|(_, m)| plate::signature(m)).collect();
    let mut pres: Vec<Pre> = Vec::new();
    for same in plate::group_same(&sigs) {
        let (name, mesh) = &joined[same[0]];
        let sources: Vec<usize> = same.iter().flat_map(|&k| groups[k].clone()).collect();
        pres.push(Pre {
            name: name.clone(),
            sources,
            count: same.len() as u32,
            mesh: mesh.clone(),
            piece: None,
            state: st.part(name),
            lie_as: None,
        });
    }

    // 3. Splits that the user accepted: each piece is a print part.
    let mut split_pres = Vec::new();
    for p in pres {
        let Some(sp) = p.state.split.clone() else {
            split_pres.push(p);
            continue;
        };
        if cancel.load(Ordering::Relaxed) {
            anyhow::bail!("cancelled");
        }
        let key = hash_of(&[&mesh_hash(&p.mesh), &sp]);
        let hit = cache.lock().unwrap().pieces.get(&key).cloned();
        let pieces = match hit {
            Some(v) => Ok(v),
            None => split::apply(cfg, &p.mesh, &sp, cancel).map(|v| {
                let v = Arc::new(v);
                cache.lock().unwrap().pieces.insert(key, v.clone());
                v
            }),
        };
        match pieces {
            Ok(pieces) => {
                // A cut with no room for its pins or tabs is a flat cut.
                // Tell the user, so the joint is not a surprise.
                for (c, pl) in split::places(&p.mesh, &sp).iter().enumerate() {
                    let at = sp.cuts.get(c).map_or(0.0, |c| c.at);
                    let none = match sp.joint {
                        split::Joint::Pins => pl.pins.is_empty(),
                        split::Joint::Dovetail => pl.tabs.is_empty(),
                        split::Joint::Flat => false,
                    };
                    if none {
                        errors.push(format!(
                            "{}: the cut at {at:.1} mm has no room for {}. It is a flat cut.",
                            p.name,
                            if sp.joint == split::Joint::Pins {
                                "pins"
                            } else {
                                "tabs"
                            }
                        ));
                    }
                }
                // The pieces lie as the whole part lies. Auto finds the bed
                // face of the whole part, not of each piece.
                let whole = match p.state.orientation.choice {
                    orient::Choice::Auto => {
                        orient::Choice::Face(auto_down(&p.mesh, &job.limits, cache))
                    }
                    c => c,
                };
                for piece in pieces.iter() {
                    let name = format!("{} {}", p.name, piece.label);
                    // A piece keeps its own orientation and copies. A pin is
                    // a part of its own: it does not lie as the whole part.
                    let mut state = st.part(&name);
                    state.split = None;
                    split_pres.push(Pre {
                        name,
                        sources: p.sources.clone(),
                        count: p.count,
                        mesh: Arc::new(piece.mesh.clone()),
                        piece: Some((p.name.clone(), piece.label.clone())),
                        state,
                        lie_as: (piece.label != "pin").then_some(whole),
                    });
                }
            }
            Err(e) => {
                errors.push(format!("{}: cannot split the part: {e:#}", p.name));
                let mut p = p;
                p.state.split = None;
                split_pres.push(p);
            }
        }
    }
    // Pins of one split are the same part: one row with a count.
    let mut pres: Vec<Pre> = Vec::new();
    for p in split_pres {
        let pin = p.piece.as_ref().is_some_and(|(_, l)| l == "pin");
        if pin
            && let Some(q) = pres
                .iter_mut()
                .find(|q| q.piece.as_ref() == p.piece.as_ref() && q.name == p.name)
        {
            q.count += p.count;
            continue;
        }
        pres.push(p);
    }
    // Pins of one part get the same name: number the rows apart.
    let mut seen: HashMap<String, usize> = HashMap::new();
    for p in &mut pres {
        let n = seen.entry(p.name.clone()).or_default();
        *n += 1;
        if *n > 1 {
            p.name = format!("{} {}", p.name, n);
        }
    }

    // 4. Orientation, 5. fixes and check, in parallel.
    let mut parts: Vec<Option<PrintPart>> = (0..pres.len()).map(|_| None).collect();
    let errs = Mutex::new(Vec::new());
    let next = AtomicUsize::new(0);
    let results = Mutex::new(Vec::new());
    let workers = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(pres.len().max(1));
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(p) = pres.get(i) else { break };
                    let r = prepare(cfg, job, p, cache, &errs, cancel);
                    match r {
                        Ok(pp) => results.lock().unwrap().push((i, pp)),
                        Err(e) => errs.lock().unwrap().push(format!("{}: {e:#}", p.name)),
                    }
                }
            });
        }
    });
    if cancel.load(Ordering::Relaxed) {
        anyhow::bail!("cancelled");
    }
    errors.extend(errs.into_inner().unwrap());
    for (i, pp) in results.into_inner().unwrap() {
        parts[i] = Some(pp);
    }
    let mut parts: Vec<PrintPart> = parts.into_iter().flatten().collect();

    // 6. Plates.
    let mut items = Vec::new();
    let mut sizes = Vec::new();
    for (k, p) in parts.iter_mut().enumerate() {
        let on = p.state.include
            && st.only.as_ref().is_none_or(|o| {
                *o == p.name || p.piece.as_ref().is_some_and(|(parent, _)| parent == o)
            });
        p.copies = if on {
            p.state.copies.unwrap_or(p.count)
        } else {
            0
        };
        if !p.fits {
            continue;
        }
        let b = super::bounds_or_zero(&p.mesh);
        let size = Vec2::new(b.max.x - b.min.x, b.max.y - b.min.y);
        for _ in 0..p.copies {
            items.push(k);
            sizes.push(size);
        }
    }
    let placed = plate::layout(&sizes, &st.area, st.gap);
    let mut instances = Vec::new();
    let mut plates = 0;
    for (k, pl) in items.iter().zip(placed) {
        let Some(n) = pl.plate else { continue };
        let b = super::bounds_or_zero(&parts[*k].mesh);
        let c = b.center();
        let turn = if pl.turned {
            Mat4::from_rotation_z(std::f32::consts::FRAC_PI_2)
        } else {
            Mat4::IDENTITY
        };
        let at = Mat4::from_translation(Vec3::new(pl.center.x, pl.center.y, 0.0))
            * turn
            * Mat4::from_translation(Vec3::new(-c.x, -c.y, 0.0));
        instances.push(Instance {
            part: *k,
            plate: n,
            at,
        });
        plates = plates.max(n + 1);
    }
    cache.lock().unwrap().trim();
    Ok(Outcome {
        parts,
        instances,
        plates,
        errors,
    })
}

/// Orientation, fixes and check of one print part.
fn prepare(
    cfg: &Config,
    job: &Job,
    p: &Pre,
    cache: &Mutex<Cache>,
    errs: &Mutex<Vec<String>>,
    cancel: &Arc<AtomicBool>,
) -> Result<PrintPart> {
    let st = &job.state;
    let mut o = p.state.orientation;
    if o.choice == orient::Choice::Auto {
        o.choice = match p.lie_as {
            // A piece of a split part on Auto lies as the whole part.
            Some(c) => c,
            None => orient::Choice::Face(auto_down(&p.mesh, &job.limits, cache)),
        };
    }
    let matrix = orient::matrix(&p.mesh, &o, &st.area, &job.limits);
    let print = super::transformed(&p.mesh, matrix);
    let fits = orient::fits(&print, &st.area);
    let proposal = if fits || p.piece.is_some() {
        None
    } else {
        split::propose(&p.mesh, &st.area, default_joint(&p.mesh))
    };
    // No split is needed when the part fits on another side.
    if !fits && proposal.is_none() && p.piece.is_none() && split::fits(&p.mesh, &st.area) {
        errs.lock().unwrap().push(format!(
            "{}: too large in this orientation. It fits on another side: select the bed face in the Orientation card.",
            p.name
        ));
    }
    // The pin holes of a split keep their size: the clearance of the
    // split sets their fit.
    let mut fix = st.fixes;
    if let Some((parent, _)) = &p.piece
        && let Some(sp) = st.part(parent).split
        && sp.joint == split::Joint::Pins
    {
        fix.hole_keep = sp.values.hole_d();
    }
    let key = hash_of(&[&mesh_hash(&print), &fix, &job.layers, &job.limits, &st.area]);
    let hit = cache.lock().unwrap().fixed.get(&key).cloned();
    let fixed = match hit {
        Some(f) => f,
        None => {
            let (mesh, report) = if fix.any() {
                fixes::apply(cfg, &print, &fix, job.layers, &job.limits, &st.area, cancel)?
            } else {
                (print, fixes::Report::default())
            };
            let issues = check::run(&mesh, job.layers, &job.limits, &st.area);
            let f = Arc::new((mesh, report, issues));
            cache.lock().unwrap().fixed.insert(key, f.clone());
            f
        }
    };
    Ok(PrintPart {
        name: p.name.clone(),
        sources: p.sources.clone(),
        count: p.count,
        copies: 0,
        state: p.state.clone(),
        model: p.mesh.clone(),
        matrix,
        mesh: Arc::new(fixed.0.clone()),
        report: fixed.1.clone(),
        issues: fixed.2.clone(),
        fits,
        proposal,
        piece: p.piece.clone(),
    })
}

/// The joint that CoCAD proposes: a dovetail for a flat part, pins for the
/// others.
pub fn default_joint(mesh: &Mesh) -> split::Joint {
    let b = super::bounds_or_zero(mesh);
    let s = b.size();
    if s.min_element() < 10.0 {
        split::Joint::Dovetail
    } else {
        split::Joint::Pins
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::cube;

    fn boxed(x: f32, y: f32, z: f32, at: Vec3) -> Arc<Mesh> {
        Arc::new(super::super::transformed(
            &cube(1.0),
            Mat4::from_translation(at) * Mat4::from_scale(Vec3::new(x, y, z)),
        ))
    }

    fn job(parts: Vec<Input>) -> Job {
        let whole: f32 = parts.iter().map(|p| p.mesh.volume().abs()).sum();
        Job {
            parts,
            whole_volume: whole,
            multicolor: false,
            model_name: "model".into(),
            state: State::new(Area {
                w: 100.0,
                d: 100.0,
                h: 100.0,
            }),
            layers: Layers::default(),
            limits: Limits::default(),
        }
    }

    fn input(name: &str, mesh: Arc<Mesh>) -> Input {
        Input {
            name: name.into(),
            mesh,
        }
    }

    fn go(j: &Job) -> Outcome {
        run(
            &Config::default(),
            j,
            &Mutex::new(Cache::default()),
            &Arc::new(AtomicBool::new(false)),
        )
        .unwrap()
    }

    #[test]
    fn same_parts_make_one_row_with_a_count() {
        let j = job(vec![
            input("Part 1", boxed(10.0, 10.0, 10.0, Vec3::ZERO)),
            input("Part 2", boxed(10.0, 10.0, 10.0, Vec3::new(50.0, 0.0, 0.0))),
            input("Part 3", boxed(20.0, 10.0, 5.0, Vec3::new(0.0, 50.0, 0.0))),
        ]);
        let o = go(&j);
        assert_eq!(o.parts.len(), 2);
        assert_eq!(o.parts[0].count, 2);
        assert_eq!(o.parts[0].sources, vec![0, 1]);
        assert_eq!(o.instances.len(), 3);
        assert_eq!(o.plates, 1);
        // All copies are on the bed and do not overlap.
        let m = o.plate_mesh(0);
        let b = m.bounds().unwrap();
        assert!(b.min.x >= -1e-3 && b.min.y >= -1e-3 && b.min.z.abs() < 1e-3);
        assert!(b.max.x <= 100.001 && b.max.y <= 100.001);
        assert!((m.volume() - 2.0 * 1000.0 - 1000.0).abs() < 0.1);
    }

    #[test]
    fn only_include_and_copies_change_the_plate() {
        let mut j = job(vec![
            input("Part 1", boxed(10.0, 10.0, 10.0, Vec3::ZERO)),
            input("Part 2", boxed(20.0, 10.0, 5.0, Vec3::new(0.0, 50.0, 0.0))),
        ]);
        j.state.only = Some("Part 2".into());
        assert_eq!(go(&j).instances.len(), 1);
        j.state.only = None;
        j.state.parts.insert(
            "Part 1".into(),
            PartState {
                include: false,
                ..PartState::default()
            },
        );
        j.state.parts.insert(
            "Part 2".into(),
            PartState {
                copies: Some(3),
                ..PartState::default()
            },
        );
        let o = go(&j);
        assert_eq!(o.instances.len(), 3);
        assert!(o.instances.iter().all(|i| o.parts[i.part].name == "Part 2"));
    }

    #[test]
    fn parts_go_to_more_plates_and_a_large_part_to_none() {
        let mut parts: Vec<Input> = (0..6)
            .map(|i| {
                input(
                    &format!("Part {i}"),
                    boxed(45.0, 45.0, 5.0, Vec3::new(i as f32 * 60.0, 0.0, 0.0)),
                )
            })
            .collect();
        parts.push(input(
            "Big",
            boxed(150.0, 20.0, 5.0, Vec3::new(0.0, 80.0, 0.0)),
        ));
        let o = go(&job(parts));
        assert_eq!(o.plates, 2);
        let big = o.parts.iter().position(|p| p.name == "Big").unwrap();
        assert!(!o.parts[big].fits);
        assert!(o.parts[big].proposal.is_some());
        assert!(o.instances.iter().all(|i| i.part != big));
        assert_eq!(o.left_out(), vec!["Big"]);
        assert!(o.left_out_note().unwrap().contains("Big"));
    }

    #[test]
    #[ignore = "needs openscad-nightly"]
    fn overlapping_parts_join_into_one() {
        let a = boxed(10.0, 10.0, 10.0, Vec3::ZERO);
        let b = boxed(10.0, 10.0, 10.0, Vec3::new(5.0, 0.0, 0.0));
        let mut j = job(vec![input("Part 1", a), input("Part 2", b)]);
        j.whole_volume = 1500.0;
        let o = go(&j);
        assert!(o.errors.is_empty(), "{:?}", o.errors);
        assert_eq!(o.parts.len(), 1);
        assert_eq!(o.parts[0].name, "Part 1 + Part 2");
        assert!((o.parts[0].mesh.volume() - 1500.0).abs() < 1.0);
    }
}

// ------------------------------------------------------------------ storage
//
// The Optimize values of a model go to `models.json` (see `app/state.rs`)
// under the key "optimize":
//
// { "area": [250, 220, 270], "gap": 10, "only": "Part 2", "plate": 0,
//   "fixes": { "teardrops": true, "bridges": false, "hole_above": "stepped",
//              "sacrificial_layers": 2, "chamfer": true, "chamfer_mm": 0.4,
//              "brim": false, "brim_w": 5, "brim_gap": 1,
//              "ears": false, "ear_d": 15, "ear_min": 100,
//              "hole_size": false, "hole_add": 0.2 },
//   "parts": { "Part 1": { "include": true, "copies": 2,
//                          "orient": "face", "down": [0, 1, 0], "turns": 1,
//                          "split": { "joint": "dovetail", "cuts": [[0, 150]],
//                                     "clearance": 0.2, ... } } } }

use serde_json::{Map, Value as Json, json};

fn f32_of(v: Option<&Json>) -> Option<f32> {
    v.and_then(Json::as_f64).map(|x| x as f32)
}

fn vec3_of(v: Option<&Json>) -> Option<Vec3> {
    let a = v?.as_array()?;
    if a.len() != 3 {
        return None;
    }
    Some(Vec3::new(
        f32_of(a.first())?,
        f32_of(a.get(1))?,
        f32_of(a.get(2))?,
    ))
}

/// `x` as a JSON number with at most 4 decimals, so that 0.4 stays 0.4 and
/// does not become 0.4000000059604645.
fn r4(x: f32) -> f64 {
    (x as f64 * 1e4).round() / 1e4
}

fn fixes_json(f: &fixes::Settings) -> Json {
    json!({
        "teardrops": f.teardrops,
        "bridges": f.bridges,
        "hole_above": f.hole_above.name(),
        "sacrificial_layers": f.sacrificial_layers,
        "chamfer": f.chamfer,
        "chamfer_mm": r4(f.chamfer_mm),
        "brim": f.brim,
        "brim_w": r4(f.brim_w),
        "brim_gap": r4(f.brim_gap),
        "ears": f.ears,
        "ear_d": r4(f.ear_d),
        "ear_min": r4(f.ear_min),
        "hole_size": f.hole_size,
        "hole_add": r4(f.hole_add),
    })
}

fn fixes_of(v: &Json) -> fixes::Settings {
    let mut f = fixes::Settings::default();
    let b = |k: &str, d: bool| v.get(k).and_then(Json::as_bool).unwrap_or(d);
    f.teardrops = b("teardrops", f.teardrops);
    f.bridges = b("bridges", f.bridges);
    f.chamfer = b("chamfer", f.chamfer);
    f.brim = b("brim", f.brim);
    f.ears = b("ears", f.ears);
    f.hole_size = b("hole_size", f.hole_size);
    if let Some(h) = v
        .get("hole_above")
        .and_then(Json::as_str)
        .and_then(fixes::HoleAbove::from_name)
    {
        f.hole_above = h;
    }
    if let Some(n) = v.get("sacrificial_layers").and_then(Json::as_u64) {
        f.sacrificial_layers = (n as u32).clamp(1, 10);
    }
    let clamp = |k: &str, d: f32, lo: f32, hi: f32| {
        f32_of(v.get(k))
            .filter(|x| x.is_finite())
            .map_or(d, |x| x.clamp(lo, hi))
    };
    f.chamfer_mm = clamp("chamfer_mm", f.chamfer_mm, 0.0, 2.0);
    f.brim_w = clamp("brim_w", f.brim_w, 0.0, 20.0);
    f.brim_gap = clamp("brim_gap", f.brim_gap, 0.2, 5.0);
    f.ear_d = clamp("ear_d", f.ear_d, 5.0, 30.0);
    f.ear_min = clamp("ear_min", f.ear_min, 20.0, 300.0);
    f.hole_add = clamp("hole_add", f.hole_add, 0.0, 1.0);
    f
}

fn split_json(s: &split::Split) -> Json {
    let v = &s.values;
    json!({
        "joint": s.joint.name(),
        "cuts": s.cuts.iter().map(|c| json!([c.axis, r4(c.at)])).collect::<Vec<_>>(),
        "clearance": r4(v.clearance),
        "pin_d": r4(v.pin_d),
        "pin_depth": r4(v.pin_depth),
        "tab_root": r4(v.tab_root),
        "tab_tip": r4(v.tab_tip),
        "tab_depth": r4(v.tab_depth),
        "tab_pitch": r4(v.tab_pitch),
    })
}

fn split_of(v: &Json) -> Option<split::Split> {
    let joint = split::Joint::from_name(v.get("joint")?.as_str()?)?;
    let mut cuts = Vec::new();
    for c in v.get("cuts")?.as_array()? {
        let a = c.as_array()?;
        let axis = a.first()?.as_u64()? as usize;
        let at = f32_of(a.get(1))?;
        if axis > 2 || !at.is_finite() {
            return None;
        }
        cuts.push(split::Cut { axis, at });
    }
    if cuts.is_empty() {
        return None;
    }
    let mut values = split::JointValues::default();
    let set = |k: &str, x: &mut f32, lo: f32, hi: f32| {
        if let Some(n) = f32_of(v.get(k)).filter(|n| n.is_finite()) {
            *x = n.clamp(lo, hi);
        }
    };
    set("clearance", &mut values.clearance, 0.0, 2.0);
    set("pin_d", &mut values.pin_d, 1.0, 30.0);
    set("pin_depth", &mut values.pin_depth, 1.0, 50.0);
    set("tab_root", &mut values.tab_root, 1.0, 100.0);
    set("tab_tip", &mut values.tab_tip, 1.0, 100.0);
    set("tab_depth", &mut values.tab_depth, 1.0, 50.0);
    set("tab_pitch", &mut values.tab_pitch, 5.0, 500.0);
    Some(split::Split {
        cuts,
        joint,
        values,
    })
}

impl PartState {
    pub fn to_json(&self) -> Json {
        let mut m = Map::new();
        m.insert("include".into(), json!(self.include));
        if let Some(c) = self.copies {
            m.insert("copies".into(), json!(c));
        }
        let o = &self.orientation;
        match o.choice {
            orient::Choice::Model => {
                m.insert("orient".into(), json!("model"));
            }
            orient::Choice::Auto => {
                m.insert("orient".into(), json!("auto"));
            }
            orient::Choice::Face(d) => {
                m.insert("orient".into(), json!("face"));
                m.insert("down".into(), json!([r4(d.x), r4(d.y), r4(d.z)]));
            }
        }
        if !o.quarter_turns.is_multiple_of(4) {
            m.insert("turns".into(), json!(o.quarter_turns % 4));
        }
        if let Some(s) = &self.split {
            m.insert("split".into(), split_json(s));
        }
        Json::Object(m)
    }

    pub fn from_json(v: &Json) -> PartState {
        let mut p = PartState::default();
        if let Some(b) = v.get("include").and_then(Json::as_bool) {
            p.include = b;
        }
        p.copies = v
            .get("copies")
            .and_then(Json::as_u64)
            .map(|c| c.min(999) as u32);
        p.orientation.choice = match v.get("orient").and_then(Json::as_str) {
            Some("model") => orient::Choice::Model,
            Some("face") => match vec3_of(v.get("down")) {
                Some(d) if d.length() > 0.5 => orient::Choice::Face(d.normalize()),
                _ => orient::Choice::Auto,
            },
            // Auto is the default. CoCAD 0.7 stored no key for the model
            // rotation: these parts also get Auto.
            _ => orient::Choice::Auto,
        };
        p.orientation.quarter_turns = v.get("turns").and_then(Json::as_u64).unwrap_or(0) as u8 % 4;
        p.split = v.get("split").and_then(split_of);
        p
    }
}

impl State {
    pub fn to_json(&self) -> Json {
        let mut m = Map::new();
        m.insert(
            "area".into(),
            json!([r4(self.area.w), r4(self.area.d), r4(self.area.h)]),
        );
        m.insert("gap".into(), json!(r4(self.gap)));
        if let Some(o) = &self.only {
            m.insert("only".into(), json!(o));
        }
        m.insert("plate".into(), json!(self.plate));
        m.insert("fixes".into(), fixes_json(&self.fixes));
        let parts: Map<String, Json> = self
            .parts
            .iter()
            .filter(|(_, p)| **p != PartState::default())
            .map(|(k, p)| (k.clone(), p.to_json()))
            .collect();
        if !parts.is_empty() {
            m.insert("parts".into(), Json::Object(parts));
        }
        Json::Object(m)
    }

    /// The stored state, or a new state with `area` for values that are
    /// missing.
    pub fn from_json(v: &Json, area: Area) -> State {
        let mut s = State::new(area);
        if let Some(a) = v.get("area").and_then(Json::as_array)
            && let [w, d, h] = a.as_slice()
            && let (Some(w), Some(d), Some(h)) = (f32_of(Some(w)), f32_of(Some(d)), f32_of(Some(h)))
            && w > 0.0
            && d > 0.0
            && h > 0.0
        {
            s.area = Area { w, d, h };
        }
        if let Some(g) = f32_of(v.get("gap")).filter(|g| g.is_finite()) {
            s.gap = g.clamp(0.0, 100.0);
        }
        s.only = v.get("only").and_then(Json::as_str).map(str::to_string);
        s.plate = v.get("plate").and_then(Json::as_u64).unwrap_or(0) as usize;
        if let Some(f) = v.get("fixes") {
            s.fixes = fixes_of(f);
        }
        if let Some(parts) = v.get("parts").and_then(Json::as_object) {
            for (k, p) in parts {
                s.parts.insert(k.clone(), PartState::from_json(p));
            }
        }
        s
    }
}

#[cfg(test)]
mod storage_tests {
    use super::*;

    #[test]
    fn state_goes_to_json_and_back() {
        let mut s = State::new(Area {
            w: 250.0,
            d: 220.0,
            h: 270.0,
        });
        s.fixes.chamfer = true;
        s.fixes.hole_above = fixes::HoleAbove::Stepped;
        s.only = Some("Part 2".into());
        s.plate = 1;
        s.parts.insert(
            "Part 1".into(),
            PartState {
                include: false,
                copies: Some(3),
                orientation: orient::Orientation {
                    choice: orient::Choice::Face(Vec3::Y),
                    quarter_turns: 1,
                },
                split: Some(split::Split {
                    cuts: vec![split::Cut { axis: 0, at: 150.0 }],
                    joint: split::Joint::Dovetail,
                    values: split::JointValues::default(),
                }),
            },
        );
        s.parts.insert(
            "Part 2".into(),
            PartState {
                orientation: orient::Orientation {
                    choice: orient::Choice::Model,
                    quarter_turns: 0,
                },
                ..PartState::default()
            },
        );
        let back = State::from_json(&s.to_json(), Area::default());
        assert_eq!(back, s);
    }

    #[test]
    fn a_part_with_no_orient_key_gets_auto() {
        // CoCAD 0.7 stored the model rotation with no "orient" key.
        let p = PartState::from_json(&json!({ "include": true }));
        assert_eq!(p.orientation.choice, orient::Choice::Auto);
        let p = PartState::from_json(&json!({ "orient": "model" }));
        assert_eq!(p.orientation.choice, orient::Choice::Model);
    }

    #[test]
    fn missing_values_take_the_defaults() {
        let a = Area {
            w: 180.0,
            d: 180.0,
            h: 180.0,
        };
        let s = State::from_json(&json!({ "fixes": { "brim": true }, "area": [0, 1, 2] }), a);
        assert_eq!(s.area, a);
        assert!(s.fixes.brim && !s.fixes.chamfer);
        assert!((s.fixes.brim_w - 5.0).abs() < 1e-6);
    }
}
