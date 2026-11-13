//! Parameter sets: the `sets` block of the model definition `<model>.yml`
//! (see `definition.rs`). A model without a `.yml` file can have a `.json`
//! file in the OpenSCAD Customizer format: CoCAD reads its sets, and the
//! first save writes the `.yml` file.

use crate::definition::{self, SetDef};
use crate::params::{Param, Value};
use anyhow::{Context, Result};
use std::path::Path;

/// The value for `scad` in the file `name` of the settings folder. Each line
/// of the file is `<model path>\t<value>`.
fn lookup(name: &str, scad: &Path) -> Option<String> {
    let key = scad.canonicalize().ok()?;
    let text = std::fs::read_to_string(crate::openscad::config_dir()?.join(name)).ok()?;
    text.lines()
        .filter_map(|l| l.split_once('\t'))
        .find(|(k, _)| Path::new(k) == key)
        .map(|(_, v)| v.to_string())
}

/// Sets the value for `scad` in the file `name`. `None` removes the line.
fn store(name: &str, scad: &Path, value: Option<&str>) -> Result<()> {
    let dir = crate::openscad::config_dir().context("no config folder")?;
    std::fs::create_dir_all(&dir)?;
    let file = dir.join(name);
    let key = scad.canonicalize()?;
    let mut lines: Vec<String> = std::fs::read_to_string(&file)
        .unwrap_or_default()
        .lines()
        .filter(|l| l.split_once('\t').is_some_and(|(k, _)| Path::new(k) != key))
        .map(String::from)
        .collect();
    if let Some(v) = value {
        lines.push(format!("{}\t{v}", key.display()));
    }
    std::fs::write(file, lines.join("\n") + "\n")?;
    Ok(())
}

/// The set that was active last for `scad`, from `<config>/last-sets`.
/// `Some(None)` is Model defaults. `None` means that there is no record.
pub fn last_set(scad: &Path) -> Option<Option<String>> {
    lookup("last-sets", scad).map(|v| (!v.is_empty()).then_some(v))
}

/// Records `name` as the active set for `scad`. `None` is Model defaults.
/// A name with a line break is not recorded.
pub fn set_last(scad: &Path, name: Option<&str>) -> Result<()> {
    match name {
        Some(n) if n.contains(['\n', '\r']) => store("last-sets", scad, None),
        Some(n) => store("last-sets", scad, Some(n)),
        None => store("last-sets", scad, Some("")),
    }
}

/// The set to load when `scad` opens: the set that was active last, else
/// the first set of the file. `None` is Model defaults.
pub fn autoload(scad: &Path, sets: &[String]) -> Option<String> {
    choose(last_set(scad), sets)
}

/// `autoload` without the record file. `last` is the value of `last_set`.
fn choose(last: Option<Option<String>>, sets: &[String]) -> Option<String> {
    match last {
        Some(None) => None,
        Some(Some(n)) if sets.contains(&n) => Some(n),
        _ => sets.first().cloned(),
    }
}

/// The value as the Customizer writes it: strings bare, the rest as source.
pub(crate) fn to_text(v: &Value) -> String {
    match v {
        Value::Str(s) => s.clone(),
        v => v.to_scad(),
    }
}

/// Saves all parameter values as set `name`. Other sets and the
/// description of the set stay.
pub fn save(path: &Path, name: &str, params: &[Param]) -> Result<()> {
    let mut def = definition::load(path)?;
    let values = params
        .iter()
        .map(|p| (p.name.clone(), definition::json_of(&p.value)))
        .collect();
    match def.sets.iter_mut().find(|s| s.name == name) {
        Some(set) => set.values = values,
        None => def.sets.push(SetDef {
            name: name.to_string(),
            description: None,
            values,
        }),
    }
    definition::write_sets(path, &def)
}

pub fn delete(path: &Path, name: &str) -> Result<()> {
    let mut def = definition::load(path)?;
    def.sets.retain(|s| s.name != name);
    definition::write_sets(path, &def)
}

/// Applies set `name`. Values of parameters that are not in the set go back
/// to the file defaults. Returns the names that could not be applied.
pub fn apply(path: &Path, name: &str, params: &mut [Param]) -> Result<Vec<String>> {
    let def = definition::load(path)?;
    let set = def
        .set(name)
        .with_context(|| format!("no parameter set '{name}' in {}", path.display()))?;
    Ok(apply_set(set, params))
}

