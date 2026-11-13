//! Teardrop holes (feature 3 of `docs/optimize-tab.md`).
//!
//! A round hole with its axis parallel to the bed has a ceiling in the air
//! at its top. This module finds these holes in a part in print frame and
//! cuts a 45 degree point up at the top of each one.
//!
//! How CoCAD finds a round surface: OpenSCAD makes it as a prism of flat
//! faces. The edges between these faces (the corner lines) are parallel to
//! the axis. CoCAD collects the corner lines near the bed plane, sorts them
//! by direction, joins the faces that they connect, and fits a circle to
//! the corner lines of each group, in the plane perpendicular to the axis.
//! When the faces point to the center, the surface is a hole. When they
//! point away from the center, it is a pin.

use super::fixes::Plan;
use super::{Layers, Limits, scad};
use crate::assembly::ray_triangle;
use crate::mesh::Mesh;
use glam::{Mat4, Vec2, Vec3};
use std::collections::HashMap;
use std::f32::consts::SQRT_2;

/// The largest angle between a hole axis and the bed plane, in degrees.
const MAX_TILT_DEG: f32 = 10.0;
/// The smallest and the largest hole diameter that gets a point, in mm.
const MIN_D: f32 = 1.0;
const MAX_D: f32 = 40.0;
/// Two faces that turn more than this (degrees) at their edge are not on a
/// round surface. A square has 90 degrees, a pentagon 72 degrees.
const MAX_TURN_DEG: f32 = 80.0;
/// Corner lines closer than this in direction (degrees) are parallel.
const SAME_DIR_DEG: f32 = 0.5;
/// A round surface has at least this number of corner lines on its circle.
const MIN_CORNERS: usize = 5;
/// Two faces with normals closer than this (cosine) are coplanar.
const COPLANAR: f32 = 1.0 - 1e-6;
/// The cut goes this far past each end of the hole, in mm.
const OVERLAP: f32 = 0.02;
/// The positions along the hole for the wall check, as parts of its length.
const RAY_AT: [f32; 5] = [0.1, 0.3, 0.5, 0.7, 0.9];

/// A round surface with its axis near the bed plane: a hole or a pin.
#[derive(Clone, Debug)]
pub struct Round {
    /// The direction of the axis (unit vector).
    pub axis: Vec3,
    /// "Up" in the plane perpendicular to the axis: +Z projected on that
    /// plane (unit vector).
    pub up: Vec3,
    /// `up` x `axis`. (side, up, axis) is a right-handed frame.
    pub side: Vec3,
    /// The center of the circle in the plane: (p . side, p . up).
    pub center: Vec2,
    /// The radius of the circle through the corner lines, in mm.
    pub r: f32,
    /// The extent along the axis: p . axis goes from `t0` to `t1`.
    pub t0: f32,
    pub t1: f32,
    /// True for a hole (the faces point to the center), false for a pin.
    pub hole: bool,
    /// The faces on the circle as 2D segments, relative to the center.
    segments: Vec<(Vec2, Vec2)>,
}

impl Round {
    /// The point on the axis at `t`.
    pub fn at(&self, t: f32) -> Vec3 {
        self.side * self.center.x + self.up * self.center.y + self.axis * t
    }

    pub fn diameter(&self) -> f32 {
        2.0 * self.r
    }

    /// The z of the axis at the middle of the extent.
    pub fn z(&self) -> f32 {
        self.at((self.t0 + self.t1) * 0.5).z
    }

    /// The highest z of the top line of the circle.
    pub fn top_z(&self) -> f32 {
        let lift = self.up * self.r;
        (self.at(self.t0) + lift).z.max((self.at(self.t1) + lift).z)
    }

    /// True when the faces cover the top of the circle: all angles from 45
    /// to 135 degrees (90 degrees is straight up).
    pub fn covers_top(&self) -> bool {
        let arcs: Vec<(f32, f32)> = self
            .segments
            .iter()
            .map(|&(a, b)| {
                let (ta, tb) = (angle_deg(a), angle_deg(b));
                let span = (tb - ta).rem_euclid(360.0);
                if span > 180.0 {
                    (tb, 360.0 - span)
                } else {
                    (ta, span)
                }
            })
            .collect();
        // One degree of tolerance at each end.
        (0..=88).all(|k| {
            let t = 46.0 + k as f32;
            arcs.iter()
                .any(|&(start, span)| (t - start).rem_euclid(360.0) <= span + 1e-3)
        })
    }

    /// True when the top is already a point: a corner straight up, with an
    /// angle of 90 degrees or less.
    pub fn pointed(&self) -> bool {
        let Some(top) = self
            .segments
            .iter()
            .flat_map(|&(a, b)| [a, b])
            .max_by(|p, q| p.y.total_cmp(&q.y))
        else {
            return false;
        };
        if top.x.abs() > 0.1 * self.r || top.y < 0.5 * self.r {
            return false;
        }
        let eps = 1e-3 + 1e-3 * self.r;
        let (mut left, mut right): (Option<Vec2>, Option<Vec2>) = (None, None);
        for &(a, b) in &self.segments {
            for (p, q) in [(a, b), (b, a)] {
                if p.distance(top) > eps || q.distance(top) <= eps {
                    continue;
                }
                let w = (q - p).normalize();
                if w.y > -1e-3 {
                    // A flat top, not a point.
                    return false;
                }
                // The most open face on each side makes the corner.
                let side = if w.x < 0.0 { &mut left } else { &mut right };
                if side.is_none_or(|s| w.y > s.y) {
                    *side = Some(w);
                }
            }
        }
        match (left, right) {
            (Some(l), Some(r)) => l.dot(r).clamp(-1.0, 1.0).acos().to_degrees() <= 90.5,
            _ => false,
        }
    }

    /// The matrix from the local frame of the cut (x = side, y = up,
    /// z = axis, origin at the center of the circle at `t`) to the part.
    pub fn frame(&self, t: f32) -> Mat4 {
        Mat4::from_cols(
            self.side.extend(0.0),
            self.up.extend(0.0),
            self.axis.extend(0.0),
            self.at(t).extend(1.0),
        )
    }
}

/// The angle of `p` in degrees, 0 to 360.
fn angle_deg(p: Vec2) -> f32 {
    p.y.atan2(p.x).to_degrees().rem_euclid(360.0)
}

/// Finds the horizontal round holes of `mesh` (print frame) and returns the
/// cuts that make them teardrops.
pub fn plan(mesh: &Mesh, layers: Layers, limits: &Limits) -> Plan {
    let mut plan = Plan::default();
    for h in rounds(mesh) {
        let d = h.diameter();
        let keep = h.hole
            && (MIN_D * 0.99..=MAX_D * 1.01).contains(&d)
            && h.top_z() > layers.first
            && h.covers_top()
            && !h.pointed();
        if !keep {
            continue;
        }
        let note = |why: &str| format!("Hole d {:.1} mm at z {:.1}: {why}", d, h.z());
        match above(mesh, &h, limits.min_wall) {
            Above::Ok => {
                plan.cuts.push(cut(&h));
                plan.fixed += 1;
            }
            Above::Thin => plan.skipped.push(note("wall above too thin")),
            Above::Breaks => plan.skipped.push(note("point breaks into the space above")),
            Above::Open => {}
        }
    }
    plan
}

