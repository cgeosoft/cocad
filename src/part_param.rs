//! The part parameter. Many OpenSCAD models select a part with a string
//! parameter, and the convention works in plain OpenSCAD too:
//!
//! ```openscad
//! part = "all"; // [all, spine, board_left]
//! ```
//!
//! `all` shows the assembled model. Each other value shows only that part,
//! in its place in use. CoCAD builds each value on its own with
//! `-D part="value"`.
//!
//! The part list comes from the `parts` of the model definition
//! (`definition.rs`), with the part names and colors. Without it, CoCAD
//! uses the dropdown list of the `.scad` file: the labels of a `part` list
//! are the names (`mark_legacy`), and a `cocad_part` list is a multicolor
//! list.
//!
//! The parameter carries the list in `Param::parts`, so each copy of the
//! parameters has it. Each solid of a value is a print part of its own.
//! The parts of a multicolor list are the colors of one object (`parts.rs`).

use crate::openscad::{self, Config};
use crate::params::{self, Param, Value, Widget};
use crate::parts::Part;
use anyhow::{Result, bail};
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// The parameter of a part list in the `.scad` file without a definition.
const PARAM: &str = "part";
/// The parameter of a multicolor list in the `.scad` file without a
/// definition. Models before CoCAD 0.8 use it.
const LEGACY_COLORS: &str = "cocad_part";
/// The value that shows all parts.
pub const ALL: &str = "all";

/// The parts that the part parameter selects.
#[derive(Clone, Debug, PartialEq)]
pub struct PartList {
    /// The parts are the colors of one object, not separate print parts.
    pub multicolor: bool,
    /// The values other than `all`, in list order.
    pub choices: Vec<Choice>,
}

/// One value of the list other than `all`.
#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    pub value: String,
    /// The name of the part.
    pub label: String,
    /// The color of the part from the definition.
    pub color: Option<[u8; 3]>,
}

/// The part parameter and its list.
pub fn selector(params: &[Param]) -> Option<(&Param, &PartList)> {
    params.iter().find_map(|p| Some((p, p.parts.as_ref()?)))
}

/// Makes `params[i]` the part parameter with `list`. Its dropdown list
/// then shows the part names. No other parameter stays a part parameter.
pub fn mark(params: &mut [Param], i: usize, list: PartList) {
    for p in params.iter_mut() {
        p.parts = None;
    }
    let p = &mut params[i];
    let mut options = vec![(Value::Str(ALL.into()), "All".to_string())];
    options.extend((list.choices.iter()).map(|c| (Value::Str(c.value.clone()), c.label.clone())));
    p.widget = Widget::Dropdown(options);
    p.parts = Some(list);
}

/// Finds the part list in the dropdown lists of the `.scad` file, for a
/// model without parts in its definition:
///
/// - A `cocad_part` list with string values: a multicolor list. The values
///   are the names.
/// - Else a `part` list with string values, `all` and two or more other
///   values. The labels are the names.
pub fn mark_legacy(params: &mut [Param]) {
    let strings = |p: &Param| -> Option<Vec<(String, String)>> {
        let Widget::Dropdown(options) = &p.widget else {
            return None;
        };
        (options.iter())
            .map(|(v, label)| match v {
                Value::Str(s) => Some((s.clone(), label.trim().to_string())),
                _ => None,
            })
            .collect()
    };
    let choices = |options: &[(String, String)], labels: bool| -> Vec<Choice> {
        (options.iter())
            .filter(|(v, _)| v != ALL)
            .map(|(v, l)| Choice {
                value: v.clone(),
                label: if labels && !l.is_empty() {
                    l.clone()
                } else {
                    v.clone()
                },
                color: None,
            })
            .collect()
    };
    if let Some(i) = params.iter().position(|p| p.name == LEGACY_COLORS)
        && let Some(options) = strings(&params[i])
    {
        let choices = choices(&options, false);
        if !choices.is_empty() {
            params[i].parts = Some(PartList {
                multicolor: true,
                choices,
            });
            return;
        }
    }
    if let Some(i) = params.iter().position(|p| p.name == PARAM)
        && let Some(options) = strings(&params[i])
        && options.iter().any(|(v, _)| v == ALL)
    {
        let choices = choices(&options, true);
        if choices.len() >= 2 {
            params[i].parts = Some(PartList {
                multicolor: false,
                choices,
            });
        }
    }
}

