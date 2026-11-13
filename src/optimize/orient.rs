//! Orientation (feature 2 of `docs/optimize-tab.md`): the rotation that
//! puts a part on the bed.
//!
//! `auto_down` finds the best face for the bed. The method follows the auto
//! orient of Tweaker-3 and OrcaSlicer (see "Prior art" in the design
//! document): the faces of the convex hull are the candidates, and a cost
//! from overhang, bed contact and height selects one. The code is our own.

use super::{Area, Limits};
use crate::mesh::Mesh;
use glam::{DMat3, DQuat, DVec2, DVec3, Mat4, Vec3};
use std::collections::HashMap;
use std::f64::consts::{FRAC_PI_2, PI};

/// How a part lies on the bed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Choice {
    /// The model rotation.
    Model,
    /// The best face, found by `auto_down`. The default: a model gives
    /// the parts in their position in use, not in print position.
    Auto,
    /// The user selected a face: this direction of the model points down.
    Face(Vec3),
}

/// The orientation of one part: the choice and quarter turns about z.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Orientation {
    pub choice: Choice,
    pub quarter_turns: u8,
}

impl Default for Orientation {
    fn default() -> Self {
        Orientation {
            choice: Choice::Auto,
            quarter_turns: 0,
        }
    }
}

/// Points and triangles nearer to the bed than this (mm) touch the bed.
const BED_TOL: f64 = 0.05;
/// A hull face with a smaller area (mm²) is not a candidate, unless no
/// other face is left.
const MIN_FACE_AREA: f64 = 5.0;
/// A contact polygon with a smaller area (mm²) is a point or a line: the
/// part does not stand on it.
const MIN_CONTACT: f64 = 1e-3;
/// The largest number of hull faces that `auto_down` measures. The largest
/// faces go first.
const MAX_CANDIDATES: usize = 64;
/// cos(5 degrees). A triangle with a normal nearer to -z than 5 degrees is
/// flat: on the bed it is contact area, above the bed it is a ceiling.
const FLAT_COS: f64 = 0.996_194_698;
/// cos(0.5 degrees). Hull facets with normals nearer than this join into
/// one face, when their corners are also near the plane (`MERGE_DIST`).
const MERGE_COS: f64 = 0.999_961_923;
/// The largest distance (mm) of a corner from the plane of a hull face.
const MERGE_DIST: f64 = 0.01;
/// cos(1 degree). `face_normal` joins triangles with normals nearer than
/// this, when their corners are also near the plane (`FACE_DIST`).
const FACE_COS: f64 = 0.999_847_695;
/// The largest distance (mm) of a corner from the plane of a picked face.
const FACE_DIST: f64 = 0.01;

// Weights of the cost terms. The terms are normalized: areas by the total
// surface area, the height by the largest size of the bounding box.
const W_OVERHANG: f64 = 1.0;
/// A flat ceiling prints as a bridge, so it counts less than an overhang.
const W_CEILING: f64 = 0.3;
const W_CONTACT: f64 = 1.0;
const W_HEIGHT: f64 = 0.25;
/// Auto keeps the model orientation when its cost is not more than this
/// fraction above the best cost.
const KEEP_MODEL: f64 = 0.05;

/// The direction of the model frame that must point down for the best
/// print. Returns exactly -z when the model orientation is good: its cost
/// is near the best cost and the part stands on its bed face.
pub fn auto_down(mesh: &Mesh, limits: &Limits) -> Vec3 {
    let prep = Prep::new(mesh);
    if prep.total <= 0.0 || prep.size <= 0.0 {
        return Vec3::NEG_Z;
    }
    let com = prep.center_of_mass();
    let sin_over = (limits.overhang_deg as f64).to_radians().sin();

    // The model orientation is a candidate when the part stands on the
    // points at the bottom.
    let model = DVec3::NEG_Z;
    let lowest = prep.pts.iter().map(|p| p.z).fold(f64::INFINITY, f64::min);
    let bottom = prep.pts.iter().copied().filter(|p| p.z <= lowest + BED_TOL);
    let (_, model_ok) = support(bottom, model, com);

    // The faces of the convex hull. The part falls over from a face when
    // the center of mass is not above it.
    let faces = hull_faces(&prep, com);
    let stable: Vec<&HullFace> = faces.iter().filter(|f| f.stable).collect();
    let mut cands: Vec<&HullFace> = stable
        .iter()
        .copied()
        .filter(|f| f.area >= MIN_FACE_AREA)
        .collect();
    if cands.is_empty() {
        cands = stable;
    }
    cands.sort_by(|a, b| b.area.total_cmp(&a.area));
    cands.truncate(MAX_CANDIDATES);

    let mut heights = Vec::new();
    let mut best: Option<(f64, DVec3)> = None;
    for f in &cands {
        if model_ok && f.down.dot(model) >= MERGE_COS {
            // This face is the bed face of the model orientation.
            continue;
        }
        let cost = prep.cost(f.down, sin_over, &mut heights);
        if best.is_none_or(|(b, _)| cost < b) {
            best = Some((cost, f.down));
        }
    }
    if model_ok {
        let cost = prep.cost(model, sin_over, &mut heights);
        match best {
            Some((b, _)) if cost > b * (1.0 + KEEP_MODEL) => {}
            _ => return Vec3::NEG_Z,
        }
    }
    best.map_or(Vec3::NEG_Z, |(_, d)| snap(d))
}

