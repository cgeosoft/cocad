//! The section cut of the 3D view: at most three planes, one across each
//! axis. Each plane removes one side of the model. The cut is in model
//! space, before the explode offset, so each part keeps the same cut when
//! it moves. The "Section" card is in the Model tab.
//!
//! The 3D view, the photo and the MCP images show the cut. The Slice tab,
//! the grid, fit and the export ignore it.

use super::{App, Mode};
use crate::mesh::Bounds;
use crate::pathtrace;
use crate::render;
use crate::ui;
use eframe::egui;
use glam::{Vec3, Vec4};
use serde_json::{Map, Value as Json, json};
use std::sync::Arc;

/// The names of the axes in the card.
const LABELS: [&str; 3] = ["X", "Y", "Z"];
/// The names of the axes in the MCP tools.
const NAMES: [&str; 3] = ["x", "y", "z"];

/// One cut, across one axis.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Cut {
    pub on: bool,
    /// The position on the axis in mm. `None` is the center of the model.
    pub at: Option<f32>,
    /// Keep the side below the position (−). `false` keeps the side above
    /// the position (+).
    pub keep_below: bool,
}

/// The cuts on X, Y and Z. A point stays when all active cuts keep it.
#[derive(Clone, Debug, Default)]
pub(super) struct Section {
    pub cuts: [Cut; 3],
    /// The bounds of all parts without offset. A position of `None` uses
    /// their center.
    bounds: Option<Bounds>,
    /// The axis of the position that the user changes in this frame. The
    /// view then shows where the plane is.
    shown: Option<usize>,
}

/// The plane of a cut across axis `k` at `at` mm. A point p stays when
/// `plane.truncate().dot(p) + plane.w >= 0.0`.
pub(super) fn plane(k: usize, at: f32, keep_below: bool) -> Vec4 {
    let mut n = Vec3::ZERO;
    n[k] = 1.0;
    if keep_below {
        (-n).extend(at)
    } else {
        n.extend(-at)
    }
}

/// The slider of axis `k` for these bounds: (min, max, step) in mm. The
/// range is the bounds with a small margin, on multiples of the step.
fn range(b: &Bounds, k: usize) -> (f32, f32, f64) {
    let size = b.size().max_element();
    let size = if size.is_finite() && size > 0.0 {
        size as f64
    } else {
        100.0
    };
    let step = 10f64.powi(size.log10().floor() as i32 - 2);
    let margin = (0.02 * size).max(step);
    let lo = ((b.min[k] as f64 - margin) / step).floor() * step;
    let hi = ((b.max[k] as f64 + margin) / step).ceil() * step;
    (lo as f32, hi as f32, step)
}

/// The rectangle where plane `k` at `at` cuts the box, with a margin.
fn outline(k: usize, at: f32, b: &Bounds, offset: Vec3) -> [Vec3; 4] {
    let (u, v) = ((k + 1) % 3, (k + 2) % 3);
    let m = b.size().max_element() * 0.03;
    let corner = |su: f32, sv: f32| {
        let mut p = Vec3::ZERO;
        p[k] = at;
        p[u] = su;
        p[v] = sv;
        p + offset
    };
    let (u0, u1) = (b.min[u] - m, b.max[u] + m);
    let (v0, v1) = (b.min[v] - m, b.max[v] + m);
    [
        corner(u0, v0),
        corner(u1, v0),
        corner(u1, v1),
        corner(u0, v1),
    ]
}

impl Section {
    /// Sets the bounds of the parts of a new assembly. The cuts and their
    /// positions stay.
    pub fn set_bounds(&mut self, b: Option<Bounds>) {
        self.bounds = b;
    }

    /// Another model: the positions go back to the center.
    pub fn new_model(&mut self) {
        for c in &mut self.cuts {
            c.at = None;
        }
    }

    /// All cuts off, at the center, keeping the + side.
    pub fn reset(&mut self) {
        self.cuts = [Cut::default(); 3];
    }

    /// A cut is on.
    pub fn any(&self) -> bool {
        self.cuts.iter().any(|c| c.on)
    }

    /// The position of the cut on axis `k` in mm. `None` without a
    /// position and without bounds.
    pub fn position(&self, k: usize) -> Option<f32> {
        (self.cuts[k].at).or_else(|| self.bounds.map(|b| b.center()[k]))
    }

