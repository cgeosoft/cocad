//! Tests of the Optimize features on the showcase model
//! `examples/desk-dock.scad`: each fix alone, the fixes together, the
//! orientation and the split. They need OpenSCAD, so they are ignored by
//! default:
//!
//! ```text
//! cargo test showcase -- --ignored --nocapture
//! ```
//!
//! With `COCAD_SHOWCASE_OUT=<dir>` the tests also write the part meshes
//! before and after each change as STL files, for pictures.

use super::fixes::{self, HoleAbove, Settings};
use super::orient::{self, Choice, Orientation};
use super::{Area, Layers, Limits, check, split, transformed};
use crate::export::stl_bytes;
use crate::mesh::Mesh;
use crate::openscad::{self, Config};
use crate::slicer;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

fn cancel() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

/// Builds one module call of the dock, as the module split does.
fn part(call: &str) -> Mesh {
    // Tests run at the same time and can build the same call. Each build
    // gets its own file, so that no test removes the file of another.
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let model = std::fs::canonicalize("examples/desk-dock.scad").unwrap();
    let path = std::env::temp_dir().join(format!(
        "cocad-showcase-{}-{}-{}.scad",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed),
        call.replace(['(', ')', '.'], "_")
    ));
    std::fs::write(
        &path,
        // $fn is a special variable: a `use` call takes it from this file.
        format!(
            "use <{}>\n$fn = 48;\n{call};\n",
            model.to_string_lossy().replace('\\', "/")
        ),
    )
    .unwrap();
    let out = openscad::render(&Config::default(), &path, &[], &cancel());
    let _ = std::fs::remove_file(&path);
    out.unwrap().mesh
}

fn out_dir() -> Option<PathBuf> {
    let d = PathBuf::from(std::env::var_os("COCAD_SHOWCASE_OUT")?);
    std::fs::create_dir_all(&d).ok()?;
    Some(d)
}

fn save(name: &str, m: &Mesh) {
    if let Some(d) = out_dir() {
        std::fs::write(d.join(format!("{name}.stl")), stl_bytes(m)).unwrap();
    }
}

const AREA: Area = Area {
    w: 220.0,
    d: 220.0,
    h: 250.0,
};

fn print_frame(m: &Mesh, choice: Choice) -> Mesh {
    let o = Orientation {
        choice,
        quarter_turns: 0,
    };
    transformed(m, orient::matrix(m, &o, &AREA, &Limits::default()))
}

/// The problems of the check, as "kind count" text.
fn issues(m: &Mesh) -> (usize, String) {
    let list = check::run(m, Layers::default(), &Limits::default(), &AREA);
    let mut kinds: Vec<(check::Kind, usize)> = Vec::new();
    for i in &list {
        match kinds.iter_mut().find(|(k, _)| *k == i.kind) {
            Some((_, n)) => *n += 1,
            None => kinds.push((i.kind, 1)),
        }
    }
    let text = kinds
        .iter()
        .map(|(k, n)| format!("{k:?} {n}"))
        .collect::<Vec<_>>()
        .join(", ");
    (
        list.len(),
        if text.is_empty() { "none".into() } else { text },
    )
}

/// The equivalent diameters (mm) of the holes in the section of `m` at
/// height `z`, smallest first. The largest loop is the outline.
fn hole_diameters(m: &Mesh, z: f32) -> Vec<f32> {
    let tris: Vec<u32> = (0..m.triangles.len() as u32).collect();
    let mut a: Vec<f32> = slicer::cut(m, &tris, z)
        .iter()
        .map(|l| slicer::area(l).abs())
        .collect();
    a.sort_by(|x, y| x.total_cmp(y));
    a.pop();
    a.iter()
        .map(|a| 2.0 * (a / std::f32::consts::PI).sqrt())
        .collect()
}

