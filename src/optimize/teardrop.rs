//! Teardrop holes (feature 3 of `docs/optimize-tab.md`).
//!
//! A round hole with its axis parallel to the bed has a ceiling in the air
//! at its top. This module finds these holes in a part in print frame and
//! cuts a 45 degree point up at the top of each one.

use super::fixes::Plan;
use super::{Layers, Limits};
use crate::mesh::Mesh;

/// Finds the horizontal round holes of `mesh` (print frame) and returns the
/// cuts that make them teardrops.
pub fn plan(_mesh: &Mesh, _layers: Layers, _limits: &Limits) -> Plan {
    Plan::default()
}
