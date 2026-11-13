//! The parts of the model in the 3D view. The display mesh holds the
//! triangles of each part in one range. The renderer then draws each part
//! with its own color, offset and transparency.
//!
//! An assembly comes from a list of source meshes (`Source`): the rendered
//! mesh, or the meshes of a background job that renders each part on its own
//! (a multicolor part list, see `parts.rs`). A source is one part, or each of
//! its shells is one part ("Part 1", "Part 2" and so on).
//!
//! Shells: two triangles are in one shell when they share an edge. A shell
//! with a negative volume is a cavity. It goes to the smallest solid shell
//! that holds it. A shell with almost no volume (a sliver that a union can
//! leave) goes to the nearest part. The shells of a source are in volume
//! order, the largest first.
//!
//! Reference parts are the `%` subtrees of the model (see `csg_split.rs`).
//! They show in the 3D view, but they are not printed: the export, the
//! slice and the photo leave them out. They come after all other parts, so
//! the triangles of the printed parts are the first part of the mesh.

use crate::mesh::{Bounds, Mesh};
use crate::parts::Part;
use crate::pathtrace::{self, Paint};
use glam::{Vec3, Vec4};
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::Arc;

/// Part colors for parts without a color of their own. Mid-bright colors,
/// so that they show on the dark and on the light view background.
pub const PALETTE: [[u8; 3]; 12] = [
    [0x4e, 0x8f, 0xd6], // blue
    [0xf2, 0x8e, 0x2b], // orange
    [0xe1, 0x57, 0x59], // red
    [0x3f, 0xb0, 0xa8], // teal
    [0x59, 0xa1, 0x4f], // green
    [0xed, 0xc9, 0x48], // yellow
    [0xb0, 0x7a, 0xa1], // mauve
    [0xff, 0x9d, 0xa7], // pink
    [0x9c, 0x75, 0x5f], // brown
    [0x7b, 0x6f, 0xd6], // violet
    [0xa5, 0xd6, 0x5a], // lime
    [0x5f, 0xc3, 0xe4], // sky
];

/// One part of the display mesh.
#[derive(Clone, Debug)]
pub struct PartInfo {
    pub name: String,
    /// The index of the `Source` that the part comes from.
    pub source: usize,
    /// The triangles of the part in `Assembly::mesh`.
    pub tris: Range<usize>,
    /// The wireframe edges of the part in `Assembly::edges`.
    pub edges: Range<usize>,
    /// The color of a named part. `None` for a shell.
    pub color: Option<[u8; 3]>,
    pub bounds: Bounds,
    /// The center of the bounds.
    pub center: Vec3,
    /// Volume in mm^3. The cavities of the part are not in it.
    pub volume: f32,
    /// A `%` part of the model: shown, but never printed or exported.
    pub reference: bool,
}

/// The model as parts, for the 3D view and the photo.
pub struct Assembly {
    /// All triangles, with the triangles of each part in one range, in the
    /// order of `parts`. For one shell, this is the rendered mesh.
    pub mesh: Arc<Mesh>,
    pub parts: Vec<PartInfo>,
    /// Unique edges of `mesh`, grouped by part.
    pub edges: Vec<[u32; 2]>,
    /// Where the parts come from. `Kind::Modules` also for the shells of
    /// one mesh.
    pub kind: Kind,
}

/// How a source divides into parts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shells {
    /// The source is one part with its name.
    Whole,
    /// Each shell of the mesh is one part, with the name "Part N". The
    /// number counts on over all sources.
    Numbered,
    /// Each shell of the mesh is one part with the name of the source:
    /// "Name" for one shell, else "Name 1", "Name 2" and so on. The number
    /// counts on over all sources with the same name.
    Named,
}

/// One mesh that goes into an assembly.
pub struct Source<'a> {
    /// The part name. Not used for `Shells::Numbered`.
    pub name: String,
    /// The color of a named part, for example from the model definition.
    pub color: Option<[u8; 3]>,
    pub mesh: &'a Mesh,
    pub shells: Shells,
    /// The part is a reference part (`PartInfo::reference`).
    pub reference: bool,
}

/// Where the parts of a background parts job come from. See
/// `Assembly::from_parts`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    /// A multicolor part list: each part is one part with its name and color.
    Colors,
    /// The `part` selector with all parts shown: each shell of each part is
    /// one part, named after its part (`Shells::Named`).
    Labeled,
    /// The module split: each shell of each part is one part, "Part N".
    #[default]
    Modules,
    /// The `part` selector with one value shown: the shells of the main
    /// mesh, named with this label. When the parts (the pieces of the
    /// module split of the value) give more solids, they replace the main
    /// mesh, for example for segments that touch.
    Shown(String),
}

/// A part before the measurements.
struct Group {
    name: String,
    color: Option<[u8; 3]>,
    source: usize,
    tris: Range<usize>,
    reference: bool,
}

impl Assembly {
    /// The parts of all sources, in source order. Empty sources give no part.
    pub fn build(sources: &[Source]) -> Assembly {
        let solids: Vec<Vec<Vec<u32>>> = (sources.iter())
            .map(|s| {
                if s.shells == Shells::Whole || s.mesh.is_empty() {
                    Vec::new()
                } else {
                    solids(s.mesh, shell_groups(s.mesh))
                }
            })
            .collect();
        // The number of solids for each name of `Shells::Named`.
        let mut totals: HashMap<&str, usize> = HashMap::new();
        for (s, list) in sources.iter().zip(&solids) {
            if s.shells == Shells::Named {
                *totals.entry(s.name.as_str()).or_default() += list.len();
            }
        }
        let mut seen: HashMap<&str, usize> = HashMap::new();
        let mut mesh = Mesh::default();
        let mut groups: Vec<Group> = Vec::new();
        let mut number = 0;
        for (k, (s, list)) in sources.iter().zip(solids).enumerate() {
            if s.mesh.is_empty() {
                continue;
            }
            let base = mesh.vertices.len() as u32;
            mesh.vertices.extend_from_slice(&s.mesh.vertices);
            let moved = |t: &[u32; 3]| t.map(|v| v + base);
            if s.shells == Shells::Whole {
                let start = mesh.triangles.len();
                mesh.triangles.extend(s.mesh.triangles.iter().map(moved));
                groups.push(Group {
                    name: s.name.clone(),
                    color: s.color,
                    source: k,
                    tris: start..mesh.triangles.len(),
                    reference: s.reference,
                });
                continue;
            }
            for tris in list {
                let start = mesh.triangles.len();
                let src = &s.mesh.triangles;
                mesh.triangles
                    .extend(tris.iter().map(|&t| moved(&src[t as usize])));
                let name = if s.shells == Shells::Named {
                    let n = seen.entry(s.name.as_str()).or_default();
                    *n += 1;
                    if totals[s.name.as_str()] == 1 {
                        s.name.clone()
                    } else {
                        format!("{} {n}", s.name)
                    }
                } else {
                    number += 1;
                    format!("Part {number}")
                };
                groups.push(Group {
                    name,
                    color: s.color,
                    source: k,
                    tris: start..mesh.triangles.len(),
                    reference: s.reference,
                });
            }
        }
        Assembly::measure(Arc::new(mesh), groups)
    }

    /// The number of solids of `mesh`, as `build` divides it.
    pub fn solid_count(mesh: &Mesh) -> usize {
        solids(mesh, shell_groups(mesh)).len()
    }

