//! The plate (feature 6 of `docs/optimize-tab.md`): parts with the same
//! shape, parts that overlap, and the layout of the parts on the plate.

use super::Area;
use crate::assembly::ray_triangle;
use crate::mesh::Mesh;
use glam::{DVec3, Vec2};
use std::collections::HashMap;

/// The relative tolerance of `same`.
const REL: f64 = 0.005;
/// Two principal moments are equal when they differ by less than this
/// fraction of their sum. Then the principal axes are not unique.
const DEGEN: f64 = 1e-3;
/// The absolute tolerance of the handedness values (no unit).
const HAND_ABS: f64 = 1e-4;
/// Bounding boxes must overlap by more than this (mm) on each axis.
const TOUCH: f64 = 1e-3;
/// `overlap` makes one mesh smaller by about this distance (mm), so that
/// faces that touch do not cross.
const SHRINK: f64 = 2e-3;
/// The test points of `overlap` are this distance (mm) inside a mesh.
const DELTA: f64 = 0.01;

/// Values of a part that do not change when the part moves or turns. A
/// part and its mirror image have different values (`hand`), unless the
/// part is the same as its mirror image.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Signature {
    /// Volume in mm³.
    pub volume: f64,
    /// Surface area in mm².
    pub area: f64,
    /// The principal moments of inertia (density 1), smallest first.
    pub moments: [f64; 3],
    /// The size along each principal axis, in the order of `moments`. When
    /// two moments are equal, their axes are not unique: then both values
    /// are the diameter about the third axis (two times the largest
    /// distance from it). When all three are equal, all values are the
    /// diameter about the center of mass.
    pub extents: [f64; 3],
    /// Two values that change sign in the mirror image: v1 v2 v3 and T123,
    /// with T the third moments in the right-handed principal frame (in
    /// units of the radius of gyration) and v_i = T_ijj. Both are 0 for a
    /// part that is the same as its mirror image, and when two moments are
    /// equal.
    pub hand: [f64; 2],
}

/// The signature of `mesh`, a closed solid.
pub fn signature(mesh: &Mesh) -> Signature {
    let pts: Vec<DVec3> = mesh.vertices.iter().map(|v| v.as_dvec3()).collect();
    let tris: Vec<[usize; 3]> = mesh
        .triangles
        .iter()
        .map(|t| t.map(|i| i as usize))
        .collect();
    let mut sig = Signature {
        volume: 0.0,
        area: 0.0,
        moments: [0.0; 3],
        extents: [0.0; 3],
        hand: [0.0; 2],
    };
    let Some((lo, hi)) = bounds(&pts) else {
        return sig;
    };

    // Volume, center of mass and area. Each triangle makes a tetrahedron
    // with the origin; `det` is 6 times its signed volume.
    let o = (lo + hi) * 0.5;
    let mut dets = 0.0;
    let mut first = DVec3::ZERO;
    for t in &tris {
        let [a, b, c] = t.map(|i| pts[i] - o);
        let det = a.dot(b.cross(c));
        dets += det;
        first += (a + b + c) * det;
        sig.area += (b - a).cross(c - a).length() * 0.5;
    }
    let size = (hi - lo).max_element();
    if dets.abs() <= 1e-9 * size.powi(3) {
        // Not a solid.
        let mut e = (hi - lo).to_array();
        e.sort_by(f64::total_cmp);
        sig.extents = e;
        return sig;
    }
    let com = o + first / (4.0 * dets);
    sig.volume = dets.abs() / 6.0;

    // Second moments about the center of mass, per unit volume. For a
    // tetrahedron (0, a, b, c): ∫ x_i x_j dV = det / 120 (S_i S_j + Q_ij),
    // with S = a + b + c and Q = aaᵀ + bbᵀ + ccᵀ.
    let mut cov = [[0.0; 3]; 3];
    for t in &tris {
        let [a, b, c] = t.map(|i| (pts[i] - com).to_array());
        let det = DVec3::from(a).dot(DVec3::from(b).cross(DVec3::from(c)));
        for (i, row) in cov.iter_mut().enumerate() {
            for (j, v) in row.iter_mut().enumerate() {
                let s = (a[i] + b[i] + c[i]) * (a[j] + b[j] + c[j]);
                *v += det * (s + a[i] * a[j] + b[i] * b[j] + c[i] * c[j]);
            }
        }
    }
    for v in cov.iter_mut().flatten() {
        *v /= 20.0 * dets;
    }
    let (mu, axes) = eigen(cov);
    let tr = mu[0] + mu[1] + mu[2];
    sig.moments = mu.map(|m| sig.volume * (tr - m));
    let eq01 = mu[0] - mu[1] <= DEGEN * tr;
    let eq12 = mu[1] - mu[2] <= DEGEN * tr;

    // The coordinates in the principal frame (right-handed).
    let e = [axes[0], axes[1], axes[0].cross(axes[1])];
    let ys: Vec<DVec3> = pts
        .iter()
        .map(|p| {
            let d = *p - com;
            DVec3::new(d.dot(e[0]), d.dot(e[1]), d.dot(e[2]))
        })
        .collect();
    let range = |k: usize| {
        let (l, h) = ys
            .iter()
            .fold((f64::MAX, f64::MIN), |(l, h), y| (l.min(y[k]), h.max(y[k])));
        h - l
    };
    let diameter = |f: fn(DVec3) -> f64| 2.0 * ys.iter().map(|y| f(*y)).fold(0.0, f64::max);
    sig.extents = match (eq01, eq12) {
        (true, true) => [diameter(|y| y.length()); 3],
        (true, false) => {
            let r = diameter(|y| y.truncate().length());
            [r, r, range(2)]
        }
        (false, true) => {
            let r = diameter(|y| (y.y * y.y + y.z * y.z).sqrt());
            [range(0), r, r]
        }
        (false, false) => [range(0), range(1), range(2)],
    };
    if !(eq01 || eq12) {
        sig.hand = handedness(&ys, &tris, tr.sqrt());
    }
    sig
}

