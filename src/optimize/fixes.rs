//! The print fixes of the Optimize tab: teardrop holes, bridges, bed
//! chamfer and brim. All fixes are off by default.
//!
//! Each fix module looks at the part in print frame and returns a `Plan`:
//! OpenSCAD statements to cut from the part, to add to it, or to add after
//! the bed chamfer. `apply` joins the plans into one `.scad` file and runs
//! OpenSCAD on it (see `scad.rs`).
//!
//! The order in the file:
//!
//! ```text
//! union() {
//!     bed_cut() union() { difference() { part; cuts } adds }
//!     late
//! }
//! ```
//!
//! The brim is a late statement, because the bed chamfer must not cut it.

use super::{Area, Layers, Limits, bridges, brim, scad, teardrop};
use crate::mesh::Mesh;
use crate::openscad::Config;
use anyhow::Result;
use std::fmt::Write;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// What to do with a hole that goes on above a bridged ceiling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoleAbove {
    /// Close the hole with whole layers. The user drills it after the print.
    Sacrificial,
    /// One layer with a slot as wide as the hole, one layer with a square,
    /// then the round hole. No drilling.
    Stepped,
}

impl HoleAbove {
    pub fn name(self) -> &'static str {
        match self {
            HoleAbove::Sacrificial => "sacrificial",
            HoleAbove::Stepped => "stepped",
        }
    }

    pub fn from_name(s: &str) -> Option<HoleAbove> {
        match s {
            "sacrificial" => Some(HoleAbove::Sacrificial),
            "stepped" => Some(HoleAbove::Stepped),
            _ => None,
        }
    }
}

/// The fix switches and values of one model.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub teardrops: bool,
    pub bridges: bool,
    pub hole_above: HoleAbove,
    /// Number of sacrificial layers over a hole above a bridge.
    pub sacrificial_layers: u32,
    pub chamfer: bool,
    /// Size of the bed chamfer in mm. It is cut in whole layers.
    pub chamfer_mm: f32,
    pub brim: bool,
    /// Width of the brim on a small bed island, in mm.
    pub brim_w: f32,
    /// Gap between the brim and the rest of the part, in mm.
    pub brim_gap: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            teardrops: false,
            bridges: false,
            hole_above: HoleAbove::Sacrificial,
            sacrificial_layers: 2,
            chamfer: false,
            chamfer_mm: 0.4,
            brim: false,
            brim_w: 5.0,
            brim_gap: 1.0,
        }
    }
}

impl Settings {
    pub fn any(&self) -> bool {
        self.teardrops || self.bridges || self.chamfer || self.brim
    }
}

/// The result of one fix module for one part.
#[derive(Clone, Debug, Default)]
pub struct Plan {
    /// OpenSCAD statements to cut from the part. They come before the bed
    /// chamfer.
    pub cuts: Vec<String>,
    /// OpenSCAD statements to add to the part. They come before the bed
    /// chamfer.
    pub adds: Vec<String>,
    /// OpenSCAD statements to add after the bed chamfer, for example a brim.
    pub late: Vec<String>,
    /// The number of places that the fix changes.
    pub fixed: usize,
    /// Places that the fix found but did not change, with the reason.
    pub skipped: Vec<String>,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.cuts.is_empty() && self.adds.is_empty() && self.late.is_empty()
    }
}

/// What `apply` did to one part, for the Optimize tab.
#[derive(Clone, Debug, Default)]
pub struct Report {
    pub teardrops: usize,
    pub bridges: usize,
    pub brims: usize,
    pub chamfer: bool,
    /// Places that a fix found but did not change.
    pub skipped: Vec<String>,
}

