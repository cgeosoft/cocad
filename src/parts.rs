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
}
