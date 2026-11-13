//! The state of each opened model, in `<config>/models.json`: the settings
//! of the right panel and the last parameter values. The key is the model
//! path. The file keeps the `MAX` models that were used last.
//!
//! ```json
//! { "/home/me/box.scad": {
//!     "mode": "optimize", "auto_render": true,
//!     "slice": { "layer_height": 0.2, ... }, "ghost": false,
//!     "photo": { "color": [255, 147, 0], "finish": "satin", ... },
//!     "parts": { "colors": true, "explode": false, "axis": "z", "custom": { "Lid": [1, 2, 3] } },
//!     "set": "big", "edited": true, "params": { "width": "80" },
//!     "closed_groups": ["Advanced"],
//!     "optimize": { "area": [220, 220, 250], "fixes": { ... }, ... },
//!     "area_preset": "Creality K1C" } }
//! ```
//!
//! "optimize" is the state of the Optimize tab (`pipeline::State::to_json`).
//! A model without it starts with the default print area of the settings,
//! custom orientation off and all fixes off. "slice" also holds the limits
//! of the check: "overhang_deg" and "bridge_mm".
//!
//! Before 0.7 the "mode" values were "slice" and "photo". They read as
//! "optimize" and "render".

use super::view3d::Axis;
use super::{App, Mode};
use crate::pathtrace::Studio;
use crate::{params, presets};
use serde_json::{Map, Value as Json, json};
use std::path::{Path, PathBuf};

const MAX: usize = 100;

fn file() -> Option<PathBuf> {
    Some(crate::openscad::config_dir()?.join("models.json"))
}

fn key(scad: &Path) -> String {
    scad.canonicalize()
        .unwrap_or_else(|_| scad.to_path_buf())
        .display()
        .to_string()
}