/// A rotation that turns `down` to -z. The rotation is the shortest arc,
/// so -z gives the identity, and +z gives a half turn about x.
pub fn rotation_down(down: Vec3) -> Mat4 {
    let d = down.as_dvec3().normalize_or_zero();
    if d == DVec3::ZERO {
        return Mat4::IDENTITY;
    }
    let axis = d.cross(DVec3::NEG_Z);
    let q = if axis.length_squared() < 1e-24 && d.z > 0.0 {
        DQuat::from_xyzw(1.0, 0.0, 0.0, 0.0)
    } else {
        DQuat::from_xyzw(axis.x, axis.y, axis.z, 1.0 - d.z).normalize()
    };
    clean_mat4(DMat3::from_quat(q))
}

/// The matrix that takes `mesh` from the model frame to the print frame.
///
/// 1. `Model` keeps the model rotation. `Auto` turns the direction of
///    `auto_down` down, `Face(d)` turns `d` down. For `Auto` and `Face`,
///    a turn about z then puts the long side of the smallest bounding
///    rectangle of the footprint along the long axis of `area`.
/// 2. The quarter turns about z.
/// 3. The lowest point goes to z = 0 and the footprint center to x = y = 0.
pub fn matrix(mesh: &Mesh, o: &Orientation, area: &Area, limits: &Limits) -> Mat4 {
    let down = match o.choice {
        Choice::Model => None,
        Choice::Auto => Some(auto_down(mesh, limits)),
        Choice::Face(d) => Some(d),
    };
    let mut r = Mat4::IDENTITY;
    if let Some(d) = down {
        r = rotation_down(d);
        let foot: Vec<DVec2> = mesh
            .vertices
            .iter()
            .map(|v| r.transform_point3(*v).as_dvec3().truncate())
            .collect();
        r = rotation_z(long_side_turn(&foot, area.w >= area.d)) * r;
    }
    r = rotation_z(FRAC_PI_2 * (o.quarter_turns % 4) as f64) * r;
    let moved = super::transformed(mesh, r);
    super::rest_on_bed(&moved) * r
}

/// True when `print_mesh` (print frame) fits the print area: the footprint
/// fits `area.w` x `area.d` as it is or turned by 90 degrees, and the
/// height is not more than `area.h`.
pub fn fits(print_mesh: &Mesh, area: &Area) -> bool {
    const EPS: f32 = 1e-3;
    let Some(b) = print_mesh.bounds() else {
        return true;
    };
    let s = b.size();
    let flat = (s.x <= area.w + EPS && s.y <= area.d + EPS)
        || (s.x <= area.d + EPS && s.y <= area.w + EPS);
    flat && s.z <= area.h + EPS
}

/// The outward normal of the flat face that contains triangle `tri`. The
/// face is the connected set of triangles in the same plane (in a small
/// tolerance). The result is their normal, weighted by area, so a click on
/// one triangle of a flat face gives the normal of the face. Returns -z for
/// a triangle that does not exist or has no area.
pub fn face_normal(mesh: &Mesh, tri: usize) -> Vec3 {
    if tri >= mesh.triangles.len() {
        return Vec3::NEG_Z;
    }
    let prep = Prep::new(mesh);
    if prep.areas[tri] <= 0.0 {
        return Vec3::NEG_Z;
    }
    let mut edges: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
    for (i, t) in prep.tris.iter().enumerate() {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            edges.entry((a.min(b), a.max(b))).or_default().push(i);
        }
    }
    // The first pass uses the plane of the picked triangle. A thin triangle
    // can have a less accurate normal, so the second pass uses the mean
    // plane of the first pass.
    let n0 = prep.normals[tri];
    let d0 = n0.dot(prep.pts[prep.tris[tri][0]]);
    let first = prep.flat_region(&edges, tri, n0, d0);
    let (n1, c1) = prep.mean_plane(&first);
    let second = prep.flat_region(&edges, tri, n1, n1.dot(c1));
    let (n2, _) = prep.mean_plane(&second);
    snap(n2)
}

/// The mesh data that this module measures, in f64.
struct Prep {
    pts: Vec<DVec3>,
    tris: Vec<[usize; 3]>,
    /// Unit normal of each triangle (zero when it has no area).
    normals: Vec<DVec3>,
    areas: Vec<f64>,
    /// Total surface area.
    total: f64,
    /// Largest size of the bounding box.
    size: f64,
}

impl Prep {
    fn new(mesh: &Mesh) -> Prep {
        let pts: Vec<DVec3> = mesh.vertices.iter().map(|v| v.as_dvec3()).collect();
        let tris: Vec<[usize; 3]> = mesh
            .triangles
            .iter()
            .map(|t| t.map(|i| i as usize))
            .collect();
        let mut normals = Vec::with_capacity(tris.len());
        let mut areas = Vec::with_capacity(tris.len());
        for t in &tris {
            let [a, b, c] = t.map(|i| pts[i]);
            let n = (b - a).cross(c - a);
            let len = n.length();
            areas.push(len * 0.5);
            normals.push(if len > 0.0 { n / len } else { DVec3::ZERO });
        }
        let total = areas.iter().sum();
        let size = bounds(&pts).map_or(0.0, |(lo, hi)| (hi - lo).max_element());
        Prep {
            pts,
            tris,
            normals,
            areas,
            total,
            size,
        }
    }

