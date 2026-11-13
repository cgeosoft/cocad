//! Bridges (feature 4 of `docs/optimize-tab.md`).
//!
//! A ceiling (a flat face that points down, above the first layer) prints
//! as a bridge. A bridge must start on straight edges. This module finds the
//! ceilings of a part in print frame and returns the fixes: a square last
//! layer under a round ceiling, a sacrificial layer or a stepped bridge
//! under a hole that goes on above the ceiling, a cone at the end of a
//! blind hole that goes up.
//!
//! The cases:
//!
//! - a. A round ceiling over a cavity: the last layer under the ceiling
//!   gets the rectangle that holds the round outline (a square for a
//!   circle). The rectangle must keep `min_wall` to the other faces.
//! - b. A round hole that goes on above the ceiling: sacrificial layers
//!   close it, or a stepped bridge (a slot, then a square) starts it.
//! - c. A round ceiling with no hole (a blind hole that goes up): a 45
//!   degree cone, when the material above holds it. Else case a.
//! - d. A ceiling wider than the bridge limit: no fix, a note.
//!
//! A ceiling only needs a fix when its outline has arcs: the same test as
//! the Check (`check::arc_points`).

use super::check::{self, Rect};
use super::fixes::{HoleAbove, Plan, Settings};
use super::{Layers, Limits, loop_area, scad, section, tri_area};
use crate::assembly::ray_triangle;
use crate::mesh::Mesh;
use glam::{Vec2, Vec3};

/// A face is a ceiling when its normal is within about 1 degree of -z.
const CEILING_NZ: f32 = check::LEVEL_NZ;
/// The overlap of a cut or an add into the faces next to it (mm), so that
/// OpenSCAD leaves no thin skin.
const EPS: f32 = 0.01;
/// The distance (mm) of the side of a cut rectangle from a line or a face
/// that it must not touch (see `cut_corners`). It is much more than the
/// error of the numbers in the `.scad` file and of an f32 at 1 m.
const CLEAR: f32 = 0.001;
/// A round loop is a circle when it has this many arc points or more and
/// all points are this near to the largest radius.
const CIRCLE_ARCS: usize = 8;
const CIRCLE_FIT: f32 = 0.93;

/// One bridged ceiling: connected coplanar faces that point down.
#[derive(Clone, Debug)]
pub struct Ceiling {
    /// The height of the ceiling.
    pub z: f32,
    /// The triangles of the ceiling.
    pub tris: Vec<u32>,
    /// The outline, counter clockwise (seen from above).
    pub outer: Vec<Vec2>,
    /// The holes in the ceiling face, clockwise.
    pub holes: Vec<Vec<Vec2>>,
    /// The area in mm².
    pub area: f32,
}

impl Ceiling {
    /// A point in the ceiling face: the center of its largest triangle.
    fn inner_point(&self, mesh: &Mesh) -> Vec2 {
        let t = self
            .tris
            .iter()
            .copied()
            .max_by(|a, b| tri_area(mesh, *a as usize).total_cmp(&tri_area(mesh, *b as usize)))
            .unwrap_or(0);
        let [a, b, c] = mesh.tri(t as usize);
        ((a + b + c) / 3.0).truncate()
    }

    /// The place for a note: the height and the center.
    fn place(&self) -> String {
        let (lo, hi) = check::bbox(&self.outer);
        let c = (lo + hi) * 0.5;
        format!("z {:.2}, x {:.1}, y {:.1}", self.z, c.x, c.y)
    }

    /// Two times the area over the length of the outline: the width of a
    /// thin strip.
    fn strip_width(&self) -> f32 {
        let p: f32 = std::iter::once(&self.outer)
            .chain(&self.holes)
            .map(|l| {
                let n = l.len();
                (0..n)
                    .map(|i| l[(i + n - 1) % n].distance(l[i]))
                    .sum::<f32>()
            })
            .sum();
        if p > 0.0 { 2.0 * self.area / p } else { 0.0 }
    }
}

