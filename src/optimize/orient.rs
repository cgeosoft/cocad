//! Orientation (feature 2 of `docs/optimize-tab.md`): the rotation that
//! puts a part on the bed.

use super::Area;
use crate::mesh::Mesh;
use glam::{Mat4, Vec3};

/// How a part lies on the bed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Choice {
    /// The model rotation. The default.
    Model,
    /// The best face, found by `auto_down`.
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
            choice: Choice::Model,
            quarter_turns: 0,
        }
    }
}

/// The matrix that takes `mesh` from the model frame to the print frame:
/// the rotation of `o`, then the lowest point to z = 0 and the footprint
/// center to x = y = 0.
pub fn matrix(mesh: &Mesh, o: &Orientation, _area: &Area) -> Mat4 {
    let r = Mat4::from_rotation_z(std::f32::consts::FRAC_PI_2 * o.quarter_turns as f32);
    let moved = super::transformed(mesh, r);
    super::rest_on_bed(&moved) * r
}