/// The handedness values of `Signature::hand`. `ys` are the points in the
/// principal frame, `r` is the radius of gyration.
fn handedness(ys: &[DVec3], tris: &[[usize; 3]], r: f64) -> [f64; 2] {
    // For a tetrahedron (0, a, b, c):
    // ∫ x_i x_j x_k dV = det / 120 (S_i S_j S_k + S_i Q_jk + S_j Q_ik
    //                    + S_k Q_ij + 2 R_ijk),
    // with R = Σ over a, b, c of p_i p_j p_k.
    let mut m = [[[0.0; 3]; 3]; 3];
    let mut dets = 0.0;
    for t in tris {
        let p = t.map(|i| ys[i] / r);
        let det = p[0].dot(p[1].cross(p[2]));
        dets += det;
        let p = p.map(|v| v.to_array());
        let s: [f64; 3] = std::array::from_fn(|i| p[0][i] + p[1][i] + p[2][i]);
        let q = |i: usize, j: usize| p.iter().map(|v| v[i] * v[j]).sum::<f64>();
        let rr = |i: usize, j: usize, k: usize| p.iter().map(|v| v[i] * v[j] * v[k]).sum::<f64>();
        for (i, mi) in m.iter_mut().enumerate() {
            for (j, mij) in mi.iter_mut().enumerate() {
                for (k, v) in mij.iter_mut().enumerate() {
                    let sum = s[i] * s[j] * s[k]
                        + s[i] * q(j, k)
                        + s[j] * q(i, k)
                        + s[k] * q(i, j)
                        + 2.0 * rr(i, j, k);
                    *v += det * sum;
                }
            }
        }
    }
    // Per unit volume: the volume is dets / 6.
    let t = |i: usize, j: usize, k: usize| m[i][j][k] / (20.0 * dets);
    let v: [f64; 3] = std::array::from_fn(|a| t(a, 0, 0) + t(a, 1, 1) + t(a, 2, 2));
    [v[0] * v[1] * v[2], t(0, 1, 2)]
}

/// Eigenvalues (largest first) and unit eigenvectors of the symmetric
/// matrix `m` (Jacobi method).
fn eigen(m: [[f64; 3]; 3]) -> ([f64; 3], [DVec3; 3]) {
    let mut a = m;
    let mut v = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    for _ in 0..50 {
        let off = a[0][1] * a[0][1] + a[0][2] * a[0][2] + a[1][2] * a[1][2];
        let diag = a[0][0] * a[0][0] + a[1][1] * a[1][1] + a[2][2] * a[2][2];
        if off <= 1e-30 * diag || off == 0.0 {
            break;
        }
        for (p, q) in [(0, 1), (0, 2), (1, 2)] {
            if a[p][q] == 0.0 {
                continue;
            }
            let theta = (a[q][q] - a[p][p]) / (2.0 * a[p][q]);
            let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
            let c = 1.0 / (t * t + 1.0).sqrt();
            let s = t * c;
            for row in a.iter_mut() {
                let (x, y) = (row[p], row[q]);
                row[p] = c * x - s * y;
                row[q] = s * x + c * y;
            }
            let (rp, rq) = (a[p], a[q]);
            a[p] = std::array::from_fn(|k| c * rp[k] - s * rq[k]);
            a[q] = std::array::from_fn(|k| s * rp[k] + c * rq[k]);
            for row in v.iter_mut() {
                let (x, y) = (row[p], row[q]);
                row[p] = c * x - s * y;
                row[q] = s * x + c * y;
            }
        }
    }
    let mut order = [0, 1, 2];
    order.sort_by(|&i, &j| a[j][j].total_cmp(&a[i][i]));
    (
        order.map(|i| a[i][i]),
        order.map(|i| DVec3::new(v[0][i], v[1][i], v[2][i]).normalize()),
    )
}

/// True when `a` and `b` are the same part: all values agree in a relative
/// tolerance of 0.5 percent plus a small absolute tolerance.
pub fn same(a: &Signature, b: &Signature) -> bool {
    let close = |x: f64, y: f64, abs: f64| (x - y).abs() <= REL * x.abs().max(y.abs()) + abs;
    let m_abs = 1e-6 * a.moments[2].max(b.moments[2]);
    close(a.volume, b.volume, 1e-3)
        && close(a.area, b.area, 1e-3)
        && (0..3).all(|k| close(a.moments[k], b.moments[k], m_abs))
        && (0..3).all(|k| close(a.extents[k], b.extents[k], 0.01))
        && (0..2).all(|k| close(a.hand[k], b.hand[k], HAND_ABS))
}

/// Groups of the same parts, in the order of the first part of each group.
/// A part joins the first group whose first part is the same.
pub fn group_same(sigs: &[Signature]) -> Vec<Vec<usize>> {
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for (i, s) in sigs.iter().enumerate() {
        match groups.iter_mut().find(|g| same(&sigs[g[0]], s)) {
            Some(g) => g.push(i),
            None => groups.push(vec![i]),
        }
    }
    groups
}