/// The ceilings of `mesh` above `z_min`: flat faces that point down,
/// joined when they share an edge and lie at the same height.
pub fn ceilings(mesh: &Mesh, z_min: f32) -> Vec<Ceiling> {
    let cand: Vec<u32> = (0..mesh.triangles.len())
        .filter(|&i| {
            let [a, b, c] = mesh.tri(i);
            // Small slivers stay in: they close the gaps in the outline.
            mesh.normal(i).z < CEILING_NZ && a.z.min(b.z).min(c.z) > z_min + 1e-3
        })
        .map(|i| i as u32)
        .collect();
    // The bottom of a sheet with no thickness is no ceiling.
    let sheet = check::sheet_bottoms(mesh, &cand);
    let cand: Vec<u32> = cand.into_iter().filter(|t| !sheet.contains(t)).collect();
    let mut out = Vec::new();
    for g in check::coplanar_groups(mesh, &cand) {
        let (loops, _) = check::outline_loops(mesh, &g);
        let mut loops: Vec<Vec<Vec2>> = loops
            .iter()
            .map(|l| {
                l.iter()
                    .map(|&v| mesh.vertices[v as usize].truncate())
                    .collect()
            })
            .collect();
        let Some(k) = (0..loops.len()).max_by(|&a, &b| {
            loop_area(&loops[a])
                .abs()
                .total_cmp(&loop_area(&loops[b]).abs())
        }) else {
            continue;
        };
        // The triangles point down, so the outline runs clockwise from
        // above. Turn the loops: outer counter clockwise, holes clockwise.
        if loop_area(&loops[k]) < 0.0 {
            for l in &mut loops {
                l.reverse();
            }
        }
        let outer = loops.swap_remove(k);
        let holes = loops.into_iter().filter(|l| loop_area(l) < 0.0).collect();
        // The height of the vertex nearest to the mean, not the mean: a
        // fix that starts at the ceiling is then in its plane (see
        // `scad::exact`).
        let zs: Vec<f32> = g
            .iter()
            .flat_map(|&t| mesh.tri(t as usize))
            .map(|v| v.z)
            .collect();
        let mean = zs.iter().map(|&z| z as f64).sum::<f64>() / zs.len() as f64;
        let z = zs
            .iter()
            .copied()
            .min_by(|a, b| (*a as f64 - mean).abs().total_cmp(&(*b as f64 - mean).abs()))
            .unwrap_or(0.0);
        let area = g.iter().map(|&t| tri_area(mesh, t as usize)).sum();
        out.push(Ceiling {
            z,
            tris: g,
            outer,
            holes,
            area,
        });
    }
    out
}

/// The circle of the closed loop `l`: the center (least squares fit) and
/// the largest distance of a point from it. A square of 2 r then holds the
/// loop. `None` when the loop is not a circle.
pub fn circle(l: &[Vec2]) -> Option<(Vec2, f32)> {
    if check::arc_points(l) < CIRCLE_ARCS {
        return None;
    }
    let n = l.len() as f64;
    let (mx, my) = l.iter().fold((0.0f64, 0.0f64), |(x, y), p| {
        (x + p.x as f64 / n, y + p.y as f64 / n)
    });
    let (mut suu, mut svv, mut suv, mut suuu, mut svvv, mut suvv, mut svuu) =
        (0.0f64, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    for p in l {
        let (u, v) = (p.x as f64 - mx, p.y as f64 - my);
        suu += u * u;
        svv += v * v;
        suv += u * v;
        suuu += u * u * u;
        svvv += v * v * v;
        suvv += u * v * v;
        svuu += v * u * u;
    }
    let det = suu * svv - suv * suv;
    if det.abs() < 1e-12 {
        return None;
    }
    let (bu, bv) = (0.5 * (suuu + suvv), 0.5 * (svvv + svuu));
    let a = (bu * svv - bv * suv) / det;
    let b = (suu * bv - suv * bu) / det;
    let c = Vec2::new((mx + a) as f32, (my + b) as f32);
    let (lo, hi) = l.iter().fold((f32::MAX, 0.0f32), |(lo, hi), p| {
        let d = p.distance(c);
        (lo.min(d), hi.max(d))
    });
    (lo >= hi * CIRCLE_FIT).then_some((c, hi))
}

/// The rectangle of straight lines that holds the round loop `l`: a square
/// along x and y for a circle, else the bounding rectangle with the
/// smallest area (a slot gets its rectangle).
pub fn holding_rect(l: &[Vec2]) -> Option<Rect> {
    match circle(l) {
        Some((c, r)) => Some(Rect {
            c,
            u: Vec2::X,
            half: Vec2::splat(r),
        }),
        None => check::min_rect(l),
    }
}

/// Why a rectangle does not fit.
enum Misfit {
    /// The point is not in a closed cavity of the section: the ceiling is
    /// the bottom of an overhang of the outer shape, not a bridge.
    Open,
    /// The rectangle comes this near (mm) to another face.
    Near(f32),
}

impl Misfit {
    fn text(&self, wall: f32) -> String {
        match self {
            Misfit::Open => "the space under it is not closed".into(),
            Misfit::Near(d) => format!(
                "the rectangle comes to {d:.2} mm from another face (minimum {} mm)",
                scad::num(wall)
            ),
        }
    }
}

/// Checks that the rectangle `r` stays in the part in the section `sec`:
/// it keeps `wall` to all loops of the section, but the loop of the cavity
/// at `p` that it widens.
fn fits(r: &Rect, sec: &[Vec<Vec2>], p: Vec2, wall: f32) -> Result<(), Misfit> {
    let cavity = sec
        .iter()
        .enumerate()
        .filter(|(_, l)| loop_area(l) < 0.0 && check::inside_loop(p, l))
        .min_by(|a, b| loop_area(a.1).abs().total_cmp(&loop_area(b.1).abs()))
        .map(|(i, _)| i);
    let Some(ci) = cavity else {
        return Err(Misfit::Open);
    };
    let d = sec
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != ci)
        .map(|(_, l)| r.dist_to_loop(l))
        .fold(f32::MAX, f32::min);
    if d < wall {
        return Err(Misfit::Near(d));
    }
    Ok(())
}