    /// The center of mass of the solid (uniform density). For a mesh with
    /// no volume, the center of its surface.
    fn center_of_mass(&self) -> DVec3 {
        let Some((lo, hi)) = bounds(&self.pts) else {
            return DVec3::ZERO;
        };
        let o = (lo + hi) * 0.5;
        let mut vol = 0.0;
        let mut sum = DVec3::ZERO;
        for t in &self.tris {
            let [a, b, c] = t.map(|i| self.pts[i] - o);
            let det = a.dot(b.cross(c));
            vol += det;
            sum += (a + b + c) * det;
        }
        if vol.abs() > 1e-9 * self.size.powi(3) {
            return o + sum / (4.0 * vol);
        }
        if self.total <= 0.0 {
            return o;
        }
        let mut s = DVec3::ZERO;
        for (i, t) in self.tris.iter().enumerate() {
            let [a, b, c] = t.map(|i| self.pts[i]);
            s += (a + b + c) / 3.0 * self.areas[i];
        }
        s / self.total
    }

    /// The cost of the print with `down` turned to -z. Lower is better.
    /// The value is not negative. `heights` is a buffer.
    fn cost(&self, down: DVec3, sin_over: f64, heights: &mut Vec<f64>) -> f64 {
        let up = -down;
        heights.clear();
        heights.extend(self.pts.iter().map(|p| up.dot(*p)));
        let lo = heights.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = heights.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let mut overhang = 0.0;
        let mut contact = 0.0;
        for (i, t) in self.tris.iter().enumerate() {
            let nz = up.dot(self.normals[i]);
            let top = heights[t[0]].max(heights[t[1]]).max(heights[t[2]]) - lo;
            if top <= BED_TOL {
                if nz < -FLAT_COS {
                    contact += self.areas[i];
                }
            } else if nz < -sin_over {
                let w = if nz < -FLAT_COS { W_CEILING } else { 1.0 };
                overhang += w * self.areas[i];
            }
        }
        W_OVERHANG * overhang / self.total
            + W_CONTACT * (0.5 - contact / self.total).max(0.0)
            + W_HEIGHT * (hi - lo) / self.size
    }

    /// The triangles connected to `seed` (through edges) that lie in the
    /// plane `n · p = d`, with a normal near `n`.
    fn flat_region(
        &self,
        edges: &HashMap<(usize, usize), Vec<usize>>,
        seed: usize,
        n: DVec3,
        d: f64,
    ) -> Vec<usize> {
        let in_plane = |t: usize| {
            self.tris[t]
                .iter()
                .all(|&v| (n.dot(self.pts[v]) - d).abs() <= FACE_DIST)
                && (self.areas[t] <= 0.0 || self.normals[t].dot(n) >= FACE_COS)
        };
        let mut seen = vec![false; self.tris.len()];
        seen[seed] = true;
        let mut out = vec![seed];
        let mut next = 0;
        while next < out.len() {
            let t = self.tris[out[next]];
            next += 1;
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                for &g in &edges[&(a.min(b), a.max(b))] {
                    if !seen[g] && in_plane(g) {
                        seen[g] = true;
                        out.push(g);
                    }
                }
            }
        }
        // A fixed order gives the same sums for each pick on the face.
        out.sort_unstable();
        out
    }

    /// The normal (weighted by area) and the center of the triangles `tris`.
    fn mean_plane(&self, tris: &[usize]) -> (DVec3, DVec3) {
        let mut n = DVec3::ZERO;
        let mut c = DVec3::ZERO;
        let mut area = 0.0;
        for &t in tris {
            let a = self.areas[t];
            n += self.normals[t] * a;
            c += self.tris[t].iter().map(|&v| self.pts[v]).sum::<DVec3>() / 3.0 * a;
            area += a;
        }
        (n.normalize_or_zero(), if area > 0.0 { c / area } else { c })
    }
}

/// The smallest and the largest corner of `pts`.
fn bounds(pts: &[DVec3]) -> Option<(DVec3, DVec3)> {
    let first = *pts.first()?;
    Some(
        pts.iter()
            .fold((first, first), |(lo, hi), p| (lo.min(*p), hi.max(*p))),
    )
}

/// `d` as a unit f32 vector. Very small components become zero, so that a
/// face normal along an axis gives a clean axis.
fn snap(d: DVec3) -> Vec3 {
    let c = |v: f64| if v.abs() < 1e-9 { 0.0 } else { v };
    let s = DVec3::new(c(d.x), c(d.y), c(d.z)).normalize_or_zero();
    if s == DVec3::ZERO {
        Vec3::NEG_Z
    } else {
        s.as_vec3()
    }
}

