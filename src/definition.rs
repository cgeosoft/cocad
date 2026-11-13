//! The model definition: `<model>.yml` next to `<model>.scad`. The `.scad`
//! file holds the OpenSCAD code. The definition holds the rest: the text
//! about the model, the names and colors of its parts, and the parameter
//! sets.
//!
//! ```yaml
//! # yaml-language-server: $schema=https://cocad.cgeosoft.com/schema/model-1.json
//! cocad: 1
//! title: Lid box
//! description: A box with a lid.
//! author: cgeosoft
//! license: MIT
//! url: https://example.com/lid-box
//! parts:
//!   - id: box
//!     name: Box
//!   - id: lid
//!     name: Lid
//! sets:
//!   Large:
//!     description: For A5 cards.
//!     values:
//!       width: 80
//!       shape: round
//!       offset: [0, 5]
//! ```
//!
//! - `cocad` is the version of the format. CoCAD reads versions up to
//!   `VERSION`. The JSON Schema is `extras/website/public/schema/model-1.json`,
//!   published at `SCHEMA_URL`.
//! - `parts` names the values of the part parameter (`part` by default, or
//!   `part_parameter`) other than `all`. See `part_param.rs`. With
//!   `multicolor: true`, the parts are the colors of one object.
//! - A model without a `.yml` file can have a `.json` file in the format of
//!   the OpenSCAD Customizer. CoCAD reads its text and its sets. The first
//!   save of a set writes the `.yml` file. The `.json` file stays.
//! - A save of a set writes only the `sets` block again. The other blocks
//!   and their comments stay as they are.

use crate::params::{Param, Value};
use anyhow::{Context, Result, bail};
use serde_json::{Map, Value as Json};
use std::path::{Path, PathBuf};

/// The format version that this CoCAD writes and the newest that it reads.
pub const VERSION: u64 = 1;
/// The published JSON Schema of version 1.
pub const SCHEMA_URL: &str = "https://cocad.cgeosoft.com/schema/model-1.json";
/// The default name of the parameter that selects a part.
pub const PART_PARAMETER: &str = "part";

/// Metadata keys at the root, in the order that CoCAD writes them.
const META_KEYS: [&str; 6] = [
    "title",
    "description",
    "author",
    "version",
    "license",
    "url",
];

/// Text about the model or about one set.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Meta {
    pub title: Option<String>,
    pub description: Option<String>,
    /// `author`, `version`, `license` and `url` as (key, value), in that order.
    pub other: Vec<(String, String)>,
}

impl Meta {
    pub fn get(&self, key: &str) -> Option<&str> {
        match key {
            "title" => self.title.as_deref(),
            "description" => self.description.as_deref(),
            _ => self
                .other
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.as_str()),
        }
    }
}

/// One value of the part parameter.
#[derive(Debug, Clone, PartialEq)]
pub struct PartDef {
    /// The value of the part parameter.
    pub id: String,
    /// The name that CoCAD shows. `None`: the id.
    pub name: Option<String>,
    /// A `#RRGGBB` value or a color name, for a multicolor print.
    pub color: Option<String>,
}

/// One parameter set.
#[derive(Debug, Clone, PartialEq)]
pub struct SetDef {
    pub name: String,
    pub description: Option<String>,
    /// Parameter name and value, in file order.
    pub values: Vec<(String, Json)>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Definition {
    pub meta: Meta,
    /// The parameter that selects a part. `None`: `part`.
    pub part_parameter: Option<String>,
    /// The parts are the colors of one object, not separate print parts.
    pub multicolor: bool,
    pub parts: Vec<PartDef>,
    pub sets: Vec<SetDef>,
    /// Problems that did not stop the read. CoCAD skips the bad entries.
    pub warnings: Vec<String>,
    /// The `.json` file in the OpenSCAD format that the definition comes
    /// from, when the model has no `.yml` file.
    pub legacy: Option<PathBuf>,
}

impl Definition {
    pub fn set(&self, name: &str) -> Option<&SetDef> {
        self.sets.iter().find(|s| s.name == name)
    }

    pub fn set_names(&self) -> Vec<String> {
        self.sets.iter().map(|s| s.name.clone()).collect()
    }

    /// The name of the parameter that selects a part.
    pub fn part_parameter(&self) -> &str {
        self.part_parameter.as_deref().unwrap_or(PART_PARAMETER)
    }