/// The OpenSCAD module `bed_cut()` for the chamfer `c` in whole layers.
/// It cuts the bed edges of its children at 45 degrees, one layer per
/// step. Each step follows the section just above the chamfer, so an edge
/// that already has a chamfer or a foot gets no second one. Holes on the
/// bed get the same chamfer. Returns `None` when `c` is less than one layer.
pub fn bed_cut_module(c: f32, layers: Layers) -> Option<String> {
    let n = layers.count_to(c);
    if n == 0 {
        return None;
    }
    let top = layers.top(n - 1);
    let zref = top + layers.height * 0.5;
    // bed_keep(s) is the section above the chamfer, made smaller by s. A
    // wall thinner than 2 s would go away, so the parts of the section that
    // an opening (smaller by s, then larger by s) loses stay at full size:
    // a thin wall keeps its first layers and gets no chamfer.
    // bed_low(z, d) keeps the material of the layer at z that is outside the
    // section above the chamfer (a flap, a sheet or a foot), and the edge
    // near it: these edges get no chamfer.
    let mut s = format!(
        "module bed_ref() projection(cut = true) translate([0, 0, -{zref}]) children();\n\
         module bed_keep(d) {{\n    \
             offset(delta = -d) bed_ref() children();\n    \
             offset(delta = 0.01) offset(delta = -0.01) difference() {{\n        \
                 bed_ref() children();\n        \
                 offset(delta = d) offset(delta = -d) bed_ref() children();\n    \
             }}\n\
         }}\n\
         module bed_layer(z) projection(cut = true) translate([0, 0, -z]) children();\n\
         module bed_low(z, d) intersection() {{\n    \
             bed_layer(z) children();\n    \
             offset(delta = d) difference() {{\n        \
                 bed_layer(z) children();\n        \
                 offset(delta = 0.01) bed_ref() children();\n    \
             }}\n\
         }}\n\
         module bed_cut() {{\n    difference() {{\n        children();\n",
        zref = scad::num(zref)
    );
    for i in 0..n {
        let lo = if i == 0 { 0.0 } else { layers.top(i - 1) };
        let hi = layers.top(i);
        let shrink = top - lo;
        let (z0, h) = if i == 0 { (-1.0, hi + 1.0) } else { (lo, hi - lo) };
        let _ = writeln!(
            s,
            "        difference() {{\n            translate([-10000, -10000, {}]) cube([20000, 20000, {}]);\n            translate([0, 0, {}]) linear_extrude({}) union() {{\n                bed_keep({}) children();\n                bed_low({}, {}) children();\n            }}\n        }}",
            scad::num(z0),
            scad::num(h),
            scad::num(lo - 1.0),
            scad::num(hi - lo + 2.0),
            scad::num(shrink),
            scad::num((lo + hi) * 0.5),
            scad::num(shrink + 0.01),
        );
    }
    s.push_str("    }\n}\n");
    Some(s)
}

/// The `.scad` text for one part. `import` is the statement that imports
/// the part mesh in print frame.
pub fn compose(import: &str, plans: &[Plan], chamfer: Option<String>) -> String {
    let mut s = String::new();
    if let Some(m) = &chamfer {
        s.push_str(m);
    }
    let _ = writeln!(s, "module part() {import}");
    let cuts: Vec<&String> = plans.iter().flat_map(|p| &p.cuts).collect();
    let adds: Vec<&String> = plans.iter().flat_map(|p| &p.adds).collect();
    let late: Vec<&String> = plans.iter().flat_map(|p| &p.late).collect();
    let mut body = String::from("part();");
    if !cuts.is_empty() {
        body = format!("difference() {{\n    {body}\n");
        for c in cuts {
            let _ = writeln!(body, "    {c}");
        }
        body.push('}');
    }
    if !adds.is_empty() {
        body = format!("union() {{\n    {body}\n");
        for a in adds {
            let _ = writeln!(body, "    {a}");
        }
        body.push('}');
    }
    if chamfer.is_some() {
        body = format!("bed_cut() {body}");
    }
    let _ = writeln!(s, "union() {{\n{body}");
    for l in late {
        let _ = writeln!(s, "{l}");
    }
    s.push_str("}\n");
    s
}

