//! The Check card (port of `overhang.py` from scad-models): the problems
//! that stay in a part in print frame after the fixes.
//!
//! The check looks at three things:
//!
//! - Down-facing regions above the bed that are steeper than the overhang
//!   limit. A flat region is a bridge. A bridge is a problem when it is
//!   longer than the bridge limit or when its outline has arcs. A sloped
//!   region is an overhang.
//! - Small bed islands (see `brim.rs`).
//! - The fit in the print area.
//!
//! This module also has the outline and 2D helpers that `bridges.rs` and
//! `brim.rs` use.

use super::{Area, Layers, Limits, brim, tri_area};
use crate::mesh::{Bounds, Mesh};
use glam::Vec2;
use std::collections::{HashMap, HashSet};

/// The kind of a problem.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Down-facing faces steeper than the overhang limit, not a bridge.
    Overhang,
    /// A bridge longer than the bridge limit.
    LongBridge,
    /// A bridge with a round outline (an arc in the air).
    Arc,
    /// A small bed island without a brim.
    SmallIsland,
    /// The part does not fit the print area.
    TooLarge,
}

impl Kind {
    /// The order in the Check card: the largest problem first.
    fn rank(self) -> u8 {
        match self {
            Kind::TooLarge => 0,
            Kind::LongBridge => 1,
            Kind::Overhang => 2,
            Kind::Arc => 3,
            Kind::SmallIsland => 4,
        }
    }
}

/// One problem of a part.
#[derive(Clone, Debug)]
pub struct Issue {
    pub kind: Kind,
    /// The triangles of the place, for the 3D view.
    pub tris: Vec<u32>,
    /// Area of the place in mm² (0 when it does not apply).
    pub area: f32,
    /// Short text for the Check card.
    pub text: String,
}

/// Regions smaller than this (mm²) are mesh slivers. The check does not
/// list them (`overhang.py`).
pub const SLIVER_MM2: f32 = 0.5;
/// A face with a normal z below this is flat in the check: within about
/// 8 degrees of horizontal (`overhang.py`).
const FLAT_NZ: f32 = -0.99;
/// A face with a normal z below this is level: within about 1 degree of
/// horizontal. Bed chamfer steps and bridged ceilings are level.
pub(super) const LEVEL_NZ: f32 = -0.99985;
/// A down-facing face must be this far above the lowest point (mm).
const ABOVE_BED: f32 = 0.05;
/// Two flat faces are coplanar when their heights are this near (mm).
const SAME_Z: f32 = 1e-3;
/// Vertices on the same point of a grid of this size (mm) are one vertex
/// in an outline (`overhang.py`). Gaps and slivers of a boolean result then
/// do not make outline points.
const WELD: f32 = 1e-3;
/// An outline point is on an arc when the outline turns by more than
/// `ARC_MIN` and less than `ARC_MAX` degrees there (`overhang.py`).
const ARC_MIN: f32 = 0.5;
const ARC_MAX: f32 = 40.0;

/// The problems of `mesh` (print frame).
pub fn run(mesh: &Mesh, layers: Layers, limits: &Limits, area: &Area) -> Vec<Issue> {
    let Some(b) = mesh.bounds() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Some(i) = too_large(mesh, &b, area) {
        out.push(i);
    }
    small_islands(mesh, layers, &mut out);
    down_regions(mesh, layers, limits, b.min.z, &mut out);
    out.sort_by(|a, b| {
        a.kind
            .rank()
            .cmp(&b.kind.rank())
            .then(b.area.total_cmp(&a.area))
    });
    out
}

/// A `TooLarge` issue when the part does not fit `area` in the 0 or the
/// 90 degree turn, or is higher than the area. The place is all triangles.
fn too_large(mesh: &Mesh, b: &Bounds, area: &Area) -> Option<Issue> {
    let s = b.size();
    let tol = 1e-3;
    let fits_bed = (s.x <= area.w + tol && s.y <= area.d + tol)
        || (s.y <= area.w + tol && s.x <= area.d + tol);
    let fits_height = s.z <= area.h + tol;
    if fits_bed && fits_height {
        return None;
    }
    let why = if !fits_bed && !fits_height {
        "too large and too high"
    } else if !fits_bed {
        "too large"
    } else {
        "too high"
    };
    Some(Issue {
        kind: Kind::TooLarge,
        tris: (0..mesh.triangles.len() as u32).collect(),
        area: 0.0,
        text: format!(
            "Part {:.1} x {:.1} x {:.1} mm is {why} for the print area {:.0} x {:.0} x {:.0} mm",
            s.x, s.y, s.z, area.w, area.d, area.h
        ),
    })
}

/// A `SmallIsland` issue for each small bed island. An island smaller
/// than `SLIVER_MM2` is a mesh sliver, for example the tip of a cusp that
/// the bed chamfer keeps: it is not listed.
fn small_islands(mesh: &Mesh, layers: Layers, out: &mut Vec<Issue>) {
    let islands = brim::islands(mesh, layers);
    let listed = |i: &brim::Island| i.small() && i.area >= SLIVER_MM2;
    if !islands.iter().any(listed) {
        return;
    }
    let z = brim::island_z(mesh, layers);
    let owner = island_triangles(mesh, &islands, z);
    for (k, isl) in islands.iter().enumerate() {
        if !listed(isl) {
            continue;
        }
        let c = isl.center();
        out.push(Issue {
            kind: Kind::SmallIsland,
            tris: owner
                .iter()
                .filter(|(_, o)| *o == k)
                .map(|(t, _)| *t)
                .collect(),
            area: isl.area,
            text: format!(
                "Small bed island {:.1} mm², {:.1} mm wide, at x {:.1}, y {:.1}",
                isl.area, isl.width, c.x, c.y
            ),
        });
    }
}

