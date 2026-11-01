//! A small FDM slicer for previews. It cuts the mesh into layers and makes
//! walls and line infill. The results are approximate: no travel moves,
//! no supports, no bridges, simple top/bottom detection.

use crate::mesh::Mesh;
use glam::{Vec2, Vec3};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub layer_height: f32,
    pub first_layer: f32,
    pub line_width: f32,
    pub walls: u32,
    /// Sparse infill density, 0..1.
    pub infill: f32,
    /// Solid layers at the bottom and at the top.
    pub solid_layers: u32,
    /// Print speed in mm/s.
    pub speed: f32,
    pub filament_diameter: f32,
    /// g/cm^3 (PLA 1.24).
    pub density: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            layer_height: 0.2,
            first_layer: 0.2,
            line_width: 0.45,
            walls: 2,
            infill: 0.15,
            solid_layers: 4,
            speed: 80.0,
            filament_diameter: 1.75,
            density: 1.24,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    OuterWall,
    InnerWall,
    Infill,
    Solid,
}

#[derive(Clone, Debug)]
pub struct Path {
    pub kind: Kind,
    pub pts: Vec<Vec2>,
    pub closed: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Layer {
    /// Top of the layer, in model coordinates.
    pub z: f32,
    /// Layer thickness.
    pub height: f32,
    pub paths: Vec<Path>,
}

#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub layers: usize,
    /// Extruded path length in mm.
    pub path_mm: f64,
    pub filament_m: f64,
    pub grams: f64,
    pub seconds: f64,
}

#[derive(Clone, Debug, Default)]
pub struct Sliced {
    pub layers: Vec<Layer>,
    pub stats: Stats,
    pub line_width: f32,
}

pub fn slice(mesh: &Mesh, s: &Settings) -> Sliced {
    let Some(b) = mesh.bounds() else {
        return Sliced::default();
    };
    let height = b.max.z - b.min.z;
    if height <= 0.0 || s.layer_height <= 0.0 {
        return Sliced::default();
    }

    // Cut heights (middle of each layer) relative to the model bottom.
    let mut tops = vec![s.first_layer.min(height)];
    while *tops.last().unwrap() + s.layer_height * 0.5 < height {
        tops.push(tops.last().unwrap() + s.layer_height);
    }
    let cuts: Vec<f32> = tops
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let h = if i == 0 {
                s.first_layer
            } else {
                s.layer_height
            };
            b.min.z + (t - h * 0.5).min(height - 1e-4)
        })
        .collect();

    // Put each triangle in the buckets of the layers it crosses.
    let mut buckets: Vec<Vec<u32>> = vec![Vec::new(); cuts.len()];
    for (ti, t) in mesh.triangles.iter().enumerate() {
        let zs = t.map(|v| mesh.vertices[v as usize].z);
        let (lo, hi) = (zs[0].min(zs[1]).min(zs[2]), zs[0].max(zs[1]).max(zs[2]));
        let first = cuts.partition_point(|&c| c < lo);
        for (li, &c) in cuts.iter().enumerate().skip(first) {
            if c >= hi {
                break;
            }
            buckets[li].push(ti as u32);
        }
    }

    let n = cuts.len();
    // Stage 1: outlines and walls of every layer.
    let geo: Vec<LayerGeo> = par_map(n, |li| {
        let loops = cut(mesh, &buckets[li], cuts[li]);
        let (walls, inner) = walls(&loops, s);
        (loops, walls, inner)
    });
    // Stage 2: infill. Skin goes where the layers above or below have no material.
    let layers: Vec<Layer> = par_map(n, |li| {
        let mut paths = geo[li].1.clone();
        paths.extend(infill(&geo, li, s));
        Layer {
            z: b.min.z + tops[li],
            height: if li == 0 {
                tops[0]
            } else {
                tops[li] - tops[li - 1]
            },
            paths,
        }
    });

    let mut stats = Stats {
        layers: layers.len(),
        ..Default::default()
    };
    let mut volume = 0.0f64;
    for (i, l) in layers.iter().enumerate() {
        let h = if i == 0 {
            s.first_layer
        } else {
            s.layer_height
        } as f64;
        for p in &l.paths {
            let len = path_len(p) as f64;
            stats.path_mm += len;
            volume += len * s.line_width as f64 * h;
        }
    }
    let r = s.filament_diameter as f64 / 2.0;
    stats.filament_m = volume / (std::f64::consts::PI * r * r) / 1000.0;
    stats.grams = volume / 1000.0 * s.density as f64;
    // Rough: print moves plus 20% for travel and acceleration, 2 s per layer change.
    stats.seconds = stats.path_mm / s.speed.max(1.0) as f64 * 1.2 + layers.len() as f64 * 2.0;
    Sliced {
        layers,
        stats,
        line_width: s.line_width,
    }
}