/// The distance from `o` straight up to the first face of `mesh`.
fn up_hit(mesh: &Mesh, o: Vec3) -> Option<f32> {
    let mut best: Option<f32> = None;
    for i in 0..mesh.triangles.len() {
        let t = mesh.tri(i);
        let lo = t[0].min(t[1]).min(t[2]);
        let hi = t[0].max(t[1]).max(t[2]);
        if o.x < lo.x || o.x > hi.x || o.y < lo.y || o.y > hi.y || hi.z < o.z {
            continue;
        }
        if let Some(d) = ray_triangle(o, Vec3::Z, t)
            && d > 1e-6
            && best.is_none_or(|b| d < b)
        {
            best = Some(d);
        }
    }
    best
}

/// The free height above `z` at the point `p` of a hole: to the face that
/// ends the hole, or to the top of the part.
fn hole_length(mesh: &Mesh, p: Vec2, z: f32, top: f32) -> f32 {
    let z0 = z + 1e-3;
    up_hit(mesh, p.extend(z0)).map_or(top - z, |t| z0 + t - z)
}

/// True when a 45 degree cone of radius `r` on the ceiling at `z`, center
/// `c`, keeps `wall` to the faces above it. Rays go up from the center and
/// from two rings of points.
fn cone_fits(mesh: &Mesh, c: Vec2, r: f32, z: f32, wall: f32) -> bool {
    let ok =
        |p: Vec2, need: f32| up_hit(mesh, p.extend(z + 1e-3)).is_some_and(|t| t + 1e-3 >= need);
    if !ok(c, r + wall) {
        return false;
    }
    for k in [0.5f32, 0.9] {
        for i in 0..8 {
            let a = std::f32::consts::TAU * i as f32 / 8.0;
            let p = c + Vec2::new(a.cos(), a.sin()) * r * k;
            if !ok(p, r * (1.0 - k) + wall) {
                return false;
            }
        }
    }
    true
}

/// The first crossing of the ray from `p` along `dir` with the closed loop
/// `l`.
fn ray_2d(p: Vec2, dir: Vec2, l: &[Vec2]) -> Option<f32> {
    let n = l.len();
    let mut best: Option<f32> = None;
    for i in 0..n {
        let (a, b) = (l[i], l[(i + 1) % n]);
        let e = b - a;
        let den = dir.perp_dot(e);
        if den.abs() < 1e-12 {
            continue;
        }
        let w = a - p;
        let t = w.perp_dot(e) / den;
        let s = w.perp_dot(dir) / den;
        if t >= 0.0 && (0.0..=1.0).contains(&s) && best.is_none_or(|b| t < b) {
            best = Some(t);
        }
    }
    best
}

/// The corners of the cut for the rectangle `r`, with `d` added to its
/// half sizes. A rectangle that holds a round loop touches its corner
/// lines, and a slot ends on the outline of the space under the ceiling. A
/// face of a cut through these lines is not exactly on them: OpenSCAD then
/// leaves triangles with no width, and a move of the part on the plate can
/// open their edges. The cut keeps `CLEAR` from them.
fn cut_corners(r: &Rect, d: Vec2) -> Vec<Vec2> {
    Rect {
        half: r.half + d,
        ..*r
    }
    .corners()
}

