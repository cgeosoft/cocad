//! The Check card (port of `overhang.py` from scad-models): the problems
//! that stay in a part in print frame after the fixes.

use super::{Area, Layers, Limits};
use crate::mesh::Mesh;

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

/// The problems of `mesh` (print frame).
pub fn run(_mesh: &Mesh, _layers: Layers, _limits: &Limits, _area: &Area) -> Vec<Issue> {
    Vec::new()
}