    /// Gives the parts of the definition to the part parameter in `params`:
    /// its list then shows the part names. Without parts in the definition,
    /// the part list of the `.scad` file stays (see `part_param::mark_legacy`).
    /// Returns the problems.
    pub fn apply_parts(&self, params: &mut [Param]) -> Vec<String> {
        if self.parts.is_empty() {
            return Vec::new();
        }
        let name = self.part_parameter();
        let Some(i) = params.iter().position(|p| p.name == name) else {
            return vec![format!(
                "the model has no parameter `{name}` that selects a part. \
                 The parts of the definition are not used"
            )];
        };
        if !matches!(params[i].default, Value::Str(_)) {
            return vec![format!(
                "the parameter `{name}` is not text. The parts of the definition are not used"
            )];
        }
        let mut warnings = Vec::new();
        let choices = self
            .parts
            .iter()
            .map(|d| {
                let color = d.color.as_deref().and_then(|c| {
                    let rgb = crate::parts::parse_color(c);
                    if rgb.is_none() {
                        warnings.push(format!("part `{}`: unknown color `{c}`", d.id));
                    }
                    rgb
                });
                crate::part_param::Choice {
                    value: d.id.clone(),
                    label: d.name.clone().unwrap_or_else(|| d.id.clone()),
                    color,
                }
            })
            .collect();
        crate::part_param::mark(
            params,
            i,
            crate::part_param::PartList {
                multicolor: self.multicolor,
                choices,
            },
        );
        warnings
    }
}

/// The definition file of `scad`: `<model>.yml` next to it.
pub fn path_for(scad: &Path) -> PathBuf {
    scad.with_extension("yml")
}

/// The file that `load(path)` reads: `path`, else the `.json` file in the
/// OpenSCAD format next to it. `None` when neither exists.
pub fn source(path: &Path) -> Option<PathBuf> {
    if path.exists() {
        return Some(path.to_path_buf());
    }
    let json = path.with_extension("json");
    (is_yaml(path) && json.exists()).then_some(json)
}

fn is_yaml(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("yml") || e.eq_ignore_ascii_case("yaml"))
}

/// Reads the definition at `path` (see `source`). No file gives an empty
/// definition. A file that cannot be parsed is an error.
pub fn load(path: &Path) -> Result<Definition> {
    let Some(src) = source(path) else {
        return Ok(Definition::default());
    };
    let text =
        std::fs::read_to_string(&src).with_context(|| format!("cannot read {}", src.display()))?;
    let mut def =
        parse(&text, !is_yaml(&src)).with_context(|| format!("cannot read {}", src.display()))?;
    if src != path || def.legacy.is_some() {
        def.legacy = Some(src);
    }
    Ok(def)
}

/// Parses a definition. `json` parses the text as JSON, else as YAML. A
/// file in the OpenSCAD Customizer format (`parameterSets`) is converted.
pub fn parse(text: &str, json: bool) -> Result<Definition> {
    let root: Json = if json {
        serde_json::from_str(text)?
    } else if text.trim().is_empty() {
        Json::Object(Map::new())
    } else {
        serde_yaml_ng::from_str(text)?
    };
    let root = match root {
        Json::Object(root) => root,
        // Only comments.
        Json::Null => Map::new(),
        _ => bail!("the file is not a mapping of keys and values"),
    };
    if !root.contains_key("cocad")
        && (root.contains_key("parameterSets") || root.contains_key("fileFormatVersion"))
    {
        let mut def = from_openscad(&root);
        def.legacy = Some(PathBuf::new());
        return Ok(def);
    }
    from_root(&root)
}