/// The statement that cuts the prism of `poly` from `z0`, `h` high.
fn prism(poly: Vec<Vec2>, z0: f32, h: f32) -> String {
    format!(
        "translate([0, 0, {}]) linear_extrude(height = {}) {}",
        scad::num(z0),
        scad::num(h),
        scad::polygon(&[poly])
    )
}

/// The statement that cuts the prism of `poly` from `top` down, `h` high.
/// The top is the literal value `top`, so that a sacrificial layer that
/// starts at the same value is exactly coplanar with it. A sum of two
/// values can be different in the last bit (-1 + 1.2 is not 0.2), and
/// OpenSCAD then can leave a sheet with no thickness.
fn prism_down(poly: Vec<Vec2>, top: f32, h: f32) -> String {
    format!(
        "translate([0, 0, {}]) mirror([0, 0, 1]) linear_extrude(height = {}) {}",
        scad::exact(top),
        scad::num(h),
        scad::polygon(&[poly])
    )
}

/// The cone cut at the end of a blind hole: the hole outline `l` (center
/// `c`, radius `r`) at the ceiling `z`, to a point above it at 45 degrees.
/// The cut starts `EPS` below the ceiling and `EPS` smaller than the hole.
/// It does not touch the wall of the hole: it only goes through the
/// ceiling face, which keeps a ring of `2 EPS` (a step, not a bridge).
/// A cone that is exactly the hole outline at `z` goes through the edges
/// where the wall meets the ceiling. OpenSCAD then leaves triangles with
/// no width and edges with four faces.
fn cone(l: &[Vec2], c: Vec2, r: f32, z: f32) -> String {
    let base = r - EPS;
    let k = base / r;
    let pts: Vec<Vec2> = l.iter().map(|p| (*p - c) * k).collect();
    format!(
        "translate({}) linear_extrude(height = {}, scale = 0) {}",
        scad::v3(c.extend(z - EPS)),
        scad::num(base),
        scad::polygon(&[pts])
    )
}

/// A round hole in a ceiling that goes on above it.
struct Hole<'a> {
    outline: &'a [Vec2],
    /// The rectangle that holds the hole.
    rect: Rect,
}

/// Finds the bridged ceilings of `mesh` (print frame) and returns the fixes.
pub fn plan(mesh: &Mesh, s: &Settings, layers: Layers, limits: &Limits) -> Plan {
    let mut plan = Plan::default();
    let Some(b) = mesh.bounds() else {
        return plan;
    };
    let mut z_min = b.min.z + layers.first;
    if s.chamfer {
        let n = layers.count_to(s.chamfer_mm);
        if n > 0 {
            z_min = z_min.max(b.min.z + layers.top(n - 1));
        }
    }
    // A thin flat strip is a step, not a bridge (as in the Check).
    let step = 1.5 * layers.first.max(layers.height);
    for c in ceilings(mesh, z_min) {
        if c.strip_width() <= step {
            continue;
        }
        if fix(mesh, &c, s, layers, limits, b.max.z, &mut plan) {
            plan.fixed += 1;
        }
    }
    plan
}