/// True when the closed meshes `a` and `b` share volume. Meshes that only
/// touch (a lid that sits on a box) do not overlap.
///
/// The test: the bounding boxes overlap with a volume. Then either the
/// surfaces cross (an edge of one mesh goes through a triangle of the
/// other, with one mesh made a little smaller so that faces that touch do
/// not count), or a point just inside one mesh is inside the other mesh
/// (one mesh contains the other).
pub fn overlap(a: &Mesh, b: &Mesh) -> bool {
    let (Some(sa), Some(sb)) = (Solid::new(a), Solid::new(b)) else {
        return false;
    };
    let lo = sa.lo.max(sb.lo);
    let hi = sa.hi.min(sb.hi);
    if (hi - lo).min_element() <= TOUCH {
        return false;
    }
    let small = sa.shrunk();
    crosses(&small, &sb, lo, hi)
        || crosses(&sb, &small, lo, hi)
        || points_inside(&sa, &sb, lo, hi)
        || points_inside(&sb, &sa, lo, hi)
}

/// Groups of parts that are joined by overlap, in the order of the first
/// part of each group. Each part is in exactly one group.
pub fn overlap_groups(meshes: &[&Mesh]) -> Vec<Vec<usize>> {
    let n = meshes.len();
    let boxes: Vec<Option<(glam::Vec3, glam::Vec3)>> = meshes
        .iter()
        .map(|m| m.bounds().map(|b| (b.min, b.max)))
        .collect();
    let mut parent: Vec<usize> = (0..n).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for i in 0..n {
        for j in i + 1..n {
            let (Some(bi), Some(bj)) = (boxes[i], boxes[j]) else {
                continue;
            };
            let gap = bi.1.min(bj.1) - bi.0.max(bj.0);
            if gap.min_element() as f64 <= TOUCH {
                continue;
            }
            let (ri, rj) = (root(&mut parent, i), root(&mut parent, j));
            if ri != rj && overlap(meshes[i], meshes[j]) {
                parent[ri.max(rj)] = ri.min(rj);
            }
        }
    }
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut index: HashMap<usize, usize> = HashMap::new();
    for i in 0..n {
        let r = root(&mut parent, i);
        let g = *index.entry(r).or_insert_with(|| {
            groups.push(Vec::new());
            groups.len() - 1
        });
        groups[g].push(i);
    }
    groups
}

/// A mesh in f64 with its bounding box.
struct Solid {
    pts: Vec<DVec3>,
    tris: Vec<[usize; 3]>,
    lo: DVec3,
    hi: DVec3,
}

impl Solid {
    fn new(mesh: &Mesh) -> Option<Solid> {
        let pts: Vec<DVec3> = mesh.vertices.iter().map(|v| v.as_dvec3()).collect();
        let (lo, hi) = bounds(&pts)?;
        let tris = mesh
            .triangles
            .iter()
            .map(|t| t.map(|i| i as usize))
            .collect();
        Some(Solid { pts, tris, lo, hi })
    }

    fn tri(&self, i: usize) -> [DVec3; 3] {
        self.tris[i].map(|v| self.pts[v])
    }

    fn tri_box(&self, i: usize) -> (DVec3, DVec3) {
        let [a, b, c] = self.tri(i);
        (a.min(b).min(c), a.max(b).max(c))
    }

    /// The solid with each point moved inside by `SHRINK`, along the mean
    /// normal (weighted by angle) of its triangles.
    fn shrunk(&self) -> Solid {
        let mut normals = vec![DVec3::ZERO; self.pts.len()];
        for t in &self.tris {
            let p = t.map(|v| self.pts[v]);
            let n = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero();
            for k in 0..3 {
                let u = (p[(k + 1) % 3] - p[k]).normalize_or_zero();
                let w = (p[(k + 2) % 3] - p[k]).normalize_or_zero();
                normals[t[k]] += n * u.dot(w).clamp(-1.0, 1.0).acos();
            }
        }
        let pts: Vec<DVec3> = self
            .pts
            .iter()
            .zip(&normals)
            .map(|(p, n)| *p - n.normalize_or_zero() * SHRINK)
            .collect();
        let (lo, hi) = bounds(&pts).unwrap_or((self.lo, self.hi));
        Solid {
            pts,
            tris: self.tris.clone(),
            lo,
            hi,
        }
    }

    /// True when `p` is inside the solid: an odd number of crossings on at
    /// least two of three rays.
    fn contains(&self, rays: &[Rays], p: DVec3) -> bool {
        let mut votes = 0;
        for (k, r) in rays.iter().enumerate() {
            let cell = r.grid.cell_of(DVec3::new(p.dot(r.u), p.dot(r.w), 0.0));
            let mut hits = 0;
            if let Some(cell) = cell {
                let (o, d) = (p.as_vec3(), r.dir.as_vec3());
                for &t in &r.grid.cells[cell] {
                    let tri = self.tri(t as usize).map(|v| v.as_vec3());
                    if ray_triangle(o, d, tri).is_some() {
                        hits += 1;
                    }
                }
            }
            if hits % 2 == 1 {
                votes += 1;
            }
            // Stop when the result is known.
            if votes >= 2 || votes + (rays.len() - k - 1) < 2 {
                break;
            }
        }
        votes >= 2
    }

