//! Multicolor models. A model lists its parts in a `cocad_part` dropdown:
//!
//! ```openscad
//! cocad_part = "all"; // [all, plate, star]
//! ```
//!
//! Each part renders on its own with `-D cocad_part="name"`. The order of the
//! list sets the filament number. A part sets its color with
//! `echo(cocad_color = "red")`, and CoCAD reads it from the OpenSCAD log.

use crate::mesh::Mesh;
use crate::openscad::{self, Config};
use crate::params::{self, Param, Value, Widget};
use anyhow::{Context, Result, bail};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

const PARAM: &str = "cocad_part";
const ALL: &str = "all";

/// Used for a part that does not set a color. Neighbors stay easy to tell apart.
const PALETTE: [[u8; 3]; 6] = [
    [0xff, 0xff, 0xff],
    [0xe0, 0x20, 0x20],
    [0x20, 0x20, 0x20],
    [0x20, 0x60, 0xd0],
    [0xf0, 0xc0, 0x20],
    [0x20, 0xa0, 0x50],
];

pub struct Part {
    pub name: String,
    pub color: [u8; 3],
    pub mesh: Mesh,
}

/// The part names in filament order. Empty for a model without parts.
pub fn names(params: &[Param]) -> Vec<String> {
    let Some(Widget::Dropdown(options)) =
        params.iter().find(|p| p.name == PARAM).map(|p| &p.widget)
    else {
        return Vec::new();
    };
    options
        .iter()
        .filter_map(|(value, _)| match value {
            Value::Str(s) if s != ALL => Some(s.clone()),
            _ => None,
        })
        .collect()
}

/// The selected `cocad_part` value, or `None` for a model without parts.
pub fn selected(params: &[Param]) -> Option<String> {
    if names(params).is_empty() {
        return None;
    }
    match &params.iter().find(|p| p.name == PARAM)?.value {
        Value::Str(s) => Some(s.clone()),
        _ => None,
    }
}

/// True when the 3D view must render each part to color the model:
/// the model has parts and shows all of them.
pub fn needs_render(params: &[Param]) -> bool {
    selected(params).is_some_and(|s| s == ALL)
}

/// The color of the one part that the model shows, from the log of the
/// main render. `None` when the model shows all parts or has no parts.
pub fn shown_color(params: &[Param], log: &str) -> Option<[u8; 3]> {
    let name = selected(params).filter(|s| s != ALL)?;
    let i = names(params).iter().position(|n| *n == name)?;
    Some(echoed_color(log).unwrap_or(PALETTE[i % PALETTE.len()]))
}

/// The part color of each triangle of `mesh`, the model with all parts.
/// A triangle gets the color of the part that holds two or more of its
/// corners. Other triangles, for example from parts of the model that are
/// in no part, get `None`.
pub fn face_colors(mesh: &Mesh, parts: &[Part]) -> Vec<Option<[u8; 3]>> {
    const SHARED: usize = usize::MAX;
    let key = |v: glam::Vec3| {
        [
            (v.x + 0.0).to_bits(),
            (v.y + 0.0).to_bits(),
            (v.z + 0.0).to_bits(),
        ]
    };
    let mut owner: HashMap<[u32; 3], usize> = HashMap::new();
    for (i, part) in parts.iter().enumerate() {
        for v in &part.mesh.vertices {
            owner
                .entry(key(*v))
                .and_modify(|o| {
                    if *o != i {
                        *o = SHARED;
                    }
                })
                .or_insert(i);
        }
    }
    mesh.triangles
        .iter()
        .map(|t| {
            let mut votes: Vec<usize> = Vec::with_capacity(3);
            let mut shared = 0;
            for &v in t {
                match owner.get(&key(mesh.vertices[v as usize])) {
                    Some(&SHARED) => shared += 1,
                    Some(&o) => votes.push(o),
                    None => {}
                }
            }
            votes.sort_unstable();
            // Two corners of one part, or one corner and the other two on a
            // seam between parts.
            let winner = match votes.as_slice() {
                [a, b, ..] if a == b => Some(*a),
                [_, b, c] if b == c => Some(*b),
                [a] if shared == 2 => Some(*a),
                _ => None,
            };
            winner.map(|i| parts[i].color)
        })
        .collect()
}