    /// Turns on the cut on axis `k` at `at` mm (`None`: the center). A
    /// position out of the slider range goes to the end of the range.
    pub fn set(&mut self, k: usize, at: Option<f32>, keep_below: bool) {
        let at = match (at, self.bounds) {
            (Some(a), Some(b)) => {
                let (lo, hi, _) = range(&b, k);
                Some(a.clamp(lo, hi))
            }
            (a, _) => a,
        };
        self.cuts[k] = Cut {
            on: true,
            at,
            keep_below,
        };
    }

    /// The planes of the active cuts, in model space.
    pub fn planes(&self) -> Vec<Vec4> {
        (0..3)
            .filter(|&k| self.cuts[k].on)
            .filter_map(|k| Some(plane(k, self.position(k)?, self.cuts[k].keep_below)))
            .collect()
    }

    /// The state for `get_state`, in the form that `set_view` takes.
    pub fn to_json(&self) -> Json {
        let mut out = Map::new();
        for (k, name) in NAMES.iter().enumerate() {
            let c = &self.cuts[k];
            let v = if c.on {
                json!({
                    "at": self.position(k),
                    "keep": if c.keep_below { "-" } else { "+" },
                })
            } else {
                Json::Null
            };
            out.insert(name.to_string(), v);
        }
        Json::Object(out)
    }

    /// Sets all cuts from the `section` object of `set_view`. A missing or
    /// null axis turns its cut off. `at` is in mm (default: the center of
    /// the model), `keep` is "+" or "-" (default "+"). Nothing changes on an
    /// error.
    pub fn set_json(&mut self, v: &Json) -> Result<(), String> {
        let Some(obj) = v.as_object() else {
            return Err(format!(
                "section must be an object such as {{\"x\": {{\"at\": 10, \"keep\": \"+\"}}}}, got {v}"
            ));
        };
        let mut cuts: [Option<(Option<f32>, bool)>; 3] = [None; 3];
        for (name, cut) in obj {
            let k = (NAMES.iter())
                .position(|n| n.eq_ignore_ascii_case(name))
                .ok_or_else(|| format!("Unknown section axis {name}. Use x, y or z."))?;
            if cut.is_null() {
                continue;
            }
            let Some(cut) = cut.as_object() else {
                return Err(format!(
                    "section.{name} must be an object with at and keep, or null, got {cut}"
                ));
            };
            let at = match cut.get("at") {
                None | Some(Json::Null) => None,
                Some(a) => match a.as_f64() {
                    Some(mm) if mm.is_finite() => Some(mm as f32),
                    _ => return Err(format!("section.{name}.at must be a number in mm, got {a}")),
                },
            };
            let keep_below = match cut.get("keep") {
                None | Some(Json::Null) => false,
                Some(s) => match s.as_str() {
                    Some("+") => false,
                    Some("-" | "\u{2212}") => true,
                    _ => {
                        return Err(format!(
                            "section.{name}.keep must be \"+\" or \"-\", got {s}"
                        ));
                    }
                },
            };
            cuts[k] = Some((at, keep_below));
        }
        for (k, c) in cuts.into_iter().enumerate() {
            match c {
                Some((at, keep_below)) => self.set(k, at, keep_below),
                None => self.cuts[k].on = false,
            }
        }
        Ok(())
    }
}

impl App {
    /// The section cut of the 3D view in this frame. The Slice tab shows
    /// the real mesh, without a cut.
    pub(super) fn section_view(&self) -> render::Section {
        let s = &self.view3d.section;
        if self.mode == Mode::Slice || !s.any() {
            return render::Section::default();
        }
        let mut outlines = Vec::new();
        // The card is in the Model tab only.
        if let (Some(k), Some(a)) = (s.shown, &self.assembly)
            && self.mode == Mode::Model
            && s.cuts[k].on
            && let (Some(at), Some(all)) = (s.position(k), s.bounds)
        {
            let styles = self.part_styles();
            if styles.iter().all(|st| st.offset == Vec3::ZERO) {
                outlines.push(outline(k, at, &all, Vec3::ZERO));
            } else {
                // Each part moves with its cut: one rectangle for each part
                // that the plane cuts.
                for (p, st) in a.parts.iter().zip(&styles) {
                    if p.bounds.min[k] <= at && at <= p.bounds.max[k] {
                        outlines.push(outline(k, at, &p.bounds, st.offset));
                    }
                }
            }
        }
        render::Section {
            planes: s.planes(),
            outlines,
        }
    }

    /// The section cut for the photo, for parts with these offsets. `None`
    /// without a cut.
    pub(super) fn photo_clip(&self, offsets: &[Vec3]) -> Option<Arc<pathtrace::Clip>> {
        let planes = self.view3d.section.planes();
        if planes.is_empty() {
            return None;
        }
        let parts = self.assembly.as_ref().map_or(&[][..], |a| &a.parts[..]);
        let ranges = (parts.iter().zip(offsets)).map(|(p, o)| (p.tris.end as u32, *o));
        Some(Arc::new(pathtrace::Clip::new(planes, ranges)))
    }