fn path_len(p: &Path) -> f32 {
    let mut l: f32 = p.pts.windows(2).map(|w| w[0].distance(w[1])).sum();
    if p.closed && p.pts.len() > 2 {
        l += p.pts[0].distance(*p.pts.last().unwrap());
    }
    l
}

/// Cuts the triangles at height `z` and joins the segments into closed loops.
/// Outer loops run counter-clockwise, holes clockwise.
fn cut(mesh: &Mesh, tris: &[u32], z: f32) -> Vec<Vec<Vec2>> {
    type Edge = (u32, u32);
    let point = |e: Edge| -> Vec2 {
        // Same edge -> same float result, whatever triangle asks.
        let (a, b) = (mesh.vertices[e.0 as usize], mesh.vertices[e.1 as usize]);
        let t = (z - a.z) / (b.z - a.z);
        Vec2::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
    };
    let above = |v: u32| mesh.vertices[v as usize].z >= z;

    // start edge -> (end edge, start point)
    let mut segs: HashMap<Edge, Edge> = HashMap::with_capacity(tris.len());
    let mut pts: HashMap<Edge, Vec2> = HashMap::with_capacity(tris.len());
    for &ti in tris {
        let t = mesh.triangles[ti as usize];
        let mut hits: [Edge; 2] = [(0, 0); 2];
        let mut k = 0;
        for e in 0..3 {
            let (a, b) = (t[e], t[(e + 1) % 3]);
            if above(a) != above(b) && k < 2 {
                hits[k] = (a.min(b), a.max(b));
                k += 1;
            }
        }
        if k != 2 {
            continue;
        }
        let (p0, p1) = (point(hits[0]), point(hits[1]));
        let n: Vec3 = mesh.normal(ti as usize);
        let tangent = Vec2::new(-n.y, n.x); // up x normal
        let (from, to) = if (p1 - p0).dot(tangent) >= 0.0 {
            (hits[0], hits[1])
        } else {
            (hits[1], hits[0])
        };
        segs.insert(from, to);
        pts.insert(from, point(from));
        pts.insert(to, point(to));
    }

    let mut loops = Vec::new();
    let mut used = std::collections::HashSet::new();
    let starts: Vec<Edge> = segs.keys().copied().collect();
    for start in starts {
        if used.contains(&start) {
            continue;
        }
        let mut lp = Vec::new();
        let mut cur = start;
        let mut closed = false;
        while used.insert(cur) {
            lp.push(pts[&cur]);
            match segs.get(&cur) {
                Some(&nx) => {
                    if nx == start {
                        closed = true;
                        break;
                    }
                    cur = nx;
                }
                None => break,
            }
        }
        if closed {
            let lp = simplify(lp, 0.02);
            if lp.len() >= 3 && area(&lp).abs() > 1e-3 {
                loops.push(lp);
            }
        }
    }
    loops
}

/// Removes points closer than `tol` to the previous kept point, and
/// points on a straight line.
fn simplify(pts: Vec<Vec2>, tol: f32) -> Vec<Vec2> {
    let mut out: Vec<Vec2> = Vec::with_capacity(pts.len());
    for p in pts {
        if out.last().is_none_or(|q: &Vec2| q.distance(p) > tol) {
            out.push(p);
        }
    }
    while out.len() > 3 && out[0].distance(*out.last().unwrap()) <= tol {
        out.pop();
    }
    let n = out.len();
    if n < 4 {
        return out;
    }
    (0..n)
        .filter(|&i| {
            let (a, b, c) = (out[(i + n - 1) % n], out[i], out[(i + 1) % n]);
            let (u, v) = (b - a, c - b);
            u.perp_dot(v).abs() > 1e-4 * u.length() * v.length() || u.dot(v) < 0.0
        })
        .map(|i| out[i])
        .collect()
}