/// Adds the fixes of the ceiling `c` to `plan`. Returns true when it
/// changes the part. `top` is the top of the part.
fn fix(
    mesh: &Mesh,
    c: &Ceiling,
    s: &Settings,
    layers: Layers,
    limits: &Limits,
    top: f32,
    plan: &mut Plan,
) -> bool {
    let h = layers.height;
    let round_outer = check::arc_points(&c.outer) > 0;
    let round_holes: Vec<&Vec<Vec2>> = c
        .holes
        .iter()
        .filter(|l| check::arc_points(l) > 0)
        .collect();
    if !round_outer && round_holes.is_empty() {
        return false;
    }
    let at = c.place();
    // A round loop in the ceiling with air above it is a hole that goes
    // on. With material above it, it is a post under the ceiling.
    let above = section(mesh, c.z + 0.5 * h);
    let holes: Vec<Hole> = round_holes
        .into_iter()
        .filter_map(|l| {
            let rect = holding_rect(l)?;
            (check::inside_loop(rect.c, l) && !check::inside(rect.c, &above))
                .then_some(Hole { outline: l, rect })
        })
        .collect();
    if !round_outer && holes.is_empty() {
        return false;
    }

    // c. A blind hole that goes up: a cone, when it fits.
    if round_outer
        && c.holes.is_empty()
        && let Some((cc, r)) = circle(&c.outer)
        && cone_fits(mesh, cc, r, c.z, limits.min_wall)
    {
        plan.cuts.push(cone(&c.outer, cc, r, c.z));
        return true;
    }

    // d. Too wide for a bridge: no fix.
    let span = check::min_rect(&c.outer).map_or(0.0, |r| r.width());
    if span > limits.bridge_mm {
        plan.skipped.push(format!(
            "Bridge at {at}: {span:.1} mm, longer than {} mm. No fix.",
            scad::num(limits.bridge_mm)
        ));
        return false;
    }

    let mut changed = false;
    // The ceiling height after the cut of case a, and the outline of the
    // space under the ceiling.
    let mut ceiling = c.z;
    let mut bound = c.outer.clone();

    // a. The last layer under a round outline gets straight edges.
    if round_outer && let Some(r) = holding_rect(&c.outer) {
        let below = section(mesh, c.z - 0.5 * h);
        match fits(&r, &below, c.inner_point(mesh), limits.min_wall) {
            Ok(()) => {
                bound = cut_corners(&r, Vec2::splat(CLEAR));
                plan.cuts.push(prism_down(bound.clone(), c.z + EPS, h + EPS));
                ceiling = c.z + EPS;
                changed = true;
            }
            // An overhang of the outer shape: the Check lists it.
            Err(Misfit::Open) => {}
            Err(why) => plan.skipped.push(format!(
                "Bridge at {at}: no straight last layer, {}.",
                why.text(limits.min_wall)
            )),
        }
    }

    // b. Holes that go on above the ceiling.
    for hole in &holes {
        let p = hole.rect.c;
        let len = hole_length(mesh, p, ceiling, top);
        match s.hole_above {
            HoleAbove::Sacrificial => {
                if s.sacrificial_layers == 0 {
                    continue;
                }
                let thick = s.sacrificial_layers as f32 * h;
                if len < thick + h - 1e-3 {
                    plan.skipped.push(format!(
                        "Hole at {at}: {len:.2} mm above the bridge, too short for {} sacrificial layers.",
                        s.sacrificial_layers
                    ));
                    continue;
                }
                // The layers start exactly in the plane of the ceiling,
                // or of the cut of case a.
                plan.adds.push(format!(
                    "translate([0, 0, {}]) linear_extrude(height = {}) offset(delta = {}) {}",
                    scad::exact(ceiling),
                    scad::num(thick),
                    scad::num(EPS),
                    scad::polygon(&[hole.outline.to_vec()])
                ));
                changed = true;
            }
            HoleAbove::Stepped => {
                if len < 3.0 * h - 1e-3 {
                    plan.skipped.push(format!(
                        "Hole at {at}: {len:.2} mm above the bridge, too short for a stepped bridge."
                    ));
                    continue;
                }
                match stepped(mesh, c.z, h, hole, &bound, limits.min_wall) {
                    Ok((slot, square)) => {
                        // The slot ends just inside the space under the
                        // ceiling. Its sides are `CLEAR` out from the sides
                        // of the square.
                        let slot = cut_corners(&slot, Vec2::new(-CLEAR, 2.0 * CLEAR));
                        plan.cuts.push(prism(slot, c.z - EPS, h + EPS));
                        let square = cut_corners(&square, Vec2::splat(CLEAR));
                        plan.cuts.push(prism(square, c.z + h - EPS, h + EPS));
                        changed = true;
                    }
                    Err(why) => plan
                        .skipped
                        .push(format!("Hole at {at}: no stepped bridge, {why}.")),
                }
            }
        }
    }
    changed
}

/// The two layers of a stepped bridge over `hole` in the ceiling at `z`:
/// a slot as wide as the hole across the space `bound` under the ceiling,
/// then the square that holds the hole. The slot goes along the axis of
/// the shorter crossing.
fn stepped(
    mesh: &Mesh,
    z: f32,
    h: f32,
    hole: &Hole,
    bound: &[Vec2],
    wall: f32,
) -> Result<(Rect, Rect), String> {
    let sq = hole.rect;
    let mut best: Option<(f32, Rect)> = None;
    for (axis, across) in [(sq.u, sq.half.y), (sq.v(), sq.half.x)] {
        let (Some(t1), Some(t2)) = (ray_2d(sq.c, axis, bound), ray_2d(sq.c, -axis, bound)) else {
            continue;
        };
        let slot = Rect {
            c: sq.c + axis * (t1 - t2) * 0.5,
            u: axis,
            half: Vec2::new((t1 + t2) * 0.5, across),
        };
        if best.is_none_or(|(l, _)| t1 + t2 < l) {
            best = Some((t1 + t2, slot));
        }
    }
    let Some((_, slot)) = best else {
        return Err("the hole is not in the space under the bridge".into());
    };
    fits(&slot, &section(mesh, z + 0.5 * h), sq.c, wall).map_err(|m| m.text(wall))?;
    fits(&sq, &section(mesh, z + 1.5 * h), sq.c, wall).map_err(|m| m.text(wall))?;
    Ok((slot, sq))
}