/// The triangles that cut the plane `z` or lie on the bed, with the index
/// of the island that each one belongs to.
fn island_triangles(mesh: &Mesh, islands: &[brim::Island], z: f32) -> Vec<(u32, usize)> {
    let boxes: Vec<(Vec2, Vec2)> = islands.iter().map(|i| bbox(&i.outer)).collect();
    let loops: Vec<Vec<Vec<Vec2>>> = islands.iter().map(brim::Island::loops).collect();
    let mut out = Vec::new();
    for ti in 0..mesh.triangles.len() {
        let [a, b, c] = mesh.tri(ti);
        let lo = a.z.min(b.z).min(c.z);
        let hi = a.z.max(b.z).max(c.z);
        if !(lo < z && hi >= z) && hi > z {
            continue;
        }
        // A point of the triangle at the height z (or its center on the bed).
        let p = if hi >= z && lo < z {
            let mut pts = Vec::new();
            for (p, q) in [(a, b), (b, c), (c, a)] {
                if (p.z < z) != (q.z < z) {
                    let t = (z - p.z) / (q.z - p.z);
                    pts.push(p.truncate() + (q.truncate() - p.truncate()) * t);
                }
            }
            if pts.len() < 2 {
                continue;
            }
            (pts[0] + pts[1]) * 0.5
        } else {
            ((a + b + c) / 3.0).truncate()
        };
        let mut best = (f32::MAX, usize::MAX);
        for (k, l) in loops.iter().enumerate() {
            let (lo, hi) = boxes[k];
            if p.x < lo.x - 0.1 || p.y < lo.y - 0.1 || p.x > hi.x + 0.1 || p.y > hi.y + 0.1 {
                continue;
            }
            let d = if inside(p, l) {
                0.0
            } else {
                l.iter().map(|l| loop_dist(p, l)).fold(f32::MAX, f32::min)
            };
            if d < best.0 {
                best = (d, k);
            }
        }
        if best.0 < 0.05 {
            out.push((ti as u32, best.1));
        }
    }
    out
}

/// The down-facing regions: `LongBridge`, `Arc` and `Overhang` issues.
fn down_regions(mesh: &Mesh, layers: Layers, limits: &Limits, zmin: f32, out: &mut Vec<Issue>) {
    // A face steeper than the limit: the normal is nearer to -z than
    // sin(limit). Faces at exactly the limit (45 degree teardrops) stay out.
    // Small slivers stay in: they close the gaps in the outline of a flat
    // region. Only triangles without a normal go out.
    let nz_max = -limits.overhang_deg.to_radians().sin() - 1e-4;
    let down: Vec<u32> = (0..mesh.triangles.len())
        .filter(|&i| {
            let [a, b, c] = mesh.tri(i);
            mesh.normal(i).z < nz_max && a.z.min(b.z).min(c.z) > zmin + ABOVE_BED
        })
        .map(|i| i as u32)
        .collect();
    // Thin level regions are steps: the steps of the bed chamfer, or a
    // stair that is not steeper than 45 degrees. They are not problems.
    let level: Vec<u32> = down
        .iter()
        .copied()
        .filter(|&t| mesh.normal(t as usize).z < LEVEL_NZ)
        .collect();
    // The bottom of a sheet with no thickness is no overhang.
    let sheet = sheet_bottoms(mesh, &level);
    let level: Vec<u32> = level.into_iter().filter(|t| !sheet.contains(t)).collect();
    let step_w = 1.5 * layers.first.max(layers.height);
    let mut steps = HashSet::new();
    for g in coplanar_groups(mesh, &level) {
        let a: f32 = g.iter().map(|&t| tri_area(mesh, t as usize)).sum();
        let p = outline_length(mesh, &g);
        if p > 0.0 && 2.0 * a / p <= step_w {
            steps.extend(g);
        }
    }
    let rest: Vec<u32> = down
        .into_iter()
        .filter(|t| !steps.contains(t) && !sheet.contains(t))
        .collect();
    for g in vertex_groups(mesh, &rest) {
        let area: f32 = g.iter().map(|&t| tri_area(mesh, t as usize)).sum();
        if area < SLIVER_MM2 {
            continue;
        }
        let zs = g.iter().flat_map(|&t| mesh.tri(t as usize)).map(|v| v.z);
        let (zlo, zhi) = zs.fold((f32::MAX, f32::MIN), |(lo, hi), z| (lo.min(z), hi.max(z)));
        let flat = g.iter().all(|&t| mesh.normal(t as usize).z < FLAT_NZ);
        if flat {
            let pts: Vec<Vec2> = g
                .iter()
                .flat_map(|&t| mesh.tri(t as usize))
                .map(|v| v.truncate())
                .collect();
            let span = min_rect(&pts).map_or(0.0, |r| r.width());
            let arcs = group_arcs(mesh, &g);
            let z = (zlo + zhi) * 0.5;
            if span > limits.bridge_mm {
                let more = if arcs > 0 {
                    format!(", {arcs} arc points")
                } else {
                    String::new()
                };
                out.push(Issue {
                    kind: Kind::LongBridge,
                    tris: g,
                    area,
                    text: format!(
                        "Bridge {span:.1} mm, longer than {:.0} mm, at z {z:.2}{more}",
                        limits.bridge_mm
                    ),
                });
            } else if arcs > 0 {
                out.push(Issue {
                    kind: Kind::Arc,
                    tris: g,
                    area,
                    text: format!(
                        "Bridge with a round outline ({arcs} arc points), {area:.1} mm² at z {z:.2}"
                    ),
                });
            }
        } else {
            let steep = g
                .iter()
                .map(|&t| {
                    (-mesh.normal(t as usize).z)
                        .clamp(-1.0, 1.0)
                        .asin()
                        .to_degrees()
                })
                .fold(0.0, f32::max);
            out.push(Issue {
                kind: Kind::Overhang,
                tris: g,
                area,
                text: format!(
                    "Overhang {area:.1} mm², up to {steep:.0} degrees, at z {zlo:.2} to {zhi:.2}"
                ),
            });
        }
    }
}