fn area(p: &[Vec2]) -> f32 {
    let n = p.len();
    (0..n).map(|i| p[i].perp_dot(p[(i + 1) % n])).sum::<f32>() * 0.5
}

/// Moves a loop `d` to its left (into the material). Returns None when the
/// loop disappears or flips.
fn offset(p: &[Vec2], d: f32) -> Option<Vec<Vec2>> {
    let n = p.len();
    let left = |a: Vec2, b: Vec2| (b - a).normalize_or_zero().perp();
    let out: Vec<Vec2> = (0..n)
        .map(|i| {
            let (a, b, c) = (p[(i + n - 1) % n], p[i], p[(i + 1) % n]);
            let (n1, n2) = (left(a, b), left(b, c));
            let m = (n1 + n2).normalize_or_zero();
            let cos = m.dot(n1).max(0.3);
            b + m * (d / cos)
        })
        .collect();
    let (a0, a1) = (area(p), area(&out));
    // Inward offset shrinks outer loops (a > 0) and grows holes (a < 0).
    let shrank = a1.abs() < a0.abs();
    if a0.signum() != a1.signum() || shrank != (a0 > 0.0) || a1.abs() < d * d {
        return None;
    }
    // Reject loops that turned inside out at a narrow part.
    let back = out
        .iter()
        .zip(p)
        .filter(|(o, q)| (**o - **q).length() > d * 3.5)
        .count();
    if back * 4 > n {
        return None;
    }
    Some(out)
}

/// One layer before infill: outline loops, wall paths, region inside the walls.
type LayerGeo = (Vec<Vec<Vec2>>, Vec<Path>, Vec<Vec<Vec2>>);

/// Runs `f` for 0..n on all cores and keeps the order.
fn par_map<T: Send>(n: usize, f: impl Fn(usize) -> T + Sync) -> Vec<T> {
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(16);
    let chunk = n.div_ceil(threads).max(1);
    std::thread::scope(|scope| {
        let f = &f;
        let handles: Vec<_> = (0..n)
            .step_by(chunk)
            .map(|start| {
                scope.spawn(move || (start..(start + chunk).min(n)).map(f).collect::<Vec<_>>())
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap())
            .collect()
    })
}

/// Wall loops, and the region inside the walls that gets infill.
fn walls(loops: &[Vec<Vec2>], s: &Settings) -> (Vec<Path>, Vec<Vec<Vec2>>) {
    let lw = s.line_width;
    let mut paths = Vec::new();
    let mut inner: Vec<Vec<Vec2>> = loops.to_vec();
    for w in 0..s.walls {
        let d = lw * (w as f32 + 0.5);
        let mut next = Vec::new();
        for lp in loops {
            if let Some(o) = offset(lp, d) {
                let kind = if w == 0 {
                    Kind::OuterWall
                } else {
                    Kind::InnerWall
                };
                paths.push(Path {
                    kind,
                    pts: o,
                    closed: true,
                });
            }
            if let Some(o) = offset(lp, d + lw * 0.5) {
                next.push(o);
            }
        }
        inner = next;
    }
    (paths, inner)
}

/// Loops turned by an angle, with the y range of each loop.
struct Shape {
    loops: Vec<(Vec<Vec2>, f32, f32)>,
}

impl Shape {
    fn new(loops: &[Vec<Vec2>], rot: Vec2) -> Shape {
        Shape {
            loops: loops
                .iter()
                .map(|l| {
                    let r: Vec<Vec2> = l.iter().map(|p| rot.rotate(*p)).collect();
                    let (lo, hi) = r.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| {
                        (lo.min(p.y), hi.max(p.y))
                    });
                    (r, lo, hi)
                })
                .collect(),
        }
    }

    fn y_range(&self) -> (f32, f32) {
        self.loops.iter().fold((f32::MAX, f32::MIN), |(lo, hi), l| {
            (lo.min(l.1), hi.max(l.2))
        })
    }

    /// Inside intervals on the line at `y` (even-odd rule), sorted.
    fn scan(&self, y: f32) -> Vec<(f32, f32)> {
        let mut xs = Vec::new();
        for (lp, lo, hi) in &self.loops {
            if y < *lo || y > *hi {
                continue;
            }
            let n = lp.len();
            for i in 0..n {
                let (a, b) = (lp[i], lp[(i + 1) % n]);
                if (a.y <= y) != (b.y <= y) {
                    xs.push(a.x + (y - a.y) / (b.y - a.y) * (b.x - a.x));
                }
            }
        }
        xs.sort_by(f32::total_cmp);
        xs.as_chunks::<2>()
            .0
            .iter()
            .map(|p| (p[0], p[1]))
            .filter(|(a, b)| b > a)
            .collect()
    }
}

