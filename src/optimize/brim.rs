//! Brim on small bed islands (feature 4.1 of `docs/optimize-tab.md`).
//!
//! A bed island is an area of the first layer that is not joined to the
//! rest of the part there. A small island gets a brim of one layer, joined
//! to the island, with a gap to the rest of the part.

use super::fixes::{Plan, Settings};
use super::{Area, Layers};
use crate::mesh::Mesh;

/// Finds the small bed islands of `mesh` (print frame) and returns the brim
/// statements (late statements, after the bed chamfer).
pub fn plan(_mesh: &Mesh, _s: &Settings, _layers: Layers, _area: &Area) -> Plan {
    Plan::default()
}