/// The level down-facing triangles of `tris` that lie on a level up-facing
/// triangle at the same height: the bottom of a sheet with no thickness.
/// Two boolean cuts that only touch leave such a sheet, for example the
/// layers of `bed_cut()` in scad-models, where -1 + 1.2 is not exactly 0.2.
pub(super) fn sheet_bottoms(mesh: &Mesh, tris: &[u32]) -> HashSet<u32> {
    let mean_z = |i: usize| {
        let [a, b, c] = mesh.tri(i);
        (a.z + b.z + c.z) / 3.0
    };
    let key = |z: f32| (z / SAME_Z).round() as i64;
    let mut ups: HashMap<i64, Vec<usize>> = HashMap::new();
    for i in 0..mesh.triangles.len() {
        if mesh.normal(i).z > -LEVEL_NZ {
            ups.entry(key(mean_z(i))).or_default().push(i);
        }
    }
    let mut out = HashSet::new();
    for &t in tris {
        let z = mean_z(t as usize);
        let [a, b, c] = mesh.tri(t as usize);
        let p = ((a + b + c) / 3.0).truncate();
        let k = key(z);
        let on_up = (k - 1..=k + 1)
            .filter_map(|k| ups.get(&k))
            .flatten()
            .any(|&u| {
                let [a, b, c] = mesh.tri(u);
                (mean_z(u) - z).abs() < SAME_Z
                    && in_triangle(p, a.truncate(), b.truncate(), c.truncate())
            });
        if on_up {
            out.insert(t);
        }
    }
    out
}

/// True when `p` is in the triangle `a`, `b`, `c` or on its edge.
fn in_triangle(p: Vec2, a: Vec2, b: Vec2, c: Vec2) -> bool {
    let d = [
        (b - a).perp_dot(p - a),
        (c - b).perp_dot(p - b),
        (a - c).perp_dot(p - c),
    ];
    !(d.iter().any(|&x| x < 0.0) && d.iter().any(|&x| x > 0.0))
}

// ---------------------------------------------------------------- groups

/// A small union-find over `0..n`.
struct Sets(Vec<usize>);

impl Sets {
    fn new(n: usize) -> Sets {
        Sets((0..n).collect())
    }

    fn find(&mut self, mut i: usize) -> usize {
        while self.0[i] != i {
            self.0[i] = self.0[self.0[i]];
            i = self.0[i];
        }
        i
    }

    fn join(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            self.0[b] = a;
        }
    }

    /// The groups of the items, in the order of their first item.
    fn groups(mut self, items: &[u32]) -> Vec<Vec<u32>> {
        let mut index: HashMap<usize, usize> = HashMap::new();
        let mut out: Vec<Vec<u32>> = Vec::new();
        for (i, &t) in items.iter().enumerate() {
            let r = self.find(i);
            let k = *index.entry(r).or_insert_with(|| {
                out.push(Vec::new());
                out.len() - 1
            });
            out[k].push(t);
        }
        out
    }
}

/// The vertices of a set of triangles, welded: vertices on the same point
/// of the `WELD` grid get one id (the first vertex index).
struct Weld(HashMap<u32, u32>);

impl Weld {
    fn new(mesh: &Mesh, tris: &[u32]) -> Weld {
        let mut by_key: HashMap<[i64; 3], u32> = HashMap::new();
        let mut map = HashMap::new();
        for &t in tris {
            for v in mesh.triangles[t as usize] {
                map.entry(v).or_insert_with(|| {
                    let p = mesh.vertices[v as usize] / WELD;
                    let key = [p.x.round() as i64, p.y.round() as i64, p.z.round() as i64];
                    *by_key.entry(key).or_insert(v)
                });
            }
        }
        Weld(map)
    }

    /// The welded corners of triangle `t`.
    fn tri(&self, mesh: &Mesh, t: u32) -> [u32; 3] {
        mesh.triangles[t as usize].map(|v| self.0[&v])
    }

    /// The welded edges of triangle `t`, in its direction. An edge that the
    /// weld makes zero long is not in the list.
    fn edges(&self, mesh: &Mesh, t: u32) -> impl Iterator<Item = (u32, u32)> {
        let tv = self.tri(mesh, t);
        (0..3)
            .map(move |k| (tv[k], tv[(k + 1) % 3]))
            .filter(|(a, b)| a != b)
    }
}