/// The part parameter of a list of separate print parts.
fn separate(params: &[Param]) -> Option<(&Param, &PartList)> {
    selector(params).filter(|(_, l)| !l.multicolor)
}

/// The values of the list other than `all`, in list order. Empty unless
/// the model has a list of separate print parts.
pub fn choices(params: &[Param]) -> Vec<Choice> {
    separate(params)
        .map(|(_, l)| l.choices.clone())
        .unwrap_or_default()
}

/// The current value of the part parameter, or `None` for a model without
/// a list of separate print parts.
fn selected(params: &[Param]) -> Option<String> {
    match &separate(params)?.0.value {
        Value::Str(s) => Some(s.clone()),
        _ => None,
    }
}

/// True when the model shows all parts, so each value must build on its own.
pub fn needs_render(params: &[Param]) -> bool {
    selected(params).is_some_and(|s| s == ALL)
}

/// The label of the one value that the model shows. `None` when it shows
/// all parts, or has no selector.
pub fn shown_label(params: &[Param]) -> Option<String> {
    let value = selected(params).filter(|s| s != ALL)?;
    let c = choices(params).into_iter().find(|c| c.value == value)?;
    Some(c.label)
}

/// The parts of all values.
pub struct Built {
    /// The parts, each one with the label of its value as its name.
    pub parts: Vec<Part>,
    /// The exact build of each value, for the "Other" test. A part that
    /// comes from the module split of a value builds from the CSG tree,
    /// which has rounded numbers.
    pub cover: Vec<Part>,
}

/// One value: its build and the module split of its CSG tree.
struct Run {
    label: String,
    /// `None`: the value gives no 3D geometry.
    mesh: Option<crate::mesh::Mesh>,
    /// The .csg text of each piece of the module split, when the pieces
    /// can give more solids than `mesh` has.
    pieces: Vec<String>,
}

/// Builds each value of the list on its own, in parallel. A value whose
/// solids touch, for example four segments of one board, gives one solid.
/// So CoCAD also applies the module split to the CSG tree of each value.
/// When the pieces give more solids than the build of the value, the
/// pieces are the parts, else the build is. Each part has the label as its
/// name. A value without 3D geometry gives no part. An error of one value
/// fails the job, and so does a list where no value gives geometry.
pub fn render(
    cfg: &Config,
    file: &Path,
    params: &[Param],
    cancel: &Arc<AtomicBool>,
) -> Result<Built> {
    let Some((param, list)) = separate(params) else {
        bail!("the model has no part parameter");
    };
    let (name, choices) = (param.name.as_str(), &list.choices);
    let prefix = format!("{name}=");
    let mut defines = params::defines(params);
    defines.retain(|d| !d.starts_with(&prefix));

    let next = AtomicUsize::new(0);
    let failed = AtomicBool::new(false);
    let results: Mutex<Vec<(usize, Result<Run>)>> = Mutex::new(Vec::new());
    let workers = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(choices.len());
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    if failed.load(Ordering::Relaxed) || cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(c) = choices.get(i) else {
                        break;
                    };
                    let r = run(cfg, file, &defines, name, c, cancel);
                    if r.is_err() {
                        failed.store(true, Ordering::Relaxed);
                    }
                    results.lock().unwrap().push((i, r));
                }
            });
        }
    });
    if cancel.load(Ordering::Relaxed) {
        bail!("cancelled");
    }
    let mut results = results.into_inner().unwrap();
    results.sort_by_key(|(i, _)| *i);
    let mut runs = Vec::with_capacity(results.len());
    for (_, r) in results {
        runs.push(r?);
    }
    if runs.len() < choices.len() {
        bail!("a part build stopped");
    }

    // All pieces of all values build in one pool.
    let texts: Vec<String> = runs.iter().flat_map(|r| r.pieces.iter().cloned()).collect();
    let mut meshes = crate::csg_split::build_texts(cfg, &texts, cancel)?.into_iter();
    let mut out = Built {
        parts: Vec::new(),
        cover: Vec::new(),
    };
    for r in runs {
        let pieces: Vec<_> = meshes.by_ref().take(r.pieces.len()).flatten().collect();
        let Some(mesh) = r.mesh else {
            continue;
        };
        let part = |mesh| Part {
            name: r.label.clone(),
            color: [0xff; 3],
            mesh,
        };
        let solids: usize = pieces
            .iter()
            .map(crate::assembly::Assembly::solid_count)
            .sum();
        if pieces.len() >= 2 && solids > crate::assembly::Assembly::solid_count(&mesh) {
            out.parts.extend(pieces.into_iter().map(part));
            out.cover.push(part(mesh));
        } else {
            out.cover.push(part(mesh.clone()));
            out.parts.push(part(mesh));
        }
    }
    if out.parts.is_empty() {
        bail!("no value of the {name} list gives 3D geometry");
    }
    Ok(out)
}