fn from_root(root: &Map<String, Json>) -> Result<Definition> {
    let mut def = Definition::default();
    let w = &mut def.warnings;
    match root.get("cocad") {
        None => w.push(format!(
            "no `cocad` version. CoCAD reads it as version {VERSION}"
        )),
        Some(v) => match v.as_u64() {
            Some(n) if n > VERSION => bail!(
                "the definition has version {n}. This CoCAD reads version {VERSION}. \
                 Update CoCAD"
            ),
            Some(n) if n >= 1 => {}
            _ => bail!("`cocad` must be a version number, for example {VERSION}"),
        },
    }
    for (key, value) in root {
        match key.as_str() {
            "cocad" | "parts" | "sets" => {}
            k if META_KEYS.contains(&k) => {}
            "part_parameter" => match value.as_str().map(str::trim) {
                Some(s) if !s.is_empty() => def.part_parameter = Some(s.to_string()),
                _ => w.push("`part_parameter` must be a parameter name".into()),
            },
            "multicolor" => match value.as_bool() {
                Some(b) => def.multicolor = b,
                None => w.push("`multicolor` must be true or false".into()),
            },
            k if k.starts_with("x-") => {}
            k => w.push(format!("unknown key `{k}`")),
        }
    }
    def.meta = meta(root, &META_KEYS, w);

    match root.get("parts") {
        None | Some(Json::Null) => {}
        Some(Json::Array(items)) => {
            for (i, item) in items.iter().enumerate() {
                let Some(p) = part(item, i, w) else { continue };
                if def.parts.iter().any(|d| d.id == p.id) {
                    w.push(format!("parts[{i}]: the id is used two times"));
                } else {
                    def.parts.push(p);
                }
            }
        }
        Some(_) => w.push("`parts` must be a list".into()),
    }
    match root.get("sets") {
        None | Some(Json::Null) => {}
        Some(Json::Object(sets)) => {
            for (name, set) in sets {
                match set {
                    Json::Object(set) => def.sets.push(set_def(name, set, w)),
                    Json::Null => def.sets.push(SetDef {
                        name: name.clone(),
                        description: None,
                        values: Vec::new(),
                    }),
                    _ => w.push(format!("set `{name}` must be a mapping")),
                }
            }
        }
        Some(_) => w.push("`sets` must be a mapping of set names".into()),
    }
    Ok(def)
}

/// Text of a scalar. Numbers and booleans become text. Empty text is `None`.
fn text(v: &Json) -> Option<String> {
    let t = match v {
        Json::String(s) => s.trim().to_string(),
        Json::Number(n) => n.to_string(),
        Json::Bool(b) => b.to_string(),
        _ => return None,
    };
    (!t.is_empty()).then_some(t)
}

fn meta(obj: &Map<String, Json>, keys: &[&str], w: &mut Vec<String>) -> Meta {
    let mut m = Meta::default();
    for key in keys {
        let Some(v) = obj.get(*key) else { continue };
        let Some(t) = text(v) else {
            if !v.is_null() && v.as_str().is_none_or(|s| !s.trim().is_empty()) {
                w.push(format!("`{key}` must be text"));
            }
            continue;
        };
        match *key {
            "title" => m.title = Some(t),
            "description" => m.description = Some(t),
            k => m.other.push((k.to_string(), t)),
        }
    }
    m
}

fn part(item: &Json, i: usize, w: &mut Vec<String>) -> Option<PartDef> {
    let empty = Map::new();
    let (id, obj) = match item {
        // The short form: only the id.
        Json::String(s) if !s.trim().is_empty() => (s.trim().to_string(), &empty),
        Json::Object(o) => {
            let Some(id) = o.get("id").and_then(text) else {
                w.push(format!("parts[{i}] must have an `id`"));
                return None;
            };
            (id, o)
        }
        _ => {
            w.push(format!("parts[{i}] must have an `id`"));
            return None;
        }
    };
    if id == crate::part_param::ALL {
        w.push(format!(
            "parts[{i}]: `all` is not a part. It shows all parts"
        ));
        return None;
    }
    for k in obj.keys() {
        if !["id", "name", "color"].contains(&k.as_str()) {
            w.push(format!("parts[{i}]: unknown key `{k}`"));
        }
    }
    Some(PartDef {
        id,
        name: obj.get("name").and_then(text),
        color: obj.get("color").and_then(text),
    })
}

fn set_def(name: &str, obj: &Map<String, Json>, w: &mut Vec<String>) -> SetDef {
    let mut values = Vec::new();
    match obj.get("values") {
        None | Some(Json::Null) => {}
        Some(Json::Object(v)) => {
            for (k, v) in v {
                if value_text(v).is_some() {
                    values.push((k.clone(), v.clone()));
                } else {
                    w.push(format!("set `{name}`: `{k}` is not a parameter value"));
                }
            }
        }
        Some(_) => w.push(format!("set `{name}`: `values` must be a mapping")),
    }
    for k in obj.keys() {
        if k != "values" && k != "description" {
            w.push(format!("set `{name}`: unknown key `{k}`"));
        }
    }
    SetDef {
        name: name.to_string(),
        description: obj.get("description").and_then(text),
        values,
    }
}

