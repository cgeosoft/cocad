//! The section cut of the 3D view: at most three planes, one across each
//! axis. Each plane removes one side of the view. The Section menu above
//! the view bar at the bottom right of the 3D view sets the cuts. All tabs
//! show it.
//!
//! The Model and the Render tabs cut the model. This cut is in model space,
//! before the explode offset, so each part keeps the same cut when it moves.
//! The Optimize tab cuts the plate, in the coordinates of the bed. The two
//! spaces share the cuts and the sides to keep. Each space has its own
//! positions.
//!
//! The 3D view, the layers of the Optimize tab, the photo and the MCP
//! images show the cut. The photo and the MCP images show the cut of the
//! model. The grid, fit and all exports ignore it.

use super::{App, Mode};
use crate::mesh::Bounds;
use crate::pathtrace;
use crate::render;
use crate::ui;
use eframe::egui::{self, Color32, RichText, Vec2};
use glam::{Vec3, Vec4};
use serde_json::{Map, Value as Json, json};
use std::sync::Arc;

/// The names of the axes in the bar.
const LABELS: [&str; 3] = ["X", "Y", "Z"];
/// The names of the axes in the MCP tools.
const NAMES: [&str; 3] = ["x", "y", "z"];
/// The colors of the axis names in the bar. X and Y have the colors of the
/// axis lines of the grid.
const AXIS_COLORS: [Color32; 3] = [
    Color32::from_rgb(0xe0, 0x55, 0x55),
    Color32::from_rgb(0x5c, 0xcc, 0x6a),
    Color32::from_rgb(0x5a, 0x9c, 0xff),
];

/// The coordinates of the cut positions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Space {
    /// The model, before the explode offset: the Model and the Render tabs.
    Model,
    /// The bed: the plate of the Optimize tab.
    Plate,
}

impl Space {
    /// The space of the cut in tab `mode`.
    pub fn of(mode: Mode) -> Space {
        if mode == Mode::Optimize {
            Space::Plate
        } else {
            Space::Model
        }
    }

    fn index(self) -> usize {
        match self {
            Space::Model => 0,
            Space::Plate => 1,
        }
    }
}

/// One cut, across one axis. Its positions are in `Section`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Cut {
    pub on: bool,
    /// Keep the side below the position (−). `false` keeps the side above
    /// the position (+).
    pub keep_below: bool,
}

/// The cuts on X, Y and Z. A point stays when all active cuts keep it.
#[derive(Clone, Debug, Default)]
pub(super) struct Section {
    pub cuts: [Cut; 3],
    /// The position of each cut in mm, in each space. `None` is the center
    /// of the bounds of that space.
    at: [[Option<f32>; 3]; 2],
    /// The bounds of each space: all parts of the model without offset, and
    /// the parts on the plate.
    bounds: [Option<Bounds>; 2],
    /// The axis of the position that the user changes. The view then shows
    /// where the plane is.
    shown: Option<usize>,
    /// The Section menu above the view bar is open.
    pub open: bool,
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
    /// Sets the bounds of a space: the parts of a new assembly, or the parts
    /// on the plate. The cuts and their positions stay.
    pub fn set_bounds(&mut self, space: Space, b: Option<Bounds>) {
        self.bounds[space.index()] = b;
    }

    /// The bounds of a space.
    pub fn bounds(&self, space: Space) -> Option<Bounds> {
        self.bounds[space.index()]
    }

    /// Another model: the positions go back to the center.
    pub fn new_model(&mut self) {
        self.at = Default::default();
    }

    /// All cuts off, at the center, keeping the + side.
    pub fn reset(&mut self) {
        self.cuts = [Cut::default(); 3];
        self.at = Default::default();
    }

    /// A cut is on.
    pub fn any(&self) -> bool {
        self.cuts.iter().any(|c| c.on)
    }

    /// A cut is on, keeps the − side or has a position: Reset has something
    /// to do.
    fn changed(&self) -> bool {
        self.cuts.iter().any(|c| *c != Cut::default())
            || self.at.iter().flatten().any(Option::is_some)
    }

    /// The position of the cut on axis `k` in mm, in `space`. `None`
    /// without a position and without bounds.
    pub fn position(&self, space: Space, k: usize) -> Option<f32> {
        let s = space.index();
        (self.at[s][k]).or_else(|| self.bounds[s].map(|b| b.center()[k]))
    }

    /// Sets the position of the cut on axis `k` in `space` (`None`: the
    /// center). A position out of the slider range goes to the end of the
    /// range.
    pub fn set_position(&mut self, space: Space, k: usize, at: Option<f32>) {
        let s = space.index();
        self.at[s][k] = match (at, self.bounds[s]) {
            (Some(a), Some(b)) => {
                let (lo, hi, _) = range(&b, k);
                Some(a.clamp(lo, hi))
            }
            (a, _) => a,
        };
    }