fn read_all() -> Map<String, Json> {
    file()
        .and_then(|f| std::fs::read_to_string(f).ok())
        .and_then(|t| serde_json::from_str::<Json>(&t).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

/// The stored state of `scad`.
pub(super) fn load(scad: &Path) -> Option<Map<String, Json>> {
    read_all().get(&key(scad))?.as_object().cloned()
}

/// `all` with `state` for `key` as the newest entry (the last), cut to `MAX`.
fn merged(mut all: Map<String, Json>, key: String, state: Map<String, Json>) -> Map<String, Json> {
    all.shift_remove(&key);
    all.insert(key, Json::Object(state));
    let extra = all.len().saturating_sub(MAX);
    all.into_iter().skip(extra).collect()
}

fn store(scad: &Path, state: Map<String, Json>) {
    let Some(file) = file() else { return };
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let all = merged(read_all(), key(scad), state);
    if let Ok(text) = serde_json::to_string_pretty(&Json::Object(all)) {
        let _ = std::fs::write(file, text + "\n");
    }
}

fn axis_name(a: Axis) -> &'static str {
    match a {
        Axis::All => "all",
        Axis::X => "x",
        Axis::Y => "y",
        Axis::Z => "z",
    }
}

fn f32_of(o: &Json, k: &str) -> Option<f32> {
    o.get(k)?.as_f64().map(|v| v as f32)
}

fn u32_of(o: &Json, k: &str) -> Option<u32> {
    o.get(k)?.as_u64().and_then(|v| u32::try_from(v).ok())
}

fn bool_of(o: &Json, k: &str) -> Option<bool> {
    o.get(k)?.as_bool()
}

/// The names of the closed parameter groups.
fn closed_groups(v: &Json) -> std::collections::HashSet<String> {
    (v.as_array().into_iter().flatten())
        .filter_map(|g| Some(g.as_str()?.to_string()))
        .collect()
}

fn rgb(v: &Json) -> Option<[u8; 3]> {
    let a = v.as_array()?;
    let c = |i: usize| a.get(i)?.as_u64().and_then(|x| u8::try_from(x).ok());
    Some([c(0)?, c(1)?, c(2)?])
}

impl App {
    /// Writes the state of the open model. Without a model it does nothing.
    pub(super) fn save_state(&self) {
        let Some(path) = &self.file else { return };
        let s = &self.settings;
        let p = &self.photo;
        let v = &self.view3d;
        let custom: Map<String, Json> = v
            .custom
            .iter()
            .map(|(k, c)| (k.clone(), json!(c)))
            .collect();
        let params: Map<String, Json> = self
            .params
            .iter()
            .map(|p| (p.name.clone(), json!(presets::to_text(&p.value))))
            .collect();
        let mut closed: Vec<&String> = self.collapsed.iter().collect();
        closed.sort();
        let state = json!({
            "mode": self.mode.name(),
            "auto_render": self.auto_render,
            "slice": {
                "layer_height": s.layer_height,
                "first_layer": s.first_layer,
                "line_width": s.line_width,
                "walls": s.walls,
                "infill": s.infill,
                "solid_layers": s.solid_layers,
                "speed": s.speed,
                "overhang_deg": self.opt.limits.overhang_deg,
                "bridge_mm": self.opt.limits.bridge_mm,
            },
            "ghost": self.ghost,
            "photo": {
                "color": p.color,
                "finish": p.finish,
                "layer_lines": p.layer_lines,
                "studio": if p.studio == Studio::Light { "light" } else { "dark" },
                "samples": p.samples,
                "width": p.width,
                "height": p.height,
                "exposure": p.exposure,
            },
            "parts": {
                "colors": v.part_colors,
                "explode": v.explode,
                "axis": axis_name(v.explode_axis),
                "explode_mm": v.explode_mm,
                "custom": custom,
            },
            "set": self.active_set,
            "edited": self.set_edited,
            "params": params,
            "closed_groups": closed,
            "optimize": self.opt.state.to_json(),
            "area_preset": self.opt.area_preset,
        });
        if let Json::Object(state) = state {
            store(path, state);
        }
    }

    /// Applies the panel settings of `st`. Values that are missing or out
    /// of range stay as they are.
    pub(super) fn restore_panels(&mut self, st: &Map<String, Json>) {
        let st = Json::Object(st.clone());
        if let Some(m) = st.get("mode").and_then(|m| m.as_str()) {
            self.mode = Mode::from_name(m).unwrap_or(Mode::Model);
        }
        if let Some(b) = bool_of(&st, "auto_render") {
            self.auto_render = b;
        }
        if let Some(b) = bool_of(&st, "ghost") {
            self.ghost = b;
        }
        if let Some(list) = st.get("closed_groups") {
            self.collapsed = closed_groups(list);
        }
        if let Some(o) = st.get("slice") {
            let s = &mut self.settings;
            let mm = |k, lo, hi| f32_of(o, k).filter(|v| (lo..=hi).contains(v));
            if let Some(v) = mm("layer_height", 0.05, 0.6) {
                s.layer_height = v;
            }
            if let Some(v) = mm("first_layer", 0.05, 0.6) {
                s.first_layer = v;
            }
            if let Some(v) = mm("line_width", 0.2, 1.2) {
                s.line_width = v;
            }
            if let Some(v) = mm("infill", 0.0, 1.0) {
                s.infill = v;
            }
            if let Some(v) = mm("speed", 10.0, 300.0) {
                s.speed = v;
            }
            if let Some(v) = u32_of(o, "walls").filter(|v| *v <= 6) {
                s.walls = v;
            }
            if let Some(v) = u32_of(o, "solid_layers").filter(|v| *v <= 10) {
                s.solid_layers = v;
            }
            let l = &mut self.opt.limits;
            if let Some(v) = mm("overhang_deg", 20.0, 80.0) {
                l.overhang_deg = v;
            }
            if let Some(v) = mm("bridge_mm", 1.0, 100.0) {
                l.bridge_mm = v;
            }
        }
        if let Some(o) = st.get("optimize") {
            self.opt.state = crate::optimize::pipeline::State::from_json(o, self.default_area());
            self.opt.restored();
        }
        if let Some(p) = st.get("area_preset").and_then(|p| p.as_str()) {
            self.opt.area_preset = p.to_string();
        }
        if let Some(o) = st.get("photo") {
            let p = &mut self.photo;
            if let Some(c) = o.get("color").and_then(rgb) {
                p.color = c;
            }
            if let Some(f) = o.get("finish").and_then(|f| f.as_str()) {
                p.finish = match f {
                    "matte" => "matte",
                    "gloss" => "gloss",
                    _ => "satin",
                };
            }
            if let Some(b) = bool_of(o, "layer_lines") {
                p.layer_lines = b;
            }
            if let Some(s) = o.get("studio").and_then(|s| s.as_str()) {
                p.studio = if s == "light" {
                    Studio::Light
                } else {
                    Studio::Dark
                };
            }
            if let Some(v) = u32_of(o, "samples").filter(|v| (16..=2048).contains(v)) {
                p.samples = v;
            }
            let px = |k| u32_of(o, k).filter(|v| (64..=8192).contains(v));
            if let (Some(w), Some(h)) = (px("width"), px("height")) {
                p.width = w as usize;
                p.height = h as usize;
            }
            if let Some(v) = f32_of(o, "exposure").filter(|v| (0.3..=3.0).contains(v)) {
                p.exposure = v;
            }
        }
        if let Some(o) = st.get("parts") {
            let v = &mut self.view3d;
            if let Some(b) = bool_of(o, "colors") {
                v.part_colors = b;
            }
            if let Some(b) = bool_of(o, "explode") {
                v.explode = b;
            }
            if let Some(a) = o.get("axis").and_then(|a| a.as_str()) {
                v.explode_axis = Axis::LIST
                    .into_iter()
                    .find(|x| axis_name(*x) == a)
                    .unwrap_or_default();
            }
            v.explode_mm = f32_of(o, "explode_mm").filter(|d| *d >= 0.0);
            if let Some(c) = o.get("custom").and_then(|c| c.as_object()) {
                v.custom = c
                    .iter()
                    .filter_map(|(k, c)| Some((k.clone(), rgb(c)?)))
                    .collect();
            }
        }
    }

    /// Applies the last parameter values of `st`, when they belong to the
    /// set that is active now: with Model defaults always, with a set only
    /// when it had edits. Returns true when a value changed.
    pub(super) fn restore_params(&mut self, st: &Map<String, Json>) -> bool {
        let set = st.get("set").and_then(|s| s.as_str());
        if set != self.active_set.as_deref() {
            return false;
        }
        let edited = st.get("edited").and_then(|e| e.as_bool()) == Some(true);
        if set.is_some() && !edited {
            return false;
        }
        let Some(values) = st.get("params").and_then(|p| p.as_object()) else {
            return false;
        };
        let mut changed = false;
        for p in &mut self.params {
            let Some(text) = values.get(&p.name).and_then(|t| t.as_str()) else {
                continue;
            };
            let before = p.value.clone();
            if params::set_from_text(p, text) && p.value != before {
                changed = true;
            }
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_last_capped() {
        let mut all = Map::new();
        for n in 0..MAX + 5 {
            all = merged(all, format!("/m/{n}.scad"), Map::new());
        }
        assert_eq!(all.len(), MAX);
        assert!(!all.contains_key("/m/0.scad"));
        all = merged(all, "/m/10.scad".into(), Map::new());
        assert_eq!(all.len(), MAX);
        assert_eq!(all.keys().next_back().unwrap(), "/m/10.scad");
    }

    #[test]
    fn closed_groups_read_strings_only() {
        let v = json!(["Advanced", 3, "Size", null]);
        let mut g: Vec<String> = closed_groups(&v).into_iter().collect();
        g.sort();
        assert_eq!(g, ["Advanced", "Size"]);
        assert!(closed_groups(&json!("Advanced")).is_empty());
    }

    #[test]
    fn old_mode_names_read_as_the_new_tabs() {
        assert_eq!(Mode::from_name("slice"), Some(Mode::Optimize));
        assert_eq!(Mode::from_name("photo"), Some(Mode::Render));
        for m in Mode::ALL {
            assert_eq!(Mode::from_name(m.name()), Some(m));
        }
        assert_eq!(Mode::Optimize.name(), "optimize");
        assert_eq!(Mode::Render.name(), "render");
        assert_eq!(Mode::from_name("other"), None);
    }
}
