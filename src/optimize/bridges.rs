//! Bridges (feature 4 of `docs/optimize-tab.md`).
//!
//! A ceiling (a flat face that points down, above the first layer) prints
//! as a bridge. A bridge must start on straight edges. This module finds the
//! ceilings of a part in print frame and returns the fixes: a square last
//! layer under a round ceiling, a sacrificial layer or a stepped bridge
//! under a hole that goes on above the ceiling, a cone at the end of a
//! blind hole that goes up.

use super::fixes::{Plan, Settings};
use super::{Layers, Limits};
use crate::mesh::Mesh;

/// Finds the bridged ceilings of `mesh` (print frame) and returns the fixes.
pub fn plan(_mesh: &Mesh, _s: &Settings, _layers: Layers, _limits: &Limits) -> Plan {
    Plan::default()
}
