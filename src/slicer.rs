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
    let layers: Vec<Layer> = std::thread::scope(|scope| {
        let threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
            .min(16);
        let chunk = n.div_ceil(threads).max(1);
        let handles: Vec<_> = (0..n)
            .step_by(chunk)
            .map(|start| {
                let (cuts, buckets, tops) = (&cuts, &buckets, &tops);
                scope.spawn(move || {
                    (start..(start + chunk).min(n))
                        .map(|li| {
                            let loops = cut(mesh, &buckets[li], cuts[li]);
                            let solid =
                                li < s.solid_layers as usize || li + (s.solid_layers as usize) >= n;
                            Layer {
                                z: b.min.z + tops[li],
                                paths: toolpaths(&loops, li, solid, s),
                            }
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap())
            .collect()
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
    Sliced { layers, stats }
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

fn toolpaths(loops: &[Vec<Vec2>], layer: usize, solid: bool, s: &Settings) -> Vec<Path> {
    let lw = s.line_width;
    let mut paths = Vec::new();
    let mut inner: Vec<Vec<Vec2>> = loops.to_vec();
    for w in 0..s.walls {
        let d = lw * (w as f32 + 0.5);
        let mut next = Vec::new();
        for lp in loops {
            if let Some(o) = offset(lp, d) {
                paths.push(Path {
                    kind: if w == 0 {
                        Kind::OuterWall
                    } else {
                        Kind::InnerWall
                    },
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

    let density = if solid { 1.0 } else { s.infill };
    if density > 0.0 && !inner.is_empty() {
        let spacing = lw / density.clamp(0.01, 1.0);
        let angle = if layer % 2 == 0 { 45f32 } else { -45f32 }.to_radians();
        let kind = if solid { Kind::Solid } else { Kind::Infill };
        for seg in hatch(&inner, angle, spacing) {
            paths.push(Path {
                kind,
                pts: seg.to_vec(),
                closed: false,
            });
        }
    }
    paths
}

/// Parallel lines at `angle`, clipped to the region (even-odd rule).
fn hatch(loops: &[Vec<Vec2>], angle: f32, spacing: f32) -> Vec<[Vec2; 2]> {
    let rot = Vec2::from_angle(-angle);
    let back = Vec2::from_angle(angle);
    let r: Vec<Vec<Vec2>> = loops
        .iter()
        .map(|l| l.iter().map(|p| rot.rotate(*p)).collect())
        .collect();
    let (mut ymin, mut ymax) = (f32::MAX, f32::MIN);
    for p in r.iter().flatten() {
        ymin = ymin.min(p.y);
        ymax = ymax.max(p.y);
    }
    let mut out = Vec::new();
    // Lines sit on a fixed grid so they line up from layer to layer.
    let mut k = (ymin / spacing).ceil();
    let mut xs = Vec::new();
    while k * spacing <= ymax {
        let y = k * spacing;
        xs.clear();
        for lp in &r {
            let n = lp.len();
            for i in 0..n {
                let (a, b) = (lp[i], lp[(i + 1) % n]);
                if (a.y <= y) != (b.y <= y) {
                    xs.push(a.x + (y - a.y) / (b.y - a.y) * (b.x - a.x));
                }
            }
        }
        xs.sort_by(f32::total_cmp);
        for pair in xs.chunks_exact(2) {
            if pair[1] - pair[0] > 1e-3 {
                out.push([
                    back.rotate(Vec2::new(pair[0], y)),
                    back.rotate(Vec2::new(pair[1], y)),
                ]);
            }
        }
        k += 1.0;
    }
    out
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

    #[test]
    fn hole_gets_walls_and_no_infill() {
        let outer = vec![
            Vec2::ZERO,
            Vec2::new(20., 0.),
            Vec2::new(20., 20.),
            Vec2::new(0., 20.),
        ];
        let hole = vec![
            Vec2::new(5., 5.),
            Vec2::new(5., 15.),
            Vec2::new(15., 15.),
            Vec2::new(15., 5.),
        ];
        let paths = toolpaths(&[outer, hole], 0, true, &Settings::default());
        let outer_walls = paths.iter().filter(|p| p.kind == Kind::OuterWall).count();
        assert_eq!(outer_walls, 2);
        for p in paths.iter().filter(|p| p.kind == Kind::Solid) {
            let mid = (p.pts[0] + p.pts[1]) * 0.5;
            assert!(
                !(mid.x > 5.5 && mid.x < 14.5 && mid.y > 5.5 && mid.y < 14.5),
                "infill in hole at {mid}"
            );
        }
    }

    #[test]
    fn hatch_fills_square() {
        let sq = vec![
            Vec2::ZERO,
            Vec2::new(10., 0.),
            Vec2::new(10., 10.),
            Vec2::new(0., 10.),
        ];
        let lines = hatch(&[sq], 0.0, 1.0);
        assert_eq!(lines.len(), 10);
    }
}
