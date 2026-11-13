//! Print preparation for the Optimize tab. The design is in
//! `docs/optimize-tab.md`.
//!
//! The pipeline, for each print part:
//!
//! 1. `split`: cut a part that does not fit the print area into pieces, with
//!    joints.
//! 2. `orient`: the rotation that puts the part on the bed.
//! 3. `fixes`: change the geometry for the printer (teardrop holes, bridges,
//!    bed chamfer, brim). Each fix finds its places in Rust and writes
//!    OpenSCAD code. `scad` runs OpenSCAD on the part mesh with this code.
//! 4. `plate`: put the parts on one or more plates.
//! 5. `check`: list the problems that stay.
//!
//! All geometry here is in mm. "Print frame" means: the part in print
//! position, with the bed at z = 0.

// The modules are filled in steps. The Optimize tab uses them at the end.
#![allow(dead_code)]

pub mod bridges;
pub mod brim;
pub mod check;
pub mod fixes;
pub mod orient;
pub mod pipeline;
pub mod plate;
pub mod scad;
pub mod split;
pub mod teardrop;

use crate::mesh::{Bounds, Mesh};
use glam::{Mat4, Vec2, Vec3};

/// The print area of the printer: width (x), depth (y) and height (z) in mm.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Area {
    pub w: f32,
    pub d: f32,
    pub h: f32,
}

impl Default for Area {
    /// The default for a new installation. The user sets the real value in
    /// the settings.
    fn default() -> Self {
        Area {
            w: 220.0,
            d: 220.0,
            h: 250.0,
        }
    }
}

/// The layer heights that the fixes use: the bed chamfer, the brim and the
/// sacrificial layers are whole layers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layers {
    /// Height of the first layer.
    pub first: f32,
    /// Height of the other layers.
    pub height: f32,
}

impl Default for Layers {
    fn default() -> Self {
        Layers {
            first: 0.2,
            height: 0.2,
        }
    }
}

impl Layers {
    /// The top of layer `i` (0 is the first layer).
    pub fn top(&self, i: usize) -> f32 {
        self.first + self.height * i as f32
    }

    /// The number of whole layers from the bed up to `z` (rounded).
    pub fn count_to(&self, z: f32) -> usize {
        if z <= self.first * 0.5 {
            return 0;
        }
        1 + ((z - self.first) / self.height).round().max(0.0) as usize
    }
}

/// Limits that the fixes and the check use.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Limits {
    /// A down-facing face steeper than this, in degrees from the vertical,
    /// is an overhang.
    pub overhang_deg: f32,
    /// Bridges longer than this (mm) need a fix or support.
    pub bridge_mm: f32,
    /// Minimum wall (mm) that a fix must leave to the outside.
    pub min_wall: f32,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            overhang_deg: 45.0,
            bridge_mm: 12.0,
            min_wall: 0.8,
        }
    }
}

/// `mesh` with each vertex moved by `m`. A matrix with a negative
/// determinant (a mirror) also turns the triangles, so that the faces still
/// point out.
pub fn transformed(mesh: &Mesh, m: Mat4) -> Mesh {
    let flip = m.determinant() < 0.0;
    Mesh {
        vertices: mesh.vertices.iter().map(|v| m.transform_point3(*v)).collect(),
        triangles: if flip {
            mesh.triangles.iter().map(|t| [t[0], t[2], t[1]]).collect()
        } else {
            mesh.triangles.clone()
        },
    }
}

/// The triangles `tris` of `mesh` as a mesh of their own. Unused vertices
/// go away.
pub fn sub_mesh(mesh: &Mesh, tris: std::ops::Range<usize>) -> Mesh {
    let mut map = std::collections::HashMap::new();
    let mut out = Mesh::default();
    for t in &mesh.triangles[tris] {
        let t = t.map(|v| {
            *map.entry(v).or_insert_with(|| {
                out.vertices.push(mesh.vertices[v as usize]);
                (out.vertices.len() - 1) as u32
            })
        });
        out.triangles.push(t);
    }
    out
}

/// Moves `mesh` so that its lowest point is at z = 0 and the center of its
/// footprint is at x = y = 0. Returns the matrix of this move.
pub fn rest_on_bed(mesh: &Mesh) -> Mat4 {
    match mesh.bounds() {
        Some(b) => {
            let c = b.center();
            Mat4::from_translation(Vec3::new(-c.x, -c.y, -b.min.z))
        }
        None => Mat4::IDENTITY,
    }
}

/// The area of triangle `i` of `mesh`, in mm².
pub fn tri_area(mesh: &Mesh, i: usize) -> f32 {
    let [a, b, c] = mesh.tri(i);
    (b - a).cross(c - a).length() * 0.5
}

/// The section of `mesh` at height `z`: closed loops, outer loops counter
/// clockwise, holes clockwise. The material is on the left of each loop.
pub fn section(mesh: &Mesh, z: f32) -> Vec<Vec<Vec2>> {
    let tris: Vec<u32> = (0..mesh.triangles.len() as u32)
        .filter(|&i| {
            let t = mesh.triangles[i as usize];
            let zs = t.map(|v| mesh.vertices[v as usize].z);
            let lo = zs[0].min(zs[1]).min(zs[2]);
            let hi = zs[0].max(zs[1]).max(zs[2]);
            lo < z && hi >= z
        })
        .collect();
    crate::slicer::cut(mesh, &tris, z)
}

/// The signed area of a closed loop (positive: counter clockwise).
pub fn loop_area(p: &[Vec2]) -> f32 {
    crate::slicer::area(p)
}

/// The bounds of `mesh`, or a zero box at the origin for an empty mesh.
pub fn bounds_or_zero(mesh: &Mesh) -> Bounds {
    mesh.bounds().unwrap_or(Bounds {
        min: Vec3::ZERO,
        max: Vec3::ZERO,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layers_count_whole_layers() {
        let l = Layers {
            first: 0.2,
            height: 0.2,
        };
        assert_eq!(l.count_to(0.0), 0);
        assert_eq!(l.count_to(0.2), 1);
        assert_eq!(l.count_to(0.4), 2);
        assert!((l.top(1) - 0.4).abs() < 1e-6);
    }

    #[test]
    fn section_of_a_cube_is_one_square() {
        let m = crate::mesh::cube(10.0);
        let b = m.bounds().unwrap();
        let loops = section(&m, (b.min.z + b.max.z) * 0.5);
        assert_eq!(loops.len(), 1);
        assert!((loop_area(&loops[0]).abs() - 100.0).abs() < 0.1);
    }

    #[test]
    fn mirror_keeps_the_volume_positive() {
        let m = crate::mesh::cube(10.0);
        let v = m.volume();
        let mirrored = transformed(&m, Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0)));
        assert!((mirrored.volume() - v).abs() < 1e-3);
    }
}
