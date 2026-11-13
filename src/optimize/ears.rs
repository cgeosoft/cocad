//! Mouse ears (feature 11 of `docs/optimize-tab.md`).
//!
//! A large flat part can lift from the bed at its corners. A mouse ear is
//! a disk of one layer on a sharp bed corner. It gives the corner more
//! area on the bed. The user cuts the disks off after the print.
//!
//! The brim of `brim.rs` holds the small bed islands. The ears hold the
//! corners of the large islands. An island gets one of the two.

use super::brim::{Island, brim_out, islands};
use super::check::min_rect;
use super::fixes::{Plan, Settings};
use super::{Area, Layers, scad};
use crate::mesh::Mesh;
use glam::Vec2;
use std::fmt::Write;

/// A convex corner of an outline is sharp when the outline turns by more
/// than this (degrees). A point on an arc turns by less than 40 degrees
/// (`check.rs`), so a round corner gets no ear.
const SHARP_DEG: f32 = 45.0;
/// Outline points nearer than this (mm) are one point. A short edge of a
/// boolean result then does not hide a corner.
const SHORT: f32 = 0.05;
/// The number of points of an ear.
const FN: u32 = 48;

/// The sharp convex corners of the closed loop `outer` (counter
/// clockwise).
pub fn corners(outer: &[Vec2]) -> Vec<Vec2> {
    let mut pts: Vec<Vec2> = Vec::new();
    for &p in outer {
        if pts.last().is_none_or(|q| q.distance(p) >= SHORT) {
            pts.push(p);
        }
    }
    while pts.len() > 1 && pts[0].distance(pts[pts.len() - 1]) < SHORT {
        pts.pop();
    }
    let n = pts.len();
    if n < 3 {
        return Vec::new();
    }
    (0..n)
        .filter(|&i| {
            let u = pts[i] - pts[(i + n - 1) % n];
            let w = pts[(i + 1) % n] - pts[i];
            // A left turn of a counter clockwise loop is a convex corner.
            u.perp_dot(w).atan2(u.dot(w)).to_degrees() > SHARP_DEG
        })
        .map(|i| pts[i])
        .collect()
}

/// The centers of the ears for `corners`: corners nearer than `d` get one
/// ear at their middle point.
pub fn places(corners: &[Vec2], d: f32) -> Vec<Vec2> {
    // The sum of the corners of each ear and their number.
    let mut groups: Vec<(Vec2, f32)> = Vec::new();
    for &c in corners {
        match groups.iter_mut().find(|(s, n)| (*s / *n).distance(c) < d) {
            Some((s, n)) => {
                *s += c;
                *n += 1.0;
            }
            None => groups.push((c, 1.0)),
        }
    }
    groups.into_iter().map(|(s, n)| s / n).collect()
}

/// True when the island is large: the long side of its smallest rectangle
/// is `min` or more.
fn large(i: &Island, min: f32) -> bool {
    min_rect(&i.outer).is_some_and(|r| 2.0 * r.half.max_element() >= min)
}

/// Finds the sharp bed corners of the large islands of `mesh` (print
/// frame) and returns the ear statements (late statements, after the bed
/// chamfer).
pub fn plan(mesh: &Mesh, s: &Settings, layers: Layers, area: &Area) -> Plan {
    let mut plan = Plan::default();
    if s.ear_d <= 0.0 {
        return plan;
    }
    let Some(b) = mesh.bounds() else {
        return plan;
    };
    let all = islands(mesh, layers);
    let out = brim_out(b.size().truncate(), s.ear_d * 0.5, area);
    let lo = b.min.truncate() - out;
    let hi = b.max.truncate() + out;
    for (k, isl) in all.iter().enumerate() {
        if isl.small() || !large(isl, s.ear_min) {
            continue;
        }
        let at = places(&corners(&isl.outer), s.ear_d);
        if at.is_empty() {
            continue;
        }
        let others: Vec<&Island> = (all.iter().enumerate())
            .filter(|(j, _)| *j != k)
            .map(|(_, i)| i)
            .collect();
        plan.late.push(statement(
            isl,
            &at,
            &others,
            lo,
            hi,
            s,
            layers.first,
            b.min.z,
        ));
        plan.fixed += at.len();
    }
    plan
}

/// The OpenSCAD statement of the ears of one island: disks of one layer
/// from `z0` at the points `at`, in the box `lo`-`hi`. The disks do not go
/// into the holes of the island, and they keep `s.brim_gap` from the
/// `others` and from their brims.
#[allow(clippy::too_many_arguments)]
fn statement(
    isl: &Island,
    at: &[Vec2],
    others: &[&Island],
    lo: Vec2,
    hi: Vec2,
    s: &Settings,
    height: f32,
    z0: f32,
) -> String {
    let mut disks = String::new();
    for p in at {
        let _ = write!(
            disks,
            "translate({}) circle(d = {}, $fn = {FN}); ",
            scad::v2(*p),
            scad::num(s.ear_d)
        );
    }
    let mut cuts = String::new();
    if !isl.holes.is_empty() {
        let _ = write!(cuts, "{} ", scad::polygon(&isl.holes));
    }
    // A small island gets a brim when the brim fix is on: keep the gap to
    // the brim.
    let (small, rest): (Vec<&Island>, Vec<&Island>) = others.iter().partition(|i| i.small());
    let brim = if s.brim { s.brim_w.max(0.0) } else { 0.0 };
    for (group, r) in [(rest, s.brim_gap), (small, s.brim_gap + brim)] {
        if !group.is_empty() {
            let loops: Vec<Vec<Vec2>> = group.iter().map(|i| i.outer.clone()).collect();
            let _ = write!(
                cuts,
                "offset(r = {}, $fn = {FN}) {} ",
                scad::num(r),
                scad::polygon(&loops)
            );
        }
    }
    let mut body = format!(
        "intersection() {{ union() {{ {disks}}} translate({}) square({}); }}",
        scad::v2(lo),
        scad::v2(hi - lo),
    );
    if !cuts.is_empty() {
        body = format!("difference() {{ {body} {cuts}}}");
    }
    format!(
        "translate([0, 0, {}]) linear_extrude(height = {}) {body}",
        scad::num(z0),
        scad::num(height)
    )
}