/// Groups the triangles `tris` that share a vertex (`overhang.py`).
pub(super) fn vertex_groups(mesh: &Mesh, tris: &[u32]) -> Vec<Vec<u32>> {
    let weld = Weld::new(mesh, tris);
    let mut sets = Sets::new(tris.len());
    let mut first: HashMap<u32, usize> = HashMap::new();
    for (i, &t) in tris.iter().enumerate() {
        for v in weld.tri(mesh, t) {
            match first.get(&v) {
                Some(&j) => sets.join(j, i),
                None => {
                    first.insert(v, i);
                }
            }
        }
    }
    sets.groups(tris)
}

/// Groups the level triangles `tris` that share an edge and lie at the
/// same height.
pub(super) fn coplanar_groups(mesh: &Mesh, tris: &[u32]) -> Vec<Vec<u32>> {
    let z = |t: u32| {
        let [a, b, c] = mesh.tri(t as usize);
        (a.z + b.z + c.z) / 3.0
    };
    let weld = Weld::new(mesh, tris);
    let mut sets = Sets::new(tris.len());
    let mut first: HashMap<(u32, u32), usize> = HashMap::new();
    for (i, &t) in tris.iter().enumerate() {
        for (a, b) in weld.edges(mesh, t) {
            let e = (a.min(b), a.max(b));
            match first.get(&e) {
                Some(&j) => {
                    if (z(tris[j]) - z(t)).abs() < SAME_Z {
                        sets.join(j, i);
                    }
                }
                None => {
                    first.insert(e, i);
                }
            }
        }
    }
    sets.groups(tris)
}

// ---------------------------------------------------------------- outlines

/// The edges of `tris` that only one triangle of the set uses, with the
/// direction of that triangle. The vertices are welded (`Weld`).
fn outline_edges(mesh: &Mesh, tris: &[u32]) -> Vec<(u32, u32)> {
    let weld = Weld::new(mesh, tris);
    let mut count: HashMap<(u32, u32), u32> = HashMap::new();
    for &t in tris {
        for (a, b) in weld.edges(mesh, t) {
            *count.entry((a.min(b), a.max(b))).or_default() += 1;
        }
    }
    let mut out = Vec::new();
    for &t in tris {
        for (a, b) in weld.edges(mesh, t) {
            if count[&(a.min(b), a.max(b))] == 1 {
                out.push((a, b));
            }
        }
    }
    out
}

/// The total length of the outline of `tris` (mm).
fn outline_length(mesh: &Mesh, tris: &[u32]) -> f32 {
    outline_edges(mesh, tris)
        .iter()
        .map(|&(a, b)| mesh.vertices[a as usize].distance(mesh.vertices[b as usize]))
        .sum()
}

/// The outline of the triangles `tris`: closed loops of vertex indices, in
/// the direction of the triangles. Also returns the vertices where more
/// than one loop meets (pinch points).
pub(super) fn outline_loops(mesh: &Mesh, tris: &[u32]) -> (Vec<Vec<u32>>, HashSet<u32>) {
    let edges = outline_edges(mesh, tris);
    let mut next: HashMap<u32, Vec<u32>> = HashMap::new();
    for &(a, b) in &edges {
        next.entry(a).or_default().push(b);
    }
    let pinch: HashSet<u32> = next
        .iter()
        .filter(|(_, v)| v.len() > 1)
        .map(|(k, _)| *k)
        .collect();
    let mut starts: Vec<u32> = next.keys().copied().collect();
    starts.sort_unstable();
    let mut loops = Vec::new();
    for s in starts {
        while let Some(b) = next.get_mut(&s).and_then(Vec::pop) {
            let mut lp = vec![s];
            let mut cur = b;
            let mut closed = false;
            for _ in 0..=edges.len() {
                if cur == s {
                    closed = true;
                    break;
                }
                lp.push(cur);
                match next.get_mut(&cur).and_then(Vec::pop) {
                    Some(n) => cur = n,
                    None => break,
                }
            }
            if closed && lp.len() >= 3 {
                loops.push(lp);
            }
        }
    }
    (loops, pinch)
}

/// The turn of the outline at `v` (from `a`, to `b`) in degrees, seen
/// from above. `None` for an edge shorter than 1e-6 mm.
fn turn_deg(a: Vec2, v: Vec2, b: Vec2) -> Option<f32> {
    let (u, w) = (v - a, b - v);
    let (lu, lw) = (u.length(), w.length());
    if lu < 1e-6 || lw < 1e-6 {
        return None;
    }
    Some((u.dot(w) / (lu * lw)).clamp(-1.0, 1.0).acos().to_degrees())
}

/// True for a turn of an outline on an arc.
fn is_arc(deg: f32) -> bool {
    deg > ARC_MIN && deg < ARC_MAX
}

/// The number of points of the closed loop `pts` that are on an arc.
pub(super) fn arc_points(pts: &[Vec2]) -> usize {
    let n = pts.len();
    if n < 3 {
        return 0;
    }
    (0..n)
        .filter(|&i| turn_deg(pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n]).is_some_and(is_arc))
        .count()
}

/// The number of outline points of the triangles `tris` that are on an
/// arc. Pinch points do not count (`overhang.py`).
pub(super) fn group_arcs(mesh: &Mesh, tris: &[u32]) -> usize {
    let (loops, pinch) = outline_loops(mesh, tris);
    let xy = |v: u32| mesh.vertices[v as usize].truncate();
    let mut n = 0;
    for lp in &loops {
        let m = lp.len();
        for i in 0..m {
            if pinch.contains(&lp[i]) {
                continue;
            }
            let t = turn_deg(xy(lp[(i + m - 1) % m]), xy(lp[i]), xy(lp[(i + 1) % m]));
            if t.is_some_and(is_arc) {
                n += 1;
            }
        }
    }
    n
}

