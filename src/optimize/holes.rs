//! Hole size (feature 10 of `docs/optimize-tab.md`).
//!
//! A round hole with its axis perpendicular to the bed prints smaller than
//! its diameter in the model. OpenSCAD makes the hole as a prism with its
//! corners on the circle, so the flat faces are inside the circle. The
//! printer also makes holes smaller.
//!
//! This module finds these holes in a part in print frame and cuts each
//! one again: the flat faces go out to the circle of the model, plus half
//! of the value **Add to diameter**.

use super::fixes::{Plan, Settings};
use super::teardrop::{self, MAX_D, MIN_D, OVERLAP, Round, hits};
use super::{Limits, scad};
use crate::mesh::Mesh;
use glam::Vec2;

/// The faces of a hole go all around the circle when no two faces that
/// are next to each other are more than this apart (degrees). A hole has
/// five faces or more, so a full hole has at most 72 degrees.
const MAX_GAP_DEG: f32 = 80.0;
/// A hole with this number of faces can be a pocket for a nut.
const NUT_FACES: usize = 6;
/// A hole shorter than this (mm) is not changed.
const MIN_LEN: f32 = 0.1;
/// The end of a hole is closed when a face is this near (mm) behind it.
const END_TOL: f32 = 0.02;
/// A cut stops this far (mm) before a closed end. A cut that stops at the
/// end face is not exactly in its plane: the axis is a fit to the faces,
/// and the numbers in the `.scad` file have 5 decimals. OpenSCAD then
/// keeps a step of no height there, a band of triangles with no width.
/// The last 0.01 mm of the hole keeps its old size, much less than a
/// layer.
const END_GAP: f32 = 0.01;
/// The positions along the hole for the wall check, as parts of its length.
const RAY_AT: [f32; 3] = [0.21, 0.52, 0.79];
/// The largest number of directions for the wall check.
const RAY_DIRS: usize = 8;

/// Finds the vertical round holes of `mesh` (print frame) and returns the
/// cuts that make them larger.
pub fn plan(mesh: &Mesh, s: &Settings, limits: &Limits) -> Plan {
    let mut plan = Plan::default();
    for h in teardrop::upright_rounds(mesh) {
        let d = h.diameter();
        let keep = h.hole
            && (MIN_D * 0.99..=MAX_D * 1.01).contains(&d)
            && h.t1 - h.t0 >= MIN_LEN
            // The pin holes of a split: the clearance of the split sets
            // their fit.
            && !(s.hole_keep > 0.0 && (d - s.hole_keep).abs() < 0.02);
        if !keep {
            continue;
        }
        let Some(poly) = outline(&h, s.hole_add) else {
            continue;
        };
        if poly.len() == NUT_FACES && s.hole_add <= 0.0 {
            // Nothing to change.
            continue;
        }
        if thin(mesh, &h, &poly, limits.min_wall) {
            plan.skipped.push(format!(
                "Hole d {:.1} mm at x {:.1}, y {:.1}: wall too thin",
                d,
                h.at(h.t0).x,
                h.at(h.t0).y
            ));
            continue;
        }
        plan.cuts.push(cut(mesh, &h, &poly));
        plan.fixed += 1;
    }
    plan
}

/// The new outline of hole `h` around its center: each flat face is at the
/// radius of the hole plus `add / 2` from the axis. `None` when the faces
/// do not go all around the circle.
///
/// A hole with six faces can be a pocket for a nut, where the distance
/// between the flat faces is the size in the model. Its faces only move
/// out by `add / 2`.
pub fn outline(h: &Round, add: f32) -> Option<Vec<Vec2>> {
    let angles = h.face_angles();
    let n = angles.len();
    if n < 3 {
        return None;
    }
    let model = if n == NUT_FACES {
        h.r * (std::f32::consts::PI / n as f32).cos()
    } else {
        h.r
    };
    let flat = model + add.max(0.0) * 0.5;
    let mut poly = Vec::with_capacity(n);
    for k in 0..n {
        let gap = (angles[(k + 1) % n] - angles[k]).rem_euclid(360.0);
        if gap > MAX_GAP_DEG {
            return None;
        }
        // The corner between two faces is on the line in the middle of
        // their directions.
        let at = (angles[k] + gap * 0.5).to_radians();
        let r = flat / (gap * 0.5).to_radians().cos();
        poly.push(Vec2::new(at.cos(), at.sin()) * r);
    }
    Some(poly)
}

/// True when the end of hole `h` at `t` is closed: the floor of a blind
/// hole, or the ceiling of a counterbore with a smaller hole in it. `sign`
/// is the direction along the axis that goes out of the hole there.
///
/// The rays go along the axis near the wall of the hole, so that a smaller
/// hole in the middle of the end face does not make the end open. The end
/// is closed when one half of the rays or more find a face there.
fn closed(mesh: &Mesh, h: &Round, t: f32, sign: f32) -> bool {
    let back = MIN_LEN * 0.5;
    let found = (0..RAY_DIRS)
        .filter(|&k| {
            // Not in the direction of `side` or `up`: a ray there can go
            // along an edge of the end face.
            let a = (k as f32 + 0.37) * std::f32::consts::TAU / RAY_DIRS as f32;
            let out = (h.side * a.cos() + h.up * a.sin()) * (0.85 * h.r);
            hits(mesh, h.at(t - sign * back) + out, h.axis * sign)
                .first()
                .is_some_and(|&d| d <= back + END_TOL)
        })
        .count();
    2 * found >= RAY_DIRS
}