/// `m` as a Mat4. Values very near -1, 0 or 1 become exact.
fn clean_mat4(m: DMat3) -> Mat4 {
    let clean = |v: f64| {
        [-1.0, 0.0, 1.0]
            .into_iter()
            .find(|t| (v - t).abs() < 1e-12)
            .unwrap_or(v)
    };
    let cols = m.to_cols_array().map(clean);
    Mat4::from_mat3(DMat3::from_cols_array(&cols).as_mat3())
}

/// A turn by `angle` (radians) about z. Quarter turns are exact.
fn rotation_z(angle: f64) -> Mat4 {
    clean_mat4(DMat3::from_rotation_z(angle))
}

/// The turn about z (radians, more than -90 and not more than 90 degrees)
/// that puts the long side of the smallest bounding rectangle of `pts`
/// along x (`along_x`) or y. The rectangle comes from the edges of the
/// convex hull (rotating calipers). When two rectangles are equal, the
/// smaller turn wins.
fn long_side_turn(pts: &[DVec2], along_x: bool) -> f64 {
    let target = if along_x { 0.0 } else { FRAC_PI_2 };
    let hull = hull2(pts.to_vec());
    match hull.len() {
        0 | 1 => return 0.0,
        2 => {
            let e = hull[1] - hull[0];
            return reduce_turn(target - e.y.atan2(e.x));
        }
        _ => {}
    }
    let mut options: Vec<(f64, f64)> = Vec::new();
    for i in 0..hull.len() {
        let e = (hull[(i + 1) % hull.len()] - hull[i]).normalize_or_zero();
        if e == DVec2::ZERO {
            continue;
        }
        let p = e.perp();
        let (mut lo_e, mut hi_e, mut lo_p, mut hi_p) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
        for q in &hull {
            let (a, b) = (q.dot(e), q.dot(p));
            lo_e = lo_e.min(a);
            hi_e = hi_e.max(a);
            lo_p = lo_p.min(b);
            hi_p = hi_p.max(b);
        }
        let (len_e, len_p) = (hi_e - lo_e, hi_p - lo_p);
        let area = len_e * len_p;
        let angle = e.y.atan2(e.x);
        if len_e >= len_p * (1.0 - 1e-9) {
            options.push((area, reduce_turn(target - angle)));
        }
        if len_p >= len_e * (1.0 - 1e-9) {
            options.push((area, reduce_turn(target - angle - FRAC_PI_2)));
        }
    }
    let min = options.iter().map(|o| o.0).fold(f64::INFINITY, f64::min);
    options
        .iter()
        .filter(|o| o.0 <= min * (1.0 + 1e-9) + 1e-12)
        .map(|o| o.1)
        .min_by(|a, b| a.abs().total_cmp(&b.abs()))
        .unwrap_or(0.0)
}

/// `a` (radians) moved into the range (-90, 90] degrees by half turns.
/// Values very near 0 or 90 degrees become exact.
fn reduce_turn(a: f64) -> f64 {
    let mut t = a.rem_euclid(PI);
    if t > FRAC_PI_2 + 1e-12 {
        t -= PI;
    }
    if t.abs() < 1e-12 {
        0.0
    } else if (t - FRAC_PI_2).abs() < 1e-12 {
        FRAC_PI_2
    } else {
        t
    }
}

/// The convex hull of `p`, counter clockwise, without collinear points
/// (monotone chain).
fn hull2(mut p: Vec<DVec2>) -> Vec<DVec2> {
    p.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    p.dedup();
    if p.len() < 3 {
        return p;
    }
    let turn = |o: DVec2, a: DVec2, b: DVec2| (a - o).perp_dot(b - o);
    let mut h: Vec<DVec2> = Vec::with_capacity(p.len() + 1);
    for &q in &p {
        while h.len() >= 2 && turn(h[h.len() - 2], h[h.len() - 1], q) <= 0.0 {
            h.pop();
        }
        h.push(q);
    }
    let lower = h.len() + 1;
    for &q in p.iter().rev().skip(1) {
        while h.len() >= lower && turn(h[h.len() - 2], h[h.len() - 1], q) <= 0.0 {
            h.pop();
        }
        h.push(q);
    }
    h.pop();
    h
}

/// The area of a counter clockwise polygon.
fn poly_area(p: &[DVec2]) -> f64 {
    (0..p.len())
        .map(|i| p[i].perp_dot(p[(i + 1) % p.len()]))
        .sum::<f64>()
        * 0.5
}

/// The contact polygon of `pts` (points in one plane with the outward
/// normal `down`): its area, and true when `com` seen along `down` is in
/// the polygon (the part stands on it and does not fall over).
fn support(pts: impl Iterator<Item = DVec3>, down: DVec3, com: DVec3) -> (f64, bool) {
    let (e1, e2) = down.normalize().any_orthonormal_pair();
    let to2 = |p: DVec3| DVec2::new(p.dot(e1), p.dot(e2));
    let poly = hull2(pts.map(to2).collect());
    if poly.len() < 3 {
        return (0.0, false);
    }
    let c = to2(com);
    let area = poly_area(&poly);
    let inside = (0..poly.len()).all(|i| {
        let a = poly[i];
        let e = poly[(i + 1) % poly.len()] - a;
        e.perp_dot(c - a) >= -1e-9 * e.length()
    });
    (area, area >= MIN_CONTACT && inside)
}