/// The value as the Customizer reads it: text bare, numbers and vectors as
/// source. `None` for a value that no parameter can take.
pub fn value_text(v: &Json) -> Option<String> {
    match v {
        Json::String(s) => Some(s.clone()),
        Json::Bool(b) => Some(b.to_string()),
        Json::Number(n) => Some(num_text(n)),
        Json::Array(items) => {
            let nums: Option<Vec<String>> =
                items.iter().map(|i| i.as_number().map(num_text)).collect();
            Some(format!("[{}]", nums?.join(", ")))
        }
        _ => None,
    }
}

fn num_text(n: &serde_json::Number) -> String {
    match n.as_f64() {
        Some(f) if n.is_f64() => crate::params::fmt_num(f),
        _ => n.to_string(),
    }
}

/// A set value of a parameter value: numbers, booleans and vectors keep
/// their type.
pub fn json_of(v: &Value) -> Json {
    let num = |n: f64| -> Json {
        let n = crate::params::clean(n);
        if n.fract() == 0.0 && n.abs() < 1e15 {
            Json::from(n as i64)
        } else {
            Json::from(n)
        }
    };
    match v {
        Value::Num(n) => num(*n),
        Value::Bool(b) => Json::Bool(*b),
        Value::Str(s) => Json::String(s.clone()),
        Value::Vec(v) => Json::Array(v.iter().map(|n| num(*n)).collect()),
    }
}

/// A value of a `.json` file in the OpenSCAD format. The Customizer writes
/// all values as text. Text that is a decimal number, a boolean or a vector
/// of numbers gets its type. Other text stays text.
fn typed(v: &Json) -> Json {
    let Json::String(s) = v else {
        return v.clone();
    };
    let t = s.trim();
    let decimal = |t: &str| {
        let digits = t.strip_prefix('-').unwrap_or(t);
        !digits.is_empty()
            && !digits.starts_with('.')
            && !digits.ends_with('.')
            && digits.chars().all(|c| c.is_ascii_digit() || c == '.')
            && digits.matches('.').count() <= 1
    };
    let parsed: Option<Json> = match t {
        "true" => Some(Json::Bool(true)),
        "false" => Some(Json::Bool(false)),
        _ if t.starts_with('[') && t.ends_with(']') => {
            let items: Vec<&str> = t[1..t.len() - 1].split(',').map(str::trim).collect();
            items.iter().all(|i| decimal(i)).then(|| {
                Json::Array(
                    (items.iter())
                        .map(|i| json_of(&Value::Num(i.parse().unwrap_or(0.0))))
                        .collect(),
                )
            })
        }
        _ if decimal(t) => t.parse::<f64>().ok().map(|n| json_of(&Value::Num(n))),
        _ => None,
    };
    parsed.unwrap_or_else(|| v.clone())
}

/// The sets in the format of the OpenSCAD Customizer, for `openscad -p`.
/// The Customizer reads all values as text.
pub fn openscad_json(def: &Definition) -> Json {
    let sets: Map<String, Json> = (def.sets.iter())
        .map(|s| {
            let values: Map<String, Json> = (s.values.iter())
                .filter_map(|(k, v)| Some((k.clone(), Json::String(value_text(v)?))))
                .collect();
            (s.name.clone(), Json::Object(values))
        })
        .collect();
    serde_json::json!({ "fileFormatVersion": "1", "parameterSets": sets })
}

/// Converts a `.json` file of the OpenSCAD Customizer. String fields at the
/// root are the metadata. In a set, keys that start with `$` are metadata.
fn from_openscad(root: &Map<String, Json>) -> Definition {
    let mut def = Definition::default();
    let w = &mut def.warnings;
    def.meta = meta(root, &META_KEYS, w);
    for k in root.keys() {
        if !META_KEYS.contains(&k.as_str())
            && !matches!(k.as_str(), "fileFormatVersion" | "parameterSets")
        {
            w.push(format!("unknown key `{k}` is not used"));
        }
    }
    if let Some(sets) = root.get("parameterSets").and_then(Json::as_object) {
        for (name, set) in sets {
            let Some(set) = set.as_object() else { continue };
            def.sets.push(SetDef {
                name: name.clone(),
                description: set.get("$description").and_then(text),
                values: set
                    .iter()
                    .filter(|(k, _)| !k.starts_with('$'))
                    .map(|(k, v)| (k.clone(), typed(v)))
                    .collect(),
            });
        }
    }
    def
}