/// Builds one value, and writes the pieces of its module split when they
/// can give more solids than the build.
fn run(
    cfg: &Config,
    file: &Path,
    defines: &[String],
    param: &str,
    c: &Choice,
    cancel: &Arc<AtomicBool>,
) -> Result<Run> {
    let mut defs = defines.to_vec();
    defs.push(format!("{param}={}", Value::Str(c.value.clone()).to_scad()));
    let mesh = match openscad::render(cfg, file, &defs, cancel) {
        Ok(out) => Some(out.mesh),
        Err(e) if crate::csg_split::is_empty_error(&e) => None,
        Err(e) => return Err(e.context(format!("{param} = \"{}\"", c.value))),
    };
    let mut pieces = Vec::new();
    if let Some(m) = &mesh {
        // A failed CSG export only costs the module split: the build stays.
        if let Ok(Some(texts)) = crate::csg_split::split_texts(cfg, file, &defs, cancel)
            && texts.len() > crate::assembly::Assembly::solid_count(m)
        {
            pieces = texts;
        }
    }
    Ok(Run {
        label: c.label.clone(),
        mesh,
        pieces,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "part = \"all\"; // [all:All, spine:Spine, board_left:Left board, pin]\n";

    #[test]
    fn choices_have_labels_and_skip_all() {
        let p = params::parse(SRC);
        let c = choices(&p);
        let pairs: Vec<_> = c
            .iter()
            .map(|c| (c.value.as_str(), c.label.as_str()))
            .collect();
        assert_eq!(
            pairs,
            [
                ("spine", "Spine"),
                ("board_left", "Left board"),
                ("pin", "pin")
            ]
        );
        assert!(needs_render(&p));
        assert_eq!(shown_label(&p), None);
    }

    #[test]
    fn one_value_shows_its_label() {
        let p = params::parse(&SRC.replace("\"all\";", "\"board_left\";"));
        assert!(!needs_render(&p));
        assert_eq!(shown_label(&p).as_deref(), Some("Left board"));
    }

    #[test]
    fn no_selector_without_all_or_two_parts() {
        for src in [
            "part = \"a\"; // [a, b, c]\n",
            "part = \"all\"; // [all, a]\n",
            "part = \"all\";\n",
            "part = 1; // [1, 2, 3]\n",
            "size = 10; // [1:20]\n",
        ] {
            let p = params::parse(src);
            assert!(choices(&p).is_empty(), "{src}");
            assert!(!needs_render(&p), "{src}");
            assert_eq!(shown_label(&p), None, "{src}");
        }
    }
}