/// A face of the convex hull: joined coplanar hull facets.
struct HullFace {
    /// The outward normal: this direction points down when the part lies
    /// on the face.
    down: DVec3,
    /// The area of the face.
    area: f64,
    /// The center of mass is above the face.
    stable: bool,
}

/// The faces of the convex hull of `prep`. Each face is a set of
/// connected hull facets in one plane.
fn hull_faces(prep: &Prep, com: DVec3) -> Vec<HullFace> {
    let facets = convex_hull(&prep.pts);
    let pts = &prep.pts;
    let info: Vec<(DVec3, f64)> = facets
        .iter()
        .map(|f| {
            let [a, b, c] = f.map(|i| pts[i]);
            let n = (b - a).cross(c - a);
            (n.normalize_or_zero(), n.length() * 0.5)
        })
        .collect();
    let mut owner: HashMap<(usize, usize), usize> = HashMap::new();
    for (i, f) in facets.iter().enumerate() {
        for k in 0..3 {
            owner.insert((f[k], f[(k + 1) % 3]), i);
        }
    }
    let mut order: Vec<usize> = (0..facets.len()).collect();
    order.sort_by(|&a, &b| info[b].1.total_cmp(&info[a].1));

    let mut group = vec![usize::MAX; facets.len()];
    let mut out = Vec::new();
    for &s in &order {
        if group[s] != usize::MAX || info[s].0 == DVec3::ZERO {
            continue;
        }
        // Grow the face from its largest facet. All facets must be near
        // the plane of this facet, so a curved surface does not join.
        let (n0, _) = info[s];
        let d0 = n0.dot(pts[facets[s][0]]);
        let id = out.len();
        group[s] = id;
        let mut members = vec![s];
        let mut next = 0;
        while next < members.len() {
            let f = facets[members[next]];
            next += 1;
            for k in 0..3 {
                let Some(&g) = owner.get(&(f[(k + 1) % 3], f[k])) else {
                    continue;
                };
                let near = facets[g]
                    .iter()
                    .all(|&v| (n0.dot(pts[v]) - d0).abs() <= MERGE_DIST);
                if group[g] == usize::MAX && info[g].0.dot(n0) >= MERGE_COS && near {
                    group[g] = id;
                    members.push(g);
                }
            }
        }
        let down = members
            .iter()
            .map(|&f| info[f].0 * info[f].1)
            .sum::<DVec3>()
            .normalize_or_zero();
        if down == DVec3::ZERO {
            continue;
        }
        let mut corners: Vec<usize> = members.iter().flat_map(|&f| facets[f]).collect();
        corners.sort_unstable();
        corners.dedup();
        let (area, stable) = support(corners.iter().map(|&i| pts[i]), down, com);
        out.push(HullFace { down, area, stable });
    }
    out
}

/// A facet of the 3D convex hull during the build.
struct Facet {
    v: [usize; 3],
    n: DVec3,
    d: f64,
    /// Points outside this facet that are not on the hull yet.
    outside: Vec<usize>,
    alive: bool,
}

impl Facet {
    fn new(pts: &[DVec3], v: [usize; 3]) -> Facet {
        let [a, b, c] = v.map(|i| pts[i]);
        let n = (b - a).cross(c - a).normalize_or_zero();
        Facet {
            v,
            n,
            d: n.dot(a),
            outside: Vec::new(),
            alive: true,
        }
    }

    fn dist(&self, p: DVec3) -> f64 {
        self.n.dot(p) - self.d
    }
}