#[cfg(test)]
mod tests {
    use super::super::check::test_util::*;
    use super::super::fixes::apply;
    use super::super::{Limits, loop_area, section};
    use super::*;
    use glam::Vec3;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    fn on() -> Settings {
        Settings {
            ears: true,
            ..Settings::default()
        }
    }

    fn plate(x: f32, y: f32) -> Mesh {
        mesh(&[block(Vec3::ZERO, Vec3::new(x, y, 3.0))])
    }

    #[test]
    fn a_square_has_four_sharp_corners() {
        let sq = [
            Vec2::new(0.0, 0.0),
            Vec2::new(50.0, 0.0),
            // A point on a straight edge and a short edge at a corner.
            Vec2::new(50.0, 20.0),
            Vec2::new(50.0, 49.99),
            Vec2::new(50.0, 50.0),
            Vec2::new(0.0, 50.0),
        ];
        assert_eq!(corners(&sq).len(), 4);
    }

    #[test]
    fn a_circle_and_an_inside_corner_get_no_ear() {
        assert!(corners(&ngon(Vec2::ZERO, 50.0, 48)).is_empty());
        // An L shape: five convex corners and one inside corner.
        let l = [
            Vec2::new(0.0, 0.0),
            Vec2::new(100.0, 0.0),
            Vec2::new(100.0, 40.0),
            Vec2::new(40.0, 40.0),
            Vec2::new(40.0, 100.0),
            Vec2::new(0.0, 100.0),
        ];
        let c = corners(&l);
        assert_eq!(c.len(), 5);
        assert!(!c.contains(&Vec2::new(40.0, 40.0)));
    }

    #[test]
    fn near_corners_get_one_ear() {
        // The end of a strip 10 mm wide: one ear at the middle of the end.
        let c = [
            Vec2::new(0.0, 0.0),
            Vec2::new(0.0, 10.0),
            Vec2::new(200.0, 0.0),
            Vec2::new(200.0, 10.0),
        ];
        let p = places(&c, 15.0);
        assert_eq!(p.len(), 2);
        assert!(p[0].distance(Vec2::new(0.0, 5.0)) < 1e-4);
    }

    #[test]
    fn a_large_plate_gets_four_ears_and_a_short_one_none() {
        let l = Layers::default();
        let p = plan(&plate(120.0, 60.0), &on(), l, &Area::default());
        assert_eq!(p.fixed, 4);
        assert_eq!(p.late.len(), 1);
        assert!(p.cuts.is_empty() && p.adds.is_empty());
        assert!(plan(&plate(60.0, 60.0), &on(), l, &Area::default()).is_empty());
        let off = Settings { ear_d: 0.0, ..on() };
        assert!(plan(&plate(120.0, 60.0), &off, l, &Area::default()).is_empty());
    }

    #[test]
    fn a_small_island_gets_no_ear() {
        // A wall 0.8 mm thin and 120 mm long is a small island: the brim
        // holds it.
        let m = mesh(&[block(Vec3::ZERO, Vec3::new(120.0, 0.8, 10.0))]);
        assert!(plan(&m, &on(), Layers::default(), &Area::default()).is_empty());
    }

    #[test]
    #[ignore = "needs openscad-nightly"]
    fn the_ears_are_one_layer_and_do_not_close_a_hole() {
        // A plate with a hole of 4 mm, 4 mm from a corner.
        let m = render(
            "difference() { cube([120, 60, 3]); translate([4, 4, -1]) cylinder(d = 4, h = 5, $fn = 24); }",
        );
        let layers = Layers::default();
        let (out, rep) = apply(
            &crate::openscad::Config::default(),
            &m,
            &on(),
            layers,
            &Limits::default(),
            &Area::default(),
            &Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        assert_eq!(rep.ears, 4);
        let b = out.bounds().unwrap();
        // The ears go 7.5 mm past the plate on each side.
        assert!(
            (b.min.x + 7.5).abs() < 0.05 && (b.max.x - 127.5).abs() < 0.05,
            "{b:?}"
        );
        assert!((b.max.z - 3.0).abs() < 1e-3);
        // Four ears of 3/4 of a disk in the first layer, none above it.
        let disk = std::f32::consts::PI * 7.5 * 7.5;
        let hole = std::f32::consts::PI * 2.0 * 2.0;
        let first = section_area(&out, 0.1);
        assert!(
            (first - (7200.0 - hole + 3.0 * disk)).abs() < 3.0,
            "{first}"
        );
        assert!((section_area(&out, 0.3) - (7200.0 - hole)).abs() < 1.0);
        // The hole stays open in the first layer.
        let holes = section(&out, 0.1)
            .iter()
            .filter(|l| loop_area(l) < 0.0)
            .count();
        assert_eq!(holes, 1);
    }
}