    /// The shells of the rendered mesh, each one a part "Part N", or named
    /// after `label` (see `Shells::Named`). When the shells keep the
    /// triangle order, the display mesh is `mesh` itself.
    pub fn shells(mesh: &Arc<Mesh>, label: Option<&str>) -> Assembly {
        let mut a = Assembly::build(&[Source {
            name: label.unwrap_or_default().to_string(),
            color: None,
            mesh,
            shells: if label.is_some() {
                Shells::Named
            } else {
                Shells::Numbered
            },
            reference: false,
        }]);
        if a.mesh.triangles == mesh.triangles && a.mesh.vertices == mesh.vertices {
            a.mesh = mesh.clone();
        }
        a
    }

    /// The parts of a background job that renders each part on its own.
    /// `main` is the rendered model with all parts.
    ///
    /// - `Kind::Colors` (a multicolor part list): each part is one part with
    ///   its name and color.
    /// - `Kind::Labeled` (the `part` selector): each shell of each part is
    ///   one part, named after its part. It has the color of its part from
    ///   the definition (`Split::colors`), else no color.
    /// - `Kind::Modules` (the module split, `csg_split.rs`): each shell of
    ///   each part is one part "Part N" without a color. With no parts (no
    ///   module split), the shells of `main` are the parts.
    /// - `Kind::Shown`: the shells of `main`, named with the label. When
    ///   `parts` are two or more and give more solids, the shells of
    ///   `parts` replace them.
    ///
    /// With `Colors` and `Labeled`, the triangles of `main` that are in no
    /// part, for example hardware previews, become the parts "Other 1",
    /// "Other 2" and so on (one shell each), or "Other" for one shell. A seam
    /// where two parts touch is not a part (see `leftover`). The
    /// module split gives no "Other" part: its parts hold all of the model,
    /// but they render from the CSG tree, which has rounded numbers, so
    /// their corners do not agree with the corners of `main`.
    ///
    /// `references` are the `%` parts. They come last, each one a part with
    /// its name.
    #[cfg(test)]
    pub fn from_parts(main: &Mesh, parts: &[Part], kind: &Kind, references: &[Part]) -> Assembly {
        Assembly::assemble(main, parts, parts, &[], kind, references)
    }

    /// The parts of a `csg_split::Split`, as `from_parts`. For the "Other"
    /// test, it uses `Split::cover` when it has meshes.
    pub fn from_split(main: &Mesh, split: &crate::csg_split::Split) -> Assembly {
        let cover = if split.cover.is_empty() {
            &split.parts
        } else {
            &split.cover
        };
        Assembly::assemble(
            main,
            &split.parts,
            cover,
            &split.colors,
            &split.kind,
            &split.references,
        )
    }

    /// `from_parts` with `cover`, the parts that the "Other" test uses, and
    /// `colors`, the colors of the `Labeled` parts.
    fn assemble(
        main: &Mesh,
        parts: &[Part],
        cover: &[Part],
        colors: &[Option<[u8; 3]>],
        kind: &Kind,
        references: &[Part],
    ) -> Assembly {
        let other = match kind {
            Kind::Colors | Kind::Labeled => leftover(main, cover),
            Kind::Modules | Kind::Shown(_) => Mesh::default(),
        };
        let mut sources: Vec<Source> = match kind {
            Kind::Shown(label) if divides(parts, main) => (parts.iter())
                .map(|p| Source {
                    name: label.clone(),
                    color: None,
                    mesh: &p.mesh,
                    shells: Shells::Named,
                    reference: false,
                })
                .collect(),
            Kind::Shown(label) => vec![Source {
                name: label.clone(),
                color: None,
                mesh: main,
                shells: Shells::Named,
                reference: false,
            }],
            Kind::Modules if parts.is_empty() => vec![Source {
                name: String::new(),
                color: None,
                mesh: main,
                shells: Shells::Numbered,
                reference: false,
            }],
            _ => (parts.iter().enumerate())
                .map(|(k, p)| Source {
                    name: p.name.clone(),
                    color: match kind {
                        Kind::Colors => Some(p.color),
                        Kind::Labeled => colors.get(k).copied().flatten(),
                        _ => None,
                    },
                    mesh: &p.mesh,
                    shells: match kind {
                        Kind::Colors => Shells::Whole,
                        Kind::Labeled => Shells::Named,
                        _ => Shells::Numbered,
                    },
                    reference: false,
                })
                .collect(),
        };
        let rest = sources.len();
        sources.push(Source {
            name: String::new(),
            color: None,
            mesh: &other,
            shells: Shells::Numbered,
            reference: false,
        });
        sources.extend(references.iter().map(|r| Source {
            name: r.name.clone(),
            color: None,
            mesh: &r.mesh,
            shells: Shells::Whole,
            reference: true,
        }));
        let mut a = Assembly::build(&sources);
        a.kind = kind.clone();
        let count = a.parts.iter().filter(|p| p.source == rest).count();
        for (i, p) in a.parts.iter_mut().filter(|p| p.source == rest).enumerate() {
            p.name = if count == 1 {
                "Other".to_string()
            } else {
                format!("Other {}", i + 1)
            };
        }
        a
    }

    /// Bounds, volume and wireframe edges of each group.
    fn measure(mesh: Arc<Mesh>, groups: Vec<Group>) -> Assembly {
        let mut edges = Vec::new();
        let parts = groups
            .into_iter()
            .filter(|g| !g.tris.is_empty())
            .map(|g| {
                let Group {
                    name,
                    color,
                    source,
                    tris,
                    reference,
                } = g;
                let start = edges.len();
                let mut keys: Vec<u64> = Vec::with_capacity(tris.len() * 3);
                let mut volume = 0.0f64;
                let first = mesh.vertices[mesh.triangles[tris.start][0] as usize];
                let mut bounds = Bounds {
                    min: first,
                    max: first,
                };
                for i in tris.clone() {
                    let t = mesh.triangles[i];
                    for k in 0..3 {
                        keys.push(edge_key(t[k], t[(k + 1) % 3]));
                    }
                    let [a, b, c] = mesh.tri(i);
                    for v in [a, b, c] {
                        bounds.min = bounds.min.min(v);
                        bounds.max = bounds.max.max(v);
                    }
                    volume += a.dot(b.cross(c)) as f64;
                }
                keys.sort_unstable();
                keys.dedup();
                edges.extend(keys.iter().map(|&k| [(k >> 32) as u32, k as u32]));
                PartInfo {
                    name,
                    source,
                    tris,
                    edges: start..edges.len(),
                    color,
                    bounds,
                    center: bounds.center(),
                    volume: (volume / 6.0).abs() as f32,
                    reference,
                }
            })
            .collect();
        Assembly {
            mesh,
            parts,
            edges,
            kind: Kind::Modules,
        }
    }

    /// Gives all parts this color, except the reference parts. For a model
    /// that shows one named part.
    pub fn paint_all(&mut self, color: [u8; 3]) {
        for p in self.parts.iter_mut().filter(|p| !p.reference) {
            p.color = Some(color);
        }
    }

    /// The number of reference parts.
    pub fn references(&self) -> usize {
        self.parts.iter().filter(|p| p.reference).count()
    }