/// The triangles of the 3D convex hull of `pts` (quickhull), counter
/// clockwise seen from outside. Empty when the points are flat. Points
/// nearer to the hull than a small tolerance are inside.
fn convex_hull(pts: &[DVec3]) -> Vec<[usize; 3]> {
    let Some((lo, hi)) = bounds(pts) else {
        return Vec::new();
    };
    let scale = (hi - lo).max_element();
    if pts.len() < 4 || scale <= 0.0 {
        return Vec::new();
    }
    let eps = scale * 1e-6;

    // The first tetrahedron: the farthest pair of extreme points, the point
    // farthest from their line, and the point farthest from that plane.
    let mut ext = [0usize; 6];
    for (i, p) in pts.iter().enumerate() {
        for k in 0..3 {
            if p[k] < pts[ext[2 * k]][k] {
                ext[2 * k] = i;
            }
            if p[k] > pts[ext[2 * k + 1]][k] {
                ext[2 * k + 1] = i;
            }
        }
    }
    let (mut i0, mut i1, mut far) = (0, 0, -1.0);
    for a in 0..6 {
        for b in a + 1..6 {
            let d = pts[ext[a]].distance_squared(pts[ext[b]]);
            if d > far {
                (i0, i1, far) = (ext[a], ext[b], d);
            }
        }
    }
    if far.sqrt() <= eps {
        return Vec::new();
    }
    let dir = (pts[i1] - pts[i0]).normalize();
    let line_dist = |p: DVec3| {
        let r = p - pts[i0];
        (r - dir * r.dot(dir)).length()
    };
    let i2 = argmax(pts, line_dist);
    if line_dist(pts[i2]) <= eps {
        return Vec::new();
    }
    let n = (pts[i1] - pts[i0]).cross(pts[i2] - pts[i0]).normalize();
    let plane_dist = |p: DVec3| n.dot(p - pts[i0]).abs();
    let i3 = argmax(pts, plane_dist);
    if plane_dist(pts[i3]) <= eps {
        return Vec::new();
    }

    let tet = [i0, i1, i2, i3];
    let mut faces: Vec<Facet> = Vec::new();
    for (f, opp) in [([0, 1, 2], 3), ([0, 1, 3], 2), ([0, 2, 3], 1), ([1, 2, 3], 0)] {
        let mut v = f.map(|k| tet[k]);
        if Facet::new(pts, v).dist(pts[tet[opp]]) > 0.0 {
            v.swap(1, 2);
        }
        faces.push(Facet::new(pts, v));
    }
    let mut edges: HashMap<(usize, usize), usize> = HashMap::new();
    for (i, f) in faces.iter().enumerate() {
        for k in 0..3 {
            edges.insert((f.v[k], f.v[(k + 1) % 3]), i);
        }
    }
    for (i, p) in pts.iter().enumerate() {
        if tet.contains(&i) {
            continue;
        }
        if let Some(f) = faces.iter_mut().find(|f| f.dist(*p) > eps) {
            f.outside.push(i);
        }
    }

    let mut mark: Vec<u32> = vec![0; faces.len()];
    let mut stamp = 0u32;
    let mut fi = 0;
    while fi < faces.len() {
        if !faces[fi].alive || faces[fi].outside.is_empty() {
            fi += 1;
            continue;
        }
        let face = &faces[fi];
        let (k, _) = face
            .outside
            .iter()
            .enumerate()
            .map(|(k, &i)| (k, face.dist(pts[i])))
            .fold((0, f64::MIN), |b, c| if c.1 > b.1 { c } else { b });
        let pi = face.outside[k];
        let p = pts[pi];

        // The facets that see p, and the horizon: their edges to facets
        // that do not see p.
        stamp += 1;
        mark[fi] = stamp;
        let mut visible = vec![fi];
        let mut horizon: Vec<(usize, usize)> = Vec::new();
        let mut next = 0;
        while next < visible.len() {
            let v = faces[visible[next]].v;
            next += 1;
            for k in 0..3 {
                let (a, b) = (v[k], v[(k + 1) % 3]);
                match edges.get(&(b, a)) {
                    Some(&g) if mark[g] == stamp => {}
                    Some(&g) if faces[g].dist(p) > eps => {
                        mark[g] = stamp;
                        visible.push(g);
                    }
                    _ => horizon.push((a, b)),
                }
            }
        }
        if !one_loop(&horizon) {
            // Rounding made the horizon invalid. Ignore this point: it is
            // very near the hull.
            faces[fi].outside.swap_remove(k);
            continue;
        }

        let mut orphans = Vec::new();
        for &f in &visible {
            faces[f].alive = false;
            orphans.append(&mut faces[f].outside);
            let v = faces[f].v;
            for k in 0..3 {
                edges.remove(&(v[k], v[(k + 1) % 3]));
            }
        }
        let first_new = faces.len();
        for &(a, b) in &horizon {
            let id = faces.len();
            for e in [(a, b), (b, pi), (pi, a)] {
                edges.insert(e, id);
            }
            faces.push(Facet::new(pts, [a, b, pi]));
        }
        mark.resize(faces.len(), 0);
        for o in orphans {
            if o == pi {
                continue;
            }
            if let Some(f) = faces[first_new..].iter_mut().find(|f| f.dist(pts[o]) > eps) {
                f.outside.push(o);
            }
        }
    }
    faces.iter().filter(|f| f.alive).map(|f| f.v).collect()
}

/// The index of the point with the largest value of `f`.
fn argmax(pts: &[DVec3], f: impl Fn(DVec3) -> f64) -> usize {
    let mut best = (0, f64::MIN);
    for (i, p) in pts.iter().enumerate() {
        let v = f(*p);
        if v > best.1 {
            best = (i, v);
        }
    }
    best.0
}

/// True when the directed edges make one closed loop.
fn one_loop(edges: &[(usize, usize)]) -> bool {
    if edges.len() < 3 {
        return false;
    }
    let mut next = HashMap::with_capacity(edges.len());
    for &(a, b) in edges {
        if next.insert(a, b).is_some() {
            return false;
        }
    }
    let start = edges[0].0;
    let mut v = start;
    for step in 1..=edges.len() {
        match next.get(&v) {
            Some(&w) => v = w,
            None => return false,
        }
        if v == start {
            return step == edges.len();
        }
    }
    false
}

/// Meshes for the tests of this module and of `plate`.
#[cfg(test)]
pub(super) mod shapes {
    use crate::mesh::Mesh;
    use glam::{Mat4, Vec2, Vec3};

    /// A box from `lo` to `hi`.
    pub fn cuboid(lo: Vec3, hi: Vec3) -> Mesh {
        let m = crate::mesh::cube(1.0);
        super::super::transformed(&m, Mat4::from_translation(lo) * Mat4::from_scale(hi - lo))
    }

