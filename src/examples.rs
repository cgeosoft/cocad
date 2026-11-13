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
pub const MENU: [(&str, &str); 12] = [
    ("Bracket", "bracket.scad"),
    ("Demo box", "demo-box.scad"),
    ("Desk dock", "desk-dock.scad"),
    ("Gear", "gear.scad"),
    ("Honeycomb tray", "honeycomb-tray.scad"),
    ("Knob", "knob.scad"),
    ("Lid box", "lid-box.scad"),
    ("Multicolor badge", "multicolor-badge.scad"),
    ("Open mesh", "open-mesh.scad"),
    ("Part selector", "part-selector.scad"),
    ("Reference part", "reference-part.scad"),
    ("Vase", "vase.scad"),
];

/// The menu models, their definitions (`definition.rs`) and the libraries
/// they include: path and source.
const FILES: [(&str, &str); 27] = [
    file!("bracket.scad"),
    file!("demo-box.scad"),
    file!("desk-dock.scad"),
    file!("gear.scad"),
    file!("honeycomb-tray.scad"),
    file!("knob.scad"),
    file!("lid-box.scad"),
    file!("multicolor-badge.scad"),
    file!("open-mesh.scad"),
    file!("part-selector.scad"),
    file!("reference-part.scad"),
    file!("vase.scad"),
    file!("bracket.yml"),
    file!("demo-box.yml"),
    file!("desk-dock.yml"),
    file!("gear.yml"),
    file!("honeycomb-tray.yml"),
    file!("knob.yml"),
    file!("lid-box.yml"),
    file!("multicolor-badge.yml"),
    file!("open-mesh.yml"),
    file!("part-selector.yml"),
    file!("reference-part.yml"),
    file!("vase.yml"),
    file!("lib/badge-ring.svg"),
    file!("lib/badge-star.svg"),
    file!("lib/hex.scad"),
];

/// Built-in thumbnails of the menu models, PNG. `scripts/example-thumbs.sh`
/// makes them. The start page shows them until the user opens the example.
const THUMBS: [(&str, &[u8]); 12] = [
    (
        "bracket.scad",
        include_bytes!("../assets/examples/bracket.png"),
    ),
    (
        "demo-box.scad",
        include_bytes!("../assets/examples/demo-box.png"),
    ),
    (
        "desk-dock.scad",
        include_bytes!("../assets/examples/desk-dock.png"),
    ),
    ("gear.scad", include_bytes!("../assets/examples/gear.png")),
    (
        "honeycomb-tray.scad",
        include_bytes!("../assets/examples/honeycomb-tray.png"),
    ),
    ("knob.scad", include_bytes!("../assets/examples/knob.png")),
    (
        "lid-box.scad",
        include_bytes!("../assets/examples/lid-box.png"),
    ),
    (
        "multicolor-badge.scad",
        include_bytes!("../assets/examples/multicolor-badge.png"),
    ),
    (
        "open-mesh.scad",
        include_bytes!("../assets/examples/open-mesh.png"),
    ),
    (
        "part-selector.scad",
        include_bytes!("../assets/examples/part-selector.png"),
    ),
    (
        "reference-part.scad",
        include_bytes!("../assets/examples/reference-part.png"),
    ),
    ("vase.scad", include_bytes!("../assets/examples/vase.png")),
];

/// The built-in thumbnail of the example `file`.
pub fn thumb(file: &str) -> Option<&'static [u8]> {
    THUMBS.iter().find(|(f, _)| *f == file).map(|(_, png)| *png)
}

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

/// The path of the example `file` in the settings folder. It does not
/// write the file.
pub fn path(file: &str) -> Option<PathBuf> {
    Some(crate::openscad::config_dir()?.join("examples").join(file))
}

/// Returns the path of the example `file`, and writes it first if necessary.
pub fn extract(file: &str) -> io::Result<PathBuf> {
    let path = path(file).ok_or_else(|| io::Error::other("no settings folder"))?;
    if let Some(dir) = path.parent() {
        write_missing(dir)?;
    }
    Ok(path)
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
            let (w, h, _) = crate::thumbs::decode(thumb(file).unwrap()).unwrap();
            assert_eq!((w, h), (256, 256), "{file}");
        }
    }

    #[test]
    fn menu_models_have_metadata() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
        for (label, file) in MENU {
            let yml = crate::definition::path_for(Path::new(file));
            let yml = yml.to_str().unwrap();
            assert!(FILES.iter().any(|(path, _)| *path == yml), "{yml}");
            let def = crate::definition::load(&dir.join(yml)).unwrap();
            assert!(def.warnings.is_empty(), "{yml}: {:?}", def.warnings);
            assert_eq!(def.legacy, None, "{yml}");
            assert_eq!(def.meta.title.as_deref(), Some(label), "{yml}");
            assert!(def.meta.description.is_some(), "{yml}");
            assert!(def.meta.get("author").is_some(), "{yml}");
            // The parts of the definition match the part parameter.
            let src = std::fs::read_to_string(dir.join(file)).unwrap();
            let mut params = crate::params::parse(&src);
            assert!(def.apply_parts(&mut params).is_empty(), "{yml}");
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
