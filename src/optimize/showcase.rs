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
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

fn cancel() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

/// Builds one module call of the dock, as the module split does.
fn part(call: &str) -> Mesh {
    let model = std::fs::canonicalize("examples/desk-dock.scad").unwrap();
    let path = std::env::temp_dir().join(format!(
        "cocad-showcase-{}-{}.scad",
        std::process::id(),
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

/// Runs each combination on `m` (print frame) and returns the reports by
/// combination name. Each result must be a closed mesh.
fn run_combos(tag: &str, m: &Mesh) -> Vec<(&'static str, fixes::Report, f32, String)> {
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
            "{tag:>10} {:<22} vol {:>10.1}  teardrops {} bridges {} brims {} chamfer {}  check {n}: {text}  skipped {}  {} ms",
            c.name,
            fixed.volume(),
            r.teardrops,
            r.bridges,
            r.brims,
            r.chamfer,
            r.skipped.len(),
            t.elapsed().as_millis()
        );
        for s in &r.skipped {
            println!("{:>12} skipped: {s}", "");
        }
        save(&format!("{tag}-{}", c.name), &fixed);
        out.push((c.name, r, fixed.volume(), text));
    }
    out
}

fn get<'a>(
    v: &'a [(&'static str, fixes::Report, f32, String)],
    name: &str,
) -> &'a (&'static str, fixes::Report, f32, String) {
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
}

#[test]
#[ignore = "needs openscad-nightly"]
fn showcase_stand_teardrops_bridges_brim() {
    let m = print_frame(&part("stand()"), Choice::Model);
    let r = run_combos("stand", &m);
    // The cable arch and the cable hole.
    assert!(get(&r, "teardrops").1.teardrops >= 2, "teardrops");
    // The two blind pilot holes from below.
    assert!(get(&r, "bridges-sacrificial").1.bridges >= 2, "bridges");
    // The front foot.
    assert_eq!(get(&r, "brim").1.brims, 1, "brim");
    let all = get(&r, "all-sacrificial");
    assert!(all.1.teardrops >= 2 && all.1.bridges >= 2 && all.1.brims == 1 && all.1.chamfer);
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
            let (n, text) = issues(&fixed);
            println!(
                "  piece {} all fixes: bridges {} chamfer {}  check {n}: {text}",
                p.label, r.bridges, r.chamfer
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