    /// The counter clockwise polygon `poly` (z = 0) moved up by `h`. The
    /// caps are a fan from `poly[fan]`, so the polygon must be star shaped
    /// from that point.
    pub fn extrude(poly: &[Vec2], fan: usize, h: f32) -> Mesh {
        let n = poly.len();
        let b = |i: usize| poly[i].extend(0.0);
        let t = |i: usize| poly[i].extend(h);
        let mut soup = Vec::new();
        for i in 0..n {
            let j = (i + 1) % n;
            soup.push([b(i), b(j), t(j)]);
            soup.push([b(i), t(j), t(i)]);
            if i != fan && j != fan {
                soup.push([b(fan), b(j), b(i)]);
                soup.push([t(fan), t(i), t(j)]);
            }
        }
        Mesh::from_soup(&soup)
    }

    /// A regular prism with `sides` sides, radius `r` and length `h`, on
    /// the z axis from 0 to h. One side faces +x.
    pub fn prism(sides: usize, r: f32, h: f32) -> Mesh {
        let step = std::f32::consts::TAU / sides as f32;
        let poly: Vec<Vec2> = (0..sides)
            .map(|k| Vec2::from_angle(step * (k as f32 + 0.5)) * r)
            .collect();
        // The +x side is between the last and the first corner.
        extrude(&poly, 0, h)
    }

    /// An L profile in the xy plane, extruded along z by 40: a 50 x 4 base
    /// on y = 0 and a 4 x 30 upright on x = 0.
    pub fn bracket() -> Mesh {
        let poly = [
            Vec2::new(0.0, 0.0),
            Vec2::new(50.0, 0.0),
            Vec2::new(50.0, 4.0),
            Vec2::new(4.0, 4.0),
            Vec2::new(4.0, 30.0),
            Vec2::new(0.0, 30.0),
        ];
        // The fan starts at the inner corner. Both legs are visible from it.
        extrude(&poly, 3, 40.0)
    }