    /// The "Section" card: cuts across X, Y and Z. The Model tab shows it.
    pub(super) fn section_card(&mut self, ui: &mut egui::Ui) {
        let s = &mut self.view3d.section;
        s.shown = None;
        let Some(bounds) = s.bounds.filter(|_| self.assembly.is_some()) else {
            return;
        };
        ui::card(ui, |ui| {
            ui.horizontal(|ui| {
                ui::section(ui, "Section");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let moved = s.cuts.iter().any(|c| *c != Cut::default());
                    if ui
                        .add_enabled(moved, ui::ghost("Reset", false))
                        .on_hover_text("Turn off all cuts and move them to the center")
                        .clicked()
                    {
                        s.reset();
                    }
                });
            });
            ui.label(ui::dim("Cuts the model to show the inside."));
            ui.add_space(2.0);
            for (k, axis) in LABELS.into_iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(format!("Cut {axis}"));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui::toggle(ui, &mut s.cuts[k].on)
                            .on_hover_text(format!("Cut the model across the {axis} axis"));
                    });
                });
                if !s.cuts[k].on {
                    continue;
                }
                let (lo, hi, step) = range(&bounds, k);
                let mut at = s.position(k).unwrap_or(0.0);
                let (changed, active) = position_row(ui, &mut at, lo..=hi, step);
                if changed {
                    s.cuts[k].at = Some(at);
                }
                if active {
                    s.shown = Some(k);
                }
                let mut below = s.cuts[k].keep_below;
                let plus = format!("Keep +{axis}");
                let minus = format!("Keep \u{2212}{axis}");
                if ui::segmented(ui, &mut below, &[(false, &plus), (true, &minus)]) {
                    s.cuts[k].keep_below = below;
                }
                ui.add_space(4.0);
            }
        });
        ui.add_space(4.0);
    }
}