/// Renders all parts at the same time. The value of `cocad_part` has no effect.
pub fn render(
    cfg: &Config,
    file: &Path,
    params: &[Param],
    cancel: &Arc<AtomicBool>,
) -> Result<Vec<Part>> {
    let names = names(params);
    if names.is_empty() {
        bail!("the model has no {PARAM} parameter with part names");
    }
    let prefix = format!("{PARAM}=");
    let mut defines = params::defines(params);
    defines.retain(|d| !d.starts_with(&prefix));
    let defines = &defines;
    std::thread::scope(|scope| {
        let jobs: Vec<_> = names
            .iter()
            .enumerate()
            .map(|(i, name)| {
                scope.spawn(move || {
                    let mut defs = defines.clone();
                    defs.push(format!("{PARAM}=\"{name}\""));
                    let out = openscad::render(cfg, file, &defs, cancel)
                        .with_context(|| format!("part '{name}'"))?;
                    Ok(Part {
                        name: name.clone(),
                        color: echoed_color(&out.log).unwrap_or(PALETTE[i % PALETTE.len()]),
                        mesh: out.mesh,
                    })
                })
            })
            .collect();
        jobs.into_iter()
            .map(|job| job.join().expect("a render thread stopped"))
            .collect()
    })
}

/// Finds `ECHO: cocad_color = "red"` in the OpenSCAD log.
fn echoed_color(log: &str) -> Option<[u8; 3]> {
    let (_, rest) = log.split_once("cocad_color = \"")?;
    let (text, _) = rest.split_once('"')?;
    parse_color(text)
}

/// A `#RRGGBB` value or a common OpenSCAD color name.
fn parse_color(text: &str) -> Option<[u8; 3]> {
    if let Some(hex) = text.strip_prefix('#') {
        if hex.len() != 6 {
            return None;
        }
        let n = u32::from_str_radix(hex, 16).ok()?;
        return Some([(n >> 16) as u8, (n >> 8) as u8, n as u8]);
    }
    Some(match text.to_ascii_lowercase().as_str() {
        "white" => [0xff, 0xff, 0xff],
        "black" => [0x00, 0x00, 0x00],
        "red" => [0xff, 0x00, 0x00],
        "green" => [0x00, 0x80, 0x00],
        "blue" => [0x00, 0x00, 0xff],
        "yellow" => [0xff, 0xff, 0x00],
        "orange" => [0xff, 0xa5, 0x00],
        "purple" => [0x80, 0x00, 0x80],
        "pink" => [0xff, 0xc0, 0xcb],
        "brown" => [0xa5, 0x2a, 0x2a],
        "cyan" => [0x00, 0xff, 0xff],
        "magenta" => [0xff, 0x00, 0xff],
        "gold" => [0xff, 0xd7, 0x00],
        "silver" => [0xc0, 0xc0, 0xc0],
        "gray" | "grey" => [0x80, 0x80, 0x80],
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_skip_all_and_keep_order() {
        let p = params::parse("cocad_part = \"all\"; // [all, plate, star, ring]\n");
        assert_eq!(names(&p), ["plate", "star", "ring"]);
    }

    #[test]
    fn no_names_without_the_parameter() {
        assert!(names(&params::parse("size = 10; // [1:20]\n")).is_empty());
    }

    #[test]
    fn color_comes_from_the_log() {
        let log = "ECHO: other = 1\nECHO: cocad_color = \"red\"\n";
        assert_eq!(echoed_color(log), Some([0xff, 0, 0]));
        assert_eq!(parse_color("#102030"), Some([0x10, 0x20, 0x30]));
        assert_eq!(echoed_color("no color"), None);
        assert_eq!(parse_color("#12"), None);
    }

    fn part(name: &str, color: [u8; 3], soup: &[[glam::Vec3; 3]]) -> Part {
        Part {
            name: name.into(),
            color,
            mesh: Mesh::from_soup(soup),
        }
    }

    #[test]
    fn faces_take_the_color_of_their_part() {
        use glam::Vec3;
        let v = |x: f32, y: f32| Vec3::new(x, y, 0.0);
        let a = [v(0.0, 0.0), v(1.0, 0.0), v(0.0, 1.0)];
        let b = [v(1.0, 0.0), v(2.0, 0.0), v(1.0, 1.0)];
        let other = [v(5.0, 5.0), v(6.0, 5.0), v(5.0, 6.0)];
        let parts = [part("a", [1, 0, 0], &[a]), part("b", [0, 0, 1], &[b])];
        let model = Mesh::from_soup(&[a, b, other]);
        assert_eq!(
            face_colors(&model, &parts),
            [Some([1, 0, 0]), Some([0, 0, 1]), None]
        );
    }

    #[test]
    fn one_selected_part_takes_its_color() {
        let p = params::parse("cocad_part = \"star\"; // [all, plate, star]\n");
        assert_eq!(shown_color(&p, ""), Some(PALETTE[1]));
        assert_eq!(
            shown_color(&p, "ECHO: cocad_color = \"blue\""),
            Some([0, 0, 0xff])
        );
        assert!(!needs_render(&p));
        let all = params::parse("cocad_part = \"all\"; // [all, plate, star]\n");
        assert_eq!(shown_color(&all, ""), None);
        assert!(needs_render(&all));
    }
}