    /// The part that a ray from `origin` in the direction `dir` hits first,
    /// and the distance to the hit in units of `dir`.
    ///
    /// - `offsets` moves each part, as in the view. A missing offset is zero.
    /// - `planes` are section planes in model space (before the offset). A
    ///   hit at the model point p counts only when
    ///   `plane.truncate().dot(p) + plane.w >= 0.0` for each plane.
    /// - `alpha` is the transparency of each part, 1 when it is missing. A
    ///   hit on a part that is not transparent (1) wins over a nearer hit on
    ///   a transparent part. A part with 0 is hidden: the ray goes through.
    ///
    /// Front and back faces count. Through a section cut, the ray hits a
    /// back face of the part that the cut shows. The search tests each
    /// triangle of each part that the ray can hit (a box test first), so
    /// use it for a click only.
    pub fn pick(
        &self,
        origin: Vec3,
        dir: Vec3,
        offsets: &[Vec3],
        planes: &[Vec4],
        alpha: &[f32],
    ) -> Option<(usize, f32)> {
        self.pick_triangle(origin, dir, offsets, planes, alpha)
            .map(|(i, _, t)| (i, t))
    }

    /// As `pick`, and also the triangle of the hit in `mesh`: (part,
    /// triangle, distance).
    pub fn pick_triangle(
        &self,
        origin: Vec3,
        dir: Vec3,
        offsets: &[Vec3],
        planes: &[Vec4],
        alpha: &[f32],
    ) -> Option<(usize, usize, f32)> {
        let inv = dir.recip();
        let kept = |p: Vec3| planes.iter().all(|c| c.truncate().dot(p) + c.w >= 0.0);
        // The nearest hit on a part that is not transparent, and on a
        // transparent part.
        let mut best: [Option<(usize, usize, f32)>; 2] = [None, None];
        for (i, part) in self.parts.iter().enumerate() {
            let o = origin - offsets.get(i).copied().unwrap_or(Vec3::ZERO);
            let a = alpha.get(i).copied().unwrap_or(1.0);
            if a <= 0.0 {
                continue;
            }
            let slot = &mut best[usize::from(a < 1.0)];
            let mut limit = slot.map_or(f32::INFINITY, |b| b.2);
            match ray_box(o, inv, &part.bounds) {
                Some(t) if t <= limit => {}
                _ => continue,
            }
            for k in part.tris.clone() {
                if let Some(t) = ray_triangle(o, dir, self.mesh.tri(k))
                    && t < limit
                    && kept(o + dir * t)
                {
                    limit = t;
                    *slot = Some((i, k, t));
                }
            }
        }
        best[0].or(best[1])
    }

    /// The part that holds triangle `tri` of the display mesh.
    #[allow(dead_code)]
    pub fn part_of(&self, tri: usize) -> Option<usize> {
        let i = self.parts.partition_point(|p| p.tris.end <= tri);
        (i < self.parts.len()).then_some(i)
    }

    /// The bounds of all parts, each one moved by its offset. A missing
    /// offset is zero.
    pub fn bounds(&self, offsets: &[Vec3]) -> Option<Bounds> {
        let mut parts = self.parts.iter().enumerate().map(|(i, p)| {
            let d = offsets.get(i).copied().unwrap_or(Vec3::ZERO);
            Bounds {
                min: p.bounds.min + d,
                max: p.bounds.max + d,
            }
        });
        let first = parts.next()?;
        Some(parts.fold(first, |a, b| Bounds {
            min: a.min.min(b.min),
            max: a.max.max(b.max),
        }))
    }

    /// The display mesh with each part moved by its offset, for the photo.
    /// The reference parts are not in it. The triangle order stays, so
    /// `paint` applies to it. Without a movement and without reference
    /// parts, this is the display mesh itself, and the path tracer keeps its
    /// geometry cache.
    pub fn moved(&self, offsets: &[Vec3]) -> Arc<Mesh> {
        if offsets.iter().all(|d| *d == Vec3::ZERO) && self.references() == 0 {
            return self.mesh.clone();
        }
        let src = &self.mesh;
        let mut out = Mesh {
            vertices: Vec::with_capacity(src.vertices.len()),
            triangles: Vec::with_capacity(src.triangles.len()),
        };
        // Parts can share a vertex (two shells that touch at a corner), so
        // each part gets its own copy of its vertices.
        let mut owner = vec![u32::MAX; src.vertices.len()];
        let mut index = vec![0u32; src.vertices.len()];
        for (i, p) in self.parts.iter().enumerate().filter(|(_, p)| !p.reference) {
            let d = offsets.get(i).copied().unwrap_or(Vec3::ZERO);
            for t in &src.triangles[p.tris.clone()] {
                out.triangles.push(t.map(|v| {
                    let v = v as usize;
                    if owner[v] != i as u32 {
                        owner[v] = i as u32;
                        index[v] = out.vertices.len() as u32;
                        out.vertices.push(src.vertices[v] + d);
                    }
                    index[v]
                }));
            }
        }
        Arc::new(out)
    }

    /// The albedo of each part for the path tracer. `colors` holds one
    /// sRGB color for each part.
    pub fn paint(&self, colors: &[[u8; 3]]) -> Paint {
        Paint::new(
            self.parts
                .iter()
                .zip(colors)
                .map(|(p, c)| (p.tris.end as u32, pathtrace::linear(*c))),
        )
    }
}

/// The distance along a ray to the triangle `[a, b, c]`, in units of `dir`,
/// or `None` when the ray does not hit it. Front and back faces count
/// (Moller-Trumbore test).
pub fn ray_triangle(origin: Vec3, dir: Vec3, [a, b, c]: [Vec3; 3]) -> Option<f32> {
    let e1 = b - a;
    let e2 = c - a;
    let p = dir.cross(e2);
    let det = e1.dot(p);
    if det == 0.0 {
        // The ray is parallel to the triangle, or the triangle has no area.
        return None;
    }
    let inv = 1.0 / det;
    let s = origin - a;
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = dir.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = e2.dot(q) * inv;
    (t >= 0.0 && t.is_finite()).then_some(t)
}

/// The distance along a ray to the point where it goes into the box, 0 when
/// it starts in the box, or `None` when it does not touch the box. `inv` is
/// 1 / the direction of the ray. The box gets a small margin, so a hit on a
/// flat face stays in the box.
fn ray_box(origin: Vec3, inv: Vec3, b: &Bounds) -> Option<f32> {
    let margin = b.size().max_element() * 1e-4 + 1e-5;
    let t1 = (b.min - margin - origin) * inv;
    let t2 = (b.max + margin - origin) * inv;
    let near = t1.min(t2).max_element().max(0.0);
    let far = t1.max(t2).min_element();
    (near <= far).then_some(near)
}

/// True when `pieces` are two or more and give more solids than `main`.
fn divides(pieces: &[Part], main: &Mesh) -> bool {
    pieces.len() >= 2
        && pieces
            .iter()
            .map(|p| Assembly::solid_count(&p.mesh))
            .sum::<usize>()
            > Assembly::solid_count(main)
}

