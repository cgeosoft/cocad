//! Brim on small bed islands (feature 4.1 of `docs/optimize-tab.md`).
//!
//! A bed island is an area of the first layer that is not joined to the
//! rest of the part there. A small island gets a brim of one layer, joined
//! to the island, with a gap to the rest of the part.
//!
//! This is `bed_brim()`, `bed_area2d()` and `brim_out()` of
//! `open-case-frame.scad` in scad-models. One change: the brim goes only
//! around the outside of an island, not into its holes, so that no brim
//! stays in a tube.

use super::check::{bbox, inside_loop, seg_dist};
use super::fixes::{Plan, Settings};
use super::{Area, Layers, loop_area, scad, section};
use crate::mesh::Mesh;
use glam::Vec2;
use std::fmt::Write;

/// An island thinner than this (mm) is small (`overhang.py --thin`).
pub const THIN_MM: f32 = 3.0;
/// An island with less area than this (mm²) is small (`overhang.py --tiny`).
pub const TINY_MM2: f32 = 25.0;

/// The number of points of a full circle in the round corners of the
/// brim. The chords then stay within 0.5 % of the radius.
const FN: u32 = 32;

/// One area of the first layer: an outer loop with its holes.
#[derive(Clone, Debug)]
pub struct Island {
    /// The outline, counter clockwise.
    pub outer: Vec<Vec2>,
    /// The holes, clockwise.
    pub holes: Vec<Vec<Vec2>>,
    /// The area without the holes, in mm².
    pub area: f32,
    /// Two times the largest distance from a point in the island to its
    /// edge (mm). A value of `THIN_MM` or more only tells that the island
    /// is not thin.
    pub width: f32,
}

impl Island {
    /// True when the island needs a brim.
    pub fn small(&self) -> bool {
        self.small_after(0.0)
    }

    /// True when the island needs a brim after its edge moves in by
    /// `shrink` (the bed chamfer takes this much from the first layer).
    pub fn small_after(&self, shrink: f32) -> bool {
        let p: f32 = self.loops().iter().map(|l| perimeter(l)).sum();
        self.width - 2.0 * shrink < THIN_MM || self.area - p * shrink < TINY_MM2
    }

    /// The outer loop and the holes.
    pub fn loops(&self) -> Vec<Vec<Vec2>> {
        let mut l = vec![self.outer.clone()];
        l.extend(self.holes.iter().cloned());
        l
    }

    /// The center of the bounding box.
    pub fn center(&self) -> Vec2 {
        let (lo, hi) = bbox(&self.outer);
        (lo + hi) * 0.5
    }
}

/// The height of the section that finds the islands: the middle of the
/// first layer. The small extra keeps the cut off the vertices.
pub fn island_z(mesh: &Mesh, layers: Layers) -> f32 {
    let zmin = mesh.bounds().map_or(0.0, |b| b.min.z);
    zmin + layers.first * 0.5 + 1.234e-4
}

/// The bed islands of `mesh` (print frame).
pub fn islands(mesh: &Mesh, layers: Layers) -> Vec<Island> {
    let loops = section(mesh, island_z(mesh, layers));
    let (outers, holes): (Vec<Vec<Vec2>>, Vec<Vec<Vec2>>) =
        loops.into_iter().partition(|l| loop_area(l) > 0.0);
    let mut own: Vec<Vec<Vec<Vec2>>> = vec![Vec::new(); outers.len()];
    // Each hole goes to the smallest outer loop that holds it.
    for h in holes {
        let best = outers
            .iter()
            .enumerate()
            .filter(|(_, o)| inside_loop(h[0], o))
            .min_by(|a, b| loop_area(a.1).total_cmp(&loop_area(b.1)));
        if let Some((k, _)) = best {
            own[k].push(h);
        }
    }
    outers
        .into_iter()
        .zip(own)
        .map(|(outer, holes)| {
            let area = loop_area(&outer) - holes.iter().map(|h| loop_area(h).abs()).sum::<f32>();
            let mut loops = vec![outer.clone()];
            loops.extend(holes.iter().cloned());
            let width = width(&loops, area, THIN_MM);
            Island {
                outer,
                holes,
                area,
                width,
            }
        })
        .collect()
}

/// The length of the closed loop `l`.
fn perimeter(l: &[Vec2]) -> f32 {
    let n = l.len();
    (0..n).map(|i| l[(i + n - 1) % n].distance(l[i])).sum()
}