    /// Turns on the cut on axis `k` at `at` mm in `space` (`None`: the
    /// center).
    pub fn set(&mut self, space: Space, k: usize, at: Option<f32>, keep_below: bool) {
        self.set_position(space, k, at);
        self.cuts[k] = Cut {
            on: true,
            keep_below,
        };
    }

    /// The planes of the active cuts, in `space`.
    pub fn planes(&self, space: Space) -> Vec<Vec4> {
        (0..3)
            .filter(|&k| self.cuts[k].on)
            .filter_map(|k| Some(plane(k, self.position(space, k)?, self.cuts[k].keep_below)))
            .collect()
    }

    /// The state for `get_state`, in the form that `set_view` takes. The
    /// positions are in `space`.
    pub fn to_json(&self, space: Space) -> Json {
        let mut out = Map::new();
        for (k, name) in NAMES.iter().enumerate() {
            let c = &self.cuts[k];
            let v = if c.on {
                json!({
                    "at": self.position(space, k),
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
    /// null axis turns its cut off. `at` is in mm in `space` (default: the
    /// center), `keep` is "+" or "-" (default "+"). Nothing changes on an
    /// error.
    pub fn set_json(&mut self, space: Space, v: &Json) -> Result<(), String> {
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
                Some((at, keep_below)) => self.set(space, k, at, keep_below),
                None => self.cuts[k].on = false,
            }
        }
        Ok(())
    }
}

impl App {
    /// The space of the cut in the current tab.
    pub(super) fn section_space(&self) -> Space {
        Space::of(self.mode)
    }

    /// The section cut of the 3D view in this frame. The Optimize tab cuts
    /// the plate. The plate parts have no offset.
    pub(super) fn section_view(&self) -> render::Section {
        let s = &self.view3d.section;
        let space = self.section_space();
        if !s.any() {
            return render::Section::default();
        }
        let mut outlines = Vec::new();
        if let Some(k) = s.shown
            && s.cuts[k].on
            && let (Some(at), Some(all)) = (s.position(space, k), s.bounds(space))
        {
            let styles = match (space, &self.assembly) {
                (Space::Model, Some(_)) => self.part_styles(),
                _ => Vec::new(),
            };
            if styles.iter().all(|st| st.offset == Vec3::ZERO) {
                outlines.push(outline(k, at, &all, Vec3::ZERO));
            } else if let Some(a) = &self.assembly {
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
            planes: s.planes(space),
            outlines,
        }
    }

    /// The section cut of the model for the photo, for parts with these
    /// offsets. `None` without a cut.
    pub(super) fn photo_clip(&self, offsets: &[Vec3]) -> Option<Arc<pathtrace::Clip>> {
        let planes = self.view3d.section.planes(Space::Model);
        if planes.is_empty() {
            return None;
        }
        let parts = self.assembly.as_ref().map_or(&[][..], |a| &a.parts[..]);
        let ranges = (parts.iter().zip(offsets)).map(|(p, o)| (p.tris.end as u32, *o));
        Some(Arc::new(pathtrace::Clip::new(planes, ranges)))
    }

    /// The Section menu: it opens above the view bar `bar` at the bottom
    /// right of the 3D view `rect`. A switch for each axis, and for each cut
    /// that is on, its position and the side to keep. Returns the rectangle
    /// of the menu when it is open. The cuts stay when the menu closes.
    pub(super) fn section_menu(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        bar: egui::Rect,
    ) -> Option<egui::Rect> {
        self.view3d.section.shown = None;
        let space = self.section_space();
        let bounds = self.view3d.section.bounds(space)?;
        if !self.view3d.section.open {
            return None;
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.view3d.section.open = false;
            return None;
        }
        let rows = self.view3d.section.any();
        let m = 16.0;
        // A fixed width: the rows of the cuts do not change it.
        let width = 300.0_f32.min(rect.width() - 2.0 * m).max(0.0);
        let what = if space == Space::Plate {
            "the plate"
        } else {
            "the model"
        };
        let info = if space == Space::Plate {
            "Cuts the plate to show the inside, across the axes of the bed. The cuts stay \
             in all tabs. The plate has its own positions. The export does not use the cut."
        } else {
            "Cuts the model to show the inside. The cuts stay in all tabs. The photo shows \
             the cut. The export does not use it."
        };
        let s = &mut self.view3d.section;
        let r = egui::Area::new(egui::Id::new("section_menu"))
            .order(egui::Order::Middle)
            .pivot(egui::Align2::RIGHT_BOTTOM)
            .fixed_pos(egui::pos2(bar.right(), bar.top() - 8.0))
            .constrain_to(rect)
            .show(ui.ctx(), |ui| {
                egui::Frame::NONE
                    .fill(ui::glass())
                    .corner_radius(10)
                    .inner_margin(egui::Margin::same(4))
                    .show(ui, |ui| {
                        ui.set_width(width - 8.0);
                        ui.spacing_mut().item_spacing = Vec2::new(2.0, 2.0);
                        ui.spacing_mut().button_padding = Vec2::new(10.0, 5.0);
                        ui.horizontal(|ui| {
                            ui.add_space(8.0);
                            ui.label(RichText::new("Section").size(13.5))
                                .on_hover_text(info);
                            ui.add_space(6.0);
                            for (k, axis) in LABELS.into_iter().enumerate() {
                                let on = s.cuts[k].on;
                                if ui
                                    .add(ui::ghost(axis, on))
                                    .on_hover_text(format!("Cut {what} across the {axis} axis"))
                                    .clicked()
                                {
                                    s.cuts[k].on = !on;
                                }
                            }
                            if s.changed() {
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui
                                            .add(ui::ghost("Reset", false))
                                            .on_hover_text(
                                                "Turn off all cuts and move them to the center",
                                            )
                                            .clicked()
                                        {
                                            s.reset();
                                        }
                                    },
                                );
                            }
                        });
                        if rows {
                            for k in 0..3 {
                                if s.cuts[k].on {
                                    cut_row(ui, s, space, &bounds, k);
                                }
                            }
                        }
                    });
            });
        Some(r.response.rect)
    }
}

/// The row of the cut on axis `k`: the axis, a slider, the position in mm
/// and the side to keep.
fn cut_row(ui: &mut egui::Ui, s: &mut Section, space: Space, bounds: &Bounds, k: usize) {
    let axis = LABELS[k];
    let (lo, hi, step) = range(bounds, k);
    let mut at = s.position(space, k).unwrap_or(0.0);
    // The center of a model at the origin can be −0.0 or a rounding error:
    // show 0.
    if at.abs() < (step * 0.5) as f32 {
        at = 0.0;
    }
    let mut active = false;
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.add_sized(
            [14.0, 26.0],
            egui::Label::new(RichText::new(axis).strong().color(AXIS_COLORS[k])),
        );
        ui.add_space(4.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let below = s.cuts[k].keep_below;
            let (side, other) = if below {
                ("\u{2212}", "+")
            } else {
                ("+", "\u{2212}")
            };
            if ui
                .add(ui::ghost(&format!("{side}{axis}"), false).min_size(Vec2::new(42.0, 0.0)))
                .on_hover_text(format!(
                    "The cut keeps the {side}{axis} side. Click to keep the {other}{axis} side."
                ))
                .clicked()
            {
                s.cuts[k].keep_below = !below;
            }
            let d = super::decimals(step);
            let r = ui.add_sized(
                [78.0, 22.0],
                egui::DragValue::new(&mut at)
                    .range(lo..=hi)
                    .speed(step)
                    .max_decimals(d)
                    .min_decimals(d)
                    .suffix(" mm"),
            );
            if r.changed() {
                s.set_position(space, k, Some(at));
            }
            active |= r.dragged() || r.has_focus();
            ui.add_space(4.0);
            ui.spacing_mut().slider_width = ui.available_width().max(40.0);
            let r = ui.add(
                egui::Slider::new(&mut at, lo..=hi)
                    .step_by(step)
                    .show_value(false),
            );
            if r.changed() {
                s.set_position(space, k, Some(at));
            }
            active |= r.dragged() || r.hovered();
        });
    });
    if active {
        s.shown = Some(k);
        ui.ctx().request_repaint();
    }
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

    const M: Space = Space::Model;
    const P: Space = Space::Plate;

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
        s.set_bounds(M, Some(bounds()));
        assert!(s.planes(M).is_empty());
        // X and Y at the center: a quarter stays.
        s.cuts[0].on = true;
        s.cuts[1].on = true;
        let planes = s.planes(M);
        assert_eq!(planes.len(), 2);
        assert!(keeps(&planes, Vec3::new(30.0, 5.0, 10.0)));
        assert!(!keeps(&planes, Vec3::new(10.0, 5.0, 10.0)));
        assert!(!keeps(&planes, Vec3::new(30.0, -5.0, 10.0)));
        assert!(!keeps(&planes, Vec3::new(10.0, -5.0, 10.0)));
        // The other side of X.
        s.cuts[0].keep_below = true;
        let planes = s.planes(M);
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
        assert_eq!(s.position(M, 0), None);
        s.cuts[0].on = true;
        // No bounds yet: no plane.
        assert!(s.planes(M).is_empty());
        s.set_bounds(M, Some(bounds()));
        assert_eq!(s.position(M, 0), Some(20.0));
        assert_eq!(s.position(M, 1), Some(0.0));
        // A new render with other bounds: the center moves.
        s.set_bounds(
            M,
            Some(Bounds {
                min: Vec3::ZERO,
                max: Vec3::splat(10.0),
            }),
        );
        assert_eq!(s.position(M, 0), Some(5.0));
        // The user sets a position: it stays after a new render.
        s.set(M, 0, Some(3.0), true);
        s.set_bounds(M, Some(bounds()));
        assert_eq!(s.position(M, 0), Some(3.0));
        assert_eq!(s.planes(M), vec![plane(0, 3.0, true)]);
        // Out of the range: the end of the range.
        s.set(M, 1, Some(500.0), false);
        let (_, hi, _) = range(&bounds(), 1);
        assert_eq!(s.position(M, 1), Some(hi));
        assert!(hi > 10.0 && hi < 12.0);
        // Another model: back to the center, the cuts stay on.
        s.new_model();
        assert_eq!(s.position(M, 0), Some(20.0));
        assert!(s.cuts[0].on);
        assert!(s.changed());
        s.reset();
        assert!(!s.any());
        assert!(!s.changed());
    }

    #[test]
    fn the_plate_shares_the_cuts_and_has_its_own_positions() {
        let mut s = Section::default();
        s.set_bounds(M, Some(bounds()));
        let bed = Bounds {
            min: Vec3::new(100.0, 80.0, 0.0),
            max: Vec3::new(160.0, 120.0, 30.0),
        };
        s.set_bounds(P, Some(bed));
        // A cut in the Model tab is on in the Optimize tab, at the center of
        // the plate, with the same side.
        s.set(M, 2, Some(4.0), true);
        assert_eq!(s.planes(M), vec![plane(2, 4.0, true)]);
        assert_eq!(s.planes(P), vec![plane(2, 15.0, true)]);
        // A position on the plate does not move the cut of the model.
        s.set_position(P, 2, Some(7.5));
        assert_eq!(s.position(P, 2), Some(7.5));
        assert_eq!(s.position(M, 2), Some(4.0));
        // The side and the switch are the same in both spaces.
        s.cuts[2].keep_below = false;
        assert_eq!(s.planes(P), vec![plane(2, 7.5, false)]);
        assert_eq!(s.planes(M), vec![plane(2, 4.0, false)]);
        s.cuts[2].on = false;
        assert!(s.planes(P).is_empty() && s.planes(M).is_empty());
        // No plate yet: no plane on the plate, the model keeps its cut.
        s.cuts[0].on = true;
        s.set_bounds(P, None);
        assert!(s.planes(P).is_empty());
        assert_eq!(s.planes(M), vec![plane(0, 20.0, false)]);
        // The tabs and their spaces.
        assert_eq!(Space::of(Mode::Optimize), P);
        assert_eq!(Space::of(Mode::Model), M);
        assert_eq!(Space::of(Mode::Render), M);
    }

    #[test]
    fn json_round_trip() {
        let mut s = Section::default();
        s.set_bounds(M, Some(bounds()));
        s.set_json(M, &json!({ "x": { "at": 12.5, "keep": "-" }, "z": {} }))
            .unwrap();
        assert_eq!(
            s.to_json(M),
            json!({
                "x": { "at": 12.5, "keep": "-" },
                "y": null,
                "z": { "at": 10.0, "keep": "+" },
            })
        );
        // The state goes back into set_view.
        let mut t = Section::default();
        t.set_bounds(M, Some(bounds()));
        t.set_json(M, &s.to_json(M)).unwrap();
        assert_eq!(t.planes(M), s.planes(M));
        // Null and missing axes turn their cut off.
        s.set_json(M, &json!({ "x": null })).unwrap();
        assert!(!s.any());
        // Errors change nothing.
        s.set_json(M, &json!({ "y": {} })).unwrap();
        for bad in [
            json!(5),
            json!({ "w": {} }),
            json!({ "x": { "keep": "up" } }),
            json!({ "x": { "at": "middle" } }),
            json!({ "x": 3 }),
        ] {
            assert!(s.set_json(M, &bad).is_err(), "{bad}");
        }
        assert_eq!(s.to_json(M)["y"], json!({ "at": 0.0, "keep": "+" }));
        assert!(s.to_json(M)["x"].is_null());
        // The positions on the plate are apart from the positions in the
        // model.
        s.set_bounds(
            P,
            Some(Bounds {
                min: Vec3::ZERO,
                max: Vec3::splat(50.0),
            }),
        );
        s.set_json(P, &json!({ "y": { "at": 30.0 } })).unwrap();
        assert_eq!(s.to_json(P)["y"], json!({ "at": 30.0, "keep": "+" }));
        assert_eq!(s.to_json(M)["y"], json!({ "at": 0.0, "keep": "+" }));
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