/// The triangles of `main` that are in no part, with only the vertices that
/// they use.
fn leftover(main: &Mesh, parts: &[Part]) -> Mesh {
    // A triangle on a seam between parts has no part color, but all its
    // corners are part corners. The parts show it already.
    let known: HashSet<[u32; 3]> = (parts.iter())
        .flat_map(|p| p.mesh.vertices.iter().map(|&v| vertex_key(v)))
        .collect();
    let colors = crate::parts::face_colors(main, parts);
    let rest = (main.triangles.iter().zip(colors))
        .filter_map(|(t, c)| {
            let corner = |v: &u32| known.contains(&vertex_key(main.vertices[*v as usize]));
            (c.is_none() && !t.iter().all(corner)).then_some(*t)
        })
        .collect();
    // Where two parts touch, for example a stand on a plate, the build of
    // all parts can divide the faces there in a different way, with corners
    // that no part has. These triangles are in no part, but they lie on the
    // surface of a part: they are a seam. A part of the model that is in no
    // part list has its own surface.
    let out = compact(&main.vertices, off_surface(main, rest, parts));
    // A seam that the surface test did not do (see `SURFACE_WORK`) is a
    // flat patch when the parts touch on one face.
    let shells = shell_groups(&out);
    if !shells.iter().any(|s| flat(&out, s)) {
        return out;
    }
    let solid = (shells.into_iter())
        .filter(|s| !flat(&out, s))
        .flatten()
        .map(|t| out.triangles[t as usize]);
    compact(&out.vertices, solid)
}

/// The distance in mm from a part surface below which a leftover triangle
/// is a seam.
const ON_SURFACE: f32 = 1e-3;
/// More work than this (leftover triangles times part triangles near them)
/// skips the surface test.
const SURFACE_WORK: usize = 20_000_000;

/// The triangles `tris` of `main` whose center is not on the surface of a
/// part. The test is per triangle, against the part triangles in the
/// bounds of `tris`.
fn off_surface(main: &Mesh, tris: Vec<[u32; 3]>, parts: &[Part]) -> Vec<[u32; 3]> {
    let corners = |t: &[u32; 3]| t.map(|v| main.vertices[v as usize]);
    let (mut min, mut max) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for v in tris.iter().flat_map(corners) {
        min = min.min(v);
        max = max.max(v);
    }
    let (min, max) = (min - ON_SURFACE, max + ON_SURFACE);
    let near: Vec<[Vec3; 3]> = (parts.iter())
        .flat_map(|p| (0..p.mesh.triangles.len()).map(|i| p.mesh.tri(i)))
        .filter(|[a, b, c]| a.min(*b).min(*c).cmple(max).all() && min.cmple(a.max(*b).max(*c)).all())
        .collect();
    if near.is_empty() || tris.len().saturating_mul(near.len()) > SURFACE_WORK {
        return tris;
    }
    (tris.into_iter())
        .filter(|t| {
            let [a, b, c] = corners(t);
            let center = (a + b + c) / 3.0;
            !near.iter().any(|&n| on_triangle(center, n))
        })
        .collect()
}

/// True when `p` is on the triangle `[a, b, c]`, to `ON_SURFACE`.
fn on_triangle(p: Vec3, [a, b, c]: [Vec3; 3]) -> bool {
    let n = (b - a).cross(c - a);
    let n2 = n.length_squared();
    if n2 <= f32::MIN_POSITIVE {
        return false;
    }
    let d = (p - a).dot(n);
    if d * d > ON_SURFACE * ON_SURFACE * n2 {
        return false;
    }
    // `p` moved onto the plane of the triangle: inside when it is on the
    // inner side of each edge. Each value is a barycentric weight times n2.
    let q = p - n * (d / n2);
    let tolerance = -1e-5 * n2;
    (b - a).cross(q - a).dot(n) >= tolerance
        && (c - b).cross(q - b).dot(n) >= tolerance
        && (a - c).cross(q - c).dot(n) >= tolerance
}

/// A mesh of the triangles `tris`, with only the vertices of `vertices`
/// that they use.
fn compact(vertices: &[Vec3], tris: impl IntoIterator<Item = [u32; 3]>) -> Mesh {
    let mut out = Mesh::default();
    let mut index = vec![u32::MAX; vertices.len()];
    for t in tris {
        let t = t.map(|v| {
            let v = v as usize;
            if index[v] == u32::MAX {
                index[v] = out.vertices.len() as u32;
                out.vertices.push(vertices[v]);
            }
            index[v]
        });
        out.triangles.push(t);
    }
    out
}

/// True when all corners of the triangles `tris` of `mesh` are in one plane,
/// to 0.001 mm.
fn flat(mesh: &Mesh, tris: &[u32]) -> bool {
    let normal = (tris.iter())
        .map(|&t| {
            let [a, b, c] = mesh.tri(t as usize);
            (b - a).cross(c - a)
        })
        .max_by(|x, y| x.length_squared().total_cmp(&y.length_squared()));
    let Some(n) = normal.and_then(|n| n.try_normalize()) else {
        return true;
    };
    let p = mesh.tri(tris[0] as usize)[0];
    (tris.iter())
        .flat_map(|&t| mesh.tri(t as usize))
        .all(|v| (v - p).dot(n).abs() < 1e-3)
}

/// The same key for the same position, as in `Mesh::from_soup`.
fn vertex_key(v: Vec3) -> [u32; 3] {
    [
        (v.x + 0.0).to_bits(),
        (v.y + 0.0).to_bits(),
        (v.z + 0.0).to_bits(),
    ]
}

fn edge_key(a: u32, b: u32) -> u64 {
    (a.min(b) as u64) << 32 | a.max(b) as u64
}

/// The connected shells of `mesh`: lists of triangle indices, in ascending
/// order. The shells are in the order of their first triangle.
fn shell_groups(mesh: &Mesh) -> Vec<Vec<u32>> {
    let n = mesh.triangles.len();
    let mut keys: Vec<(u64, u32)> = Vec::with_capacity(n * 3);
    for (i, t) in mesh.triangles.iter().enumerate() {
        for k in 0..3 {
            keys.push((edge_key(t[k], t[(k + 1) % 3]), i as u32));
        }
    }
    keys.sort_unstable();
    let mut parent: Vec<u32> = (0..n as u32).collect();
    fn root(parent: &mut [u32], mut i: u32) -> u32 {
        while parent[i as usize] != i {
            let up = parent[parent[i as usize] as usize];
            parent[i as usize] = up;
            i = up;
        }
        i
    }
    for w in keys.windows(2) {
        if w[0].0 == w[1].0 {
            let (a, b) = (root(&mut parent, w[0].1), root(&mut parent, w[1].1));
            parent[a.max(b) as usize] = a.min(b);
        }
    }
    let mut group = vec![u32::MAX; n];
    let mut groups: Vec<Vec<u32>> = Vec::new();
    for i in 0..n as u32 {
        let r = root(&mut parent, i) as usize;
        if group[r] == u32::MAX {
            group[r] = groups.len() as u32;
            groups.push(Vec::new());
        }
        groups[group[r] as usize].push(i);
    }
    groups
}