/// The OpenSCAD statement that cuts the point of hole `h`: a prism along
/// the hole with the section (0, 0), (r/√2, r/√2), (0, r√2), (-r/√2, r/√2)
/// around the center, `OVERLAP` longer at each end.
pub fn cut(h: &Round) -> String {
    let a = h.r / SQRT_2;
    let kite = vec![
        Vec2::ZERO,
        Vec2::new(a, a),
        Vec2::new(0.0, 2.0 * a),
        Vec2::new(-a, a),
    ];
    let len = h.t1 - h.t0 + 2.0 * OVERLAP;
    format!(
        "{} linear_extrude(height = {}) {}",
        scad::multmatrix(h.frame(h.t0 - OVERLAP)),
        scad::num(len),
        scad::polygon(&[kite])
    )
}

/// The result of the wall check above a hole.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Above {
    /// The wall above the point is thick enough.
    Ok,
    /// The wall above the point is thinner than the minimum wall.
    Thin,
    /// The point goes into a cavity or through the wall above.
    Breaks,
    /// No ray found a ceiling at the top of the circle.
    Open,
}

/// Casts rays straight up (along `h.up`) from points on the axis and
/// measures the material above the tip of the point (r√2 above the axis).
fn above(mesh: &Mesh, h: &Round, min_wall: f32) -> Above {
    let tip = h.r * SQRT_2;
    // A small step to the side, so that a ray does not go along the corner
    // line at the top of a polygon (it can miss both faces there).
    let step = h.side * (0.01 * h.r + 0.003);
    let (mut ok, mut thin) = (false, false);
    for f in RAY_AT {
        let hits = hits(mesh, h.at(h.t0 + (h.t1 - h.t0) * f) + step, h.up);
        // The first hit is the ceiling of the hole, the second is the end
        // of the material above it.
        let (Some(&ceiling), Some(&end)) = (hits.first(), hits.get(1)) else {
            continue;
        };
        if ceiling > h.r * 1.05 {
            // No ceiling at the top of the circle here.
            continue;
        }
        if end < tip + 1e-3 {
            return Above::Breaks;
        }
        if end - tip < min_wall {
            thin = true;
        } else {
            ok = true;
        }
    }
    if thin {
        Above::Thin
    } else if ok {
        Above::Ok
    } else {
        Above::Open
    }
}

/// The distances along a ray to the faces of `mesh`, sorted, with no
/// doubles (a ray through an edge hits two triangles).
fn hits(mesh: &Mesh, origin: Vec3, dir: Vec3) -> Vec<f32> {
    let mut t: Vec<f32> = (0..mesh.triangles.len())
        .filter_map(|i| ray_triangle(origin, dir, mesh.tri(i)))
        .filter(|&t| t > 1e-4)
        .collect();
    t.sort_by(f32::total_cmp);
    t.dedup_by(|a, b| (*a - *b).abs() < 1e-4);
    t
}

/// A simple union-find.
struct Sets(Vec<u32>);

impl Sets {
    fn new(n: usize) -> Sets {
        Sets((0..n as u32).collect())
    }

    fn find(&mut self, mut i: u32) -> u32 {
        while self.0[i as usize] != i {
            let up = self.0[self.0[i as usize] as usize];
            self.0[i as usize] = up;
            i = up;
        }
        i
    }

    fn union(&mut self, a: u32, b: u32) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            self.0[a.max(b) as usize] = a.min(b);
        }
    }
}

/// An edge of the mesh between two faces of a possible round surface.
struct Line {
    tris: [u32; 2],
    /// One end of the edge.
    v: u32,
    dir: Vec3,
}

/// A flat face of the mesh: triangles that touch and are coplanar.
struct Patch {
    tris: Vec<u32>,
    normal: Vec3,
    area: f32,
}

/// One group of faces with a circle fit, before the groups of the same
/// hole join.
struct Piece {
    center: Vec2,
    r: f32,
    t0: f32,
    t1: f32,
    /// Face area that points to the center, and away from it.
    inward: f32,
    outward: f32,
    /// The faces on the circle, as absolute 2D segments.
    segments: Vec<(Vec2, Vec2)>,
}