/// The area (mm²) of the section of `m` at height `z`, without the holes.
fn section_area(m: &Mesh, z: f32) -> f32 {
    let tris: Vec<u32> = (0..m.triangles.len() as u32).collect();
    let mut a: Vec<f32> = slicer::cut(m, &tris, z)
        .iter()
        .map(|l| slicer::area(l).abs())
        .collect();
    a.sort_by(|x, y| y.total_cmp(x));
    // One island: the outline less the holes.
    a.first().copied().unwrap_or(0.0) - a.iter().skip(1).sum::<f32>()
}

struct Combo {
    name: &'static str,
    s: Settings,
}

fn combos() -> Vec<Combo> {
    let off = Settings::default();
    let all = Settings {
        teardrops: true,
        bridges: true,
        chamfer: true,
        brim: true,
        ears: true,
        hole_size: true,
        ..off
    };
    vec![
        Combo {
            name: "teardrops",
            s: Settings {
                teardrops: true,
                ..off
            },
        },
        Combo {
            name: "bridges-sacrificial",
            s: Settings {
                bridges: true,
                ..off
            },
        },
        Combo {
            name: "bridges-stepped",
            s: Settings {
                bridges: true,
                hole_above: HoleAbove::Stepped,
                ..off
            },
        },
        Combo {
            name: "chamfer",
            s: Settings {
                chamfer: true,
                ..off
            },
        },
        Combo {
            name: "brim",
            s: Settings { brim: true, ..off },
        },
        Combo {
            name: "holes",
            s: Settings {
                hole_size: true,
                ..off
            },
        },
        Combo {
            name: "ears",
            s: Settings { ears: true, ..off },
        },
        Combo {
            name: "teardrops+chamfer",
            s: Settings {
                teardrops: true,
                chamfer: true,
                ..off
            },
        },
        Combo {
            name: "bridges+chamfer",
            s: Settings {
                bridges: true,
                chamfer: true,
                ..off
            },
        },
        Combo {
            name: "brim+chamfer",
            s: Settings {
                brim: true,
                chamfer: true,
                ..off
            },
        },
        Combo {
            name: "holes+chamfer",
            s: Settings {
                hole_size: true,
                chamfer: true,
                ..off
            },
        },
        Combo {
            name: "ears+chamfer",
            s: Settings {
                ears: true,
                chamfer: true,
                ..off
            },
        },
        Combo {
            name: "ears+brim",
            s: Settings {
                ears: true,
                brim: true,
                ..off
            },
        },
        Combo {
            name: "holes+bridges",
            s: Settings {
                hole_size: true,
                bridges: true,
                ..off
            },
        },
        Combo {
            name: "all-sacrificial",
            s: all,
        },
        Combo {
            name: "all-stepped",
            s: Settings {
                hole_above: HoleAbove::Stepped,
                ..all
            },
        },
    ]
}

/// The result of one combination: the name, the report, the volume, the
/// problems of the check and the fixed mesh.
type Run = (&'static str, fixes::Report, f32, String, Mesh);

/// Runs each combination on `m` (print frame) and returns the reports by
/// combination name. Each result must be a closed mesh.
fn run_combos(tag: &str, m: &Mesh) -> Vec<Run> {
    let (n, text) = issues(m);
    println!(
        "{tag:>10} {:<22} vol {:>10.1}  check {n}: {text}",
        "before",
        m.volume()
    );
    save(&format!("{tag}-before"), m);
    let mut out = Vec::new();
    for c in combos() {
        let t = std::time::Instant::now();
        let (fixed, r) = fixes::apply(
            &Config::default(),
            m,
            &c.s,
            Layers::default(),
            &Limits::default(),
            &AREA,
            &cancel(),
        )
        .unwrap_or_else(|e| panic!("{tag} {}: {e:#}", c.name));
        assert_eq!(fixed.open_edges(), 0, "{tag} {}: open mesh", c.name);
        let (n, text) = issues(&fixed);
        println!(
            "{tag:>10} {:<22} vol {:>10.1}  teardrops {} bridges {} brims {} ears {} holes {} chamfer {}  check {n}: {text}  skipped {}  {} ms",
            c.name,
            fixed.volume(),
            r.teardrops,
            r.bridges,
            r.brims,
            r.ears,
            r.holes,
            r.chamfer,
            r.skipped.len(),
            t.elapsed().as_millis()
        );
        for s in &r.skipped {
            println!("{:>12} skipped: {s}", "");
        }
        save(&format!("{tag}-{}", c.name), &fixed);
        out.push((c.name, r, fixed.volume(), text, fixed));
    }
    out
}

fn get<'a>(v: &'a [Run], name: &str) -> &'a Run {
    v.iter().find(|r| r.0 == name).unwrap()
}