    /// The meshes as one mesh (no boolean operation).
    pub fn join(parts: &[Mesh]) -> Mesh {
        let mut out = Mesh::default();
        for m in parts {
            let base = out.vertices.len() as u32;
            out.vertices.extend_from_slice(&m.vertices);
            out.triangles
                .extend(m.triangles.iter().map(|t| t.map(|v| v + base)));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::shapes::*;
    use super::*;
    use crate::optimize::transformed;
    use glam::{Quat, Vec4};

    fn limits() -> Limits {
        Limits::default()
    }

    fn print_bounds(mesh: &Mesh, o: &Orientation, area: &Area) -> crate::mesh::Bounds {
        let m = matrix(mesh, o, area, &limits());
        transformed(mesh, m).bounds().unwrap()
    }

    fn auto() -> Orientation {
        Orientation {
            choice: Choice::Auto,
            quarter_turns: 0,
        }
    }

    fn near(a: Vec3, b: Vec3, tol: f32) -> bool {
        (a - b).length() <= tol
    }

    /// An arbitrary rotation with no special axis.
    fn tilt() -> Mat4 {
        Mat4::from_quat(Quat::from_euler(glam::EulerRot::XYZ, 1.1, 0.7, 0.3))
    }

    #[test]
    fn standing_box_lies_down_on_its_largest_face() {
        let m = cuboid(Vec3::ZERO, Vec3::new(10.0, 20.0, 30.0));
        let d = auto_down(&m, &limits());
        assert!(d == Vec3::X || d == Vec3::NEG_X, "down = {d}");
        let b = print_bounds(&m, &auto(), &Area::default());
        assert!(near(b.size(), Vec3::new(30.0, 20.0, 10.0), 1e-4), "{:?}", b.size());
    }

    #[test]
    fn box_on_its_largest_face_keeps_the_model_orientation() {
        let m = cuboid(Vec3::ZERO, Vec3::new(30.0, 20.0, 10.0));
        assert_eq!(auto_down(&m, &limits()), Vec3::NEG_Z);
    }

    #[test]
    fn lying_cylinder_stands_up_on_an_end() {
        // The axis along x, one side on the bed.
        let m = transformed(
            &prism(32, 10.0, 40.0),
            Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2),
        );
        let b = m.bounds().unwrap();
        assert!(b.size().x > 39.9 && b.size().z < 20.0);
        let d = auto_down(&m, &limits());
        assert!(d.x.abs() > 0.9999, "down = {d}");
        let b = print_bounds(&m, &auto(), &Area::default());
        assert!((b.size().z - 40.0).abs() < 1e-3, "{:?}", b.size());
    }

    #[test]
    fn bracket_lies_on_its_large_flat_side() {
        // In print position the base (y = 0 in the profile) is on the bed.
        let print = Mat4::from_rotation_x(std::f32::consts::FRAC_PI_2);
        let m = transformed(&bracket(), print);
        assert_eq!(auto_down(&m, &limits()), Vec3::NEG_Z);

        // Turned in all axes, Auto finds the same face.
        let t = tilt();
        let turned = transformed(&m, t);
        let d = auto_down(&turned, &limits());
        let want = t.transform_vector3(Vec3::NEG_Z);
        assert!(near(d, want, 1e-4), "down = {d}, want {want}");
        let b = print_bounds(&turned, &auto(), &Area::default());
        assert!(near(b.size(), Vec3::new(50.0, 40.0, 30.0), 1e-3), "{:?}", b.size());
    }

    #[test]
    fn rotation_down_turns_each_axis_down() {
        assert_eq!(rotation_down(Vec3::NEG_Z), Mat4::IDENTITY);
        assert_eq!(
            rotation_down(Vec3::Z),
            Mat4::from_cols(Vec4::X, Vec4::NEG_Y, Vec4::NEG_Z, Vec4::W)
        );
        for d in [Vec3::X, Vec3::NEG_X, Vec3::Y, Vec3::NEG_Y, Vec3::Z, Vec3::NEG_Z] {
            let r = rotation_down(d);
            assert_eq!(r.transform_vector3(d), Vec3::NEG_Z, "{d}");
            assert!((r.determinant() - 1.0).abs() < 1e-6);
            // Axis directions give clean matrices.
            for v in r.to_cols_array() {
                assert!(v == 0.0 || v.abs() == 1.0, "{d}: {r}");
            }
        }
        let d = Vec3::new(0.3, -0.5, 0.8).normalize();
        assert!(near(rotation_down(d).transform_vector3(d), Vec3::NEG_Z, 1e-6));
    }

    #[test]
    fn matrix_puts_the_part_on_the_bed_at_the_center() {
        let m = transformed(&bracket(), tilt() * Mat4::from_translation(Vec3::new(5.0, -7.0, 3.0)));
        let area = Area::default();
        for o in [
            Orientation::default(),
            Orientation {
                choice: Choice::Model,
                quarter_turns: 3,
            },
            auto(),
            Orientation {
                choice: Choice::Face(Vec3::new(0.2, 0.3, -0.9).normalize()),
                quarter_turns: 1,
            },
        ] {
            let b = print_bounds(&m, &o, &area);
            assert!(b.min.z.abs() < 1e-4, "{o:?}: {:?}", b.min);
            let c = b.center();
            assert!(c.x.abs() < 1e-4 && c.y.abs() < 1e-4, "{o:?}: {c}");
        }
    }

    #[test]
    fn long_side_goes_along_the_long_axis_of_the_area() {
        let face = |q| Orientation {
            choice: Choice::Face(Vec3::NEG_Z),
            quarter_turns: q,
        };
        let wide = Area {
            w: 300.0,
            d: 200.0,
            h: 100.0,
        };
        let deep = Area {
            w: 200.0,
            d: 300.0,
            h: 100.0,
        };
        // Long along y in the model.
        let m = cuboid(Vec3::ZERO, Vec3::new(10.0, 40.0, 5.0));
        let size = |m: &Mesh, o, a| print_bounds(m, &o, a).size();
        assert!(near(size(&m, face(0), &wide), Vec3::new(40.0, 10.0, 5.0), 1e-4));
        assert!(near(size(&m, face(0), &deep), Vec3::new(10.0, 40.0, 5.0), 1e-4));
        assert!(near(size(&m, face(1), &wide), Vec3::new(10.0, 40.0, 5.0), 1e-4));
        // Turned by 30 degrees about z in the model.
        let turned = transformed(&m, Mat4::from_rotation_z(std::f32::consts::FRAC_PI_6));
        assert!(near(size(&turned, face(0), &wide), Vec3::new(40.0, 10.0, 5.0), 1e-3));
        // The model choice does not turn.
        let model = Orientation {
            choice: Choice::Model,
            quarter_turns: 0,
        };
        assert!(near(size(&m, model, &wide), Vec3::new(10.0, 40.0, 5.0), 1e-4));
    }

    #[test]
    fn fits_checks_both_turns_and_the_height() {
        let area = Area {
            w: 100.0,
            d: 50.0,
            h: 30.0,
        };
        let fit = |x, y, z| fits(&cuboid(Vec3::ZERO, Vec3::new(x, y, z)), &area);
        assert!(fit(100.0, 50.0, 30.0));
        assert!(fit(40.0, 90.0, 10.0));
        assert!(!fit(101.0, 10.0, 10.0));
        assert!(!fit(60.0, 60.0, 10.0));
        assert!(!fit(10.0, 10.0, 31.0));
    }

    #[test]
    fn face_normal_gives_the_normal_of_the_whole_face() {
        // Each cap of the bracket is 4 triangles, each side 2.
        let t = tilt();
        let m = transformed(&bracket(), t);
        for i in 0..m.triangles.len() {
            let n = face_normal(&m, i);
            let flat = m.normal(i);
            assert!(near(n, flat, 1e-4), "{i}: {n} {flat}");
        }
        // A side of the 32 sided prism is its own face.
        let p = prism(32, 10.0, 40.0);
        for i in 0..p.triangles.len() {
            assert!(near(face_normal(&p, i), p.normal(i), 1e-5));
        }
        // Each pick on the 4 triangles of the bottom cap gives the same
        // normal.
        let cap = t.transform_vector3(Vec3::NEG_Z);
        let picks: Vec<Vec3> = (0..m.triangles.len())
            .map(|i| face_normal(&m, i))
            .filter(|n| near(*n, cap, 1e-3))
            .collect();
        assert_eq!(picks.len(), 4);
        assert!(picks.iter().all(|n| *n == picks[0]));
    }
}