#[cfg(test)]
mod tests {
    use super::super::check::test_util::*;
    use super::super::check::{self, Kind};
    use super::super::{Area, fixes};
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    fn on(hole_above: HoleAbove) -> Settings {
        Settings {
            bridges: true,
            hole_above,
            ..Settings::default()
        }
    }

    fn apply(m: &Mesh, s: &Settings) -> (Mesh, fixes::Report) {
        fixes::apply(
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

    /// The loops of the section at `z` that are holes, with their area.
    fn holes_at(m: &Mesh, z: f32) -> Vec<(usize, f32)> {
        section(m, z)
            .iter()
            .filter(|l| loop_area(l) < 0.0)
            .map(|l| (l.len(), -loop_area(l)))
            .collect()
    }

    fn kinds(m: &Mesh) -> Vec<Kind> {
        check::run(m, Layers::default(), &Limits::default(), &Area::default())
            .iter()
            .map(|i| i.kind)
            .collect()
    }

    /// A 30 x 30 x 10 block with a round counterbore `d` x 3 deep on the
    /// bed side and a hole of 3.4 mm through.
    fn counterbore(d: f32, extra: &str) -> Mesh {
        render(&format!(
            "$fn = 48; difference() {{ translate([-15, -15, 0]) cube([30, 30, 10]); \
             translate([0, 0, -1]) cylinder(d = {d}, h = 4); \
             translate([0, 0, -1]) cylinder(d = 3.4, h = 12); {extra} }}"
        ))
    }

    #[test]
    fn circle_fit_finds_center_and_radius() {
        let (c, r) = circle(&ngon(Vec2::new(3.0, -2.0), 5.0, 48)).unwrap();
        assert!(c.distance(Vec2::new(3.0, -2.0)) < 1e-3, "{c}");
        assert!((r - 5.0).abs() < 1e-3);
        // A hexagon has no arcs, a slot is not a circle.
        assert!(circle(&ngon(Vec2::ZERO, 5.0, 6)).is_none());
        let mut slot = ngon(Vec2::new(5.0, 0.0), 2.0, 32)[24..].to_vec();
        slot.extend(&ngon(Vec2::new(5.0, 0.0), 2.0, 32)[..=8]);
        slot.extend(&ngon(Vec2::new(-5.0, 0.0), 2.0, 32)[8..=24]);
        assert!(circle(&slot).is_none());
        let r = holding_rect(&slot).unwrap();
        assert!(
            (r.length() - 14.0).abs() < 1e-3 && (r.width() - 4.0).abs() < 1e-3,
            "{r:?}"
        );
    }

    #[test]
    fn a_floating_disk_is_a_ceiling() {
        let disk = ngon(Vec2::ZERO, 5.0, 24);
        let m = mesh(&[
            block(Vec3::new(-10.0, -10.0, 0.0), Vec3::new(10.0, 10.0, 2.0)),
            prism_tris(&disk),
        ]);
        let c = ceilings(&m, 0.2);
        assert_eq!(c.len(), 1);
        assert!((c[0].z - 5.0).abs() < 1e-6);
        assert_eq!(c[0].outer.len(), 24);
        assert!(loop_area(&c[0].outer) > 0.0);
        assert!(c[0].holes.is_empty());
        // No cavity under it: an overhang, not a bridge. No fix, no note
        // (the Check lists it).
        let p = plan(
            &m,
            &on(HoleAbove::Sacrificial),
            Layers::default(),
            &Limits::default(),
        );
        assert!(p.is_empty() && p.skipped.is_empty(), "{p:?}");
        assert_eq!(p.fixed, 0);
    }

    fn prism_tris(poly: &[Vec2]) -> Vec<[Vec3; 3]> {
        super::super::check::test_util::prism(poly, 5.0, 6.0)
    }

    #[test]
    fn ray_2d_finds_the_first_edge() {
        let sq = Rect::aligned(Vec2::splat(-5.0), Vec2::splat(5.0)).corners();
        assert!((ray_2d(Vec2::new(1.0, 0.0), Vec2::X, &sq).unwrap() - 4.0).abs() < 1e-5);
        assert!((ray_2d(Vec2::new(1.0, 0.0), -Vec2::X, &sq).unwrap() - 6.0).abs() < 1e-5);
        assert!(ray_2d(Vec2::new(9.0, 0.0), Vec2::X, &sq).is_none());
    }

    #[test]
    #[ignore = "needs openscad-nightly"]
    fn counterbore_gets_a_square_and_sacrificial_layers() {
        let m = counterbore(10.0, "");
        let s = on(HoleAbove::Sacrificial);
        let p = plan(&m, &s, Layers::default(), &Limits::default());
        assert_eq!(p.fixed, 1, "{p:?}");
        assert_eq!((p.cuts.len(), p.adds.len()), (1, 1), "{p:?}");
        assert!(p.skipped.is_empty(), "{p:?}");
        assert_eq!(kinds(&m), vec![Kind::Arc]);
        let (out, rep) = apply(&m, &s);
        assert_eq!(rep.bridges, 1);
        // The layer under the ceiling: the counterbore is round.
        let h = holes_at(&out, 2.7);
        assert_eq!(h.len(), 1);
        assert!((h[0].1 - 78.3).abs() < 0.5, "{h:?}");
        // The last layer under the ceiling: a square 10 x 10.
        let h = holes_at(&out, 2.9);
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].0, 4, "{h:?}");
        assert!((h[0].1 - 100.0).abs() < 0.1, "{h:?}");
        // Two sacrificial layers close the hole.
        assert!(holes_at(&out, 3.1).is_empty());
        assert!(holes_at(&out, 3.3).is_empty());
        // Above them, the hole is round again.
        let h = holes_at(&out, 3.6);
        assert_eq!(h.len(), 1);
        assert!((h[0].1 - 9.05).abs() < 0.1, "{h:?}");
        assert!(kinds(&out).is_empty(), "{:?}", kinds(&out));
    }