    /// The ray lookup of `contains`, for three fixed ray directions.
    fn rays(&self) -> Vec<Rays> {
        let margin = 1e-6 * (self.hi - self.lo).max_element() + 1e-9;
        [
            DVec3::new(0.267, 0.534, 0.802),
            DVec3::new(-0.625, 0.282, 0.728),
            DVec3::new(0.431, -0.786, 0.444),
        ]
        .map(|d| {
            let dir = d.normalize();
            let (u, w) = dir.any_orthonormal_pair();
            let flat = |p: DVec3| DVec3::new(p.dot(u), p.dot(w), 0.0);
            let boxes: Vec<(DVec3, DVec3)> = (0..self.tris.len())
                .map(|i| {
                    let [a, b, c] = self.tri(i).map(flat);
                    (a.min(b).min(c) - margin, a.max(b).max(c) + margin)
                })
                .collect();
            let (lo, hi) = boxes.iter().fold((DVec3::MAX, DVec3::MIN), |(l, h), b| {
                (l.min(b.0), h.max(b.1))
            });
            let mut grid = Grid::new(lo, hi, boxes.len());
            for (i, b) in boxes.iter().enumerate() {
                grid.insert(b.0, b.1, i as u32);
            }
            Rays { dir, u, w, grid }
        })
        .into()
    }
}

/// The triangles of a solid seen along one ray direction.
struct Rays {
    dir: DVec3,
    /// Two axes at right angles to `dir`.
    u: DVec3,
    w: DVec3,
    /// The triangles by their box in (u, w).
    grid: Grid,
}

/// A uniform grid of cells over a box. Each cell lists the items whose box
/// touches it.
struct Grid {
    lo: DVec3,
    cell: DVec3,
    n: [usize; 3],
    cells: Vec<Vec<u32>>,
}

impl Grid {
    /// A grid over `lo..hi` with about `count` cells. An axis with no size
    /// gets one cell.
    fn new(lo: DVec3, hi: DVec3, count: usize) -> Grid {
        let size = (hi - lo).max(DVec3::ZERO);
        let tiny = 1e-9 * size.max_element().max(1e-9);
        let used: Vec<f64> = size.to_array().into_iter().filter(|s| *s > tiny).collect();
        let count = count.clamp(1, 1 << 18) as f64;
        let h = if used.is_empty() {
            1.0
        } else {
            (used.iter().product::<f64>() / count).powf(1.0 / used.len() as f64)
        };
        let n = size.to_array().map(|s| {
            if s > tiny {
                (s / h).ceil().clamp(1.0, 256.0) as usize
            } else {
                1
            }
        });
        let cell = DVec3::new(
            size.x.max(tiny) / n[0] as f64,
            size.y.max(tiny) / n[1] as f64,
            size.z.max(tiny) / n[2] as f64,
        );
        Grid {
            lo,
            cell,
            n,
            cells: vec![Vec::new(); n[0] * n[1] * n[2]],
        }
    }

    /// The cell index range on each axis for the box `lo..hi`, clamped to
    /// the grid. None when the box is outside.
    fn span(&self, lo: DVec3, hi: DVec3) -> Option<[(usize, usize); 3]> {
        let mut out = [(0, 0); 3];
        for (k, o) in out.iter_mut().enumerate() {
            let a = ((lo[k] - self.lo[k]) / self.cell[k]).floor();
            let b = ((hi[k] - self.lo[k]) / self.cell[k]).floor();
            let top = (self.n[k] - 1) as f64;
            if b < 0.0 || a > top {
                return None;
            }
            *o = (a.max(0.0) as usize, b.min(top) as usize);
        }
        Some(out)
    }

    fn index(&self, x: usize, y: usize, z: usize) -> usize {
        (z * self.n[1] + y) * self.n[0] + x
    }

    fn insert(&mut self, lo: DVec3, hi: DVec3, id: u32) {
        if let Some(s) = self.span(lo, hi) {
            for z in s[2].0..=s[2].1 {
                for y in s[1].0..=s[1].1 {
                    for x in s[0].0..=s[0].1 {
                        let i = self.index(x, y, z);
                        self.cells[i].push(id);
                    }
                }
            }
        }
    }

    /// The cells that touch the box `lo..hi`.
    fn cells_in(&self, lo: DVec3, hi: DVec3) -> Vec<usize> {
        let mut out = Vec::new();
        if let Some(s) = self.span(lo, hi) {
            for z in s[2].0..=s[2].1 {
                for y in s[1].0..=s[1].1 {
                    for x in s[0].0..=s[0].1 {
                        out.push(self.index(x, y, z));
                    }
                }
            }
        }
        out
    }

    fn cell_of(&self, p: DVec3) -> Option<usize> {
        let s = self.span(p, p)?;
        Some(self.index(s[0].0, s[1].0, s[2].0))
    }
}