#[test]
#[ignore = "needs openscad-nightly"]
fn showcase_plate_bridges_and_chamfer() {
    let m = print_frame(&part("plate()"), Choice::Model);
    let r = run_combos("plate", &m);
    // Two screw counterbores (square layer + hole above) and four magnet
    // pockets (square layer).
    assert!(get(&r, "bridges-sacrificial").1.bridges >= 6, "bridges");
    assert!(get(&r, "bridges-stepped").1.bridges >= 6, "bridges");
    assert!(get(&r, "chamfer").1.chamfer);
    assert_eq!(get(&r, "teardrops").1.teardrops, 0, "no horizontal hole");
    assert_eq!(get(&r, "brim").1.brims, 0, "no small island");
    let all = get(&r, "all-sacrificial");
    assert!(all.1.bridges >= 6 && all.1.chamfer);
    assert_eq!(all.1.holes, 8);
    // Hole size: the two screw holes, the two counterbores and the four
    // magnet pockets get 0.2 mm more diameter.
    let holes = get(&r, "holes");
    assert_eq!(holes.1.holes, 8, "holes");
    for z in [1.0, 2.8] {
        let (b, a) = (hole_diameters(&m, z), hole_diameters(&holes.4, z));
        assert_eq!(a.len(), b.len(), "z {z}: {b:?} {a:?}");
        for (b, a) in b.iter().zip(&a) {
            assert!((0.15..0.3).contains(&(a - b)), "z {z}: {b} to {a}");
        }
    }
    // The blind ends stay: above the magnet pockets (3 mm) and the
    // counterbores (2.6 mm), only the two screw holes are there.
    let above = hole_diameters(&holes.4, 3.05);
    assert_eq!(above.len(), 2, "{above:?}");
    assert!(above.iter().all(|d| *d < 5.0), "{above:?}");
    // The corners of the plate are round: no mouse ears.
    assert_eq!(get(&r, "ears").1.ears, 0, "round corners");
}

#[test]
#[ignore = "needs openscad-nightly"]
fn showcase_stand_teardrops() {
    // Auto lays the stand on its side: the profile is on the bed.
    let m = print_frame(&part("stand()"), Choice::Auto);
    let size = m.bounds().unwrap().size();
    assert!(size.z < size.x, "the stand lies on its side: {size}");
    let r = run_combos("stand", &m);
    // The cable hole and the two pilot holes are horizontal.
    assert!(get(&r, "teardrops").1.teardrops >= 3, "teardrops");
    assert!(get(&r, "chamfer").1.chamfer);
    let all = get(&r, "all-sacrificial");
    assert!(all.1.teardrops >= 3 && all.1.chamfer);
    assert_eq!(all.3, "none", "no problems with all fixes");
    // On its side the stand has no vertical hole.
    assert_eq!(all.1.holes, 0, "hole size");
    // Mouse ears: the outline on the bed is 115 mm long and has 6 sharp
    // corners. The ears are one layer: the first layer gets larger, the
    // second layer stays.
    let ears = get(&r, "ears");
    assert_eq!(ears.1.ears, 6, "ears");
    assert_eq!(all.1.ears, 6, "ears with all fixes");
    let added = section_area(&ears.4, 0.1) - section_area(&m, 0.1);
    assert!(added > 6.0 * 50.0, "first layer: {added} mm² more");
    let second = section_area(&ears.4, 0.3) - section_area(&m, 0.3);
    assert!(second.abs() < 0.5, "second layer: {second} mm² more");
}