/// The OpenSCAD statement that cuts hole `h` to the outline `poly`. The
/// cut goes `OVERLAP` past an open end. It stops `END_GAP` before a closed
/// end, so that a blind hole does not get deeper.
fn cut(mesh: &Mesh, h: &Round, poly: &[Vec2]) -> String {
    let past = |t: f32, sign: f32| {
        if closed(mesh, h, t, sign) {
            -END_GAP
        } else {
            OVERLAP
        }
    };
    let t0 = h.t0 - past(h.t0, -1.0);
    let t1 = h.t1 + past(h.t1, 1.0);
    format!(
        "{} linear_extrude(height = {}) {}",
        scad::multmatrix(h.frame(t0)),
        scad::num(t1 - t0),
        scad::polygon(&[poly.to_vec()])
    )
}

/// True when the wall around the new outline `poly` of hole `h` is thinner
/// than `min_wall` at some place, or when the outline breaks through it.
///
/// The rays go from the axis out through the corners of the new outline.
/// The first face on a ray is the wall of the hole, and the second is the
/// other side of that wall.
fn thin(mesh: &Mesh, h: &Round, poly: &[Vec2], min_wall: f32) -> bool {
    let step = poly.len().div_ceil(RAY_DIRS).max(1);
    for f in RAY_AT {
        let on_axis = h.at(h.t0 + (h.t1 - h.t0) * f);
        for v in poly.iter().step_by(step) {
            let corner = v.length();
            let dir = (h.side * v.x + h.up * v.y) / corner;
            let hs = hits(mesh, on_axis, dir);
            let Some(&wall) = hs.first() else {
                continue;
            };
            if wall > corner + 1e-3 {
                // The hole is open to the side here: no wall to check.
                continue;
            }
            // No second face: the mesh is open there. Do not cut.
            if hs.get(1).is_none_or(|&end| end - corner < min_wall) {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::super::check::test_util::*;
    use super::super::fixes::apply;
    use super::super::{Area, Layers};
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    fn on() -> Settings {
        Settings {
            hole_size: true,
            ..Settings::default()
        }
    }

    fn run(m: &Mesh, s: &Settings) -> (Mesh, super::super::fixes::Report) {
        apply(
            &crate::openscad::Config::default(),
            m,
            s,
            Layers::default(),
            &Limits::default(),
            &Area::default(),
            &Arc::new(AtomicBool::new(false)),
        )
        .unwrap()
    }

    /// The distance between opposite flat faces of the hole at the center
    /// of the section of `m` at `z`: the smallest loop with negative area.
    fn across_flats(m: &Mesh, z: f32, c: Vec2) -> f32 {
        let loops = super::super::section(m, z);
        let hole = loops
            .iter()
            .filter(|l| super::super::loop_area(l) < 0.0)
            .min_by(|a, b| {
                let d = |l: &Vec<Vec2>| l[0].distance(c);
                d(a).total_cmp(&d(b))
            })
            .unwrap();
        let n = hole.len();
        let near = (0..n)
            .map(|i| super::super::check::seg_dist(c, hole[i], hole[(i + 1) % n]))
            .fold(f32::MAX, f32::min);
        2.0 * near
    }

    /// The area of a hole with `n` faces and `d` between its flat faces.
    fn section_hole(d: f32, n: u32) -> f32 {
        n as f32 * (d * 0.5).powi(2) * (std::f32::consts::PI / n as f32).tan()
    }

    #[test]
    #[ignore = "needs openscad-nightly"]
    fn a_vertical_hole_gets_its_diameter_plus_the_value() {
        // A hole of 4 mm with 16 faces: 3.92 mm between the flat faces.
        let m = render(
            "difference() { cube([20, 20, 6]); translate([10, 10, -1]) cylinder(d = 4, h = 8, $fn = 16); }",
        );
        let c = Vec2::new(10.0, 10.0);
        assert!((across_flats(&m, 3.0, c) - 3.923).abs() < 0.01);
        let p = plan(&m, &on(), &Limits::default());
        assert_eq!((p.fixed, p.cuts.len()), (1, 1), "{:?}", p.skipped);
        let (out, rep) = run(&m, &on());
        assert_eq!(rep.holes, 1);
        assert!((across_flats(&out, 3.0, c) - 4.2).abs() < 0.01);
        // With 0, the flat faces go to the circle of the model.
        let zero = Settings {
            hole_add: 0.0,
            ..on()
        };
        let (out, _) = run(&m, &zero);
        assert!((across_flats(&out, 3.0, c) - 4.0).abs() < 0.01);
        // The part keeps its size.
        assert!((out.bounds().unwrap().size() - m.bounds().unwrap().size()).length() < 1e-3);
    }

    #[test]
    #[ignore = "needs openscad-nightly"]
    fn a_hole_with_six_faces_only_gets_the_value() {
        // A nut pocket: 5.5 mm between the flat faces.
        let m = render(
            "difference() { cube([20, 20, 6]); translate([10, 10, -1]) cylinder(d = 5.5 / cos(30), h = 8, $fn = 6); }",
        );
        let c = Vec2::new(10.0, 10.0);
        let (out, rep) = run(&m, &on());
        assert_eq!(rep.holes, 1);
        assert!((across_flats(&out, 3.0, c) - 5.7).abs() < 0.01);
        let zero = Settings {
            hole_add: 0.0,
            ..on()
        };
        assert!(plan(&m, &zero, &Limits::default()).is_empty());
    }

    #[test]
    #[ignore = "needs openscad-nightly"]
    fn a_wall_check_ray_in_a_face_plane_is_no_thin_wall() {
        // A hub on a disk. The middle of the hole is at the height of the
        // top face of the disk.
        let m = render(
            "$fn = 64; difference() { union() { cylinder(d = 50, h = 6); cylinder(d = 13, h = 12); } translate([0, 0, -1]) cylinder(d = 5, h = 14); }",
        );
        let p = plan(&m, &on(), &Limits::default());
        assert_eq!((p.fixed, p.skipped.len()), (1, 0), "{:?}", p.skipped);
    }

    #[test]
    #[ignore = "needs openscad-nightly"]
    fn a_blind_hole_does_not_get_deeper() {
        let m = render(
            "difference() { cube([20, 20, 6]); translate([10, 10, 2]) cylinder(d = 5, h = 8, $fn = 24); }",
        );
        let (out, rep) = run(&m, &on());
        assert_eq!(rep.holes, 1);
        let c = Vec2::new(10.0, 10.0);
        assert!((across_flats(&out, 4.0, c) - 5.2).abs() < 0.01);
        // The floor of the hole stays at z 2: below it the part is full.
        assert!((section_area(&out, 1.9) - 400.0).abs() < 0.01);
        assert!(section_area(&out, 2.1) < 400.0 - 15.0);
    }

    #[test]
    #[ignore = "needs openscad-nightly"]
    fn horizontal_holes_pins_and_thin_walls_stay() {
        // A horizontal hole and a pin: no change.
        let m = render(
            "$fn = 24; difference() { cube([20, 20, 10]); translate([-1, 10, 5]) rotate([0, 90, 0]) cylinder(d = 4, h = 22); } translate([10, 10, 10]) cylinder(d = 4, h = 5);",
        );
        let p = plan(&m, &on(), &Limits::default());
        assert!(p.is_empty() && p.skipped.is_empty(), "{p:?}");
        // A hole 0.5 mm from the side of the part: skipped.
        let m = render(
            "difference() { cube([20, 20, 6]); translate([2.5, 10, -1]) cylinder(d = 4, h = 8, $fn = 24); }",
        );
        let p = plan(&m, &on(), &Limits::default());
        assert!(p.is_empty(), "{p:?}");
        assert_eq!(p.skipped.len(), 1);
        assert!(p.skipped[0].contains("wall too thin"), "{:?}", p.skipped);
    }

    #[test]
    #[ignore = "needs openscad-nightly"]
    fn the_pin_holes_of_a_split_stay() {
        let m = render(
            "difference() { cube([20, 20, 6]); translate([10, 10, -1]) cylinder(d = 4.2, h = 8, $fn = 24); }",
        );
        let s = Settings {
            hole_keep: 4.2,
            ..on()
        };
        assert!(plan(&m, &s, &Limits::default()).is_empty());
        assert_eq!(plan(&m, &on(), &Limits::default()).fixed, 1);
    }

    #[test]
    #[ignore = "needs openscad-nightly"]
    fn the_bridge_fix_uses_the_new_hole() {
        // A counterbore on the bed with a screw hole above it.
        let m = render(
            "$fn = 24; difference() { cube([30, 30, 10]); translate([15, 15, -1]) cylinder(d = 10, h = 5); translate([15, 15, 0]) cylinder(d = 4, h = 11); }",
        );
        let s = Settings {
            bridges: true,
            ..on()
        };
        let (out, rep) = run(&m, &s);
        assert_eq!(rep.holes, 2);
        assert!(rep.bridges >= 1, "{rep:?}");
        // The counterbore has the new size. The square layer of the bridge
        // fix ends 0.02 mm above the ceiling at z 4, as without this fix.
        assert!((section_area(&out, 3.5) - (900.0 - section_hole(10.2, 24))).abs() < 0.5);
        assert!((section_area(&out, 4.03) - 900.0).abs() < 0.5);
        // The sacrificial layers close the hole: no ring stays open
        // around them.
        assert!((section_area(&out, 4.1) - 900.0).abs() < 0.5);
        // Above them the hole has the new size.
        assert!((across_flats(&out, 8.0, Vec2::new(15.0, 15.0)) - 4.2).abs() < 0.01);
    }
}