// ---------------------------------------------------------------- 2D

/// True when `p` is inside the loops (even-odd rule).
pub(super) fn inside(p: Vec2, loops: &[Vec<Vec2>]) -> bool {
    loops.iter().filter(|l| inside_loop(p, l)).count() % 2 == 1
}

/// True when `p` is inside the closed loop `l`.
pub(super) fn inside_loop(p: Vec2, l: &[Vec2]) -> bool {
    let mut c = false;
    let n = l.len();
    for i in 0..n {
        let (a, b) = (l[(i + n - 1) % n], l[i]);
        if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) * (b.x - a.x) / (b.y - a.y) {
            c = !c;
        }
    }
    c
}

/// The distance from `p` to the segment `a`-`b`.
pub(super) fn seg_dist(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let d = b - a;
    let ll = d.length_squared();
    let s = if ll == 0.0 {
        0.0
    } else {
        ((p - a).dot(d) / ll).clamp(0.0, 1.0)
    };
    p.distance(a + d * s)
}

/// The distance from `p` to the closed loop `l`.
pub(super) fn loop_dist(p: Vec2, l: &[Vec2]) -> f32 {
    let n = l.len();
    (0..n)
        .map(|i| seg_dist(p, l[(i + n - 1) % n], l[i]))
        .fold(f32::MAX, f32::min)
}

/// True when the segments `a`-`b` and `c`-`d` cross or touch.
fn segs_cross(a: Vec2, b: Vec2, c: Vec2, d: Vec2) -> bool {
    let o = |p: Vec2, q: Vec2, r: Vec2| (q - p).perp_dot(r - p);
    let (d1, d2) = (o(c, d, a), o(c, d, b));
    let (d3, d4) = (o(a, b, c), o(a, b, d));
    (d1 > 0.0) != (d2 > 0.0) && (d3 > 0.0) != (d4 > 0.0)
}

/// The distance between the segments `a`-`b` and `c`-`d`.
fn seg_seg_dist(a: Vec2, b: Vec2, c: Vec2, d: Vec2) -> f32 {
    if segs_cross(a, b, c, d) {
        return 0.0;
    }
    seg_dist(a, c, d)
        .min(seg_dist(b, c, d))
        .min(seg_dist(c, a, b))
        .min(seg_dist(d, a, b))
}

/// The smallest and the largest point of `pts`.
pub(super) fn bbox(pts: &[Vec2]) -> (Vec2, Vec2) {
    pts.iter().fold(
        (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)),
        |(lo, hi), p| (lo.min(*p), hi.max(*p)),
    )
}

/// The convex hull of `pts`, counter clockwise.
pub(super) fn hull(pts: &[Vec2]) -> Vec<Vec2> {
    let mut p: Vec<Vec2> = pts.to_vec();
    p.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    p.dedup_by(|a, b| a.distance(*b) < 1e-6);
    if p.len() < 3 {
        return p;
    }
    let cross = |o: Vec2, a: Vec2, b: Vec2| (a - o).perp_dot(b - o);
    let mut h: Vec<Vec2> = Vec::with_capacity(p.len() * 2);
    for &q in p.iter().chain(p.iter().rev().skip(1)) {
        while h.len() >= 2 && cross(h[h.len() - 2], h[h.len() - 1], q) <= 0.0 {
            h.pop();
        }
        h.push(q);
    }
    h.pop();
    h
}

/// A rectangle: center `c`, unit axis `u`, and half sizes along `u` and
/// along `u.perp()`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub c: Vec2,
    pub u: Vec2,
    pub half: Vec2,
}

impl Rect {
    /// An axis-aligned rectangle from `lo` to `hi`.
    pub fn aligned(lo: Vec2, hi: Vec2) -> Rect {
        Rect {
            c: (lo + hi) * 0.5,
            u: Vec2::X,
            half: (hi - lo) * 0.5,
        }
    }

    pub fn v(&self) -> Vec2 {
        self.u.perp()
    }

    /// The corners, counter clockwise.
    pub fn corners(&self) -> Vec<Vec2> {
        let (u, v) = (self.u * self.half.x, self.v() * self.half.y);
        vec![
            self.c - u - v,
            self.c + u - v,
            self.c + u + v,
            self.c - u + v,
        ]
    }

    /// The short side.
    pub fn width(&self) -> f32 {
        2.0 * self.half.min_element()
    }

    /// The long side.
    pub fn length(&self) -> f32 {
        2.0 * self.half.max_element()
    }

    pub fn area(&self) -> f32 {
        4.0 * self.half.x * self.half.y
    }

    /// True when `p` is in the rectangle or on its edge.
    pub fn contains(&self, p: Vec2) -> bool {
        let d = p - self.c;
        d.dot(self.u).abs() <= self.half.x + 1e-5 && d.dot(self.v()).abs() <= self.half.y + 1e-5
    }

    /// The distance from the rectangle to the closed loop `l` (0 when they
    /// touch or overlap).
    pub fn dist_to_loop(&self, l: &[Vec2]) -> f32 {
        let k = self.corners();
        let n = l.len();
        let mut best = f32::MAX;
        for i in 0..n {
            let (a, b) = (l[(i + n - 1) % n], l[i]);
            if self.contains(a) || self.contains(b) {
                return 0.0;
            }
            for j in 0..4 {
                best = best.min(seg_seg_dist(a, b, k[j], k[(j + 1) % 4]));
            }
        }
        best
    }
}