#[test]
#[ignore = "needs openscad-nightly"]
fn showcase_stand_upright_brim() {
    // With custom orientation off, the stand stands on its 2.4 mm front
    // foot. The foot gets a brim.
    let m = print_frame(&part("stand()"), Choice::Model);
    let r = run_combos("stand-upright", &m);
    assert_eq!(get(&r, "brim").1.brims, 1, "brim");
    assert!(
        get(&r, "bridges-sacrificial").1.bridges >= 2,
        "pilot hole cones"
    );
    // Upright, the two blind pilot holes go up.
    assert_eq!(get(&r, "holes").1.holes, 2, "pilot holes");
    // The base is 70 mm wide and 60 mm deep: no mouse ears.
    assert_eq!(get(&r, "ears").1.ears, 0, "ears");
    let all = get(&r, "all-sacrificial");
    assert_eq!((all.1.brims, all.1.holes), (1, 2), "brim and holes");
}

#[test]
#[ignore = "needs openscad-nightly"]
fn showcase_clip_orientation() {
    let raw = part("clip_at(0.3)");
    let model = print_frame(&raw, Choice::Model);
    let auto = print_frame(&raw, Choice::Auto);
    save("clip-model", &model);
    save("clip-auto", &auto);
    let (nm, tm) = issues(&model);
    let (na, ta) = issues(&auto);
    let hm = model.bounds().unwrap().size().z;
    let ha = auto.bounds().unwrap().size().z;
    println!("clip model rotation: height {hm:.1}, check {nm}: {tm}");
    println!("clip auto:           height {ha:.1}, check {na}: {ta}");
    // Auto puts the clip on a face: lower and fewer problems.
    assert!(ha < hm, "auto is lower");
    assert!(na <= nm, "auto has no more problems");
    let r = run_combos("clip-auto", &auto);
    let all = get(&r, "all-sacrificial");
    assert!(all.1.chamfer);
    // Flat on the bed, the cable hole of the clip goes up.
    assert_eq!(get(&r, "holes").1.holes, 1, "cable hole");
    assert_eq!(all.1.holes, 1, "cable hole with all fixes");
}

#[test]
#[ignore = "needs openscad-nightly"]
fn showcase_plate_split() {
    let raw = part("plate()");
    for joint in split::Joint::ALL {
        let sp = split::propose(&raw, &AREA, joint).expect("the plate needs a split");
        let pieces = split::apply(&Config::default(), &raw, &sp, &cancel()).unwrap();
        let cuts: Vec<String> = sp.cuts.iter().map(|c| format!("{:.1}", c.at)).collect();
        let vol: f32 = pieces.iter().map(|p| p.mesh.volume()).sum();
        println!(
            "split {:<9} cuts at x {}: {} pieces, volume {:.1} of {:.1}",
            joint.name(),
            cuts.join(", "),
            pieces.len(),
            vol,
            raw.volume()
        );
        let main: Vec<&split::Piece> = pieces.iter().filter(|p| !p.pin).collect();
        assert_eq!(main.len(), 2, "two pieces");
        for (i, p) in pieces.iter().enumerate() {
            assert_eq!(p.mesh.open_edges(), 0, "piece {i} closed");
            let pm = print_frame(&p.mesh, Choice::Model);
            assert!(orient::fits(&pm, &AREA), "piece {} fits", p.label);
            save(&format!("split-{}-{}", joint.name(), i + 1), &pm);
        }
        // All fixes on the pieces.
        let all = Settings {
            teardrops: true,
            bridges: true,
            chamfer: true,
            brim: true,
            ears: true,
            hole_size: true,
            ..Settings::default()
        };
        for (i, p) in main.iter().enumerate() {
            let pm = print_frame(&p.mesh, Choice::Model);
            let (fixed, r) = fixes::apply(
                &Config::default(),
                &pm,
                &all,
                Layers::default(),
                &Limits::default(),
                &AREA,
                &cancel(),
            )
            .unwrap();
            assert_eq!(fixed.open_edges(), 0);
            // Each piece is 130 mm long, with two sharp corners at the
            // cut: they get mouse ears.
            assert!(r.ears >= 2, "piece {} ears {}", p.label, r.ears);
            let (n, text) = issues(&fixed);
            println!(
                "  piece {} all fixes: bridges {} ears {} holes {} chamfer {}  check {n}: {text}",
                p.label, r.bridges, r.ears, r.holes, r.chamfer
            );
            save(&format!("split-{}-{}-fixed", joint.name(), i + 1), &fixed);
        }
    }
}