/// The position of a cut: a number field and a slider. Returns (changed,
/// active): active is true while the user drags or points at the slider.
fn position_row(
    ui: &mut egui::Ui,
    v: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    step: f64,
) -> (bool, bool) {
    let (mut changed, mut active) = (false, false);
    ui.horizontal(|ui| {
        ui.label(ui::dim("Position"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let d = super::decimals(step);
            let r = ui.add(
                egui::DragValue::new(v)
                    .range(range.clone())
                    .speed(step)
                    .max_decimals(d)
                    .min_decimals(d)
                    .suffix(" mm"),
            );
            changed |= r.changed();
            active |= r.dragged() || r.has_focus();
        });
    });
    ui.spacing_mut().slider_width = ui.available_width();
    let r = ui.add(egui::Slider::new(v, range).step_by(step).show_value(false));
    changed |= r.changed();
    active |= r.dragged() || r.hovered();
    ui.add_space(2.0);
    if active {
        ui.ctx().request_repaint();
    }
    (changed, active)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Point `p` passes all planes.
    fn keeps(planes: &[Vec4], p: Vec3) -> bool {
        planes.iter().all(|c| c.truncate().dot(p) + c.w >= 0.0)
    }

    fn bounds() -> Bounds {
        Bounds {
            min: Vec3::new(0.0, -10.0, 0.0),
            max: Vec3::new(40.0, 10.0, 20.0),
        }
    }

    #[test]
    fn plane_keeps_one_side() {
        let plus = plane(0, 12.5, false);
        assert!(keeps(&[plus], Vec3::new(13.0, 0.0, 0.0)));
        assert!(keeps(&[plus], Vec3::new(12.5, 0.0, 0.0)));
        assert!(!keeps(&[plus], Vec3::new(12.0, 0.0, 0.0)));
        let minus = plane(2, -3.0, true);
        assert!(keeps(&[minus], Vec3::new(0.0, 0.0, -4.0)));
        assert!(!keeps(&[minus], Vec3::new(0.0, 0.0, -2.0)));
        // Y: the other components do not count.
        let y = plane(1, 1.0, false);
        assert!(keeps(&[y], Vec3::new(-99.0, 2.0, 99.0)));
        assert!(!keeps(&[y], Vec3::new(99.0, 0.0, -99.0)));
    }

    #[test]
    fn planes_combine() {
        let mut s = Section::default();
        s.set_bounds(Some(bounds()));
        assert!(s.planes().is_empty());
        // X and Y at the center: a quarter stays.
        s.cuts[0].on = true;
        s.cuts[1].on = true;
        let planes = s.planes();
        assert_eq!(planes.len(), 2);
        assert!(keeps(&planes, Vec3::new(30.0, 5.0, 10.0)));
        assert!(!keeps(&planes, Vec3::new(10.0, 5.0, 10.0)));
        assert!(!keeps(&planes, Vec3::new(30.0, -5.0, 10.0)));
        assert!(!keeps(&planes, Vec3::new(10.0, -5.0, 10.0)));
        // The other side of X.
        s.cuts[0].keep_below = true;
        let planes = s.planes();
        assert!(keeps(&planes, Vec3::new(10.0, 5.0, 10.0)));
        assert!(!keeps(&planes, Vec3::new(30.0, 5.0, 10.0)));
    }

    #[test]
    fn cut_is_in_model_space() {
        // A part point and the same point after an explode offset: the
        // plane applies to the point before the offset, so the cut moves
        // with the part.
        let planes = [plane(0, 10.0, false)];
        let p = Vec3::new(8.0, 0.0, 0.0);
        let offset = Vec3::new(5.0, 0.0, 0.0);
        assert!(!keeps(&planes, p));
        assert!(keeps(&planes, p + offset));
        // The renderer and the photo test the point minus the offset.
        assert!(!keeps(&planes, (p + offset) - offset));
    }

    #[test]
    fn position_follows_the_model_until_the_user_sets_one() {
        let mut s = Section::default();
        assert_eq!(s.position(0), None);
        s.cuts[0].on = true;
        // No bounds yet: no plane.
        assert!(s.planes().is_empty());
        s.set_bounds(Some(bounds()));
        assert_eq!(s.position(0), Some(20.0));
        assert_eq!(s.position(1), Some(0.0));
        // A new render with other bounds: the center moves.
        s.set_bounds(Some(Bounds {
            min: Vec3::ZERO,
            max: Vec3::splat(10.0),
        }));
        assert_eq!(s.position(0), Some(5.0));
        // The user sets a position: it stays after a new render.
        s.set(0, Some(3.0), true);
        s.set_bounds(Some(bounds()));
        assert_eq!(s.position(0), Some(3.0));
        assert_eq!(s.planes(), vec![plane(0, 3.0, true)]);
        // Out of the range: the end of the range.
        s.set(1, Some(500.0), false);
        let (_, hi, _) = range(&bounds(), 1);
        assert_eq!(s.position(1), Some(hi));
        assert!(hi > 10.0 && hi < 12.0);
        // Another model: back to the center, the cuts stay on.
        s.new_model();
        assert_eq!(s.position(0), Some(20.0));
        assert!(s.cuts[0].on);
        s.reset();
        assert!(!s.any());
    }

    #[test]
    fn json_round_trip() {
        let mut s = Section::default();
        s.set_bounds(Some(bounds()));
        s.set_json(&json!({ "x": { "at": 12.5, "keep": "-" }, "z": {} }))
            .unwrap();
        assert_eq!(
            s.to_json(),
            json!({
                "x": { "at": 12.5, "keep": "-" },
                "y": null,
                "z": { "at": 10.0, "keep": "+" },
            })
        );
        // The state goes back into set_view.
        let mut t = Section::default();
        t.set_bounds(Some(bounds()));
        t.set_json(&s.to_json()).unwrap();
        assert_eq!(t.planes(), s.planes());
        // Null and missing axes turn their cut off.
        s.set_json(&json!({ "x": null })).unwrap();
        assert!(!s.any());
        // Errors change nothing.
        s.set_json(&json!({ "y": {} })).unwrap();
        for bad in [
            json!(5),
            json!({ "w": {} }),
            json!({ "x": { "keep": "up" } }),
            json!({ "x": { "at": "middle" } }),
            json!({ "x": 3 }),
        ] {
            assert!(s.set_json(&bad).is_err(), "{bad}");
        }
        assert_eq!(s.to_json()["y"], json!({ "at": 0.0, "keep": "+" }));
        assert!(s.to_json()["x"].is_null());
    }

    #[test]
    fn slider_range_has_a_margin() {
        let (lo, hi, step) = range(&bounds(), 0);
        assert!((step - 0.1).abs() < 1e-12);
        assert!(lo < 0.0 && lo > -1.0);
        assert!(hi > 40.0 && hi < 41.0);
        let r = outline(2, 5.0, &bounds(), Vec3::new(0.0, 0.0, 1.0));
        assert!(r.iter().all(|p| p.z == 6.0));
    }
}