/// The bounding rectangle of `pts` with the smallest area (rotating
/// calipers on the convex hull). When the axis-aligned box is less than 1 %
/// larger, it is the result, so that a circle gets a square along x and y.
pub fn min_rect(pts: &[Vec2]) -> Option<Rect> {
    if pts.is_empty() {
        return None;
    }
    let (lo, hi) = bbox(pts);
    let aligned = Rect::aligned(lo, hi);
    let h = hull(pts);
    if h.len() < 3 {
        return Some(aligned);
    }
    let mut best = aligned;
    let mut best_area = f32::MAX;
    for i in 0..h.len() {
        let e = h[(i + 1) % h.len()] - h[i];
        if e.length() < 1e-6 {
            continue;
        }
        let u = e.normalize();
        let v = u.perp();
        let (mut ulo, mut uhi, mut vlo, mut vhi) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
        for p in &h {
            let (a, b) = (p.dot(u), p.dot(v));
            ulo = ulo.min(a);
            uhi = uhi.max(a);
            vlo = vlo.min(b);
            vhi = vhi.max(b);
        }
        let area = (uhi - ulo) * (vhi - vlo);
        if area < best_area {
            best_area = area;
            best = Rect {
                c: u * (ulo + uhi) * 0.5 + v * (vlo + vhi) * 0.5,
                u,
                half: Vec2::new((uhi - ulo) * 0.5, (vhi - vlo) * 0.5),
            };
        }
    }
    if aligned.area() <= best_area * 1.01 + 1e-6 {
        return Some(aligned);
    }
    Some(best)
}

#[cfg(test)]
pub(super) mod test_util {
    //! Meshes for the tests of the optimize modules.

    use crate::mesh::Mesh;
    use glam::{Vec2, Vec3};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    /// The triangles of a prism: the convex polygon `poly` (counter
    /// clockwise) from `z0` up to `z1`.
    pub fn prism(poly: &[Vec2], z0: f32, z1: f32) -> Vec<[Vec3; 3]> {
        let n = poly.len();
        let lo = |i: usize| poly[i % n].extend(z0);
        let hi = |i: usize| poly[i % n].extend(z1);
        let mut s = Vec::new();
        for i in 1..n - 1 {
            s.push([lo(0), lo(i + 1), lo(i)]);
            s.push([hi(0), hi(i), hi(i + 1)]);
        }
        for i in 0..n {
            s.push([lo(i), lo(i + 1), hi(i + 1)]);
            s.push([lo(i), hi(i + 1), hi(i)]);
        }
        s
    }

    /// The triangles of a box from `lo` to `hi`.
    pub fn block(lo: Vec3, hi: Vec3) -> Vec<[Vec3; 3]> {
        let r = [
            Vec2::new(lo.x, lo.y),
            Vec2::new(hi.x, lo.y),
            Vec2::new(hi.x, hi.y),
            Vec2::new(lo.x, hi.y),
        ];
        prism(&r, lo.z, hi.z)
    }

    /// A regular polygon with `n` points on a circle of radius `r`.
    pub fn ngon(c: Vec2, r: f32, n: usize) -> Vec<Vec2> {
        (0..n)
            .map(|i| {
                let a = std::f32::consts::TAU * i as f32 / n as f32;
                c + Vec2::new(a.cos(), a.sin()) * r
            })
            .collect()
    }

    /// A mesh of separate closed shapes.
    pub fn mesh(parts: &[Vec<[Vec3; 3]>]) -> Mesh {
        Mesh::from_soup(&parts.concat())
    }