/// Finds the round surfaces (holes and pins) of `mesh` with an axis near
/// the bed plane.
pub fn rounds(mesh: &Mesh) -> Vec<Round> {
    let n = mesh.triangles.len();
    let normals: Vec<Vec3> = (0..n).map(|i| mesh.normal(i)).collect();

    // Edge (low vertex, high vertex) -> the first two triangles on it and
    // the number of triangles on it.
    let mut edges: HashMap<(u32, u32), (u32, u32, u32)> = HashMap::with_capacity(n * 3 / 2);
    for (i, t) in mesh.triangles.iter().enumerate() {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            let e = edges
                .entry((a.min(b), a.max(b)))
                .or_insert((i as u32, u32::MAX, 0));
            if e.2 == 1 {
                e.1 = i as u32;
            }
            e.2 += 1;
        }
    }
    // Only edges with two faces, in a fixed order.
    let mut pairs: Vec<((u32, u32), (u32, u32))> = edges
        .into_iter()
        .filter(|e| e.1.2 == 2)
        .map(|(k, e)| (k, (e.0, e.1)))
        .collect();
    pairs.sort_unstable();

    let cos_turn = MAX_TURN_DEG.to_radians().cos();
    let max_dz = (MAX_TILT_DEG + 0.5).to_radians().sin();
    let mut flat = Sets::new(n);
    let mut lines = Vec::new();
    for (key, (ta, tb)) in pairs {
        let (na, nb) = (normals[ta as usize], normals[tb as usize]);
        if na == Vec3::ZERO || nb == Vec3::ZERO {
            continue;
        }
        let cos = na.dot(nb);
        if cos > COPLANAR {
            flat.union(ta, tb);
            continue;
        }
        if cos < cos_turn {
            continue;
        }
        let e = mesh.vertices[key.1 as usize] - mesh.vertices[key.0 as usize];
        let len = e.length();
        if len < 1e-3 || (e.z / len).abs() > max_dz {
            continue;
        }
        lines.push(Line {
            tris: [ta, tb],
            v: key.0,
            dir: e / len,
        });
    }

    // The flat patches.
    let mut patch_of = vec![u32::MAX; n];
    let mut patches: Vec<Patch> = Vec::new();
    let mut index: HashMap<u32, u32> = HashMap::new();
    for t in 0..n as u32 {
        let root = flat.find(t);
        let p = *index.entry(root).or_insert_with(|| {
            patches.push(Patch {
                tris: Vec::new(),
                normal: normals[root as usize],
                area: 0.0,
            });
            (patches.len() - 1) as u32
        });
        patch_of[t as usize] = p;
        patches[p as usize].tris.push(t);
        patches[p as usize].area += super::tri_area(mesh, t as usize);
    }

    let dirs: Vec<Vec3> = lines.iter().map(|l| l.dir).collect();
    let group = group_directions(&dirs, SAME_DIR_DEG);
    let mut by_group: Vec<Vec<usize>> = Vec::new();
    for (i, &g) in group.iter().enumerate() {
        if g >= by_group.len() {
            by_group.resize(g + 1, Vec::new());
        }
        by_group[g].push(i);
    }

    let mut out = Vec::new();
    for members in by_group {
        if members.len() < MIN_CORNERS {
            continue;
        }
        let rep = lines[members[0]].dir;
        let sum: Vec3 = members
            .iter()
            .map(|&i| {
                let d = lines[i].dir;
                if d.dot(rep) < 0.0 { -d } else { d }
            })
            .sum();
        let axis = sum.normalize();
        if axis.z.abs() > MAX_TILT_DEG.to_radians().sin() {
            continue;
        }
        let up = (Vec3::Z - axis * axis.z).normalize();
        let side = up.cross(axis);
        let to2d = |p: Vec3| Vec2::new(p.dot(side), p.dot(up));

        // Join the patches over the lines of this direction. `local` gives
        // the patches on these lines the numbers 0, 1, 2 and so on.
        let mut local: HashMap<u32, u32> = HashMap::new();
        for &i in &members {
            for t in lines[i].tris {
                let next = local.len() as u32;
                local.entry(patch_of[t as usize]).or_insert(next);
            }
        }
        let mut sets = Sets::new(local.len());
        let local_of = |i: usize| lines[i].tris.map(|t| local[&patch_of[t as usize]]);
        for &i in &members {
            let [a, b] = local_of(i);
            sets.union(a, b);
        }
        let mut comps: HashMap<u32, Vec<usize>> = HashMap::new();
        for &i in &members {
            let root = sets.find(local_of(i)[0]);
            comps.entry(root).or_default().push(i);
        }
        let mut comps: Vec<Vec<usize>> = comps.into_values().collect();
        comps.sort_unstable();

        let mut parts: Vec<Piece> = Vec::new();
        for comp in comps {
            let mut points: Vec<Vec2> = Vec::new();
            for &i in &comp {
                let p = to2d(mesh.vertices[lines[i].v as usize]);
                if points.iter().all(|q| q.distance(p) > 1e-4) {
                    points.push(p);
                }
            }
            if points.len() < MIN_CORNERS {
                continue;
            }
            let Some((center, r)) = fit_robust(&points) else {
                continue;
            };
            let tol = fit_tol(r);
            let mut ids: Vec<u32> = comp
                .iter()
                .flat_map(|&i| lines[i].tris.map(|t| patch_of[t as usize]))
                .collect();
            ids.sort_unstable();
            ids.dedup();
            let mut part = Piece {
                center,
                r,
                t0: f32::MAX,
                t1: f32::MIN,
                inward: 0.0,
                outward: 0.0,
                segments: Vec::new(),
            };
            for id in ids {
                let patch = &patches[id as usize];
                let n2 = to2d(patch.normal);
                if n2.length() < 0.5 {
                    continue;
                }
                // The patch is a line in the plane: find its two ends.
                let along = n2.perp().normalize();
                let (mut lo, mut hi) = ((f32::MAX, Vec2::ZERO), (f32::MIN, Vec2::ZERO));
                let (mut t0, mut t1) = (f32::MAX, f32::MIN);
                for &t in &patch.tris {
                    for v in mesh.tri(t as usize) {
                        let p = to2d(v);
                        let s = p.dot(along);
                        if s < lo.0 {
                            lo = (s, p);
                        }
                        if s > hi.0 {
                            hi = (s, p);
                        }
                        let ax = v.dot(axis);
                        t0 = t0.min(ax);
                        t1 = t1.max(ax);
                    }
                }
                let on = |p: Vec2| (p.distance(center) - r).abs() <= tol;
                if !on(lo.1) || !on(hi.1) || lo.1.distance(hi.1) < 1e-4 {
                    continue;
                }
                let mid = (lo.1 + hi.1) * 0.5;
                if n2.dot(center - mid) > 0.0 {
                    part.inward += patch.area;
                } else {
                    part.outward += patch.area;
                }
                part.segments.push((lo.1, hi.1));
                part.t0 = part.t0.min(t0);
                part.t1 = part.t1.max(t1);
            }
            if !part.segments.is_empty() {
                parts.push(part);
            }
        }

        // Groups on the same circle with overlapping extents are one
        // surface (for example a hole that a slot cuts in two).
        let mut join = Sets::new(parts.len());
        for i in 0..parts.len() {
            for j in i + 1..parts.len() {
                let (a, b) = (&parts[i], &parts[j]);
                let tol = 2.0 * fit_tol(a.r.max(b.r));
                if a.center.distance(b.center) <= tol
                    && (a.r - b.r).abs() <= tol
                    && a.t0 <= b.t1 + 0.05
                    && b.t0 <= a.t1 + 0.05
                {
                    join.union(i as u32, j as u32);
                }
            }
        }
        let mut merged: Vec<Option<Piece>> = Vec::new();
        merged.resize_with(parts.len(), || None);
        for (i, p) in parts.into_iter().enumerate() {
            let root = join.find(i as u32) as usize;
            match &mut merged[root] {
                None => merged[root] = Some(p),
                Some(m) => {
                    m.t0 = m.t0.min(p.t0);
                    m.t1 = m.t1.max(p.t1);
                    m.inward += p.inward;
                    m.outward += p.outward;
                    m.segments.extend(p.segments);
                }
            }
        }
        for p in merged.into_iter().flatten() {
            let total = p.inward + p.outward;
            let hole = if p.inward >= 0.9 * total {
                true
            } else if p.outward >= 0.9 * total {
                false
            } else {
                // Faces in both directions: not a plain hole or pin.
                continue;
            };
            out.push(Round {
                axis,
                up,
                side,
                center: p.center,
                r: p.r,
                t0: p.t0,
                t1: p.t1,
                hole,
                segments: p
                    .segments
                    .iter()
                    .map(|&(a, b)| (a - p.center, b - p.center))
                    .collect(),
            });
        }
    }
    out.sort_by(|a, b| {
        let (pa, pb) = (a.at(a.t0), b.at(b.t0));
        (pa.z, pa.x, pa.y, a.r)
            .partial_cmp(&(pb.z, pb.x, pb.y, b.r))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
}

/// Groups unit vectors by direction. A vector and its opposite have the
/// same direction. Returns the group number of each vector.
fn group_directions(dirs: &[Vec3], max_deg: f32) -> Vec<usize> {
    // The bins are larger than the angle, so a match is in a bin next to
    // the bin of the vector.
    const BIN: f32 = 0.01;
    let cos = max_deg.to_radians().cos();
    let key = |v: Vec3| (v / BIN).floor().as_ivec3().to_array();
    let mut bins: HashMap<[i32; 3], Vec<usize>> = HashMap::new();
    let mut reps: Vec<Vec3> = Vec::new();
    let mut out = Vec::with_capacity(dirs.len());
    for &d in dirs {
        let k = key(d);
        let mut found = None;
        'search: for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    let Some(list) = bins.get(&[k[0] + dx, k[1] + dy, k[2] + dz]) else {
                        continue;
                    };
                    if let Some(&g) = list.iter().find(|&&g| reps[g].dot(d).abs() >= cos) {
                        found = Some(g);
                        break 'search;
                    }
                }
            }
        }
        let g = found.unwrap_or_else(|| {
            reps.push(d);
            reps.len() - 1
        });
        for kk in [k, key(-d)] {
            let list = bins.entry(kk).or_default();
            if !list.contains(&g) {
                list.push(g);
            }
        }
        out.push(g);
    }
    out
}