/// Two times the largest distance from a point in the area `loops` to its
/// edge, on a grid. It stops at `cap`. As `overhang.py`.
fn width(loops: &[Vec<Vec2>], area: f32, cap: f32) -> f32 {
    let p: f32 = loops.iter().map(|l| perimeter(l)).sum();
    if p <= 0.0 || area <= 0.0 {
        return 0.0;
    }
    // An area is at most half its perimeter times the half width.
    let quick = 2.0 * area / p;
    if quick >= cap {
        return quick;
    }
    let segs: Vec<(Vec2, Vec2)> = loops
        .iter()
        .flat_map(|l| (0..l.len()).map(move |i| (l[(i + l.len() - 1) % l.len()], l[i])))
        .collect();
    let all: Vec<Vec2> = loops.iter().flatten().copied().collect();
    let (lo, hi) = bbox(&all);
    let st = (area / 20000.0).sqrt().max(0.05);
    let mut best = 0.0f32;
    let mut y = lo.y + st * 0.5;
    while y < hi.y {
        // The parts of the grid row in the area, from the crossings of the
        // row with the loops.
        let mut xs: Vec<f32> = segs
            .iter()
            .filter(|(a, b)| (a.y > y) != (b.y > y))
            .map(|(a, b)| a.x + (y - a.y) * (b.x - a.x) / (b.y - a.y))
            .collect();
        xs.sort_by(f32::total_cmp);
        for span in xs.as_chunks::<2>().0 {
            let k = ((span[0] - lo.x - st * 0.5) / st).ceil().max(0.0);
            let mut x = lo.x + st * 0.5 + k * st;
            while x < span[1] {
                let q = Vec2::new(x, y);
                let d = segs
                    .iter()
                    .map(|(a, b)| seg_dist(q, *a, *b))
                    .fold(f32::MAX, f32::min);
                if d > best {
                    best = d;
                    if 2.0 * best >= cap {
                        return 2.0 * best;
                    }
                }
                x += st;
            }
        }
        y += st;
    }
    2.0 * best
}

/// How far the brim may go past the footprint box on x and on y: `w` when
/// the part and two brims fit the print area on that axis, else 0 (an
/// inner brim). The part is in the 0 degree turn when it fits so, else in
/// the 90 degree turn (`brim_out()`).
pub fn brim_out(size: Vec2, w: f32, area: &Area) -> Vec2 {
    let bed = if size.x <= area.w && size.y <= area.d {
        Vec2::new(area.w, area.d)
    } else {
        Vec2::new(area.d, area.w)
    };
    Vec2::new(
        if size.x + 2.0 * w <= bed.x { w } else { 0.0 },
        if size.y + 2.0 * w <= bed.y { w } else { 0.0 },
    )
}

/// Finds the small bed islands of `mesh` (print frame) and returns the brim
/// statements (late statements, after the bed chamfer).
pub fn plan(mesh: &Mesh, s: &Settings, layers: Layers, area: &Area) -> Plan {
    let mut plan = Plan::default();
    if s.brim_w <= 0.0 {
        return plan;
    }
    let Some(b) = mesh.bounds() else {
        return plan;
    };
    let all = islands(mesh, layers);
    // With the bed chamfer, the first layer is smaller by the chamfer: an
    // island that the chamfer makes small gets a brim too.
    let shrink = if s.chamfer { s.chamfer_mm } else { 0.0 };
    let (small, rest): (Vec<&Island>, Vec<&Island>) =
        all.iter().partition(|i| i.small_after(shrink));
    if small.is_empty() {
        return plan;
    }
    let out = brim_out(b.size().truncate(), s.brim_w, area);
    let lo = b.min.truncate() - out;
    let hi = b.max.truncate() + out;
    plan.late
        .push(statement(&small, &rest, lo, hi, s, layers.first, b.min.z));
    plan.fixed = small.len();
    plan
}