/// Applies the fixes that are on to `mesh`, a part in print frame. Without
/// a change, it returns a copy of `mesh` and runs no OpenSCAD.
pub fn apply(
    cfg: &Config,
    mesh: &Mesh,
    s: &Settings,
    layers: Layers,
    limits: &Limits,
    area: &Area,
    cancel: &Arc<AtomicBool>,
) -> Result<(Mesh, Report)> {
    let mut report = Report::default();
    let mut plans = Vec::new();
    if s.teardrops {
        let p = teardrop::plan(mesh, layers, limits);
        report.teardrops = p.fixed;
        report.skipped.extend(p.skipped.iter().cloned());
        plans.push(p);
    }
    if s.bridges {
        let p = bridges::plan(mesh, s, layers, limits);
        report.bridges = p.fixed;
        report.skipped.extend(p.skipped.iter().cloned());
        plans.push(p);
    }
    let height = mesh.bounds().map_or(0.0, |b| b.max.z - b.min.z);
    // A sheet of one or two layers above the chamfer gets no chamfer.
    let chamfer = if s.chamfer && height > s.chamfer_mm + 2.0 * layers.height {
        bed_cut_module(s.chamfer_mm, layers)
    } else {
        None
    };
    report.chamfer = chamfer.is_some();
    if s.brim {
        let p = brim::plan(mesh, s, layers, area);
        report.brims = p.fixed;
        report.skipped.extend(p.skipped.iter().cloned());
        plans.push(p);
    }
    if chamfer.is_none() && plans.iter().all(Plan::is_empty) {
        return Ok((mesh.clone(), report));
    }
    let out = scad::run(
        cfg,
        &[mesh],
        |imports| compose(&imports[0], &plans, chamfer),
        cancel,
    )?;
    Ok((out, report))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bed_cut_has_one_step_per_layer() {
        let l = Layers {
            first: 0.2,
            height: 0.2,
        };
        let m = bed_cut_module(0.4, l).unwrap();
        assert_eq!(m.matches("bed_keep(").count(), 3, "{m}");
        assert!(m.contains("bed_keep(0.4)"), "{m}");
        assert!(m.contains("bed_keep(0.2)"), "{m}");
        assert!(bed_cut_module(0.05, l).is_none());
    }

    #[test]
    #[ignore = "needs openscad-nightly"]
    fn thin_wall_keeps_its_first_layer() {
        use std::sync::atomic::AtomicBool;
        // A block and, apart from it, a wall of 0.8 mm (thinner than two
        // times the chamfer).
        let path = std::env::temp_dir().join(format!("cocad-fix-thin-{}.scad", std::process::id()));
        std::fs::write(
            &path,
            "cube([20, 20, 10]); translate([30, 0, 0]) cube([0.8, 20, 5]);",
        )
        .unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let cfg = Config::default();
        let mesh = crate::openscad::render(&cfg, &path, &[], &cancel).unwrap().mesh;
        let _ = std::fs::remove_file(&path);
        let s = Settings {
            chamfer: true,
            ..Settings::default()
        };
        let (out, report) = apply(
            &cfg,
            &mesh,
            &s,
            Layers::default(),
            &Limits::default(),
            &Area::default(),
            &cancel,
        )
        .unwrap();
        assert!(report.chamfer);
        let a: f32 = super::super::section(&out, 0.1)
            .iter()
            .map(|l| super::super::loop_area(l))
            .sum();
        // The block is 19.2 x 19.2 at z 0.1, the wall stays 0.8 x 20.
        let want = 19.2 * 19.2 + 0.8 * 20.0;
        assert!((a - want).abs() < 0.5, "area {a}, want {want}");
    }

    #[test]
    #[ignore = "needs openscad-nightly"]
    fn flap_on_the_bed_stays() {
        use std::sync::atomic::AtomicBool;
        // A board of 2.4 mm with a flap of 0.4 mm (two layers) on one side.
        let path = std::env::temp_dir().join(format!("cocad-fix-flap-{}.scad", std::process::id()));
        std::fs::write(&path, "cube([20, 20, 2.4]); cube([30, 20, 0.4]);").unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let cfg = Config::default();
        let mesh = crate::openscad::render(&cfg, &path, &[], &cancel).unwrap().mesh;
        let _ = std::fs::remove_file(&path);
        let s = Settings {
            chamfer: true,
            ..Settings::default()
        };
        let l = Layers::default();
        let (out, _) = apply(&cfg, &mesh, &s, l, &Limits::default(), &Area::default(), &cancel).unwrap();
        let a: f32 = super::super::section(&out, 0.1)
            .iter()
            .map(|lp| super::super::loop_area(lp))
            .sum();
        // The board edges away from the flap get the chamfer, the flap and
        // the edge at the flap stay.
        let want = 19.2 * 19.2 + 10.4 * 20.0;
        assert!((a - want).abs() < 1.0, "area {a}, want {want}");
        let b = out.bounds().unwrap();
        assert!((b.max.x - 30.0).abs() < 1e-3);
    }

    #[test]
    fn compose_puts_the_brim_after_the_chamfer() {
        let p = Plan {
            cuts: vec!["cube(1);".into()],
            late: vec!["cube(2);".into()],
            ..Plan::default()
        };
        let s = compose("import(\"a.stl\");", &[p], Some("module bed_cut() children();\n".into()));
        let cut = s.find("bed_cut() difference()").unwrap();
        let late = s.find("cube(2);").unwrap();
        assert!(cut < late, "{s}");
    }
}