/// Puts each cavity shell with its solid, and each sliver shell with the
/// nearest part. Then sorts the parts by volume, the largest first. Each
/// result holds the triangles of the solid first.
fn solids(mesh: &Mesh, shells: Vec<Vec<u32>>) -> Vec<Vec<u32>> {
    let measure = |tris: &[u32]| {
        let first = mesh.vertices[mesh.triangles[tris[0] as usize][0] as usize];
        let (mut min, mut max) = (first, first);
        let mut volume = 0.0f64;
        for &t in tris {
            let [a, b, c] = mesh.tri(t as usize);
            min = min.min(a).min(b).min(c);
            max = max.max(a).max(b).max(c);
            volume += a.dot(b.cross(c)) as f64;
        }
        (volume / 6.0, min, max)
    };
    let stats: Vec<(f64, Vec3, Vec3)> = shells.iter().map(|s| measure(s)).collect();
    // Slivers: less than 1 mm^3 and less than a millionth of the total.
    let total: f64 = stats.iter().map(|s| s.0.abs()).sum();
    let tiny = (total * 1e-6).min(1.0);
    let mut real: Vec<bool> = stats.iter().map(|s| s.0.abs() >= tiny).collect();
    if !real.contains(&true) {
        real.fill(true);
    }
    // Solid shells, the smallest first, so a cavity finds its nearest solid.
    let mut solid: Vec<usize> = (0..shells.len())
        .filter(|&i| real[i] && stats[i].0 >= 0.0)
        .collect();
    solid.sort_by(|&a, &b| stats[a].0.total_cmp(&stats[b].0));
    let mut owner: Vec<usize> = (0..shells.len()).collect();
    for (i, &(volume, min, max)) in stats.iter().enumerate() {
        if !real[i] || volume >= 0.0 {
            continue;
        }
        let holds = |&&j: &&usize| {
            let (_, smin, smax) = stats[j];
            smin.cmple(min).all() && max.cmple(smax).all()
        };
        if let Some(&j) = solid.iter().find(holds) {
            owner[i] = j;
        }
    }
    let mut heads: Vec<usize> = (0..shells.len())
        .filter(|&i| real[i] && owner[i] == i)
        .collect();
    for i in (0..shells.len()).filter(|&i| !real[i]) {
        let (_, min, max) = stats[i];
        let gap = |&j: &usize| {
            let (_, jmin, jmax) = stats[j];
            (jmin - max).max(min - jmax).max(Vec3::ZERO).length()
        };
        if let Some(&j) = heads.iter().min_by(|a, b| gap(a).total_cmp(&gap(b))) {
            owner[i] = j;
        }
    }
    let mut net = vec![0.0f64; shells.len()];
    for (i, &o) in owner.iter().enumerate() {
        net[o] += stats[i].0;
    }
    heads.sort_by(|&a, &b| net[b].abs().total_cmp(&net[a].abs()).then(a.cmp(&b)));
    let mut members: Vec<Vec<usize>> = vec![Vec::new(); shells.len()];
    for (i, &o) in owner.iter().enumerate() {
        if o != i {
            members[o].push(i);
        }
    }
    let mut shells: Vec<Option<Vec<u32>>> = shells.into_iter().map(Some).collect();
    heads
        .into_iter()
        .map(|h| {
            let mut tris = shells[h].take().unwrap_or_default();
            for &m in &members[h] {
                tris.extend(shells[m].take().unwrap_or_default());
            }
            tris
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::cube;

    /// Triangles of a cube with this size and corner. `inside` turns the
    /// faces inwards: a cavity.
    fn cube_soup(size: f32, at: Vec3, inside: bool) -> Vec<[Vec3; 3]> {
        let m = cube(size);
        (0..m.triangles.len())
            .map(|i| {
                let [a, b, c] = m.tri(i).map(|v| v + at);
                if inside { [a, c, b] } else { [a, b, c] }
            })
            .collect()
    }

    /// Each triangle of the display mesh is in exactly one part, and the
    /// edge ranges follow each other.
    fn check_ranges(a: &Assembly) {
        let mut next = 0;
        let mut next_edge = 0;
        for p in &a.parts {
            assert_eq!(p.tris.start, next, "{}", p.name);
            assert!(!p.tris.is_empty());
            assert_eq!(p.edges.start, next_edge);
            next = p.tris.end;
            next_edge = p.edges.end;
        }
        assert_eq!(next, a.mesh.triangles.len());
        assert_eq!(next_edge, a.edges.len());
    }

    #[test]
    fn two_cubes_are_two_parts() {
        let mut soup = cube_soup(5.0, Vec3::ZERO, false);
        soup.extend(cube_soup(10.0, Vec3::new(20.0, 0.0, 0.0), false));
        let mesh = Arc::new(Mesh::from_soup(&soup));
        let a = Assembly::shells(&mesh, None);
        check_ranges(&a);
        assert_eq!(a.parts.len(), 2);
        // The larger cube is first.
        assert_eq!(a.parts[0].name, "Part 1");
        assert!((a.parts[0].volume - 1000.0).abs() < 1e-2);
        assert!((a.parts[1].volume - 125.0).abs() < 1e-2);
        assert_eq!(a.parts[0].center, Vec3::new(25.0, 5.0, 5.0));
        // A cube has 18 unique edges (12 sides and 6 diagonals).
        assert_eq!(a.parts[0].edges.len(), 18);
        assert_eq!(a.parts[1].edges.len(), 18);
        assert_eq!(a.part_of(0), Some(0));
        assert_eq!(a.part_of(12), Some(1));
        assert_eq!(a.part_of(24), None);
        // The order changed, so the display mesh is a new one.
        assert!(!Arc::ptr_eq(&a.mesh, &mesh));
    }

    #[test]
    fn hollow_box_is_one_part() {
        let mut soup = cube_soup(10.0, Vec3::ZERO, false);
        soup.extend(cube_soup(4.0, Vec3::splat(3.0), true));
        // A separate small cube after the cavity.
        soup.extend(cube_soup(2.0, Vec3::new(30.0, 0.0, 0.0), false));
        let mesh = Arc::new(Mesh::from_soup(&soup));
        let a = Assembly::shells(&mesh, None);
        check_ranges(&a);
        assert_eq!(a.parts.len(), 2);
        assert_eq!(a.parts[0].tris, 0..24);
        assert!((a.parts[0].volume - (1000.0 - 64.0)).abs() < 1e-2);
        assert_eq!(a.parts[0].bounds.max, Vec3::splat(10.0));
        assert_eq!(a.parts[1].tris, 24..36);
    }

    #[test]
    fn one_shell_keeps_the_mesh() {
        let mesh = Arc::new(cube(10.0));
        let a = Assembly::shells(&mesh, None);
        check_ranges(&a);
        assert_eq!(a.parts.len(), 1);
        assert!(Arc::ptr_eq(&a.mesh, &mesh));
        assert!(Arc::ptr_eq(&a.moved(&[Vec3::ZERO]), &mesh));
        assert!(
            Assembly::shells(&Arc::new(Mesh::default()), None)
                .parts
                .is_empty()
        );
    }

    #[test]
    fn cavity_goes_to_the_smallest_solid_that_holds_it() {
        // A big box, a small box in it with a cavity in the small box.
        let mut soup = cube_soup(40.0, Vec3::ZERO, false);
        soup.extend(cube_soup(10.0, Vec3::splat(15.0), false));
        soup.extend(cube_soup(2.0, Vec3::splat(19.0), true));
        let a = Assembly::shells(&Arc::new(Mesh::from_soup(&soup)), None);
        check_ranges(&a);
        assert_eq!(a.parts.len(), 2);
        assert_eq!(a.parts[1].tris.len(), 24);
        assert!((a.parts[1].volume - 992.0).abs() < 1e-2);
    }

    fn part(name: &str, color: [u8; 3], soup: &[[Vec3; 3]]) -> Part {
        Part {
            name: name.into(),
            color,
            mesh: Mesh::from_soup(soup),
        }
    }

    #[test]
    fn named_parts_and_the_rest() {
        let a_soup = cube_soup(10.0, Vec3::ZERO, false);
        // Touches part a on a face: the main mesh welds them together.
        let b_soup = cube_soup(10.0, Vec3::new(10.0, 0.0, 0.0), false);
        let screw = cube_soup(2.0, Vec3::new(50.0, 0.0, 0.0), false);
        let nut = cube_soup(3.0, Vec3::new(60.0, 0.0, 0.0), false);
        let main: Vec<_> = [&a_soup, &b_soup, &screw, &nut]
            .into_iter()
            .flatten()
            .copied()
            .collect();
        let main = Mesh::from_soup(&main);
        let parts = [
            part("plate", [1, 2, 3], &a_soup),
            part("empty", [0, 0, 0], &[]),
            part("star", [4, 5, 6], &b_soup),
        ];
        let a = Assembly::from_parts(&main, &parts, &Kind::Colors, &[]);
        check_ranges(&a);
        let names: Vec<_> = a.parts.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["plate", "star", "Other 1", "Other 2"]);
        assert_eq!(a.parts[0].color, Some([1, 2, 3]));
        assert_eq!(a.parts[1].color, Some([4, 5, 6]));
        assert_eq!(a.parts[2].color, None);
        // The larger leftover shell is first.
        assert!((a.parts[2].volume - 27.0).abs() < 1e-3);
        assert_eq!(a.parts[1].tris.len(), 12);

        // One leftover shell is just "Other".
        let main: Vec<_> = a_soup.iter().chain(&screw).copied().collect();
        let a = Assembly::from_parts(&Mesh::from_soup(&main), &parts[..1], &Kind::Colors, &[]);
        check_ranges(&a);
        assert_eq!(a.parts[1].name, "Other");
    }

    #[test]
    fn labeled_parts_name_each_solid_after_its_part() {
        let spine = cube_soup(10.0, Vec3::ZERO, false);
        // One value with two solids: the larger one is first.
        let small = cube_soup(2.0, Vec3::new(30.0, 0.0, 0.0), false);
        let large = cube_soup(4.0, Vec3::new(40.0, 0.0, 0.0), false);
        let pins: Vec<_> = small.iter().chain(&large).copied().collect();
        // In the `all` build, but in no value.
        let rest = cube_soup(3.0, Vec3::new(60.0, 0.0, 0.0), false);
        let main: Vec<_> = [&spine, &pins, &rest]
            .into_iter()
            .flatten()
            .copied()
            .collect();
        let parts = [
            part("Spine", [0xff; 3], &spine),
            part("Pin", [0xff; 3], &pins),
        ];
        let a = Assembly::from_parts(&Mesh::from_soup(&main), &parts, &Kind::Labeled, &[]);
        check_ranges(&a);
        assert_eq!(a.kind, Kind::Labeled);
        let names: Vec<_> = a.parts.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Spine", "Pin 1", "Pin 2", "Other"]);
        assert!((a.parts[1].volume - 64.0).abs() < 1e-3);
        assert!(a.parts.iter().all(|p| p.color.is_none()));

        // With colors from the definition: each solid takes the color of its
        // part. A part without a color and the "Other" part get none.
        let split = crate::csg_split::Split {
            parts: vec![
                part("Spine", [0xff; 3], &spine),
                part("Pin", [0xff; 3], &pins),
            ],
            kind: Kind::Labeled,
            colors: vec![None, Some([1, 2, 3])],
            ..Default::default()
        };
        let a = Assembly::from_split(&Mesh::from_soup(&main), &split);
        let colors: Vec<_> = a.parts.iter().map(|p| p.color).collect();
        assert_eq!(colors, [None, Some([1, 2, 3]), Some([1, 2, 3]), None]);
    }

    #[test]
    fn a_flat_seam_between_parts_is_no_other_part() {
        let plate = cube_soup(10.0, Vec3::ZERO, false);
        let stand = cube_soup(10.0, Vec3::new(10.0, 0.0, 0.0), false);
        let screw = cube_soup(2.0, Vec3::new(50.0, 0.0, 0.0), false);
        // The build of all parts gives the face at z = 10 a new corner that
        // no part has. The triangle has one corner of each part.
        let seam = [[
            Vec3::new(0.0, 0.0, 10.0),
            Vec3::new(20.0, 10.0, 10.0),
            Vec3::new(7.5, 3.3, 10.0),
        ]];
        let main: Vec<_> = [&plate[..], &stand, &seam, &screw]
            .into_iter()
            .flatten()
            .copied()
            .collect();
        let parts = [
            part("Plate", [0xff; 3], &plate),
            part("Stand", [0xff; 3], &stand),
        ];
        let a = Assembly::from_parts(&Mesh::from_soup(&main), &parts, &Kind::Labeled, &[]);
        check_ranges(&a);
        let names: Vec<_> = a.parts.iter().map(|p| p.name.as_str()).collect();
        // The screw is a solid in no part: it stays.
        assert_eq!(names, ["Plate", "Stand", "Other"]);
        assert!((a.parts[2].volume - 8.0).abs() < 1e-3);
    }

    #[test]
    fn a_seam_around_an_edge_is_no_other_part() {
        let plate = cube_soup(10.0, Vec3::ZERO, false);
        let stand = cube_soup(10.0, Vec3::new(10.0, 0.0, 0.0), false);
        let screw = cube_soup(2.0, Vec3::new(50.0, 0.0, 0.0), false);
        // Two triangles with new corners, on the top and the front face of
        // the plate. They share an edge, so they are one shell, not flat.
        let seam = [
            [
                Vec3::new(0.0, 0.0, 10.0),
                Vec3::new(7.5, 0.0, 10.0),
                Vec3::new(5.0, 5.0, 10.0),
            ],
            [
                Vec3::new(0.0, 0.0, 10.0),
                Vec3::new(3.0, 0.0, 5.0),
                Vec3::new(7.5, 0.0, 10.0),
            ],
        ];
        let main: Vec<_> = [&plate[..], &stand, &seam, &screw]
            .into_iter()
            .flatten()
            .copied()
            .collect();
        let parts = [
            part("Plate", [0xff; 3], &plate),
            part("Stand", [0xff; 3], &stand),
        ];
        let a = Assembly::from_parts(&Mesh::from_soup(&main), &parts, &Kind::Labeled, &[]);
        check_ranges(&a);
        let names: Vec<_> = a.parts.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Plate", "Stand", "Other"]);
        assert!((a.parts[2].volume - 8.0).abs() < 1e-3);
    }

    #[test]
    fn a_point_is_on_a_triangle_to_the_tolerance() {
        let t = [Vec3::ZERO, Vec3::new(10.0, 0.0, 0.0), Vec3::new(0.0, 10.0, 0.0)];
        assert!(on_triangle(Vec3::new(2.0, 2.0, 0.0), t));
        assert!(on_triangle(Vec3::new(2.0, 2.0, 0.0005), t));
        // On an edge.
        assert!(on_triangle(Vec3::new(5.0, 0.0, 0.0), t));
        assert!(!on_triangle(Vec3::new(2.0, 2.0, 0.01), t));
        assert!(!on_triangle(Vec3::new(6.0, 6.0, 0.0), t));
        assert!(!on_triangle(Vec3::new(-0.1, 2.0, 0.0), t));
    }

    #[test]
    fn named_solids_count_on_over_sources_with_one_name() {
        // Two pieces of one value (the module split), and the exact build
        // of the value, where the two pieces touch and are one solid.
        let a_soup = cube_soup(10.0, Vec3::ZERO, false);
        let b_soup = cube_soup(10.0, Vec3::new(10.0, 0.0, 0.0), false);
        let both: Vec<_> = a_soup.iter().chain(&b_soup).copied().collect();
        let split = crate::csg_split::Split {
            parts: vec![
                part("Segment", [0xff; 3], &a_soup),
                part("Segment", [0xff; 3], &b_soup),
            ],
            cover: vec![part("Segment", [0xff; 3], &both)],
            kind: Kind::Labeled,
            ..Default::default()
        };
        let a = Assembly::from_split(&Mesh::from_soup(&both), &split);
        check_ranges(&a);
        let names: Vec<_> = a.parts.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Segment 1", "Segment 2"]);
        assert_eq!(Assembly::solid_count(&Mesh::from_soup(&both)), 1);
    }

    #[test]
    fn shown_part_names_its_shells_with_the_label() {
        let mut soup = cube_soup(10.0, Vec3::ZERO, false);
        let one = Assembly::shells(&Arc::new(Mesh::from_soup(&soup)), Some("Spine"));
        assert_eq!(one.parts[0].name, "Spine");
        soup.extend(cube_soup(2.0, Vec3::new(30.0, 0.0, 0.0), false));
        let main = Mesh::from_soup(&soup);
        let a = Assembly::from_parts(&main, &[], &Kind::Shown("Pin".into()), &[]);
        let names: Vec<_> = a.parts.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Pin 1", "Pin 2"]);

        // Two pieces that touch are one solid of the build: the pieces win.
        let a_soup = cube_soup(10.0, Vec3::ZERO, false);
        let b_soup = cube_soup(10.0, Vec3::new(10.0, 0.0, 0.0), false);
        let both: Vec<_> = a_soup.iter().chain(&b_soup).copied().collect();
        let pieces = [
            part("Segment", [0xff; 3], &a_soup),
            part("Segment", [0xff; 3], &b_soup),
        ];
        let kind = Kind::Shown("Segment".into());
        let a = Assembly::from_parts(&Mesh::from_soup(&both), &pieces, &kind, &[]);
        check_ranges(&a);
        let names: Vec<_> = a.parts.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Segment 1", "Segment 2"]);
        // Pieces that give no more solids than the build: the build wins.
        let a = Assembly::from_parts(&main, &pieces, &Kind::Shown("Pin".into()), &[]);
        assert!(a.parts.iter().all(|p| p.source == 0), "{:?}", a.parts);
    }

    #[test]
    fn moved_parts_get_their_own_vertices() {
        // Two cubes that touch at one corner share that vertex.
        let mut soup = cube_soup(10.0, Vec3::ZERO, false);
        soup.extend(cube_soup(10.0, Vec3::splat(10.0), false));
        let mesh = Arc::new(Mesh::from_soup(&soup));
        assert_eq!(mesh.vertices.len(), 15);
        let a = Assembly::shells(&mesh, None);
        assert_eq!(a.parts.len(), 2);
        let d = Vec3::new(0.0, 0.0, 5.0);
        let moved = a.moved(&[Vec3::ZERO, d]);
        assert_eq!(moved.vertices.len(), 16);
        assert_eq!(moved.triangles.len(), 24);
        let b = moved.bounds().unwrap();
        assert_eq!(b.max, Vec3::new(20.0, 20.0, 25.0));
        assert_eq!(a.bounds(&[Vec3::ZERO, d]).unwrap().max, b.max);
        let p = &a.parts[1];
        for i in p.tris.clone() {
            let [x, y, z] = moved.tri(i);
            let [u, v, w] = a.mesh.tri(i);
            assert_eq!([x, y, z], [u + d, v + d, w + d]);
        }
    }

    #[test]
    fn paint_follows_the_part_ranges() {
        let mut soup = cube_soup(10.0, Vec3::ZERO, false);
        soup.extend(cube_soup(5.0, Vec3::splat(20.0), false));
        let a = Assembly::shells(&Arc::new(Mesh::from_soup(&soup)), None);
        let paint = a.paint(&[[255, 0, 0], [0, 0, 255]]);
        assert_eq!(paint.color(0), Some(Vec3::new(1.0, 0.0, 0.0)));
        assert_eq!(paint.color(11), Some(Vec3::new(1.0, 0.0, 0.0)));
        assert_eq!(paint.color(12), Some(Vec3::new(0.0, 0.0, 1.0)));
        assert_eq!(paint.color(24), None);
    }

    #[test]
    fn split_parts_number_their_shells() {
        let mut first = cube_soup(10.0, Vec3::ZERO, false);
        first.extend(cube_soup(2.0, Vec3::new(20.0, 0.0, 0.0), false));
        let second = cube_soup(5.0, Vec3::new(40.0, 0.0, 0.0), false);
        let mut main: Vec<_> = first.iter().chain(&second).copied().collect();
        // The CSG tree has rounded numbers, so the corners of the main mesh
        // can be a little different. This gives no "Other" part.
        main.extend(cube_soup(5.0, Vec3::new(40.0, 0.0, 0.001), false));
        let parts = [
            part("Part 1", [9, 9, 9], &first),
            part("Part 2", [9, 9, 9], &second),
        ];
        let a = Assembly::from_parts(&Mesh::from_soup(&main), &parts, &Kind::Modules, &[]);
        check_ranges(&a);
        let names: Vec<_> = a.parts.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Part 1", "Part 2", "Part 3"]);
        let sources: Vec<_> = a.parts.iter().map(|p| p.source).collect();
        assert_eq!(sources, [0, 0, 1]);
        assert!(a.parts.iter().all(|p| p.color.is_none()));
        assert!((a.parts[1].volume - 8.0).abs() < 1e-3);
    }

    #[test]
    fn reference_parts_come_last_and_are_not_in_the_photo_mesh() {
        // The main mesh has two solids and no module split. The screw is a
        // `%` part: it is not in the main mesh.
        let mut main = cube_soup(10.0, Vec3::ZERO, false);
        main.extend(cube_soup(4.0, Vec3::new(30.0, 0.0, 0.0), false));
        let main = Mesh::from_soup(&main);
        let screw = part(
            "Reference",
            [0xff; 3],
            &cube_soup(2.0, Vec3::new(4.0, 4.0, 12.0), false),
        );
        let a = Assembly::from_parts(&main, &[], &Kind::Modules, std::slice::from_ref(&screw));
        check_ranges(&a);
        let names: Vec<_> = a.parts.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Part 1", "Part 2", "Reference"]);
        let refs: Vec<_> = a.parts.iter().map(|p| p.reference).collect();
        assert_eq!(refs, [false, false, true]);
        assert_eq!(a.references(), 1);
        assert_eq!(a.parts[2].color, None);
        // The photo mesh has the printed parts only, in the same order.
        let photo = a.moved(&[]);
        assert_eq!(photo.triangles.len(), a.parts[1].tris.end);
        assert_eq!(photo.triangles.len(), main.triangles.len());
        // paint_all leaves the reference part as it is.
        let mut b = Assembly::from_parts(&main, &[], &Kind::Modules, std::slice::from_ref(&screw));
        b.paint_all([1, 2, 3]);
        assert_eq!(b.parts[0].color, Some([1, 2, 3]));
        assert_eq!(b.parts[2].color, None);
        // A hidden part (alpha 0) does not stop a ray. The screw is above
        // the large cube: from above, the ray hits the screw first.
        let none = [Vec3::ZERO; 3];
        let o = Vec3::new(5.0, 5.0, 50.0);
        assert_eq!(
            a.pick(o, -Vec3::Z, &none, &[], &[1.0, 1.0, 1.0]).unwrap().0,
            2
        );
        let hidden = [1.0, 1.0, 0.0];
        assert_eq!(a.pick(o, -Vec3::Z, &none, &[], &hidden).unwrap().0, 0);
    }

    #[test]
    fn ray_hits_front_and_back_faces() {
        let tri = [Vec3::ZERO, Vec3::X, Vec3::Y];
        let down = Vec3::new(0.0, 0.0, -1.0);
        let at = Vec3::new(0.25, 0.25, 2.0);
        assert_eq!(ray_triangle(at, down, tri), Some(2.0));
        // From below: the back face.
        assert_eq!(ray_triangle(-at, -down, tri), None);
        let below = Vec3::new(0.25, 0.25, -3.0);
        assert_eq!(ray_triangle(below, -down, tri), Some(3.0));
        // The distance is in units of the direction.
        assert_eq!(ray_triangle(at, down * 2.0, tri), Some(1.0));
        // Behind the start, beside the triangle, parallel to it.
        assert_eq!(ray_triangle(at, -down, tri), None);
        assert_eq!(ray_triangle(Vec3::new(0.8, 0.8, 2.0), down, tri), None);
        assert_eq!(ray_triangle(at, Vec3::X, tri), None);
        // No area.
        assert_eq!(ray_triangle(at, down, [Vec3::ZERO, Vec3::X, Vec3::X]), None);
    }

    /// Two cubes along X: "Part 1" (10 mm) from 20 to 30, "Part 2" (5 mm)
    /// from 0 to 5.
    fn two_cubes() -> Assembly {
        let mut soup = cube_soup(5.0, Vec3::ZERO, false);
        soup.extend(cube_soup(10.0, Vec3::new(20.0, 0.0, 0.0), false));
        Assembly::shells(&Arc::new(Mesh::from_soup(&soup)), None)
    }

    #[test]
    fn pick_finds_the_nearest_part() {
        let a = two_cubes();
        let none = [Vec3::ZERO; 2];
        // From the left along +X through both cubes: the small cube is first.
        let o = Vec3::new(-10.0, 1.0, 3.0);
        assert_eq!(a.pick(o, Vec3::X, &none, &[], &[]), Some((1, 10.0)));
        // From the right: the large cube.
        let o = Vec3::new(50.0, 1.0, 3.0);
        assert_eq!(a.pick(o, -Vec3::X, &none, &[], &[]), Some((0, 20.0)));
        // Above the small cube only.
        let o = Vec3::new(1.0, 3.0, 30.0);
        assert_eq!(a.pick(o, -Vec3::Z, &none, &[], &[]), Some((1, 25.0)));
        // Between the cubes, and away from them.
        assert_eq!(
            a.pick(Vec3::new(12.0, 1.0, 30.0), -Vec3::Z, &none, &[], &[]),
            None
        );
        assert_eq!(
            a.pick(Vec3::new(-10.0, 50.0, 3.0), Vec3::X, &none, &[], &[]),
            None
        );
        // From in the large cube: its far face, a back face.
        let o = Vec3::new(25.0, 4.0, 3.0);
        assert_eq!(a.pick(o, Vec3::X, &none, &[], &[]), Some((0, 5.0)));
    }

    #[test]
    fn pick_triangle_gives_the_face_that_the_ray_hits() {
        let a = two_cubes();
        let o = Vec3::new(1.0, 3.0, 30.0);
        let (part, tri, _) = a.pick_triangle(o, -Vec3::Z, &[], &[], &[]).unwrap();
        assert_eq!(part, 1);
        assert!(a.parts[1].tris.contains(&tri));
        // The top face of the small cube.
        assert!(a.mesh.tri(tri).iter().all(|v| (v.z - 5.0).abs() < 1e-4));
    }

    #[test]
    fn pick_follows_the_offsets() {
        let a = two_cubes();
        // The small cube moves 40 mm up: the ray above it now hits nothing,
        // and a ray at its new place hits it.
        let up = [Vec3::ZERO, Vec3::new(0.0, 0.0, 40.0)];
        let down = -Vec3::Z;
        assert_eq!(a.pick(Vec3::new(1.0, 3.0, 30.0), down, &up, &[], &[]), None);
        let hit = a.pick(Vec3::new(1.0, 3.0, 60.0), down, &up, &[], &[]);
        assert_eq!(hit, Some((1, 15.0)));
    }

    #[test]
    fn pick_skips_the_cut_away_side() {
        let a = two_cubes();
        let none = [Vec3::ZERO; 2];
        // Keep x >= 25: the cut goes through the large cube.
        let cut = [Vec4::new(1.0, 0.0, 0.0, -25.0)];
        // From the left: the small cube is cut away, and the near face of
        // the large cube too. The ray hits the far face from the inside.
        let o = Vec3::new(-10.0, 1.0, 3.0);
        assert_eq!(a.pick(o, Vec3::X, &none, &cut, &[]), Some((0, 40.0)));
        // Keep x <= 3: only the small cube, and only its near face.
        let cut = [Vec4::new(-1.0, 0.0, 0.0, 3.0)];
        assert_eq!(a.pick(o, Vec3::X, &none, &cut, &[]), Some((1, 10.0)));
        // From the right: the large cube is cut away. The ray hits the
        // near face of the small cube from the inside.
        let o = Vec3::new(50.0, 1.0, 3.0);
        assert_eq!(a.pick(o, -Vec3::X, &none, &cut, &[]), Some((1, 50.0)));
    }

    #[test]
    fn pick_prefers_parts_that_are_not_transparent() {
        let a = two_cubes();
        let none = [Vec3::ZERO; 2];
        let o = Vec3::new(-10.0, 1.0, 3.0);
        // The small cube is first, but it is transparent.
        let ghost = [1.0, 0.5];
        assert_eq!(a.pick(o, Vec3::X, &none, &[], &ghost), Some((0, 30.0)));
        // A ray that hits only the transparent cube still finds it.
        let o = Vec3::new(1.0, 3.0, 30.0);
        assert_eq!(a.pick(o, -Vec3::Z, &none, &[], &ghost), Some((1, 25.0)));
        // All transparent: the nearest part.
        let o = Vec3::new(-10.0, 1.0, 3.0);
        assert_eq!(a.pick(o, Vec3::X, &none, &[], &[0.5, 0.5]), Some((1, 10.0)));
    }

    #[test]
    fn slivers_go_to_the_nearest_part() {
        let mut soup = cube_soup(10.0, Vec3::ZERO, false);
        soup.extend(cube_soup(10.0, Vec3::new(30.0, 0.0, 0.0), false));
        // A flat closed shell (two faces back to back) next to the second cube.
        let (a, b, c) = (
            Vec3::new(41.0, 0.0, 0.0),
            Vec3::new(42.0, 0.0, 0.0),
            Vec3::new(41.0, 0.0, 5.0),
        );
        soup.push([a, b, c]);
        soup.push([a, c, b]);
        let a = Assembly::shells(&Arc::new(Mesh::from_soup(&soup)), None);
        check_ranges(&a);
        assert_eq!(a.parts.len(), 2);
        let near = a.part_of(a.mesh.triangles.len() - 1).unwrap();
        assert_eq!(a.parts[near].bounds.max.x, 42.0);
        assert_eq!(a.parts[near].tris.len(), 14);
    }
}