/// The OpenSCAD statement of the brim: one layer from `z0`, `s.brim_w`
/// around the `small` islands, in the box `lo`-`hi`, `s.brim_gap` from the
/// `rest`.
fn statement(
    small: &[&Island],
    rest: &[&Island],
    lo: Vec2,
    hi: Vec2,
    s: &Settings,
    height: f32,
    z0: f32,
) -> String {
    let w = scad::num(s.brim_w);
    // The brim around the islands without holes, then around each island
    // with holes, without the holes.
    let plain: Vec<Vec<Vec2>> = small
        .iter()
        .filter(|i| i.holes.is_empty())
        .map(|i| i.outer.clone())
        .collect();
    let mut around = String::new();
    if !plain.is_empty() {
        let _ = write!(
            around,
            "offset(r = {w}, $fn = {FN}) {} ",
            scad::polygon(&plain)
        );
    }
    for i in small.iter().filter(|i| !i.holes.is_empty()) {
        let _ = write!(
            around,
            "difference() {{ offset(r = {w}, $fn = {FN}) {} {} }} ",
            scad::polygon(&i.loops()),
            scad::polygon(&i.holes),
        );
    }
    let size = hi - lo;
    let mut s2 = format!(
        "intersection() {{ union() {{ {around}}} translate({}) square({}); }}",
        scad::v2(lo),
        scad::v2(size),
    );
    if !rest.is_empty() {
        let loops: Vec<Vec<Vec2>> = rest.iter().flat_map(|i| i.loops()).collect();
        s2 = format!(
            "difference() {{ {s2} offset(r = {}, $fn = {FN}) {} }}",
            scad::num(s.brim_gap),
            scad::polygon(&loops)
        );
    }
    format!(
        "translate([0, 0, {}]) linear_extrude(height = {}) {s2}",
        scad::num(z0),
        scad::num(height)
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn chamfer_makes_an_island_small() {
        // A wall of 3.4 mm is not small, but 3.4 - 2 x 0.4 = 2.6 is.
        let wall = Island {
            outer: vec![
                Vec2::new(0.0, 0.0),
                Vec2::new(3.4, 0.0),
                Vec2::new(3.4, 40.0),
                Vec2::new(0.0, 40.0),
            ],
            holes: Vec::new(),
            area: 136.0,
            width: 3.4,
        };
        assert!(!wall.small());
        assert!(wall.small_after(0.4));
    }

    use super::super::check::test_util::*;
    use super::super::fixes::apply;
    use super::*;
    use glam::Vec3;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    fn on() -> Settings {
        Settings {
            brim: true,
            ..Settings::default()
        }
    }

    /// A plate 30 x 30 x 2 and a wall 0.8 x 20 x 10, 5 mm apart.
    fn plate_and_wall() -> Mesh {
        mesh(&[
            block(Vec3::ZERO, Vec3::new(30.0, 30.0, 2.0)),
            block(Vec3::new(35.0, 0.0, 0.0), Vec3::new(35.8, 20.0, 10.0)),
        ])
    }

    #[test]
    fn islands_have_area_and_width() {
        let isl = islands(&plate_and_wall(), Layers::default());
        assert_eq!(isl.len(), 2);
        let wall = isl.iter().find(|i| i.small()).unwrap();
        assert!((wall.area - 16.0).abs() < 0.15, "{wall:?}");
        assert!((wall.width - 0.8).abs() < 0.08, "{wall:?}");
        let plate = isl.iter().find(|i| !i.small()).unwrap();
        assert!((plate.area - 900.0).abs() < 0.1);
        assert!(plate.width >= THIN_MM);
    }

    #[test]
    fn a_ring_keeps_its_hole() {
        // A square ring 20 x 20, 2 mm wide: 144 mm², thin.
        let m = mesh(&[block(Vec3::ZERO, Vec3::new(20.0, 20.0, 5.0))]);
        let ring = Island {
            outer: section(&m, 1.0).remove(0),
            holes: vec![vec![
                Vec2::new(2.0, 2.0),
                Vec2::new(2.0, 18.0),
                Vec2::new(18.0, 18.0),
                Vec2::new(18.0, 2.0),
            ]],
            area: 144.0,
            width: 0.0,
        };
        // The largest circle is in a corner: 2 x 2√2 / (1 + √2) = 2.34 mm.
        let w = width(&ring.loops(), 144.0, THIN_MM);
        assert!((w - 2.343).abs() < 0.1, "{w} {ring:?}");
        let st = statement(
            &[&ring],
            &[],
            Vec2::ZERO,
            Vec2::splat(20.0),
            &on(),
            0.2,
            0.0,
        );
        assert!(
            st.starts_with("translate([0, 0, 0]) linear_extrude(height = 0.2)"),
            "{st}"
        );
        assert!(
            st.contains("difference() { offset(r = 5, $fn = 32)"),
            "{st}"
        );
        assert!(!st.contains("offset(r = 1"), "{st}");
    }

    #[test]
    fn brim_out_follows_the_print_area() {
        let a = Area {
            w: 250.0,
            d: 220.0,
            h: 250.0,
        };
        assert_eq!(brim_out(Vec2::new(100.0, 100.0), 5.0, &a), Vec2::splat(5.0));
        // 245 + 10 > 250: an inner brim on x.
        assert_eq!(
            brim_out(Vec2::new(245.0, 100.0), 5.0, &a),
            Vec2::new(0.0, 5.0)
        );
        // 100 x 240 fits only in the 90 degree turn: y goes along the 250.
        assert_eq!(
            brim_out(Vec2::new(100.0, 240.0), 5.0, &a),
            Vec2::new(5.0, 5.0)
        );
        assert_eq!(
            brim_out(Vec2::new(100.0, 245.0), 5.0, &a),
            Vec2::new(5.0, 0.0)
        );
    }

    #[test]
    fn a_thin_wall_gets_one_brim_statement() {
        let p = plan(
            &plate_and_wall(),
            &on(),
            Layers::default(),
            &Area::default(),
        );
        assert_eq!(p.fixed, 1);
        assert_eq!(p.late.len(), 1);
        assert!(p.cuts.is_empty() && p.adds.is_empty());
        let st = &p.late[0];
        assert!(st.contains("offset(r = 5, $fn = 32)"), "{st}");
        assert!(st.contains("offset(r = 1, $fn = 32)"), "{st}");
        // The box: the footprint 35.8 x 30 plus 5 mm on each side.
        assert!(
            st.contains("translate([-5, -5]) square([45.8, 40])"),
            "{st}"
        );
    }

    #[test]
    fn no_small_island_gives_no_statement() {
        let m = crate::mesh::cube(10.0);
        let p = plan(&m, &on(), Layers::default(), &Area::default());
        assert!(p.is_empty());
        assert_eq!(p.fixed, 0);
        let off = Settings {
            brim_w: 0.0,
            ..on()
        };
        assert!(plan(&plate_and_wall(), &off, Layers::default(), &Area::default()).is_empty());
    }

    #[test]
    #[ignore = "needs openscad-nightly"]
    fn the_brim_keeps_the_gap_to_the_plate() {
        let m = plate_and_wall();
        let layers = Layers::default();
        let (out, rep) = apply(
            &crate::openscad::Config::default(),
            &m,
            &on(),
            layers,
            &super::super::Limits::default(),
            &Area::default(),
            &Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        assert_eq!(rep.brims, 1);
        let isl = islands(&out, layers);
        assert_eq!(isl.len(), 2, "{isl:?}");
        // The wall and its brim: no longer small.
        assert!(isl.iter().all(|i| !i.small()), "{isl:?}");
        let plate = isl
            .iter()
            .find(|i| i.area > 899.0 && i.area < 901.0)
            .unwrap();
        let brim = isl
            .iter()
            .find(|i| i.area < 899.0 || i.area > 901.0)
            .unwrap();
        // The gap: no point of the brim nearer to the plate than brim_gap.
        let gap = brim
            .outer
            .iter()
            .map(|p| super::super::check::loop_dist(*p, &plate.outer))
            .fold(f32::MAX, f32::min);
        assert!(gap > 0.95 && gap < 1.1, "gap {gap}");
        // The brim: one layer only. Above it, the wall is 0.8 mm again.
        // (The section can lose a corner of 0.1 mm².)
        let a = section_area(&out, 0.3);
        assert!((a - 900.0 - 16.0).abs() < 0.3, "{a}");
        // The brim is 5 mm around the wall, but not nearer than 1 mm to the
        // plate (x 31 next to the plate, a little less below its corner).
        let (lo, hi) = bbox(&brim.outer);
        assert!(
            lo.x > 30.0 && lo.x < 31.01 && (hi.x - 40.8).abs() < 0.05,
            "{lo} {hi}"
        );
        assert!(
            (lo.y + 5.0).abs() < 0.05 && (hi.y - 25.0).abs() < 0.05,
            "{lo} {hi}"
        );
        // The check finds no small island after the brim.
        let i = super::super::check::run(
            &out,
            layers,
            &super::super::Limits::default(),
            &Area::default(),
        );
        assert!(i.is_empty(), "{i:?}");
    }
}