    #[test]
    #[ignore = "needs openscad-nightly"]
    fn counterbore_gets_a_stepped_bridge() {
        let m = counterbore(10.0, "");
        let s = on(HoleAbove::Stepped);
        let p = plan(&m, &s, Layers::default(), &Limits::default());
        assert_eq!(p.fixed, 1, "{p:?}");
        assert_eq!((p.cuts.len(), p.adds.len()), (3, 0), "{p:?}");
        let (out, _) = apply(&m, &s);
        let h = holes_at(&out, 2.9);
        assert!(h.len() == 1 && (h[0].1 - 100.0).abs() < 0.1, "{h:?}");
        // Layer 1 above the ceiling: a slot 3.4 x 10.
        let h = holes_at(&out, 3.1);
        assert!(
            h.len() == 1 && h[0].0 == 4 && (h[0].1 - 34.0).abs() < 0.1,
            "{h:?}"
        );
        // Layer 2: a square 3.4 x 3.4.
        let h = holes_at(&out, 3.3);
        assert!(
            h.len() == 1 && h[0].0 == 4 && (h[0].1 - 11.56).abs() < 0.1,
            "{h:?}"
        );
        // Then the round hole.
        let h = holes_at(&out, 3.6);
        assert!(h.len() == 1 && (h[0].1 - 9.05).abs() < 0.1, "{h:?}");
        // The four corners between the square and the round hole are
        // ceilings with arcs, but only 0.2 mm wide on average: steps.
        assert!(kinds(&out).is_empty(), "{:?}", kinds(&out));
    }

    #[test]
    #[ignore = "needs openscad-nightly"]
    fn blind_hole_gets_a_cone_or_a_square() {
        let src = |hgt: f32| {
            format!(
                "difference() {{ translate([-10, -10, 0]) cube([20, 20, {hgt}]); \
                 translate([0, 0, -1]) cylinder(d = 6, h = 6, $fn = 32); }}"
            )
        };
        let s = on(HoleAbove::Sacrificial);
        let m = render(&src(15.0));
        assert_eq!(kinds(&m), vec![Kind::Arc]);
        let p = plan(&m, &s, Layers::default(), &Limits::default());
        assert_eq!((p.fixed, p.cuts.len()), (1, 1), "{p:?}");
        assert!(p.cuts[0].contains("scale = 0"), "{p:?}");
        let (out, _) = apply(&m, &s);
        // Halfway up the cone, the hole is half as wide.
        let h = holes_at(&out, 6.5);
        assert!(h.len() == 1 && (h[0].1 - 7.0).abs() < 0.2, "{h:?}");
        assert!(holes_at(&out, 8.1).is_empty());
        assert!(kinds(&out).is_empty(), "{:?}", kinds(&out));

        // 2 mm above the ceiling: no room for the cone (3 mm + 0.8 mm).
        let m = render(&src(7.0));
        let p = plan(&m, &s, Layers::default(), &Limits::default());
        assert_eq!((p.fixed, p.cuts.len()), (1, 1), "{p:?}");
        assert!(!p.cuts[0].contains("scale"), "{p:?}");
        let (out, _) = apply(&m, &s);
        let h = holes_at(&out, 4.9);
        assert!(
            h.len() == 1 && h[0].0 == 4 && (h[0].1 - 36.0).abs() < 0.1,
            "{h:?}"
        );
        assert!(kinds(&out).is_empty(), "{:?}", kinds(&out));
    }