/// The largest distance (mm) of a corner line from the fitted circle.
fn fit_tol(r: f32) -> f32 {
    0.002 * r + 0.003
}

/// Fits a circle to `pts`. When some points are not on one circle (for
/// example the corners of a slot that joins a hole), it keeps the circle
/// through three of the points that has the most points on it. At least
/// 70 % of the points must be on the circle. Returns the center and the
/// radius.
fn fit_robust(pts: &[Vec2]) -> Option<(Vec2, f32)> {
    let on = |c: Vec2, r: f32| -> Vec<Vec2> {
        let tol = fit_tol(r);
        pts.iter()
            .copied()
            .filter(|p| (p.distance(c) - r).abs() <= tol)
            .collect()
    };
    let n = pts.len();
    if let Some((c, r)) = fit_circle(pts)
        && on(c, r).len() == n
    {
        return Some((c, r));
    }
    let need = MIN_CORNERS.max((n * 7).div_ceil(10));
    let mut best: Vec<Vec2> = Vec::new();
    let mut tries = 0;
    'search: for step in 1..n.div_ceil(3).max(2) {
        for i in 0..n {
            let (j, k) = ((i + step) % n, (i + 2 * step) % n);
            if j == i || k == i || k == j {
                continue;
            }
            tries += 1;
            if tries > 600 {
                break 'search;
            }
            if let Some((c, r)) = fit_circle(&[pts[i], pts[j], pts[k]]) {
                let set = on(c, r);
                if set.len() > best.len() {
                    best = set;
                }
            }
        }
    }
    if best.len() < need {
        return None;
    }
    let (c, r) = fit_circle(&best)?;
    let tol = fit_tol(r);
    best.iter()
        .all(|p| (p.distance(c) - r).abs() <= tol)
        .then_some((c, r))
}