#[test]
#[ignore = "needs openscad-nightly"]
fn showcase_pins_without_room_are_reported() {
    use super::pipeline::{self, Input, Job, PartState, State};
    let raw = std::sync::Arc::new(part("plate()"));
    let mut state = State::new(AREA);
    let sp = split::propose(&raw, &AREA, split::Joint::Pins).unwrap();
    state.parts.insert(
        "plate".into(),
        PartState {
            split: Some(sp),
            ..PartState::default()
        },
    );
    let job = Job {
        whole_volume: raw.volume(),
        parts: vec![Input {
            name: "plate".into(),
            mesh: raw,
        }],
        multicolor: false,
        model_name: "desk-dock".into(),
        state,
        layers: Layers::default(),
        limits: Limits::default(),
    };
    let o = pipeline::run(
        &Config::default(),
        &job,
        &std::sync::Mutex::new(pipeline::Cache::default()),
        &cancel(),
    )
    .unwrap();
    println!("{:?}", o.errors);
    assert!(
        o.errors.iter().any(|e| e.contains("no room for pins")),
        "{:?}",
        o.errors
    );
}

#[test]
#[ignore = "needs openscad-nightly"]
fn showcase_split_pieces_lie_as_the_whole_part() {
    use super::pipeline::{self, Input, Job, PartState, State};
    let raw = std::sync::Arc::new(part("plate()"));
    // Auto puts the whole plate on its bottom face: the wordmark is on top.
    let down = orient::auto_down(&raw, &Limits::default());
    let mut state = State::new(AREA);
    state.fixes.bridges = true;
    let sp = split::propose(&raw, &AREA, split::Joint::Dovetail).unwrap();
    state.parts.insert(
        "plate".into(),
        PartState {
            split: Some(sp),
            ..PartState::default()
        },
    );
    let job = Job {
        whole_volume: raw.volume(),
        parts: vec![Input {
            name: "plate".into(),
            mesh: raw,
        }],
        multicolor: false,
        model_name: "desk-dock".into(),
        state,
        layers: Layers::default(),
        limits: Limits::default(),
    };
    let o = pipeline::run(
        &Config::default(),
        &job,
        &std::sync::Mutex::new(pipeline::Cache::default()),
        &cancel(),
    )
    .unwrap();
    let pieces: Vec<&pipeline::PrintPart> = o.parts.iter().filter(|p| p.piece.is_some()).collect();
    assert_eq!(pieces.len(), 2, "two pieces");
    let mut bridges = 0;
    for p in pieces {
        // Each piece on Auto has the bed face of the whole part.
        let d = p.matrix.transform_vector3(down).normalize();
        assert!(
            d.distance(glam::Vec3::NEG_Z) < 1e-3,
            "{}: down goes to {d}",
            p.name
        );
        // The stored choice of the piece stays Auto.
        assert_eq!(p.state.orientation.choice, Choice::Auto, "{}", p.name);
        println!("{}: bridges {}", p.name, p.report.bridges);
        bridges += p.report.bridges;
    }
    // The two counterbores and the four magnet pockets open to the bed.
    assert!(bridges >= 6, "bridges {bridges}");
}