// ---- Write ----------------------------------------------------------------

/// The YAML value of `key: <out>` reads back as `want`.
fn reads_as(out: &str, want: &Json) -> bool {
    let doc = format!("k: {out}\n");
    serde_yaml_ng::from_str::<Json>(&doc)
        .ok()
        .and_then(|v| v.get("k").cloned())
        .is_some_and(|v| v == *want)
}

/// Text as a YAML scalar on one line: plain when it reads back the same,
/// else in double quotes.
fn scalar(s: &str) -> String {
    let want = Json::String(s.to_string());
    if !s.is_empty() && !s.contains(['\n', '\r', '\t']) && reads_as(s, &want) {
        return s.to_string();
    }
    Json::String(s.to_string()).to_string()
}

/// A mapping key.
fn key(k: &str) -> String {
    let plain = !k.is_empty()
        && !k.contains(['\n', '\r', '\t'])
        && serde_yaml_ng::from_str::<Json>(&format!("{k}: 0\n"))
            .ok()
            .and_then(|v| v.as_object().map(|o| o.len() == 1 && o.contains_key(k)))
            .unwrap_or(false);
    if plain {
        k.to_string()
    } else {
        Json::String(k.to_string()).to_string()
    }
}

/// Long text as a folded block (`>-`), with lines of at most `width`
/// characters. Other text as a scalar.
fn text_value(s: &str, indent: usize, width: usize) -> String {
    let one = scalar(s);
    if indent + one.len() <= width || s.contains("  ") || s != s.trim() || s.contains('\n') {
        return one;
    }
    let pad = " ".repeat(indent + 2);
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in s.split(' ') {
        if !line.is_empty() && pad.len() + line.len() + 1 + word.len() > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    lines.push(line);
    let block = format!(
        ">-\n{}",
        lines
            .iter()
            .map(|l| format!("{pad}{l}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    if reads_as(&block, &Json::String(s.to_string())) {
        block
    } else {
        one
    }
}

/// A set value: numbers, booleans and vectors as they are, text as a scalar.
fn value_yaml(v: &Json) -> String {
    match v {
        Json::String(s) => scalar(s),
        Json::Number(n) => num_text(n),
        Json::Bool(b) => b.to_string(),
        Json::Array(items) => format!(
            "[{}]",
            items.iter().map(value_yaml).collect::<Vec<_>>().join(", ")
        ),
        other => other.to_string(),
    }
}

const WIDTH: usize = 78;

/// The `sets` block, with a line break at the end. Empty for no sets.
fn sets_yaml(sets: &[SetDef]) -> String {
    if sets.is_empty() {
        return String::new();
    }
    let mut out = String::from("sets:\n");
    for s in sets {
        out += &format!("  {}:", key(&s.name));
        if s.description.is_none() && s.values.is_empty() {
            out += " {}\n";
            continue;
        }
        out += "\n";
        if let Some(d) = &s.description {
            out += &format!("    description: {}\n", text_value(d, 4, WIDTH));
        }
        if !s.values.is_empty() {
            out += "    values:\n";
            for (k, v) in &s.values {
                out += &format!("      {}: {}\n", key(k), value_yaml(v));
            }
        }
    }
    out
}

/// The full file.
pub fn to_yaml(def: &Definition) -> String {
    let mut out = format!("# yaml-language-server: $schema={SCHEMA_URL}\ncocad: {VERSION}\n");
    for k in META_KEYS {
        if let Some(v) = def.meta.get(k) {
            out += &format!("{k}: {}\n", text_value(v, 0, WIDTH));
        }
    }
    if let Some(p) = &def.part_parameter {
        out += &format!("part_parameter: {}\n", scalar(p));
    }
    if def.multicolor {
        out += "multicolor: true\n";
    }
    if !def.parts.is_empty() {
        out += "\nparts:\n";
        for p in &def.parts {
            out += &format!("  - id: {}\n", scalar(&p.id));
            if let Some(n) = &p.name {
                out += &format!("    name: {}\n", scalar(n));
            }
            if let Some(c) = &p.color {
                out += &format!("    color: {}\n", scalar(c));
            }
        }
    }
    let sets = sets_yaml(&def.sets);
    if !sets.is_empty() {
        out += "\n";
        out += &sets;
    }
    out
}

/// A line at the start of the line, that is not blank and not a comment:
/// a top-level key or a document marker.
fn top_level(line: &str) -> bool {
    line.starts_with(|c: char| !c.is_whitespace() && c != '#')
}

/// `text` with the `sets` block replaced by `block`. Blank lines and
/// comments at the start of the line after the block stay: they belong to
/// the next key. `None` when the file has no `sets:` line at the start of a
/// line.
fn splice(text: &str, block: &str) -> Option<String> {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let start = lines.iter().position(|l| {
        l.strip_prefix("sets")
            .map(str::trim_start)
            .is_some_and(|r| r.starts_with(':'))
    })?;
    let mut end = (start + 1..lines.len())
        .find(|&i| top_level(lines[i]))
        .unwrap_or(lines.len());
    while end > start + 1 {
        let l = lines[end - 1];
        if l.trim().is_empty() || l.starts_with('#') {
            end -= 1;
        } else {
            break;
        }
    }
    let mut out: String = lines[..start].concat();
    out += block;
    out += &lines[end..].concat();
    Some(out)
}

/// Writes the sets of `def` to the definition file `path`. An existing
/// `.yml` file keeps all its other lines. A new file gets all of `def`.
pub fn write_sets(path: &Path, def: &Definition) -> Result<()> {
    let block = sets_yaml(&def.sets);
    let old = match std::fs::read_to_string(path) {
        Ok(t) => Some(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e).with_context(|| format!("cannot read {}", path.display())),
    };
    let text = match old {
        Some(old) => {
            let spliced = splice(&old, &block).or_else(|| {
                if block.is_empty() {
                    return Some(old.clone());
                }
                let mut t = old.clone();
                if !t.is_empty() && !t.ends_with('\n') {
                    t.push('\n');
                }
                if !t.trim().is_empty() {
                    t.push('\n');
                }
                t += &block;
                Some(t)
            });
            // The result must read back with the same blocks and the new
            // sets. If not (for example a file in the JSON style), all of
            // the file is written again.
            let want = parse(&old, false)?;
            match spliced {
                Some(t)
                    if parse(&t, false)
                        .is_ok_and(|d| d.sets == def.sets && same_but_sets(&d, &want)) =>
                {
                    t
                }
                _ => {
                    let mut full = want;
                    full.sets = def.sets.clone();
                    to_yaml(&full)
                }
            }
        }
        None => to_yaml(def),
    };
    std::fs::write(path, text).with_context(|| format!("cannot write {}", path.display()))
}

fn same_but_sets(a: &Definition, b: &Definition) -> bool {
    a.meta == b.meta
        && a.part_parameter == b.part_parameter
        && a.multicolor == b.multicolor
        && a.parts == b.parts
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cocad-def-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const FILE: &str = "\
# yaml-language-server: $schema=https://cocad.cgeosoft.com/schema/model-1.json
cocad: 1
title: Case
# The text of the model.
description: >-
  A box with
  a lid.
author: me
version: 2
x-note: for tools
parts:
  - id: box
    name: Box
  - id: lid
  - star
sets:
  # The big variant.
  Big:
    description: For B
    values:
      w: 42
      on: true
      label: \"5\"
      v: [1, 2.5]
  plain: {}

# Comment of the next key.
multicolor: false
";

    #[test]
    fn reads_all_blocks() {
        let d = parse(FILE, false).unwrap();
        assert!(d.warnings.is_empty(), "{:?}", d.warnings);
        assert_eq!(d.meta.title.as_deref(), Some("Case"));
        assert_eq!(d.meta.description.as_deref(), Some("A box with a lid."));
        assert_eq!(
            d.meta.other,
            [
                ("author".into(), "me".into()),
                ("version".into(), "2".into())
            ]
        );
        assert_eq!(d.part_parameter(), "part");
        assert!(!d.multicolor);
        let ids: Vec<_> = d.parts.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["box", "lid", "star"]);
        assert_eq!(d.parts[0].name.as_deref(), Some("Box"));
        assert_eq!(d.parts[1].name, None);
        assert_eq!(d.set_names(), ["Big", "plain"]);
        let big = d.set("Big").unwrap();
        assert_eq!(big.description.as_deref(), Some("For B"));
        let texts: Vec<_> = big
            .values
            .iter()
            .map(|(k, v)| (k.as_str(), value_text(v).unwrap()))
            .collect();
        assert_eq!(
            texts,
            [
                ("w", "42".to_string()),
                ("on", "true".into()),
                ("label", "5".into()),
                ("v", "[1, 2.5]".into())
            ]
        );
    }

    #[test]
    fn warnings_and_errors() {
        let d = parse("cocad: 1\ntitel: x\nparts: [{name: A}, all, a, a]\n", false).unwrap();
        assert_eq!(
            d.warnings,
            [
                "unknown key `titel`",
                "parts[0] must have an `id`",
                "parts[1]: `all` is not a part. It shows all parts",
                "parts[3]: the id is used two times"
            ]
        );
        assert_eq!(d.parts.len(), 1);
        assert!(parse("cocad: 2\n", false).is_err());
        assert!(parse("cocad: one\n", false).is_err());
        assert!(parse("- a\n", false).is_err());
        assert!(parse("a: [\n", false).is_err());
        assert_eq!(parse("", false).unwrap().warnings.len(), 1);
    }

    #[test]
    fn converts_an_openscad_file() {
        let d = parse(
            r#"{ "fileFormatVersion": "1", "title": "Case", "license": "MIT", "tags": "x",
                 "parameterSets": { "big": { "$description": "For B", "$title": "B",
                   "w": "42", "s": "round", "v": "[0, 5]", "on": "true", "n": "1e3" } } }"#,
            true,
        )
        .unwrap();
        assert!(d.legacy.is_some());
        assert_eq!(d.meta.title.as_deref(), Some("Case"));
        assert_eq!(d.meta.other, [("license".into(), "MIT".into())]);
        assert_eq!(d.warnings, ["unknown key `tags` is not used"]);
        let big = d.set("big").unwrap();
        assert_eq!(big.description.as_deref(), Some("For B"));
        assert_eq!(
            big.values,
            [
                ("w".into(), json!(42)),
                ("s".into(), json!("round")),
                ("v".into(), json!([0, 5])),
                ("on".into(), json!(true)),
                // Does not read back the same: stays text.
                ("n".into(), json!("1e3")),
            ]
        );
    }

    #[test]
    fn full_file_reads_back() {
        let mut d = parse(FILE, false).unwrap();
        d.meta.description = Some(
            "A long description that does not fit on one line of the file, so it goes into \
             a folded block: it reads back the same."
                .into(),
        );
        d.sets.push(SetDef {
            name: "2024: new".into(),
            description: Some("a: b # c".into()),
            values: vec![
                ("null".into(), json!("yes")),
                ("t".into(), json!("true")),
                ("e".into(), json!("")),
                ("q".into(), json!("say \"hi\"\n")),
            ],
        });
        let text = to_yaml(&d);
        assert!(
            text.starts_with("# yaml-language-server: $schema="),
            "{text}"
        );
        assert!(text.contains("description: >-\n  A long"), "{text}");
        let back = parse(&text, false).unwrap();
        assert!(back.warnings.is_empty(), "{:?}", back.warnings);
        assert_eq!(back, d);
    }

    #[test]
    fn save_keeps_other_lines() {
        let dir = temp("splice");
        let path = dir.join("m.yml");
        std::fs::write(&path, FILE).unwrap();
        let mut d = load(&path).unwrap();
        d.sets.retain(|s| s.name != "plain");
        d.sets.push(SetDef {
            name: "New".into(),
            description: None,
            values: vec![("w".into(), json!(7))],
        });
        write_sets(&path, &d).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# The text of the model."), "{text}");
        assert!(
            text.contains("# Comment of the next key.\nmulticolor: false"),
            "{text}"
        );
        assert!(text.contains("  New:\n    values:\n      w: 7\n"), "{text}");
        assert_eq!(load(&path).unwrap().sets, d.sets);

        // No sets: the block goes away. A new set adds it at the end.
        d.sets.clear();
        write_sets(&path, &d).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("sets:"), "{text}");
        assert!(text.contains("multicolor: false"), "{text}");
        d.sets.push(SetDef {
            name: "A".into(),
            description: None,
            values: vec![("w".into(), json!(1))],
        });
        write_sets(&path, &d).unwrap();
        assert_eq!(load(&path).unwrap().sets, d.sets);

        // A file in the JSON style is written again in full.
        std::fs::write(&path, r#"{"cocad": 1, "title": "J", "sets": {"A": {}}}"#).unwrap();
        write_sets(&path, &d).unwrap();
        let back = load(&path).unwrap();
        assert_eq!(back.meta.title.as_deref(), Some("J"));
        assert_eq!(back.sets, d.sets);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn json_file_is_the_fallback() {
        let dir = temp("legacy");
        let yml = dir.join("m.yml");
        let json = dir.join("m.json");
        assert_eq!(load(&yml).unwrap(), Definition::default());
        std::fs::write(
            &json,
            r#"{"fileFormatVersion": "1", "title": "T", "parameterSets": {"a": {"w": "3"}}}"#,
        )
        .unwrap();
        let d = load(&yml).unwrap();
        assert_eq!(d.legacy.as_deref(), Some(json.as_path()));
        assert_eq!(source(&yml).as_deref(), Some(json.as_path()));
        // The first save writes the .yml file with all of the definition.
        write_sets(&yml, &d).unwrap();
        let back = load(&yml).unwrap();
        assert_eq!(back.legacy, None);
        assert_eq!(back.meta.title.as_deref(), Some("T"));
        assert_eq!(back.sets, d.sets);
        assert!(json.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn writes_the_openscad_format() {
        let d = parse(FILE, false).unwrap();
        let j = openscad_json(&d);
        assert_eq!(j["fileFormatVersion"], "1");
        assert_eq!(
            j["parameterSets"]["Big"],
            json!({ "w": "42", "on": "true", "label": "5", "v": "[1, 2.5]" })
        );
        assert_eq!(j["parameterSets"]["plain"], json!({}));
        // It reads back as the same sets.
        let back = parse(&j.to_string(), true).unwrap();
        assert_eq!(back.sets.len(), 2);
        assert_eq!(back.set("Big").unwrap().values[0], ("w".into(), json!(42)));
    }

    #[test]
    fn schema_matches_the_version() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("extras/website/public/schema")
            .join(SCHEMA_URL.rsplit('/').next().unwrap());
        let schema: Json = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(schema["$id"], SCHEMA_URL);
        assert_eq!(schema["properties"]["cocad"]["const"], VERSION);
        // The schema and the reader know the same root keys.
        let mut keys: Vec<&str> = (schema["properties"].as_object().unwrap().keys())
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        let mut known: Vec<&str> = META_KEYS.to_vec();
        known.extend(["cocad", "part_parameter", "multicolor", "parts", "sets"]);
        known.sort_unstable();
        assert_eq!(keys, known);
    }

    #[test]
    fn parts_go_to_the_part_parameter() {
        let src = "part = \"all\"; // [all, box, lid]\nsize = 1;\n";
        let mut p = crate::params::parse(src);
        let d = parse(
            "cocad: 1\nparts:\n  - {id: box, name: Box, color: red}\n  - {id: lid, color: nope}\n",
            false,
        )
        .unwrap();
        assert_eq!(d.apply_parts(&mut p), ["part `lid`: unknown color `nope`"]);
        let (param, list) = crate::part_param::selector(&p).unwrap();
        assert_eq!(param.name, "part");
        assert!(!list.multicolor);
        assert_eq!(list.choices[0].label, "Box");
        assert_eq!(list.choices[0].color, Some([0xff, 0, 0]));
        assert_eq!(list.choices[1].label, "lid");
        assert_eq!(
            param.widget,
            crate::params::Widget::Dropdown(vec![
                (Value::Str("all".into()), "All".into()),
                (Value::Str("box".into()), "Box".into()),
                (Value::Str("lid".into()), "lid".into()),
            ])
        );

        // Another parameter name, and multicolor.
        let mut p = crate::params::parse("color_part = \"all\";\n");
        let d = parse(
            "cocad: 1\npart_parameter: color_part\nmulticolor: true\nparts: [a, b]\n",
            false,
        )
        .unwrap();
        assert!(d.apply_parts(&mut p).is_empty());
        assert_eq!(crate::parts::names(&p), ["a", "b"]);

        // No such parameter.
        let mut p = crate::params::parse("size = 1;\n");
        assert_eq!(d.apply_parts(&mut p).len(), 1);
        assert!(crate::part_param::selector(&p).is_none());
    }
}
