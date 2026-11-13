//! Example models for the File menu. The binary embeds them, so each package
//! has them. A pick writes them to `<config>/examples` and opens the copy.

use std::io;
use std::path::{Path, PathBuf};

macro_rules! file {
    ($path:literal) => {
        ($path, include_str!(concat!("../examples/", $path)))
    };
}

/// Menu label and file name. Models that must fail (`// expect: error`) are not here.
pub const MENU: [(&str, &str); 11] = [
    ("Bracket", "bracket.scad"),
    ("Demo box", "demo-box.scad"),
    ("Desk dock", "desk-dock.scad"),
    ("Gear", "gear.scad"),
    ("Honeycomb tray", "honeycomb-tray.scad"),
    ("Knob", "knob.scad"),
    ("Lid box", "lid-box.scad"),
    ("Multicolor badge", "multicolor-badge.scad"),
    ("Open mesh", "open-mesh.scad"),
    ("Reference part", "reference-part.scad"),
    ("Vase", "vase.scad"),
];

/// The menu models and the libraries they include: path and source.
const FILES: [(&str, &str); 14] = [
    file!("bracket.scad"),
    file!("demo-box.scad"),
    file!("desk-dock.scad"),
    file!("gear.scad"),
    file!("honeycomb-tray.scad"),
    file!("knob.scad"),
    file!("lid-box.scad"),
    file!("multicolor-badge.scad"),
    file!("open-mesh.scad"),
    file!("reference-part.scad"),
    file!("vase.scad"),
    file!("lib/badge-ring.svg"),
    file!("lib/badge-star.svg"),
    file!("lib/hex.scad"),
];

/// Writes the files that are missing. A file that exists stays as it is, so
/// the user's edits are not lost.
fn write_missing(dir: &Path) -> io::Result<()> {
    for (path, source) in FILES {
        let path = dir.join(path);
        if path.exists() {
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, source)?;
    }
    Ok(())
}

/// Returns the path of the example `file`, and writes it first if necessary.
pub fn extract(file: &str) -> io::Result<PathBuf> {
    let dir = crate::openscad::config_dir()
        .ok_or_else(|| io::Error::other("no settings folder"))?
        .join("examples");
    write_missing(&dir)?;
    Ok(dir.join(file))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_has_every_model_that_renders() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
        let mut models: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "scad"))
            .filter(|p| {
                let src = std::fs::read_to_string(p).unwrap();
                !src.starts_with("// expect: error")
            })
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        models.sort();
        let menu: Vec<&str> = MENU.iter().map(|(_, file)| *file).collect();
        assert_eq!(models, menu);
    }

    #[test]
    fn menu_files_are_embedded() {
        for (_, file) in MENU {
            assert!(FILES.iter().any(|(path, _)| *path == file), "{file}");
        }
    }

    #[test]
    fn writes_missing_files_and_keeps_edits() {
        let dir = std::env::temp_dir().join(format!("cocad-examples-{}", std::process::id()));
        write_missing(&dir).unwrap();
        assert!(dir.join("lib/hex.scad").exists());

        let gear = dir.join("gear.scad");
        std::fs::write(&gear, "// edited").unwrap();
        std::fs::remove_file(dir.join("vase.scad")).unwrap();
        write_missing(&dir).unwrap();
        assert_eq!(std::fs::read_to_string(&gear).unwrap(), "// edited");
        assert!(dir.join("vase.scad").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