/// The least squares circle through `pts` (the Kasa method): the center
/// and the radius.
fn fit_circle(pts: &[Vec2]) -> Option<(Vec2, f32)> {
    if pts.len() < 3 {
        return None;
    }
    let n = pts.len() as f64;
    let (mx, my) = pts.iter().fold((0.0, 0.0), |(x, y), p| {
        (x + p.x as f64 / n, y + p.y as f64 / n)
    });
    // Sums for the normal equations of x² + y² + D x + E y + F = 0.
    let (mut sxx, mut sxy, mut syy, mut sx, mut sy) = (0.0, 0.0, 0.0, 0.0, 0.0);
    let (mut sxz, mut syz, mut sz) = (0.0, 0.0, 0.0);
    for p in pts {
        let (x, y) = (p.x as f64 - mx, p.y as f64 - my);
        let z = x * x + y * y;
        sxx += x * x;
        sxy += x * y;
        syy += y * y;
        sx += x;
        sy += y;
        sxz += x * z;
        syz += y * z;
        sz += z;
    }
    let a = [[sxx, sxy, sx], [sxy, syy, sy], [sx, sy, n]];
    let b = [-sxz, -syz, -sz];
    let det = |m: [[f64; 3]; 3]| {
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    };
    let d0 = det(a);
    if d0.abs() < 1e-12 {
        return None;
    }
    // Cramer's rule.
    let col = |k: usize| {
        let mut m = a;
        for (row, v) in m.iter_mut().zip(b) {
            row[k] = v;
        }
        det(m) / d0
    };
    let (dd, ee, ff) = (col(0), col(1), col(2));
    let (cx, cy) = (-dd / 2.0, -ee / 2.0);
    let r2 = cx * cx + cy * cy - ff;
    if r2 <= 0.0 || !r2.is_finite() {
        return None;
    }
    Some((
        Vec2::new((cx + mx) as f32, (cy + my) as f32),
        r2.sqrt() as f32,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A regular polygon of `n` corners with radius `r`, the first corner
    /// at `start` degrees.
    fn ngon(n: usize, r: f32, start: f32) -> Vec<Vec2> {
        (0..n)
            .map(|i| {
                let a = (start + 360.0 * i as f32 / n as f32).to_radians();
                Vec2::new(r * a.cos(), r * a.sin())
            })
            .collect()
    }

    /// Points at the angles `deg` (counter clockwise) with radius `r`.
    fn at_angles(deg: &[f32], r: f32) -> Vec<Vec2> {
        deg.iter()
            .map(|a| Vec2::new(r * a.to_radians().cos(), r * a.to_radians().sin()))
            .collect()
    }

    /// A closed tube: the ring between `outer` and `inner` (same number of
    /// points, counter clockwise, star-shaped around the origin), extruded
    /// `len` along local z. `frame` puts the local frame in the part.
    fn tube(inner: &[Vec2], outer: &[Vec2], len: f32, frame: Mat4) -> Mesh {
        assert_eq!(inner.len(), outer.len());
        let n = inner.len();
        let p = |q: Vec2, z: f32| frame.transform_point3(Vec3::new(q.x, q.y, z));
        let mut soup = Vec::new();
        for i in 0..n {
            let j = (i + 1) % n;
            let (o0i, o0j, o1i, o1j) = (
                p(outer[i], 0.0),
                p(outer[j], 0.0),
                p(outer[i], len),
                p(outer[j], len),
            );
            let (i0i, i0j, i1i, i1j) = (
                p(inner[i], 0.0),
                p(inner[j], 0.0),
                p(inner[i], len),
                p(inner[j], len),
            );
            soup.push([o0i, o0j, o1j]);
            soup.push([o0i, o1j, o1i]);
            soup.push([i0i, i1j, i0j]);
            soup.push([i0i, i1i, i1j]);
            soup.push([o0i, i0i, i0j]);
            soup.push([o0i, i0j, o0j]);
            soup.push([o1i, i1j, i1i]);
            soup.push([o1i, o1j, i1j]);
        }
        let m = Mesh::from_soup(&soup);
        assert_eq!(m.open_edges(), 0);
        assert!(m.volume() > 0.0);
        m
    }

    /// The frame of a tube with its axis along `axis` (unit vector) and
    /// its section center at `at`. Local y is up when the axis is level.
    fn frame(axis: Vec3, at: Vec3) -> Mat4 {
        let up = if axis.z.abs() > 0.99 {
            Vec3::X
        } else {
            (Vec3::Z - axis * axis.z).normalize()
        };
        let side = up.cross(axis);
        Mat4::from_cols(
            side.extend(0.0),
            up.extend(0.0),
            axis.extend(0.0),
            at.extend(1.0),
        )
    }

    fn level_tube(r: f32, big_r: f32, axis: Vec3, at: Vec3) -> Mesh {
        tube(
            &ngon(24, r, 0.0),
            &ngon(24, big_r, 0.0),
            20.0,
            frame(axis, at),
        )
    }

    fn run(mesh: &Mesh) -> Plan {
        plan(mesh, Layers::default(), &Limits::default())
    }

    #[test]
    fn circle_fit_finds_center_and_radius() {
        let pts: Vec<Vec2> = ngon(7, 2.5, 10.0)
            .iter()
            .map(|p| *p + Vec2::new(4.0, -3.0))
            .collect();
        let (c, r) = fit_circle(&pts).unwrap();
        assert!(c.distance(Vec2::new(4.0, -3.0)) < 1e-4, "{c}");
        assert!((r - 2.5).abs() < 1e-4, "{r}");
        // A square corner far from the circle goes away.
        let mut with_out = pts.clone();
        with_out.extend(ngon(5, 2.5, 3.0).iter().map(|p| *p + Vec2::new(4.0, -3.0)));
        with_out.push(Vec2::new(9.0, 9.0));
        let (c, r) = fit_robust(&with_out).unwrap();
        assert!(c.distance(Vec2::new(4.0, -3.0)) < 1e-3 && (r - 2.5).abs() < 1e-3);
        // Points on a square are not on a circle with the corners of a
        // rectangle and a point at the middle of an edge.
        let rect = [
            Vec2::new(0.0, 0.0),
            Vec2::new(10.0, 0.0),
            Vec2::new(10.0, 4.0),
            Vec2::new(0.0, 4.0),
            Vec2::new(5.0, 0.0),
        ];
        assert!(fit_robust(&rect).is_none());
    }

    #[test]
    fn directions_group_with_their_opposites() {
        let d = Vec3::new(1.0, 1e-7, 0.0).normalize();
        let e = Vec3::new(-1.0, 1e-7, 0.0).normalize();
        let f = Vec3::new(0.0, 1.0, 0.0);
        let g = group_directions(&[d, e, f, -f, d], SAME_DIR_DEG);
        assert_eq!(g, vec![0, 0, 1, 1, 0]);
    }

    #[test]
    fn level_tube_has_one_hole_and_one_pin() {
        let m = level_tube(3.0, 10.0, Vec3::X, Vec3::new(0.0, 0.0, 10.0));
        let r = rounds(&m);
        assert_eq!(r.len(), 2, "{r:?}");
        let hole = r.iter().find(|h| h.hole).unwrap();
        let pin = r.iter().find(|h| !h.hole).unwrap();
        assert!((hole.r - 3.0).abs() < 1e-3 && (pin.r - 10.0).abs() < 1e-3);
        assert!(hole.at(0.0).distance(Vec3::new(0.0, 0.0, 10.0)) < 1e-3);
        assert!((hole.t1 - hole.t0 - 20.0).abs() < 1e-3);
        assert!(hole.covers_top() && !hole.pointed());

        let p = run(&m);
        assert_eq!(p.fixed, 1, "{p:?}");
        assert!(p.skipped.is_empty());
        assert!(p.cuts[0].starts_with("multmatrix("), "{}", p.cuts[0]);
        assert!(
            p.cuts[0].contains("linear_extrude(height = 20.04)"),
            "{}",
            p.cuts[0]
        );
        // The tip of the point is r√2 straight above the axis.
        let f = hole.frame(hole.t0);
        let tip = f.transform_point3(Vec3::new(0.0, 3.0 * SQRT_2, 0.0));
        assert!(
            tip.distance(Vec3::new(hole.t0 * hole.axis.x, 0.0, 10.0 + 3.0 * SQRT_2)) < 1e-3,
            "{tip}"
        );
    }

    #[test]
    fn turned_and_tilted_holes() {
        // 30 degrees about z: still level.
        let a = Vec3::new(30f32.to_radians().cos(), 30f32.to_radians().sin(), 0.0);
        let m = level_tube(3.0, 10.0, a, Vec3::new(5.0, -2.0, 10.0));
        let p = run(&m);
        assert_eq!(p.fixed, 1, "{p:?}");
        let h = rounds(&m).into_iter().find(|h| h.hole).unwrap();
        assert!(h.axis.dot(a).abs() > 0.9999);
        let f = h.frame(0.0);
        let tip = f.transform_point3(Vec3::new(0.0, h.r * SQRT_2, 0.0));
        let axis_point = f.transform_point3(Vec3::ZERO);
        assert!((tip - axis_point - Vec3::Z * h.r * SQRT_2).length() < 1e-3);
        // The axis line goes through the tube center.
        let c = Vec3::new(5.0, -2.0, 10.0);
        let off = (c - axis_point) - h.axis * (c - axis_point).dot(h.axis);
        assert!(off.length() < 1e-3, "{off}");

        // 5 degrees up: a hole. 20 degrees up: not horizontal.
        for (deg, want) in [(5.0f32, 1), (20.0, 0)] {
            let a = Vec3::new(deg.to_radians().cos(), 0.0, deg.to_radians().sin());
            let m = level_tube(3.0, 10.0, a, Vec3::new(0.0, 0.0, 12.0));
            assert_eq!(run(&m).fixed, want, "{deg} degrees");
        }
    }

    #[test]
    fn vertical_hole_gets_no_point() {
        let m = level_tube(3.0, 10.0, Vec3::Z, Vec3::ZERO);
        let p = run(&m);
        assert_eq!(p.fixed, 0);
        assert!(p.cuts.is_empty() && p.skipped.is_empty());
        assert!(rounds(&m).is_empty());
    }

    #[test]
    fn thin_wall_above_is_skipped() {
        // The wall above the tip is 4.6 - 4.24 = 0.36 mm.
        let m = level_tube(3.0, 4.6, Vec3::X, Vec3::new(0.0, 0.0, 5.0));
        let p = run(&m);
        assert_eq!(p.fixed, 0);
        assert_eq!(
            p.skipped,
            vec!["Hole d 6.0 mm at z 5.0: wall above too thin".to_string()]
        );
        // The tip goes through the top: 4.0 < 4.24.
        let m = level_tube(3.0, 4.0, Vec3::X, Vec3::new(0.0, 0.0, 5.0));
        let p = run(&m);
        assert_eq!(p.fixed, 0);
        assert!(
            p.skipped[0].ends_with("point breaks into the space above"),
            "{p:?}"
        );
        // A lower limit lets it pass.
        let limits = Limits {
            min_wall: 0.3,
            ..Limits::default()
        };
        let m = level_tube(3.0, 4.6, Vec3::X, Vec3::new(0.0, 0.0, 5.0));
        assert_eq!(plan(&m, Layers::default(), &limits).fixed, 1);
    }

    #[test]
    fn size_and_height_limits() {
        // d 0.6 and d 50: out of the limits.
        let m = level_tube(0.3, 5.0, Vec3::X, Vec3::new(0.0, 0.0, 5.0));
        assert_eq!(run(&m).fixed, 0);
        let m = level_tube(25.0, 40.0, Vec3::X, Vec3::new(0.0, 0.0, 40.0));
        assert_eq!(run(&m).fixed, 0);
        // The top of the hole is in the first layer.
        let m = level_tube(3.0, 10.0, Vec3::X, Vec3::new(0.0, 0.0, -2.9));
        assert_eq!(run(&m).fixed, 0);
        let m = level_tube(3.0, 10.0, Vec3::X, Vec3::new(0.0, 0.0, -2.7));
        assert_eq!(run(&m).fixed, 1);
    }

    #[test]
    fn pointed_top_gets_no_change() {
        // A vertex straight up with a corner of 90 degrees.
        let deg = [0.0, 90.0, 180.0, 225.0, 270.0, 315.0];
        let m = tube(
            &at_angles(&deg, 3.0),
            &at_angles(&deg, 10.0),
            20.0,
            frame(Vec3::X, Vec3::new(0.0, 0.0, 10.0)),
        );
        let h = rounds(&m).into_iter().find(|h| h.hole).unwrap();
        assert!(h.covers_top() && h.pointed());
        assert_eq!(run(&m).fixed, 0);
        // The same shape upside down has a flat corner of 135 degrees at
        // the top: it gets a point.
        let deg = [0.0, 45.0, 90.0, 135.0, 180.0, 270.0];
        let m = tube(
            &at_angles(&deg, 3.0),
            &at_angles(&deg, 10.0),
            20.0,
            frame(Vec3::X, Vec3::new(0.0, 0.0, 10.0)),
        );
        let h = rounds(&m).into_iter().find(|h| h.hole).unwrap();
        assert!(!h.pointed());
        assert_eq!(run(&m).fixed, 1);
        // A teardrop: the arc does not cover the top.
        let mut deg: Vec<f32> = (0..=18).map(|i| 135.0 + 15.0 * i as f32).collect();
        deg.insert(0, 90.0);
        let mut inner = at_angles(&deg, 3.0);
        inner[0] = Vec2::new(0.0, 3.0 * SQRT_2);
        let m = tube(
            &inner,
            &at_angles(&deg, 10.0),
            20.0,
            frame(Vec3::X, Vec3::new(0.0, 0.0, 10.0)),
        );
        let p = run(&m);
        assert_eq!(p.fixed, 0, "{p:?}");
        assert!(p.skipped.is_empty());
    }

    #[test]
    fn counterbore_gets_two_points_and_a_gap_makes_two_holes() {
        // A counterbore d 6.5 from 0 to 4, then the hole d 3.4 to 20.
        let f0 = frame(Vec3::X, Vec3::new(0.0, 0.0, 10.0));
        let f1 = frame(Vec3::X, Vec3::new(4.0, 0.0, 10.0));
        let mut m = tube(&ngon(24, 3.25, 0.0), &ngon(24, 9.0, 0.0), 4.0, f0);
        let b = tube(&ngon(24, 1.7, 0.0), &ngon(24, 9.0, 0.0), 16.0, f1);
        let base = m.vertices.len() as u32;
        m.vertices.extend(b.vertices);
        m.triangles
            .extend(b.triangles.iter().map(|t| t.map(|v| v + base)));
        let p = run(&m);
        assert_eq!(p.fixed, 2, "{p:?}");
        let holes: Vec<Round> = rounds(&m).into_iter().filter(|h| h.hole).collect();
        assert_eq!(holes.len(), 2);
        let mut radii: Vec<f32> = holes.iter().map(|h| h.r).collect();
        radii.sort_by(f32::total_cmp);
        assert!(
            (radii[0] - 1.7).abs() < 1e-3 && (radii[1] - 3.25).abs() < 1e-3,
            "{radii:?}"
        );

        // Two walls on one axis with a gap: two holes, two cuts.
        let f2 = frame(Vec3::X, Vec3::new(12.0, 0.0, 10.0));
        let mut m = tube(&ngon(24, 3.0, 0.0), &ngon(24, 9.0, 0.0), 8.0, f0);
        let b = tube(&ngon(24, 3.0, 0.0), &ngon(24, 9.0, 0.0), 8.0, f2);
        let base = m.vertices.len() as u32;
        m.vertices.extend(b.vertices);
        m.triangles
            .extend(b.triangles.iter().map(|t| t.map(|v| v + base)));
        let p = run(&m);
        assert_eq!(p.fixed, 2, "{p:?}");
        assert!(
            p.cuts
                .iter()
                .all(|c| c.contains("linear_extrude(height = 8.04)")),
            "{p:?}"
        );
    }

    #[test]
    fn coarse_pentagon_hole_is_found() {
        // d 1 with the OpenSCAD defaults is a pentagon.
        let m = tube(
            &ngon(5, 0.5, 0.0),
            &ngon(5, 4.0, 0.0),
            10.0,
            frame(Vec3::Y, Vec3::new(0.0, 0.0, 5.0)),
        );
        let h = rounds(&m).into_iter().find(|h| h.hole).unwrap();
        assert!((h.r - 0.5).abs() < 1e-3);
        assert_eq!(run(&m).fixed, 1);
    }
}

/// Tests with OpenSCAD: the teardrop cuts, and the fix runner of
/// `fixes.rs` (`apply`, `compose`, `bed_cut_module`) on parts that OpenSCAD
/// makes. Run them with `cargo test teardrop -- --ignored`.
#[cfg(test)]
mod openscad_tests {
    use super::super::fixes::{self, Report, Settings};
    use super::super::{Area, loop_area, section, transformed};
    use super::*;
    use crate::openscad::{self, Config};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    /// The mesh that OpenSCAD makes from `src`.
    fn build(src: &str) -> Mesh {
        static N: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "cocad-td-test-{}-{}.scad",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, src).unwrap();
        let out = openscad::render(
            &Config::default(),
            &path,
            &[],
            &Arc::new(AtomicBool::new(false)),
        );
        let _ = std::fs::remove_file(&path);
        out.unwrap().mesh
    }

    fn apply(mesh: &Mesh, s: &Settings) -> (Mesh, Report) {
        fixes::apply(
            &Config::default(),
            mesh,
            s,
            Layers::default(),
            &Limits::default(),
            &Area::default(),
            &Arc::new(AtomicBool::new(false)),
        )
        .unwrap()
    }

    fn teardrops() -> Settings {
        Settings {
            teardrops: true,
            ..Settings::default()
        }
    }

    fn chamfer() -> Settings {
        Settings {
            chamfer: true,
            chamfer_mm: 0.4,
            ..Settings::default()
        }
    }

    /// The size of the outer outlines of the section at `z`.
    fn outer(mesh: &Mesh, z: f32) -> Vec2 {
        let (mut lo, mut hi) = (Vec2::MAX, Vec2::MIN);
        for lp in section(mesh, z).iter().filter(|l| loop_area(l) > 0.0) {
            for p in lp {
                lo = lo.min(*p);
                hi = hi.max(*p);
            }
        }
        hi - lo
    }

    /// The sizes of the holes in the section at `z`.
    fn holes(mesh: &Mesh, z: f32) -> Vec<Vec2> {
        section(mesh, z)
            .iter()
            .filter(|l| loop_area(l) < 0.0)
            .map(|lp| {
                let (mut lo, mut hi) = (Vec2::MAX, Vec2::MIN);
                for p in lp {
                    lo = lo.min(*p);
                    hi = hi.max(*p);
                }
                hi - lo
            })
            .collect()
    }

    /// The free gap around `y` on the line x = `x` in the section at `z`:
    /// the distance between the material on the two sides. 0 when `y` is
    /// in the material.
    fn gap(mesh: &Mesh, z: f32, x: f32, y: f32) -> f32 {
        let mut ys: Vec<f32> = Vec::new();
        for lp in section(mesh, z) {
            for i in 0..lp.len() {
                let (a, b) = (lp[i], lp[(i + 1) % lp.len()]);
                if (a.x < x) != (b.x < x) {
                    ys.push(a.y + (b.y - a.y) * (x - a.x) / (b.x - a.x));
                }
            }
        }
        ys.sort_by(f32::total_cmp);
        if ys.iter().filter(|&&v| v < y).count() % 2 == 1 {
            return 0.0;
        }
        let lo = ys.iter().rev().find(|&&v| v < y);
        let hi = ys.iter().find(|&&v| v > y);
        match (lo, hi) {
            (Some(lo), Some(hi)) => hi - lo,
            _ => f32::INFINITY,
        }
    }

    /// The area that the point adds to a hole of radius `r` made with
    /// `$fn = 32` and a corner straight up: the kite (r²) less the 8
    /// polygon sectors in it.
    fn cap32(r: f32) -> f32 {
        r * r * (1.0 - 4.0 * 11.25f32.to_radians().sin())
    }

    const BLOCK: &str = "difference() { cube([30, 20, 15]); \
        translate([-1, 10, 7.5]) rotate([0, 90, 0]) cylinder(d = 6, h = 32, $fn = 32); }";

    #[test]
    #[ignore = "needs openscad-nightly"]
    fn block_with_horizontal_hole_gets_a_point() {
        let m = build(BLOCK);
        let p = plan(&m, Layers::default(), &Limits::default());
        assert_eq!(p.fixed, 1, "{p:?}");
        let (out, report) = apply(&m, &teardrops());
        assert_eq!(report.teardrops, 1);
        assert!(!report.chamfer);
        // At the axis the hole is 6 wide. At 1.2 r above the axis there
        // was material, and now there is the point: 2 (r√2 - 1.2 r) wide.
        let z = 7.5 + 1.2 * 3.0;
        assert!((gap(&m, 7.5, 15.0, 10.0) - 6.0).abs() < 0.01);
        assert!((gap(&out, 7.5, 15.0, 10.0) - 6.0).abs() < 0.01);
        assert_eq!(gap(&m, z, 15.0, 10.0), 0.0);
        let w = gap(&out, z, 15.0, 10.0);
        let want = 2.0 * (3.0 * SQRT_2 - 3.6);
        eprintln!("point width at z {z}: {w:.3} (want {want:.3})");
        assert!((w - want).abs() < 0.02, "{w}");
        // The volume goes down by the cap of the point over 30 mm.
        let dv = m.volume() - out.volume();
        let want = cap32(3.0) * 30.0;
        eprintln!("volume change {dv:.2} mm³ (want {want:.2})");
        assert!((dv - want).abs() < 0.01 * want, "{dv}");
        assert_eq!(out.open_edges(), 0);
        // The fixed part has nothing more to fix.
        assert_eq!(plan(&out, Layers::default(), &Limits::default()).fixed, 0);
    }

    #[test]
    #[ignore = "needs openscad-nightly"]
    fn hole_with_default_segments_and_turned_hole() {
        // d 6 with the OpenSCAD defaults has 10 faces.
        let m = build(
            "difference() { cube([30, 20, 15]); \
             translate([-1, 10, 7.5]) rotate([0, 90, 0]) cylinder(d = 6, h = 32); }",
        );
        let p = plan(&m, Layers::default(), &Limits::default());
        assert_eq!(p.fixed, 1, "{p:?}");
        let (out, _) = apply(&m, &teardrops());
        let dv = m.volume() - out.volume();
        // 10 faces: the 90 degree sector holds 2.5 polygon sectors.
        let want = 9.0 * (1.0 - 2.5 * 36f32.to_radians().sin() / 2.0) * 30.0;
        eprintln!("10 faces: volume change {dv:.2} mm³ (about {want:.2})");
        assert!(dv > 0.8 * want && dv < 1.2 * want, "{dv}");

        // A hole at 30 degrees to the x axis, 46.19 mm long in the block.
        let m = build(
            "difference() { cube([40, 40, 15]); translate([20, 20, 7.5]) rotate([0, 0, 30]) \
             rotate([0, 90, 0]) cylinder(d = 6, h = 80, center = true, $fn = 32); }",
        );
        let p = plan(&m, Layers::default(), &Limits::default());
        assert_eq!(p.fixed, 1, "{p:?}");
        let (out, _) = apply(&m, &teardrops());
        let dv = m.volume() - out.volume();
        let want = cap32(3.0) * 40.0 / 30f32.to_radians().cos();
        eprintln!("turned hole: volume change {dv:.2} mm³ (want {want:.2})");
        assert!((dv - want).abs() < 0.02 * want, "{dv}");
    }

    #[test]
    #[ignore = "needs openscad-nightly"]
    fn vertical_hole_and_horizontal_pin_get_no_change() {
        let m = build(
            "difference() { cube([30, 20, 15]); \
             translate([15, 10, -1]) cylinder(d = 6, h = 17, $fn = 32); }",
        );
        let p = plan(&m, Layers::default(), &Limits::default());
        assert_eq!((p.fixed, p.cuts.len(), p.skipped.len()), (0, 0, 0));
        let (out, report) = apply(&m, &teardrops());
        assert_eq!(report.teardrops, 0);
        assert_eq!(out.triangles, m.triangles);

        let m = build(
            "cube([20, 20, 15]); \
             translate([19, 10, 7.5]) rotate([0, 90, 0]) cylinder(d = 6, h = 11, $fn = 32);",
        );
        let r = rounds(&m);
        assert!(
            r.iter().any(|h| !h.hole && (h.r - 3.0).abs() < 0.01),
            "{r:?}"
        );
        let p = plan(&m, Layers::default(), &Limits::default());
        assert_eq!((p.fixed, p.cuts.len(), p.skipped.len()), (0, 0, 0));
    }

    #[test]
    #[ignore = "needs openscad-nightly"]
    fn counterbore_gets_two_points() {
        let m = build(
            "difference() { cube([30, 20, 15]); \
             translate([-1, 10, 7.5]) rotate([0, 90, 0]) cylinder(d = 3.4, h = 32, $fn = 32); \
             translate([-1, 10, 7.5]) rotate([0, 90, 0]) cylinder(d = 6.5, h = 5, $fn = 32); }",
        );
        let p = plan(&m, Layers::default(), &Limits::default());
        assert_eq!(p.fixed, 2, "{p:?}");
        let (out, report) = apply(&m, &teardrops());
        assert_eq!(report.teardrops, 2);
        // The point of the counterbore at x = 2, of the hole at x = 15.
        let z = 7.5 + 1.2 * 3.25;
        let w = gap(&out, z, 2.0, 10.0);
        assert!((w - 2.0 * (3.25 * SQRT_2 - 1.2 * 3.25)).abs() < 0.02, "{w}");
        assert_eq!(gap(&out, z, 15.0, 10.0), 0.0);
        let z = 7.5 + 1.2 * 1.7;
        let w = gap(&out, z, 15.0, 10.0);
        assert!((w - 2.0 * (1.7 * SQRT_2 - 1.2 * 1.7)).abs() < 0.02, "{w}");
        let dv = m.volume() - out.volume();
        let want = cap32(3.25) * 4.0 + cap32(1.7) * 26.0;
        eprintln!("counterbore: volume change {dv:.2} mm³ (want {want:.2})");
        assert!((dv - want).abs() < 0.02 * want, "{dv}");
    }

    #[test]
    #[ignore = "needs openscad-nightly"]
    fn thin_wall_above_gets_no_point() {
        let at = |z: f32| {
            build(&format!(
                "difference() {{ cube([30, 20, 15]); translate([-1, 10, {z}]) \
                 rotate([0, 90, 0]) cylinder(d = 6, h = 32, $fn = 32); }}"
            ))
        };
        let p = plan(&at(11.0), Layers::default(), &Limits::default());
        assert_eq!(p.fixed, 0);
        assert_eq!(
            p.skipped,
            vec!["Hole d 6.0 mm at z 11.0: point breaks into the space above"]
        );
        let p = plan(&at(10.0), Layers::default(), &Limits::default());
        assert_eq!(p.fixed, 0);
        assert_eq!(
            p.skipped,
            vec!["Hole d 6.0 mm at z 10.0: wall above too thin"]
        );
        let p = plan(&at(9.5), Layers::default(), &Limits::default());
        assert_eq!((p.fixed, p.skipped.len()), (1, 0));
    }

    /// The volume that a 0.4 mm chamfer in two layers of 0.2 mm takes from
    /// a box of `w` x `d`.
    fn chamfer_volume(w: f32, d: f32) -> f32 {
        let a = w * d;
        0.2 * (a - (w - 0.8) * (d - 0.8)) + 0.2 * (a - (w - 0.4) * (d - 0.4))
    }

    #[test]
    #[ignore = "needs openscad-nightly"]
    fn chamfer_steps_follow_the_layers() {
        let m = build("cube([20, 20, 10]);");
        let (out, report) = apply(&m, &chamfer());
        assert!(report.chamfer);
        for (z, w) in [(0.1, 19.2), (0.3, 19.6), (1.0, 20.0)] {
            let s = outer(&out, z);
            eprintln!("chamfer: section at z {z}: {:.3} x {:.3}", s.x, s.y);
            assert!(
                (s.x - w).abs() < 0.005 && (s.y - w).abs() < 0.005,
                "z {z}: {s}"
            );
        }
        let dv = m.volume() - out.volume();
        let want = chamfer_volume(20.0, 20.0);
        eprintln!("chamfer: volume change {dv:.3} mm³ (want {want:.3})");
        assert!((dv - want).abs() < 0.01, "{dv}");
        assert_eq!(out.open_edges(), 0);
    }

    #[test]
    #[ignore = "needs openscad-nightly"]
    fn chamfer_makes_a_hole_on_the_bed_larger() {
        let m = build(
            "difference() { cube([20, 20, 10]); \
             translate([10, 10, -1]) cylinder(d = 5, h = 12, $fn = 48); }",
        );
        let (out, _) = apply(&m, &chamfer());
        let (h0, h1) = (holes(&out, 0.1), holes(&out, 1.0));
        assert_eq!((h0.len(), h1.len()), (1, 1));
        let grow = h0[0] - h1[0];
        eprintln!(
            "hole on the bed: {} at z 0.1, {} at z 1, larger by {grow}",
            h0[0], h1[0]
        );
        // The faces move 0.4 out. The corners of the 48-gon move
        // 0.4 / cos(3.75 degrees).
        let want = 0.8 / 3.75f32.to_radians().cos();
        assert!(
            (grow.x - want).abs() < 0.01 && (grow.y - want).abs() < 0.01,
            "{grow}"
        );
    }

    #[test]
    #[ignore = "needs openscad-nightly"]
    fn chamfer_is_not_cut_two_times() {
        // A box that has the chamfer of CoCAD: no change.
        let plain = build("cube([20, 20, 10]);");
        let (once, _) = apply(&plain, &chamfer());
        let (twice, report) = apply(&once, &chamfer());
        assert!(report.chamfer);
        let dv = once.volume() - twice.volume();
        eprintln!("second chamfer on a stepped foot: volume change {dv:.4} mm³");
        assert!(dv.abs() < 0.01, "{dv}");
        assert!((outer(&twice, 0.1).x - 19.2).abs() < 0.005);

        // A box with a smooth 45 degree chamfer of 0.4: the section at
        // z 0.1 goes to 19.2 (the same as a plain box), not to 19.2 - 0.8.
        // The steps of the layers cut the small wedges under the slope.
        let smooth = build(
            "hull() { translate([0.4, 0.4, 0]) cube([19.2, 19.2, 10]); \
             translate([0, 0, 0.4]) cube([20, 20, 9.6]); }",
        );
        let (out, _) = apply(&smooth, &chamfer());
        let s = outer(&out, 0.1);
        assert!(
            (s.x - 19.2).abs() < 0.005 && (s.y - 19.2).abs() < 0.005,
            "{s}"
        );
        let dv = smooth.volume() - out.volume();
        let full = chamfer_volume(20.0, 20.0);
        eprintln!(
            "chamfer on a smooth 45 degree foot: volume change {dv:.3} mm³ (plain box {full:.3})"
        );
        assert!(dv < 0.4 * full, "{dv}");
    }

    #[test]
    #[ignore = "needs openscad-nightly"]
    fn chamfer_and_teardrops_together() {
        let m = build(BLOCK);
        let s = Settings {
            teardrops: true,
            ..chamfer()
        };
        let (out, report) = apply(&m, &s);
        assert!(report.chamfer);
        assert_eq!(report.teardrops, 1);
        let b = outer(&out, 0.1);
        assert!(
            (b.x - 29.2).abs() < 0.005 && (b.y - 19.2).abs() < 0.005,
            "{b}"
        );
        let w = gap(&out, 7.5 + 3.6, 15.0, 10.0);
        assert!((w - 2.0 * (3.0 * SQRT_2 - 3.6)).abs() < 0.02, "{w}");
        let dv = m.volume() - out.volume();
        let want = chamfer_volume(30.0, 20.0) + cap32(3.0) * 30.0;
        eprintln!("chamfer and point: volume change {dv:.2} mm³ (want {want:.2})");
        assert!((dv - want).abs() < 0.01 * want, "{dv}");
        assert_eq!(out.open_edges(), 0);
    }

    #[test]
    fn no_work_runs_no_openscad() {
        // A binary that does not exist and a cancel flag that is set: a run
        // of OpenSCAD gives an error.
        let cfg = Config {
            binary: "/nonexistent/cocad-test-openscad".into(),
            ..Config::default()
        };
        let cancel = Arc::new(AtomicBool::new(true));
        let cube = crate::mesh::cube(10.0);
        let sheet = transformed(&cube, Mat4::from_scale(Vec3::new(1.0, 1.0, 0.06)));
        let cases = [
            (&cube, Settings::default()),
            (&cube, teardrops()),
            (
                &cube,
                Settings {
                    chamfer_mm: 0.0,
                    ..chamfer()
                },
            ),
            // A sheet of 0.6 mm is too thin for a chamfer of 0.4 mm.
            (
                &sheet,
                Settings {
                    teardrops: true,
                    ..chamfer()
                },
            ),
        ];
        for (mesh, s) in cases {
            let (out, report) = fixes::apply(
                &cfg,
                mesh,
                &s,
                Layers::default(),
                &Limits::default(),
                &Area::default(),
                &cancel,
            )
            .unwrap();
            assert_eq!(out.triangles, mesh.triangles);
            assert_eq!(out.vertices, mesh.vertices);
            assert!(!report.chamfer && report.teardrops == 0);
        }
        // With work to do, the run fails as expected.
        let r = fixes::apply(
            &cfg,
            &cube,
            &chamfer(),
            Layers::default(),
            &Limits::default(),
            &Area::default(),
            &cancel,
        );
        assert!(r.is_err());
    }
}