/// True when an edge of `a` goes through a triangle of `b`. Only the parts
/// in the box `lo..hi` (the overlap of the two bounding boxes) count.
fn crosses(a: &Solid, b: &Solid, lo: DVec3, hi: DVec3) -> bool {
    let (lo, hi) = (lo - TOUCH, hi + TOUCH);
    let hits = |(l, h): (DVec3, DVec3)| l.cmple(hi).all() && h.cmpge(lo).all();
    let ids: Vec<usize> = (0..b.tris.len()).filter(|&i| hits(b.tri_box(i))).collect();
    if ids.is_empty() {
        return false;
    }
    let mut grid = Grid::new(lo, hi, ids.len());
    for &i in &ids {
        let (l, h) = b.tri_box(i);
        grid.insert(l, h, i as u32);
    }
    let mut mark = vec![0u32; b.tris.len()];
    let mut stamp = 0u32;
    for t in &a.tris {
        for k in 0..3 {
            let (i, j) = (t[k], t[(k + 1) % 3]);
            // In a closed mesh each edge is in two triangles: test it once.
            if i > j {
                continue;
            }
            let (p, q) = (a.pts[i], a.pts[j]);
            let (el, eh) = (p.min(q), p.max(q));
            if !hits((el, eh)) {
                continue;
            }
            stamp += 1;
            for cell in grid.cells_in(el, eh) {
                for &id in &grid.cells[cell] {
                    let id = id as usize;
                    if mark[id] == stamp {
                        continue;
                    }
                    mark[id] = stamp;
                    if edge_crosses(p, q, b.tri(id)) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// True when the segment `p`-`q` goes through the triangle: the ends are
/// on different sides of its plane, and the point where it crosses the
/// plane is in the triangle or on its border.
fn edge_crosses(p: DVec3, q: DVec3, [a, b, c]: [DVec3; 3]) -> bool {
    let n = (b - a).cross(c - a);
    let dp = n.dot(p - a);
    let dq = n.dot(q - a);
    if dp * dq >= 0.0 {
        return false;
    }
    let x = p + (q - p) * (dp / (dp - dq));
    (b - a).cross(x - a).dot(n) >= 0.0
        && (c - b).cross(x - b).dot(n) >= 0.0
        && (a - c).cross(x - c).dot(n) >= 0.0
}

/// True when a point just inside `a` (the center of a triangle, moved by
/// `DELTA` against its normal) is inside `b`. Only points in the box
/// `lo..hi` count.
fn points_inside(a: &Solid, b: &Solid, lo: DVec3, hi: DVec3) -> bool {
    let mut rays: Option<Vec<Rays>> = None;
    for i in 0..a.tris.len() {
        let [p, q, r] = a.tri(i);
        let n = (q - p).cross(r - p);
        let len = n.length();
        if len <= 1e-12 {
            continue;
        }
        let c = (p + q + r) / 3.0 - n / len * DELTA;
        if !(c.cmpge(lo).all() && c.cmple(hi).all()) {
            continue;
        }
        let rays = rays.get_or_insert_with(|| b.rays());
        if b.contains(rays, c) {
            return true;
        }
    }
    false
}

/// The smallest and the largest corner of `pts`.
fn bounds(pts: &[DVec3]) -> Option<(DVec3, DVec3)> {
    let first = *pts.first()?;
    Some(
        pts.iter()
            .fold((first, first), |(lo, hi), p| (lo.min(*p), hi.max(*p))),
    )
}

/// The place of one part on the plate.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed {
    /// The center of the footprint. The bed goes from (0, 0) to
    /// (area.w, area.d). When the parts do not fit the bed, some centers
    /// are outside it.
    pub center: Vec2,
    /// The part is turned by 90 degrees about z.
    pub turned: bool,
}

/// Puts footprints of the sizes `sizes` (x, y in print frame) on the one
/// plate of `area.w` x `area.d`, with `gap` between the parts. Returns one
/// place for each size, in the order of `sizes`. All parts get a place:
/// the slicer arranges the parts again on its own plates.
///
/// The method is skyline packing: the parts go on the bed one by one, the
/// tallest (in y) first. Each part goes to the place where its top edge is
/// lowest, and then most to the left. A part turns so that its short side
/// is along y, when it fits the bed so. It also turns when the turned part
/// gets a lower place. A part that does not fit the bed does not turn. The
/// block of parts is centered on the bed. There is no gap at the edge of
/// the bed. When the block is deeper than the bed, the block gets wider,
/// until it has about the shape of the bed. The block is then larger than
/// the bed.
pub fn layout(sizes: &[Vec2], area: &Area, gap: f32) -> Vec<Placed> {
    const EPS: f32 = 1e-3;
    let gap = gap.max(0.0);
    let bed = Vec2::new(area.w.max(1.0), area.d.max(1.0));
    let on_bed = |s: Vec2| s.x <= bed.x + EPS && s.y <= bed.y + EPS;

    struct Item {
        index: usize,
        /// The sizes that the part can have, with the turn. The first one
        /// is the preferred one.
        options: Vec<(Vec2, bool)>,
    }
    let mut items = Vec::new();
    for (index, s) in sizes.iter().enumerate() {
        let s = s.max(Vec2::ZERO);
        let t = Vec2::new(s.y, s.x);
        let order = if s.y > s.x {
            [(t, true), (s, false)]
        } else {
            [(s, false), (t, true)]
        };
        let mut options: Vec<(Vec2, bool)> = order.into_iter().filter(|o| on_bed(o.0)).collect();
        if options.is_empty() {
            options.push((s, false));
        }
        items.push(Item { index, options });
    }
    items.sort_by(|a, b| {
        let (sa, sb) = (a.options[0].0, b.options[0].0);
        (sb.y.total_cmp(&sa.y))
            .then(sb.x.total_cmp(&sa.x))
            .then(a.index.cmp(&b.index))
    });

    // Lower left corner, size and turn of each part in a block of `width`,
    // and the size of the block. Each part takes its size plus `gap` to the
    // right and to the top, in a block of `width + gap`. So the gap is
    // between the parts and not at the edges.
    let pack = |width: f32| -> (Vec<(Vec2, Vec2, bool)>, Vec2) {
        let bin = width + gap;
        // The skyline: segments (x, width, top) from left to right.
        let mut sky: Vec<(f32, f32, f32)> = vec![(0.0, bin, 0.0)];
        let mut spots = vec![(Vec2::ZERO, Vec2::ZERO, false); sizes.len()];
        let mut extent = Vec2::ZERO;
        for it in &items {
            // Top, x, y, size, turn.
            let mut best: Option<(f32, f32, f32, Vec2, bool)> = None;
            for &(size, turned) in &it.options {
                let (w, h) = (size.x + gap, size.y + gap);
                for (i, seg) in sky.iter().enumerate() {
                    let x = seg.0;
                    if x + w > bin + EPS {
                        break;
                    }
                    let y = (sky[i..].iter())
                        .take_while(|s| s.0 < x + w - EPS)
                        .fold(0.0f32, |y, s| y.max(s.2));
                    let top = y + h;
                    let better = best.is_none_or(|(bt, bx, ..)| {
                        top < bt - EPS || (top <= bt + EPS && x < bx - EPS)
                    });
                    if better {
                        best = Some((top, x, y, size, turned));
                    }
                }
            }
            // `width` is at least the width of the first option, so the
            // left end always has room. The fallback is for safety only.
            let (top, x, y, size, turned) = best.unwrap_or_else(|| {
                let (size, turned) = it.options[0];
                let y = sky.iter().fold(0.0f32, |y, s| y.max(s.2));
                (y + size.y + gap, 0.0, y, size, turned)
            });
            let w = size.x + gap;
            let mut next: Vec<(f32, f32, f32)> = Vec::with_capacity(sky.len() + 2);
            for &(sx, sw, sy) in &sky {
                let end = sx + sw;
                if end <= x + EPS || sx >= x + w - EPS {
                    next.push((sx, sw, sy));
                    continue;
                }
                if sx < x - EPS {
                    next.push((sx, x - sx, sy));
                }
                if end > x + w + EPS {
                    next.push((x + w, end - x - w, sy));
                }
            }
            next.push((x, w, top));
            next.sort_by(|a, b| a.0.total_cmp(&b.0));
            sky.clear();
            for seg in next {
                match sky.last_mut() {
                    Some(last) if (last.2 - seg.2).abs() < EPS => last.1 += seg.1,
                    _ => sky.push(seg),
                }
            }
            let at = Vec2::new(x, y);
            spots[it.index] = (at, size, turned);
            extent = extent.max(at + size);
        }
        (spots, extent)
    };

    let widest = (items.iter())
        .map(|it| it.options[0].0.x)
        .fold(bed.x, f32::max);
    let (mut spots, mut extent) = pack(widest);
    if extent.y > bed.y + EPS {
        // A wider block, until it fits a bed that is `scale` times larger.
        let mut scale = widest / bed.x;
        for _ in 0..200 {
            scale *= 1.1;
            let (s, e) = pack(bed.x * scale);
            (spots, extent) = (s, e);
            if extent.y <= bed.y * scale + EPS {
                break;
            }
        }
    }

    // Center the block on the bed.
    let offset = (bed - extent) * 0.5;
    spots
        .into_iter()
        .map(|(at, size, turned)| Placed {
            center: at + size * 0.5 + offset,
            turned,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::orient::shapes::{bracket, cuboid, join, prism};
    use super::super::transformed;
    use super::*;
    use glam::{Mat4, Quat, Vec3};

    fn tilt() -> Mat4 {
        Mat4::from_translation(Vec3::new(12.0, -30.0, 7.5))
            * Mat4::from_quat(Quat::from_euler(glam::EulerRot::XYZ, 1.1, 0.7, 0.3))
    }

    fn mirror() -> Mat4 {
        Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0))
    }

    /// Three arms of different lengths from one corner: a part that is not
    /// the same as its mirror image. The boxes touch and do not overlap.
    fn tripod() -> Mesh {
        join(&[
            cuboid(Vec3::ZERO, Vec3::new(30.0, 4.0, 4.0)),
            cuboid(Vec3::new(0.0, 4.0, 0.0), Vec3::new(4.0, 24.0, 4.0)),
            cuboid(Vec3::new(0.0, 0.0, 4.0), Vec3::new(4.0, 4.0, 14.0)),
        ])
    }

    fn sig(m: &Mesh) -> Signature {
        signature(m)
    }

    #[test]
    fn signature_of_a_box() {
        let s = sig(&cuboid(Vec3::ZERO, Vec3::new(10.0, 20.0, 30.0)));
        assert!((s.volume - 6000.0).abs() < 1e-6);
        assert!((s.area - 2200.0).abs() < 1e-6);
        assert!((s.extents[0] - 30.0).abs() < 1e-6, "{s:?}");
        assert!((s.extents[2] - 10.0).abs() < 1e-6, "{s:?}");
        // I = m (b² + c²) / 12 about the long axis.
        assert!((s.moments[0] - 6000.0 * (100.0 + 400.0) / 12.0).abs() < 1e-3);
        assert!(s.hand.iter().all(|v| v.abs() < 1e-9), "{s:?}");
    }

    #[test]
    fn same_part_in_other_poses_matches() {
        let a = tripod();
        let s = sig(&a);
        assert!(s.hand.iter().any(|v| v.abs() > 1e-2), "{s:?}");
        for m in [
            tilt(),
            Mat4::from_rotation_z(std::f32::consts::FRAC_PI_2),
            Mat4::from_translation(Vec3::splat(100.0)),
        ] {
            let t = sig(&transformed(&a, m));
            assert!(same(&s, &t), "{s:?}\n{t:?}");
        }
    }

    #[test]
    fn mirror_image_does_not_match() {
        let a = tripod();
        let m = transformed(&a, mirror());
        assert!(!same(&sig(&a), &sig(&m)));
        assert!(!same(
            &sig(&transformed(&a, tilt())),
            &sig(&transformed(&m, tilt()))
        ));
        // The mirror image of a part with a mirror plane is the same part.
        let b = bracket();
        assert!(same(&sig(&b), &sig(&transformed(&b, tilt() * mirror()))));
    }

    #[test]
    fn symmetric_parts_match_in_any_pose() {
        let turn = Mat4::from_quat(Quat::from_euler(glam::EulerRot::XYZ, 0.4, -0.9, 0.25));
        let parts = [
            crate::mesh::cube(10.0),
            prism(32, 10.0, 40.0),
            cuboid(Vec3::ZERO, Vec3::new(10.0, 10.0, 50.0)),
        ];
        for p in &parts {
            let s = sig(p);
            for m in [turn, tilt(), Mat4::from_rotation_z(0.3)] {
                let t = sig(&transformed(p, m));
                assert!(same(&s, &t), "{s:?}\n{t:?}");
            }
        }
        // The square prism keeps its diameter, not a size on a random axis.
        let s = sig(&parts[2]);
        assert!((s.extents[1] - 200f64.sqrt()).abs() < 1e-4, "{s:?}");
    }

    #[test]
    fn other_parts_do_not_match() {
        let a = sig(&cuboid(Vec3::ZERO, Vec3::new(10.0, 20.0, 30.0)));
        let b = sig(&cuboid(Vec3::ZERO, Vec3::new(10.0, 20.0, 31.0)));
        let c = sig(&cuboid(Vec3::ZERO, Vec3::new(10.0, 20.1, 30.0)));
        assert!(!same(&a, &b));
        assert!(!same(&a, &c));
        assert!(!same(&a, &sig(&tripod())));
    }

    #[test]
    fn group_same_keeps_the_first_seen_order() {
        let boxy = cuboid(Vec3::ZERO, Vec3::new(10.0, 20.0, 30.0));
        let t = tripod();
        let sigs = [
            sig(&boxy),
            sig(&t),
            sig(&transformed(&boxy, tilt())),
            sig(&transformed(&t, mirror())),
            sig(&transformed(&t, tilt())),
        ];
        assert_eq!(group_same(&sigs), vec![vec![0, 2], vec![1, 4], vec![3]]);
    }

    #[test]
    fn overlap_of_boxes() {
        let a = cuboid(Vec3::ZERO, Vec3::splat(10.0));
        let at = |lo: Vec3, hi: Vec3| cuboid(lo, hi);
        // Share volume, with faces in the same planes.
        assert!(overlap(
            &a,
            &at(Vec3::new(5.0, 0.0, 0.0), Vec3::new(15.0, 10.0, 10.0))
        ));
        // Share volume, in general position.
        assert!(overlap(&a, &at(Vec3::splat(7.0), Vec3::splat(17.0))));
        // Share one face, or a part of a face.
        assert!(!overlap(
            &a,
            &at(Vec3::new(10.0, 0.0, 0.0), Vec3::new(20.0, 10.0, 10.0))
        ));
        assert!(!overlap(
            &a,
            &at(Vec3::new(10.0, 5.0, 0.0), Vec3::new(20.0, 15.0, 10.0))
        ));
        // Apart.
        assert!(!overlap(&a, &at(Vec3::splat(11.0), Vec3::splat(20.0))));
        // One inside the other.
        assert!(overlap(&a, &at(Vec3::splat(2.0), Vec3::splat(4.0))));
        assert!(overlap(&at(Vec3::splat(2.0), Vec3::splat(4.0)), &a));
        assert!(overlap(&a, &a.clone()));
        // Two bars that cross, with the same height.
        let bar_x = at(Vec3::new(0.0, 13.0, 0.0), Vec3::new(30.0, 17.0, 4.0));
        let bar_y = at(Vec3::new(13.0, 0.0, 0.0), Vec3::new(17.0, 30.0, 4.0));
        assert!(overlap(&bar_x, &bar_y));
    }

    #[test]
    fn parts_that_touch_in_a_corner_do_not_overlap() {
        // A box in the inner corner of the bracket touches two faces. The
        // bounding boxes overlap.
        let b = bracket();
        let inner = cuboid(Vec3::new(4.0, 4.0, 0.0), Vec3::new(20.0, 20.0, 40.0));
        assert!(!overlap(&b, &inner));
        assert!(!overlap(&inner, &b));
        let deeper = cuboid(Vec3::new(3.9, 4.0, 0.0), Vec3::new(20.0, 20.0, 40.0));
        assert!(overlap(&b, &deeper));
        // Turned in all axes, the result stays.
        let t = tilt();
        assert!(!overlap(&transformed(&b, t), &transformed(&inner, t)));
        assert!(overlap(&transformed(&b, t), &transformed(&deeper, t)));
    }

    #[test]
    fn overlap_groups_join_in_first_seen_order() {
        let c = |lo: f32, hi: f32| cuboid(Vec3::splat(lo), Vec3::splat(hi));
        let parts = [
            c(0.0, 10.0),  // 0
            c(50.0, 60.0), // 1
            c(5.0, 15.0),  // 2 overlaps 0
            c(10.0, 20.0), // 3 overlaps 2, touches 0 at a corner
            c(60.0, 70.0), // 4 touches 1 at a corner
        ];
        let refs: Vec<&Mesh> = parts.iter().collect();
        assert_eq!(overlap_groups(&refs), vec![vec![0, 2, 3], vec![1], vec![4]]);
    }

    /// The footprint rectangle (min, max) of a placed part.
    fn rect(p: &Placed, size: Vec2) -> (Vec2, Vec2) {
        let s = if p.turned {
            Vec2::new(size.y, size.x)
        } else {
            size
        };
        (p.center - s * 0.5, p.center + s * 0.5)
    }

    /// The layout of `sizes`, with a check that no two parts are nearer
    /// than `gap`.
    fn check_layout(sizes: &[Vec2], area: &Area, gap: f32) -> Vec<Placed> {
        let out = layout(sizes, area, gap);
        assert_eq!(out.len(), sizes.len());
        for (i, p) in out.iter().enumerate() {
            let (lo, hi) = rect(p, sizes[i]);
            for (j, q) in out.iter().enumerate().skip(i + 1) {
                let (l2, h2) = rect(q, sizes[j]);
                let apart = l2.x - hi.x >= gap - 1e-3
                    || lo.x - h2.x >= gap - 1e-3
                    || l2.y - hi.y >= gap - 1e-3
                    || lo.y - h2.y >= gap - 1e-3;
                assert!(apart, "{i} and {j}: {p:?} {q:?}");
            }
        }
        out
    }

    /// The smallest and the largest corner of all footprints.
    fn block(out: &[Placed], sizes: &[Vec2]) -> (Vec2, Vec2) {
        let rects: Vec<_> = out.iter().zip(sizes).map(|(p, s)| rect(p, *s)).collect();
        let lo = rects.iter().fold(Vec2::MAX, |a, r| a.min(r.0));
        let hi = rects.iter().fold(Vec2::MIN, |a, r| a.max(r.1));
        (lo, hi)
    }

    #[test]
    fn layout_keeps_parts_on_the_bed_with_the_gap() {
        let area = Area::default();
        let sizes = [
            Vec2::new(60.0, 40.0),
            Vec2::new(20.0, 80.0),
            Vec2::new(30.0, 30.0),
            Vec2::new(100.0, 15.0),
            Vec2::new(10.0, 10.0),
            Vec2::new(45.0, 25.0),
        ];
        let out = check_layout(&sizes, &area, 10.0);
        // The block is on the bed and centered: the margins are equal.
        let (lo, hi) = block(&out, &sizes);
        assert!(lo.x >= -1e-3 && lo.y >= -1e-3, "{lo}");
        assert!(hi.x <= area.w + 1e-3 && hi.y <= area.d + 1e-3, "{hi}");
        assert!((lo.x - (area.w - hi.x)).abs() < 1e-3, "{lo} {hi}");
        assert!((lo.y - (area.d - hi.y)).abs() < 1e-3, "{lo} {hi}");
        // A part goes with its short side along y.
        assert!(out[1].turned);
    }

    #[test]
    fn layout_puts_all_parts_on_one_plate() {
        let area = Area {
            w: 100.0,
            d: 100.0,
            h: 100.0,
        };
        // Four fit the bed. Nine need a block larger than the bed, with
        // about the shape of the bed, centered on it.
        let sizes = vec![Vec2::splat(45.0); 9];
        let out = check_layout(&sizes, &area, 10.0);
        let (lo, hi) = block(&out, &sizes);
        let size = hi - lo;
        assert!(size.x > area.w && size.y > area.d, "{size}");
        assert!((size.x / size.y - 1.0).abs() < 0.3, "{size}");
        assert!(((lo + hi) * 0.5 - Vec2::splat(50.0)).length() < 1e-3, "{lo} {hi}");

        // Parts that are too large for the bed are on the plate too. They do
        // not turn.
        let sizes = [
            Vec2::new(120.0, 10.0),
            Vec2::new(20.0, 20.0),
            Vec2::new(10.0, 101.0),
        ];
        let out = check_layout(&sizes, &area, 5.0);
        assert!(!out[0].turned && !out[2].turned, "{out:?}");
        let (lo, hi) = block(&out, &sizes);
        assert!(hi.x - lo.x >= 120.0 - 1e-3, "{lo} {hi}");
    }

    #[test]
    fn layout_fills_the_room_beside_a_tall_stack() {
        // The split desk dock on a 220 x 220 bed: two pieces of the base
        // plate, the stand and three clips. Rows do not fit them; the
        // stand turned beside the two pieces does.
        let area = Area {
            w: 220.0,
            d: 220.0,
            h: 250.0,
        };
        let mut sizes = vec![
            Vec2::new(135.0, 80.0),
            Vec2::new(130.0, 80.0),
            Vec2::new(100.0, 60.0),
        ];
        sizes.extend([Vec2::new(17.0, 14.0); 3]);
        let out = check_layout(&sizes, &area, 10.0);
        let (lo, hi) = block(&out, &sizes);
        assert!(lo.x >= -1e-3 && lo.y >= -1e-3, "{lo}");
        assert!(hi.x <= area.w + 1e-3 && hi.y <= area.d + 1e-3, "{hi}");
    }

    #[test]
    fn layout_of_many_copies_is_fast() {
        let sizes = vec![Vec2::new(12.0, 7.0); 999];
        let t = std::time::Instant::now();
        let out = layout(&sizes, &Area::default(), 5.0);
        assert_eq!(out.len(), 999);
        assert!(t.elapsed().as_secs_f32() < 2.0, "{:?}", t.elapsed());
    }

    #[test]
    fn layout_turns_a_part_that_fits_only_turned() {
        let area = Area {
            w: 100.0,
            d: 200.0,
            h: 100.0,
        };
        let sizes = [Vec2::new(150.0, 20.0)];
        let out = check_layout(&sizes, &area, 10.0);
        assert!(out[0].turned);
        assert!((out[0].center - Vec2::new(50.0, 100.0)).length() < 1e-3);
    }
}
