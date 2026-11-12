//! Parameter sets in the OpenSCAD Customizer format: `<model>.json` next to
//! `<model>.scad`. OpenSCAD reads the same file (`-p model.json -P name`).
//!
//! ```json
//! { "fileFormatVersion": "1",
//!   "parameterSets": { "big": { "width": "80", "shape": "round", "offset": "[0, 5]" } } }
//! ```

use crate::params::{Param, Value};
use anyhow::{Context, Result};
use serde_json::{Map, Value as Json, json};
use std::path::{Path, PathBuf};

pub fn path_for(scad: &Path) -> PathBuf {
    scad.with_extension("json")
}

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

/// The sets file last used for `scad`, from `<config>/sets-files`.
pub fn remembered(scad: &Path) -> Option<PathBuf> {
    lookup("sets-files", scad).map(PathBuf::from)
}

/// Records `json` as the sets file for `scad`.
pub fn remember(scad: &Path, json: &Path) -> Result<()> {
    let key = scad.canonicalize()?;
    let value = (json != path_for(&key)).then(|| json.display().to_string());
    store("sets-files", scad, value.as_deref())
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

/// The sets file to use for `scad`: the remembered one when it still
/// exists, else `<model>.json` next to the model.
pub fn file_for(scad: &Path) -> PathBuf {
    remembered(scad)
        .filter(|p| p.exists())
        .unwrap_or_else(|| path_for(scad))
}

fn read(path: &Path) -> Result<Map<String, Json>> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            let v: Json = serde_json::from_str(&text)
                .with_context(|| format!("cannot parse {}", path.display()))?;
            Ok(v.as_object().cloned().unwrap_or_default())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Map::new()),
        Err(e) => Err(e.into()),
    }
}

fn write(path: &Path, mut root: Map<String, Json>) -> Result<()> {
    root.insert("fileFormatVersion".into(), json!("1"));
    std::fs::write(
        path,
        serde_json::to_string_pretty(&Json::Object(root))? + "\n",
    )?;
    Ok(())
}

/// Names of the sets in the file, in file order.
pub fn names(path: &Path) -> Vec<String> {
    try_names(path).unwrap_or_default()
}

/// Like `names`, but a file that cannot be read or parsed is an error.
/// A missing file has no sets.
pub fn try_names(path: &Path) -> Result<Vec<String>> {
    Ok(read(path)?
        .get("parameterSets")
        .and_then(|s| s.as_object())
        .map(|s| s.keys().cloned().collect())
        .unwrap_or_default())
}

/// The value as the Customizer writes it: strings bare, the rest as source.
fn to_text(v: &Value) -> String {
    match v {
        Value::Str(s) => s.clone(),
        v => v.to_scad(),
    }
}

/// Saves all parameter values as set `name`. Other sets stay.
pub fn save(path: &Path, name: &str, params: &[Param]) -> Result<()> {
    let mut root = read(path)?;
    let sets = root.entry("parameterSets").or_insert_with(|| json!({}));
    let set: Map<String, Json> = params
        .iter()
        .map(|p| (p.name.clone(), json!(to_text(&p.value))))
        .collect();
    sets.as_object_mut()
        .context("parameterSets is not an object")?
        .insert(name.to_string(), Json::Object(set));
    write(path, root)
}

pub fn delete(path: &Path, name: &str) -> Result<()> {
    let mut root = read(path)?;
    if let Some(sets) = root
        .get_mut("parameterSets")
        .and_then(|s| s.as_object_mut())
    {
        sets.shift_remove(name);
    }
    write(path, root)
}

/// Applies set `name`. Values of parameters that are not in the set go back
/// to the file defaults. Returns the names that could not be applied.
pub fn apply(path: &Path, name: &str, params: &mut [Param]) -> Result<Vec<String>> {
    let root = read(path)?;
    let set = root
        .get("parameterSets")
        .and_then(|s| s.get(name))
        .and_then(|s| s.as_object())
        .with_context(|| format!("no parameter set '{name}' in {}", path.display()))?;
    let mut skipped = Vec::new();
    for p in params.iter_mut() {
        p.value = p.default.clone();
    }
    for (key, raw) in set {
        let text = match raw {
            Json::String(s) => s.clone(),
            other => other.to_string(),
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
    Ok(skipped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn try_names_reports_a_bad_file() {
        let dir = std::env::temp_dir().join(format!("cocad-names-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("m.json");
        assert!(try_names(&path).unwrap().is_empty());
        std::fs::write(&path, "{ \"parameterSets\": { \"a\"").unwrap();
        assert!(try_names(&path).is_err());
        std::fs::write(&path, "{ \"parameterSets\": { \"b\": {}, \"a\": {} } }").unwrap();
        assert_eq!(try_names(&path).unwrap(), ["b", "a"]);
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
        let dir = std::env::temp_dir().join(format!("cocad-presets-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("m.json");
        let src = "w = 10; // [1:100]\nshape = \"round\"; // [round, square]\nlabel = \"5\";\non = true;\nv = [1, 2];\n";
        let mut p = crate::params::parse(src);
        crate::params::apply_define(&mut p, "w=42").unwrap();
        crate::params::apply_define(&mut p, "shape=square").unwrap();
        crate::params::apply_define(&mut p, "on=false").unwrap();
        crate::params::apply_define(&mut p, "v=[3, 4]").unwrap();
        save(&path, "big", &p).unwrap();
        save(&path, "other", &crate::params::parse(src)).unwrap();
        assert_eq!(names(&path), ["big", "other"]);

        let mut q = crate::params::parse(src);
        assert!(apply(&path, "big", &mut q).unwrap().is_empty());
        assert_eq!(q[0].value, Value::Num(42.0));
        assert_eq!(q[1].value, Value::Str("square".into()));
        assert_eq!(q[2].value, Value::Str("5".into())); // stays a string
        assert_eq!(q[3].value, Value::Bool(false));
        assert_eq!(q[4].value, Value::Vec(vec![3.0, 4.0]));

        delete(&path, "big").unwrap();
        assert_eq!(names(&path), ["other"]);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