fn intersect(a: &[(f32, f32)], b: &[(f32, f32)]) -> Vec<(f32, f32)> {
    let (mut i, mut j, mut out) = (0, 0, Vec::new());
    while i < a.len() && j < b.len() {
        let (lo, hi) = (a[i].0.max(b[j].0), a[i].1.min(b[j].1));
        if hi > lo {
            out.push((lo, hi));
        }
        if a[i].1 < b[j].1 { i += 1 } else { j += 1 }
    }
    out
}

fn subtract(a: &[(f32, f32)], b: &[(f32, f32)]) -> Vec<(f32, f32)> {
    let mut out = Vec::new();
    for &(mut lo, hi) in a {
        for &(blo, bhi) in b {
            if bhi <= lo || blo >= hi {
                continue;
            }
            if blo > lo {
                out.push((lo, blo));
            }
            lo = lo.max(bhi);
        }
        if hi > lo {
            out.push((lo, hi));
        }
    }
    out
}

/// Sparse infill and top/bottom skin for layer `li`. A spot is skin when one
/// of the `solid_layers` layers above or below has no material there.
fn infill(geo: &[LayerGeo], li: usize, s: &Settings) -> Vec<Path> {
    let inner = &geo[li].2;
    if inner.is_empty() {
        return Vec::new();
    }
    let lw = s.line_width;
    let angle = if li.is_multiple_of(2) { 45f32 } else { -45f32 }.to_radians();
    let (rot, back) = (Vec2::from_angle(-angle), Vec2::from_angle(angle));
    let cur = Shape::new(inner, rot);

    // The neighbour layers; a missing one (past the model) means no material.
    let k = s.solid_layers as usize;
    let mut open = false;
    let mut near = Vec::new();
    for d in 1..=k {
        for j in [li.checked_sub(d), Some(li + d).filter(|&j| j < geo.len())] {
            match j {
                Some(j) => near.push(Shape::new(&geo[j].0, rot)),
                None => open = true,
            }
        }
    }
    // Intervals at `y` that have material in all neighbour layers.
    let core = |y: f32, cur: &[(f32, f32)]| -> Vec<(f32, f32)> {
        if open {
            return Vec::new();
        }
        near.iter()
            .fold(cur.to_vec(), |acc, sh| intersect(&acc, &sh.scan(y)))
    };

    let mut paths = Vec::new();
    let mut emit = |kind: Kind, y: f32, (a, b): (f32, f32)| {
        if b - a > lw * 0.3 {
            paths.push(Path {
                kind,
                pts: vec![back.rotate(Vec2::new(a, y)), back.rotate(Vec2::new(b, y))],
                closed: false,
            });
        }
    };
    let (ymin, ymax) = cur.y_range();
    // Lines sit on fixed grids so they line up from layer to layer.
    let mut y = (ymin / lw).ceil() * lw;
    while y <= ymax {
        let c = cur.scan(y);
        for iv in subtract(&c, &core(y, &c)) {
            emit(Kind::Solid, y, iv);
        }
        y += lw;
    }
    if s.infill > 0.0 {
        let spacing = lw / s.infill.clamp(0.01, 1.0);
        let mut y = (ymin / spacing).ceil() * spacing;
        while y <= ymax {
            let c = cur.scan(y);
            for iv in core(y, &c) {
                emit(Kind::Infill, y, iv);
            }
            y += spacing;
        }
    }
    paths
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::cube;

    #[test]
    fn slices_a_cube() {
        let r = slice(&cube(10.0), &Settings::default());
        assert_eq!(r.stats.layers, 50);
        for l in &r.layers {
            let outer: Vec<_> = l
                .paths
                .iter()
                .filter(|p| p.kind == Kind::OuterWall)
                .collect();
            assert_eq!(outer.len(), 1, "layer at z={}", l.z);
            assert!((area(&outer[0].pts) - 9.55f32.powi(2)).abs() < 0.2);
            assert!(
                l.paths
                    .iter()
                    .any(|p| matches!(p.kind, Kind::Infill | Kind::Solid))
            );
        }
        assert!(r.stats.grams > 0.5 && r.stats.grams < 1.24);
    }

    fn square(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<Vec2> {
        vec![
            Vec2::new(x0, y0),
            Vec2::new(x1, y0),
            Vec2::new(x1, y1),
            Vec2::new(x0, y1),
        ]
    }

    #[test]
    fn hole_gets_walls_and_no_infill() {
        let outer = square(0., 0., 20., 20.);
        let mut hole = square(5., 5., 15., 15.);
        hole.reverse();
        let s = Settings::default();
        let (walls, inner) = walls(&[outer.clone(), hole.clone()], &s);
        assert_eq!(
            walls.iter().filter(|p| p.kind == Kind::OuterWall).count(),
            2
        );
        let geo = vec![(vec![outer, hole], walls, inner)];
        for p in infill(&geo, 0, &s) {
            let mid = (p.pts[0] + p.pts[1]) * 0.5;
            assert!(
                !(mid.x > 5.5 && mid.x < 14.5 && mid.y > 5.5 && mid.y < 14.5),
                "infill in hole at {mid}"
            );
        }
    }

    #[test]
    fn intervals() {
        let a = [(0.0, 10.0), (20.0, 30.0)];
        let b = [(5.0, 25.0)];
        assert_eq!(intersect(&a, &b), [(5.0, 10.0), (20.0, 25.0)]);
        assert_eq!(subtract(&a, &b), [(0.0, 5.0), (25.0, 30.0)]);
    }

    /// A 20 mm block with a 10 mm deep pocket: the pocket floor must get top skin.
    #[test]
    fn pocket_floor_gets_top_skin() {
        let s = Settings::default();
        let outer = square(0., 0., 20., 20.);
        let mut pocket = square(5., 5., 15., 15.);
        pocket.reverse();
        // 20 layers: 0..9 solid block, 10..19 with the pocket.
        let layer = |with_pocket: bool| {
            let loops = if with_pocket {
                vec![outer.clone(), pocket.clone()]
            } else {
                vec![outer.clone()]
            };
            let (w, inner) = walls(&loops, &s);
            (loops, w, inner)
        };
        let geo: Vec<_> = (0..20).map(|i| layer(i >= 10)).collect();
        let in_pocket = |p: &Path| {
            let m = (p.pts[0] + p.pts[1]) * 0.5;
            m.x > 6.0 && m.x < 14.0 && m.y > 6.0 && m.y < 14.0
        };
        // Layer 9 is the pocket floor: solid under the pocket.
        let floor = infill(&geo, 9, &s);
        assert!(floor.iter().any(|p| p.kind == Kind::Solid && in_pocket(p)));
        assert!(!floor.iter().any(|p| p.kind == Kind::Infill && in_pocket(p)));
        // Layer 4 is deep inside: sparse only.
        let mid = infill(&geo, 4, &s);
        assert!(mid.iter().all(|p| p.kind == Kind::Infill));
        // Layer 15 is beside the pocket, far from the top: sparse only.
        let wall = infill(&geo, 14, &s);
        assert!(wall.iter().all(|p| p.kind == Kind::Infill));
    }
}