    #[test]
    #[ignore = "needs openscad-nightly"]
    fn a_long_bridge_gets_a_note_only() {
        let m = counterbore(16.0, "");
        let p = plan(
            &m,
            &on(HoleAbove::Sacrificial),
            Layers::default(),
            &Limits::default(),
        );
        assert!(p.is_empty(), "{p:?}");
        assert_eq!(p.fixed, 0);
        assert_eq!(p.skipped.len(), 1);
        assert!(p.skipped[0].contains("16.0 mm"), "{p:?}");
        assert_eq!(kinds(&m), vec![Kind::LongBridge]);
    }

    #[test]
    #[ignore = "needs openscad-nightly"]
    fn a_square_too_near_a_pocket_is_skipped() {
        // A square pocket through the block, 0.71 mm from the corner of
        // the square under the counterbore.
        let m = counterbore(10.0, "translate([5.5, 5.5, -1]) cube([6.5, 6.5, 12]);");
        let p = plan(
            &m,
            &on(HoleAbove::Sacrificial),
            Layers::default(),
            &Limits::default(),
        );
        assert_eq!((p.fixed, p.cuts.len(), p.adds.len()), (1, 0, 1), "{p:?}");
        assert_eq!(p.skipped.len(), 1, "{p:?}");
        assert!(p.skipped[0].contains("0.71 mm"), "{p:?}");
    }

    /// The faces of a step with no height: triangles that are not level,
    /// with a height under 0.1 µm on their longest side.
    fn steps_with_no_height(m: &Mesh) -> usize {
        (0..m.triangles.len())
            .filter(|&i| {
                let [a, b, c] = m.tri(i);
                let long = (b - a).length().max((c - b).length()).max((a - c).length());
                let h = (b - a).cross(c - a).length() / long.max(1e-30);
                h > 0.0 && h < 1e-4 && m.normal(i).z.abs() < 0.5
            })
            .count()
    }

    /// `m` moved far from the origin in f32, as on a plate in an STL file.
    fn moved(m: &Mesh) -> Mesh {
        let off = Vec3::new(311.1, 329.9, 0.0);
        let soup: Vec<[Vec3; 3]> = (0..m.triangles.len())
            .map(|i| m.tri(i).map(|v| v + off))
            .collect();
        Mesh::from_soup(&soup)
    }

    #[test]
    #[ignore = "needs openscad-nightly"]
    fn the_fixed_part_is_closed() {
        // As in the spine of open-case-frame.scad: a block 8 mm thick, away
        // from the origin, with a blind hole of 5.3 mm (4 mm deep, for an
        // insert: the cone) and a counterbore 2.8 mm deep with a hole
        // above it (the square last layer and the sacrificial layers). 2.8
        // is not an f32 value. 40 faces put corners on the square.
        let m = render(
            "$fn = 40; difference() { translate([-60, 40, 0]) cube([45, 35, 8]); \
             translate([-43.18, 62.77, -1]) cylinder(d = 5.3, h = 5); \
             translate([-30.5, 52.3, -1]) cylinder(d = 6.5, h = 3.8); \
             translate([-30.5, 52.3, -1]) cylinder(d = 3.4, h = 10); }",
        );
        assert_eq!(m.open_edges(), 0);
        let bridges = on(HoleAbove::Sacrificial);
        for s in [
            bridges,
            Settings {
                hole_above: HoleAbove::Stepped,
                ..bridges
            },
            Settings {
                hole_size: true,
                ..bridges
            },
        ] {
            let (out, rep) = apply(&m, &s);
            assert_eq!(rep.bridges, 2, "{s:?} {rep:?}");
            assert_eq!(out.open_edges(), 0, "{s:?}");
            assert_eq!(moved(&out).open_edges(), 0, "{s:?}");
            assert_eq!(steps_with_no_height(&out), 0, "{s:?}");
        }
    }
}