/// Applies `set`, as `apply` does.
pub fn apply_set(set: &SetDef, params: &mut [Param]) -> Vec<String> {
    let mut skipped = Vec::new();
    for p in params.iter_mut() {
        p.value = p.default.clone();
    }
    for (key, raw) in &set.values {
        let Some(text) = definition::value_text(raw) else {
            skipped.push(key.clone());
            continue;
        };
        match params.iter_mut().find(|p| &p.name == key) {
            Some(p) => {
                if !crate::params::set_from_text(p, &text) {
                    skipped.push(key.clone());
                }
            }
            None => skipped.push(key.clone()),
        }
    }
    skipped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(path: &Path) -> Vec<String> {
        try_names(path).unwrap_or_default()
    }

    fn try_names(path: &Path) -> Result<Vec<String>> {
        Ok(definition::load(path)?.set_names())
    }

    fn temp(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("cocad-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn try_names_reports_a_bad_file() {
        let dir = temp("names");
        let path = dir.join("m.yml");
        assert!(try_names(&path).unwrap().is_empty());
        std::fs::write(&path, "cocad: 1\nsets: {a: [\n").unwrap();
        assert!(try_names(&path).is_err());
        std::fs::write(&path, "cocad: 1\nsets:\n  b: {}\n  a: {}\n").unwrap();
        assert_eq!(try_names(&path).unwrap(), ["b", "a"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn save_keeps_the_description() {
        let dir = temp("meta");
        let path = dir.join("m.yml");
        std::fs::write(
            &path,
            "cocad: 1\ntitle: Case\nsets:\n  big:\n    description: For B\n    values:\n      w: 42\n",
        )
        .unwrap();
        let src = "w = 10;\n";
        let mut p = crate::params::parse(src);
        assert!(apply(&path, "big", &mut p).unwrap().is_empty());
        assert_eq!(p[0].value, Value::Num(42.0));
        crate::params::apply_define(&mut p, "w=7").unwrap();
        save(&path, "big", &p).unwrap();
        let d = definition::load(&path).unwrap();
        assert_eq!(d.meta.title.as_deref(), Some("Case"));
        let big = d.set("big").unwrap();
        assert_eq!(big.description.as_deref(), Some("For B"));
        assert_eq!(big.values, [("w".to_string(), serde_json::json!(7))]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn autoload_choice() {
        let sets = ["big".to_string(), "small".to_string()];
        let s = |n: &str| Some(n.to_string());
        // No record: the first set.
        assert_eq!(choose(None, &sets), s("big"));
        // The last active set, when it is still in the file.
        assert_eq!(choose(Some(s("small")), &sets), s("small"));
        assert_eq!(choose(Some(s("gone")), &sets), s("big"));
        // Model defaults was active last.
        assert_eq!(choose(Some(None), &sets), None);
        assert_eq!(choose(None, &[]), None);
    }

    #[test]
    fn save_apply_delete() {
        let dir = temp("presets");
        let path = dir.join("m.yml");
        let src = "w = 10; // [1:100]\nshape = \"round\"; // [round, square]\nlabel = \"5\";\non = true;\nv = [1, 2];\n";
        let mut p = crate::params::parse(src);
        crate::params::apply_define(&mut p, "w=42").unwrap();
        crate::params::apply_define(&mut p, "shape=square").unwrap();
        crate::params::apply_define(&mut p, "on=false").unwrap();
        crate::params::apply_define(&mut p, "v=[3, 4.5]").unwrap();
        save(&path, "big", &p).unwrap();
        save(&path, "other", &crate::params::parse(src)).unwrap();
        assert_eq!(names(&path), ["big", "other"]);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# yaml-language-server"), "{text}");
        assert!(text.contains("      w: 42\n      shape: square\n      label: \"5\"\n      on: false\n      v: [3, 4.5]\n"), "{text}");

        let mut q = crate::params::parse(src);
        assert!(apply(&path, "big", &mut q).unwrap().is_empty());
        assert_eq!(q[0].value, Value::Num(42.0));
        assert_eq!(q[1].value, Value::Str("square".into()));
        assert_eq!(q[2].value, Value::Str("5".into())); // stays a string
        assert_eq!(q[3].value, Value::Bool(false));
        assert_eq!(q[4].value, Value::Vec(vec![3.0, 4.5]));

        delete(&path, "big").unwrap();
        assert_eq!(names(&path), ["other"]);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