    /// The mesh that OpenSCAD makes from `src`.
    pub fn render(src: &str) -> Mesh {
        static N: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "cocad-opt-test-{}-{}.scad",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, src).unwrap();
        let cfg = crate::openscad::Config::default();
        let out = crate::openscad::render(&cfg, &path, &[], &Arc::new(AtomicBool::new(false)));
        let _ = std::fs::remove_file(&path);
        out.unwrap().mesh
    }

    /// The area of the material in the section of `mesh` at `z`.
    pub fn section_area(mesh: &Mesh, z: f32) -> f32 {
        super::super::section(mesh, z)
            .iter()
            .map(|l| super::super::loop_area(l))
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::test_util::*;
    use super::*;
    use glam::Vec3;

    fn kinds(issues: &[Issue]) -> Vec<Kind> {
        issues.iter().map(|i| i.kind).collect()
    }

    #[test]
    fn a_cube_has_no_problems() {
        let m = crate::mesh::cube(10.0);
        let i = run(&m, Layers::default(), &Limits::default(), &Area::default());
        assert!(i.is_empty(), "{i:?}");
    }

    #[test]
    fn a_wide_flat_ceiling_is_a_long_bridge() {
        // A column with a slab 30 x 20 at z 10 that is not joined to it.
        let m = mesh(&[
            block(Vec3::new(0.0, 0.0, 0.0), Vec3::new(30.0, 30.0, 5.0)),
            block(Vec3::new(0.0, 0.0, 10.0), Vec3::new(30.0, 20.0, 12.0)),
        ]);
        let i = run(&m, Layers::default(), &Limits::default(), &Area::default());
        assert_eq!(kinds(&i), vec![Kind::LongBridge], "{i:?}");
        assert!((i[0].area - 600.0).abs() < 0.1);
        assert!(i[0].text.contains("20.0 mm"), "{}", i[0].text);
        assert_eq!(i[0].tris.len(), 2);
    }

    #[test]
    fn a_short_straight_bridge_is_no_problem() {
        let m = mesh(&[
            block(Vec3::new(0.0, 0.0, 0.0), Vec3::new(30.0, 30.0, 5.0)),
            block(Vec3::new(0.0, 0.0, 10.0), Vec3::new(30.0, 8.0, 12.0)),
        ]);
        let i = run(&m, Layers::default(), &Limits::default(), &Area::default());
        assert!(i.is_empty(), "{i:?}");
    }

    #[test]
    fn a_round_ceiling_has_arcs() {
        // A floating disk of 24 points: each point turns by 15 degrees.
        let disk = ngon(Vec2::new(15.0, 15.0), 5.0, 24);
        let m = mesh(&[
            block(Vec3::new(0.0, 0.0, 0.0), Vec3::new(30.0, 30.0, 5.0)),
            prism(&disk, 10.0, 12.0),
        ]);
        let i = run(&m, Layers::default(), &Limits::default(), &Area::default());
        assert_eq!(kinds(&i), vec![Kind::Arc], "{i:?}");
        assert!(i[0].text.contains("24 arc points"), "{}", i[0].text);
    }

    #[test]
    fn a_coarse_polygon_has_no_arcs() {
        // A hexagon turns by 60 degrees: straight edges, as overhang.py.
        let hex = ngon(Vec2::new(15.0, 15.0), 5.0, 6);
        let m = mesh(&[
            block(Vec3::new(0.0, 0.0, 0.0), Vec3::new(30.0, 30.0, 5.0)),
            prism(&hex, 10.0, 12.0),
        ]);
        let i = run(&m, Layers::default(), &Limits::default(), &Area::default());
        assert!(i.is_empty(), "{i:?}");
    }

    #[test]
    fn a_sloped_underside_is_an_overhang_and_45_degrees_is_not() {
        // A wedge on a block: the underside rises 1 mm for each `run` mm.
        let wedge = |run: f32| {
            let a = Vec3::new(10.0, 0.0, 10.0);
            let b = Vec3::new(10.0 + run * 5.0, 0.0, 15.0);
            let c = Vec3::new(10.0, 0.0, 15.0);
            let (a2, b2, c2) = (a + Vec3::Y * 10.0, b + Vec3::Y * 10.0, c + Vec3::Y * 10.0);
            vec![
                [a, b, c],
                [a2, c2, b2],
                [a, b2, b],
                [a, a2, b2],
                [b, b2, c2],
                [b, c2, c],
                [c, c2, a2],
                [c, a2, a],
            ]
        };
        let base = block(Vec3::ZERO, Vec3::new(10.0, 10.0, 15.0));
        let steep = mesh(&[base.clone(), wedge(2.0)]);
        let i = run(
            &steep,
            Layers::default(),
            &Limits::default(),
            &Area::default(),
        );
        assert_eq!(kinds(&i), vec![Kind::Overhang], "{i:?}");
        assert!(i[0].text.contains("63 degrees"), "{}", i[0].text);
        let at45 = mesh(&[base, wedge(1.0)]);
        let i = run(
            &at45,
            Layers::default(),
            &Limits::default(),
            &Area::default(),
        );
        assert!(i.is_empty(), "{i:?}");
    }

    #[test]
    fn thin_flat_steps_and_slivers_are_no_problem() {
        // A step 0.2 mm wide and a flat face of 0.3 mm², both in the air.
        let m = mesh(&[
            block(Vec3::ZERO, Vec3::new(30.0, 30.0, 5.0)),
            block(Vec3::new(0.0, 0.0, 8.0), Vec3::new(0.2, 30.0, 9.0)),
            block(Vec3::new(10.0, 10.0, 8.0), Vec3::new(10.5, 10.6, 9.0)),
        ]);
        let i = run(&m, Layers::default(), &Limits::default(), &Area::default());
        assert!(i.is_empty(), "{i:?}");
    }

    #[test]
    fn a_face_on_a_face_is_no_bridge() {
        // Two blocks on each other, not joined: the bottom of the upper
        // block lies on the top of the lower one.
        let m = mesh(&[
            block(Vec3::ZERO, Vec3::new(30.0, 30.0, 5.0)),
            block(Vec3::new(0.0, 0.0, 5.0), Vec3::new(30.0, 30.0, 10.0)),
        ]);
        let i = run(&m, Layers::default(), &Limits::default(), &Area::default());
        assert!(i.is_empty(), "{i:?}");
    }

    #[test]
    fn the_fit_tries_the_quarter_turn() {
        let area = Area {
            w: 250.0,
            d: 210.0,
            h: 220.0,
        };
        let check = |x: f32, y: f32, z: f32| {
            let m = mesh(&[block(Vec3::ZERO, Vec3::new(x, y, z))]);
            run(&m, Layers::default(), &Limits::default(), &area)
                .iter()
                .any(|i| i.kind == Kind::TooLarge)
        };
        assert!(!check(240.0, 100.0, 10.0));
        assert!(!check(100.0, 240.0, 10.0));
        assert!(check(240.0, 215.0, 10.0));
        assert!(check(100.0, 100.0, 230.0));
    }

    #[test]
    fn a_thin_wall_on_the_bed_is_a_small_island() {
        let m = mesh(&[
            block(Vec3::ZERO, Vec3::new(30.0, 30.0, 2.0)),
            block(Vec3::new(35.0, 0.0, 0.0), Vec3::new(35.8, 20.0, 10.0)),
        ]);
        let i = run(&m, Layers::default(), &Limits::default(), &Area::default());
        assert_eq!(kinds(&i), vec![Kind::SmallIsland], "{i:?}");
        assert!((i[0].area - 16.0).abs() < 0.15, "{}", i[0].area);
        // The four walls and the bottom of the thin wall.
        assert_eq!(i[0].tris.len(), 10, "{:?}", i[0].tris);
    }

    #[test]
    fn a_sliver_on_the_bed_is_no_small_island() {
        // A block, and a spike of 0.2 x 0.2 mm (0.04 mm²) on the bed.
        let m = mesh(&[
            block(Vec3::ZERO, Vec3::new(30.0, 30.0, 2.0)),
            block(Vec3::new(35.0, 0.0, 0.0), Vec3::new(35.2, 0.2, 1.0)),
        ]);
        let i = run(&m, Layers::default(), &Limits::default(), &Area::default());
        assert!(i.iter().all(|i| i.kind != Kind::SmallIsland), "{i:?}");
    }

    #[test]
    fn min_rect_finds_a_turned_rectangle() {
        let a = 0.5f32;
        let (u, v) = (Vec2::new(a.cos(), a.sin()), Vec2::new(-a.sin(), a.cos()));
        let pts: Vec<Vec2> = [(0.0, 0.0), (8.0, 0.0), (8.0, 3.0), (0.0, 3.0)]
            .iter()
            .map(|&(x, y)| u * x + v * y)
            .collect();
        let r = min_rect(&pts).unwrap();
        assert!((r.width() - 3.0).abs() < 1e-3, "{r:?}");
        assert!((r.length() - 8.0).abs() < 1e-3, "{r:?}");
        // A circle gets the square along x and y.
        let r = min_rect(&ngon(Vec2::ZERO, 5.0, 48)).unwrap();
        assert_eq!(r.u, Vec2::X);
        assert!((r.width() - 10.0).abs() < 1e-3, "{r:?}");
    }

    #[test]
    fn rect_distance_to_a_loop() {
        let r = Rect::aligned(Vec2::ZERO, Vec2::new(10.0, 10.0));
        let far = vec![
            Vec2::new(12.0, 0.0),
            Vec2::new(20.0, 0.0),
            Vec2::new(20.0, 10.0),
            Vec2::new(12.0, 10.0),
        ];
        assert!((r.dist_to_loop(&far) - 2.0).abs() < 1e-5);
        let across = vec![
            Vec2::new(-1.0, 5.0),
            Vec2::new(11.0, 5.0),
            Vec2::new(5.0, 20.0),
        ];
        assert_eq!(r.dist_to_loop(&across), 0.0);
    }

    #[test]
    #[ignore = "needs openscad-nightly"]
    fn a_horizontal_round_hole_is_an_overhang() {
        let m = render(
            "difference() { translate([-15, -10, 0]) cube([30, 20, 20]); \
             translate([-20, 0, 10]) rotate([0, 90, 0]) cylinder(d = 6, h = 40, $fn = 32); }",
        );
        let i = run(&m, Layers::default(), &Limits::default(), &Area::default());
        assert_eq!(kinds(&i), vec![Kind::Overhang], "{i:?}");
        // The 8 faces of the hole that are less than 45 degrees from the
        // top: 8 x 0.588 mm wide, 30 mm long.
        assert!((i[0].area - 141.1).abs() < 3.0, "{i:?}");
    }

    #[test]
    #[ignore = "needs openscad-nightly"]
    fn the_bed_chamfer_steps_are_no_problem() {
        // A bed chamfer of 0.4 mm in two layers, on the outside and in a
        // hole on the bed, as `bed_cut()` makes it.
        let m = render(
            "$fn = 32; difference() { cube([30, 30, 10]); \
             translate([15, 15, -1]) cylinder(d = 8, h = 20); \
             difference() { translate([-1, -1, -1]) cube([32, 32, 1.2]); \
                translate([0.4, 0.4, -2]) cube([29.2, 29.2, 5]); } \
             difference() { translate([-1, -1, 0.19]) cube([32, 32, 0.21]); \
                translate([0.2, 0.2, -2]) cube([29.6, 29.6, 5]); } \
             translate([15, 15, -1]) cylinder(d = 8.8, h = 1.2); \
             translate([15, 15, 0.19]) cylinder(d = 8.4, h = 0.21); }",
        );
        assert!(section_area(&m, 0.1) < section_area(&m, 1.0) - 10.0);
        let i = run(&m, Layers::default(), &Limits::default(), &Area::default());
        assert!(i.is_empty(), "{i:?}");
    }

    #[test]
    #[ignore = "needs openscad-nightly"]
    fn the_cocad_bed_chamfer_is_no_problem() {
        use super::super::fixes::{Settings, apply};
        use std::sync::Arc;
        use std::sync::atomic::AtomicBool;
        let m = render(
            "difference() { cube([30, 30, 10]); translate([15, 15, -1]) cylinder(d = 8, h = 20); }",
        );
        let s = Settings {
            chamfer: true,
            ..Settings::default()
        };
        let (out, rep) = apply(
            &crate::openscad::Config::default(),
            &m,
            &s,
            Layers::default(),
            &Limits::default(),
            &Area::default(),
            &Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        assert!(rep.chamfer);
        assert!(section_area(&out, 0.1) < section_area(&out, 1.0) - 10.0);
        let i = run(
            &out,
            Layers::default(),
            &Limits::default(),
            &Area::default(),
        );
        assert!(i.is_empty(), "{i:?}");
    }
}
