//! Options of the 3D view for the parts of the model: part colors and
//! explode. The style of each part comes from `View3d::styles` every frame.
//! The "Parts" and "Explode" cards are in the Model and Render tabs.

use super::{App, Mode};
use crate::assembly::{PALETTE, PartInfo};
use crate::mesh::{Bounds, Mesh};
use crate::pathtrace;
use crate::render::PartStyle;
use crate::ui;
use eframe::egui::{self, Vec2};
use glam::Vec3;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// The color of a part without a color of its own, when the part colors are off.
const ORANGE: [u8; 3] = [0xff, 0x93, 0x00];

/// Options of the 3D view. They apply to the parts of the current assembly.
#[derive(Default)]
pub(super) struct View3d {
    /// Show each part in its own color.
    pub part_colors: bool,
    /// Colors that the user selected, by part name. A new render keeps the
    /// names ("Part 2", or a name from the `cocad_part` list), so the colors
    /// stay. Opening another model clears them.
    pub custom: HashMap<String, [u8; 3]>,
    /// The part colors window is open.
    pub colors_open: bool,
    /// The last moved mesh for the photo, with its key.
    photo_mesh: Option<(u64, Arc<Mesh>)>,
    /// Move the parts away from each other.
    pub explode: bool,
    /// The explode distance in mm that the user selected. `None` uses the
    /// default distance of the model. Opening another model clears it.
    pub explode_mm: Option<f32>,
    pub explode_axis: Axis,
    /// The selected part and the part in focus (see `select.rs`).
    pub sel: super::select::Selection,
    /// The section cut (see `section.rs`).
    pub section: super::section::Section,
}

/// The direction of the explode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Axis {
    /// Away from the center of the model.
    #[default]
    All,
    X,
    Y,
    Z,
}

impl Axis {
    pub const LIST: [Axis; 4] = [Axis::All, Axis::X, Axis::Y, Axis::Z];

    pub fn label(self) -> &'static str {
        match self {
            Axis::All => "All",
            Axis::X => "X",
            Axis::Y => "Y",
            Axis::Z => "Z",
        }
    }

    /// The name in the MCP tools.
    pub fn name(self) -> &'static str {
        match self {
            Axis::All => "all",
            Axis::X => "x",
            Axis::Y => "y",
            Axis::Z => "z",
        }
    }

    pub fn from_name(name: &str) -> Option<Axis> {
        (Axis::LIST.into_iter()).find(|a| a.name().eq_ignore_ascii_case(name))
    }

    /// The components of a movement that this direction keeps.
    fn mask(self) -> Vec3 {
        match self {
            Axis::All => Vec3::ONE,
            Axis::X => Vec3::X,
            Axis::Y => Vec3::Y,
            Axis::Z => Vec3::Z,
        }
    }
}

/// The range of the explode distance for one model, in mm.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Range {
    /// The end of the slider: about 2 times the largest size of the model.
    pub max: f32,
    /// The slider step: a power of ten.
    pub step: f64,
    /// About half the largest size of the model.
    pub default: f32,
}

impl Range {
    /// The range for a model with this largest size in mm.
    pub fn for_size(size: f32) -> Range {
        let size = if size.is_finite() {
            (size as f64).max(0.1)
        } else {
            100.0
        };
        let max = round_up(2.0 * size);
        let step = 10f64.powi(max.log10().floor() as i32 - 2);
        let coarse = 5.0 * step;
        let default = ((0.5 * size / coarse).round() * coarse).max(step);
        Range {
            max: max as f32,
            step,
            default: default as f32,
        }
    }
}

/// Rounds `x` up to a multiple of half its power of ten. For example,
/// 174 gives 200, 240 gives 250 and 36 gives 40.
fn round_up(x: f64) -> f64 {
    let half = 10f64.powi(x.log10().floor() as i32) * 0.5;
    (x / half - 1e-6).ceil() * half
}

/// The bounds of all parts without movement.
fn part_bounds(parts: &[PartInfo]) -> Option<Bounds> {
    let mut all = parts.iter().map(|p| p.bounds);
    let first = all.next()?;
    Some(all.fold(first, |a, b| Bounds {
        min: a.min.min(b.min),
        max: a.max.max(b.max),
    }))
}

/// The movement of each part for an explode of `distance` mm.
///
/// Let c be the center of the bounds of all parts. Each part moves away
/// from c, along the line from c to its center (`Axis::All`), or along one
/// axis only. The movement is in proportion to the distance of the part
/// from c, and the part that is farthest from c moves by `distance`. A part
/// at c does not move.
///
/// When all parts are at c (for example, concentric parts), the parts make
/// a stack: the largest part does not move, and the other parts move in
/// the positive direction of the axis (Z for `Axis::All`), in volume
/// order. The smallest part moves by `distance`.
pub(super) fn explode_offsets(parts: &[PartInfo], axis: Axis, distance: f32) -> Vec<Vec3> {
    let n = parts.len();
    let mut out = vec![Vec3::ZERO; n];
    let Some(bounds) = part_bounds(parts) else {
        return out;
    };
    if n < 2 || !distance.is_finite() || distance <= 0.0 {
        return out;
    }
    let c = bounds.center();
    let mask = axis.mask();
    let d: Vec<Vec3> = parts.iter().map(|p| (p.center - c) * mask).collect();
    let far = d.iter().map(|v| v.length()).fold(0.0, f32::max);
    // Smaller distances from c are rounding errors.
    let tiny = (bounds.size().max_element() * 1e-3).max(1e-6);
    if far > tiny {
        let k = distance / far;
        return d.into_iter().map(|v| v * k).collect();
    }
    let dir = if axis == Axis::All { Vec3::Z } else { mask };
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| (parts[b].volume.total_cmp(&parts[a].volume)).then(a.cmp(&b)));
    let step = distance / (n - 1) as f32;
    for (rank, i) in order.into_iter().enumerate() {
        out[i] = dir * (step * rank as f32);
    }
    out
}

impl View3d {
    /// The active section planes in model space, before the explode offset.
    /// A point p stays when `plane.truncate().dot(p) + plane.w >= 0.0` for
    /// each plane.
    pub fn clip_planes(&self) -> Vec<glam::Vec4> {
        self.section.planes()
    }

    /// Forgets the colors, the explode distance and the selection of the
    /// previous model.
    pub fn new_model(&mut self) {
        self.custom.clear();
        self.photo_mesh = None;
        self.explode_mm = None;
        self.sel = Default::default();
        self.section.new_model();
    }

    /// Part colors or explode use the parts.
    pub fn shows_parts(&self) -> bool {
        self.part_colors || self.explode
    }

    /// The explode distance in mm for these parts, and its range. A
    /// distance above the range uses the end of the range.
    pub fn explode_distance(&self, parts: &[PartInfo]) -> (f32, Range) {
        let size = part_bounds(parts).map_or(0.0, |b| b.size().max_element());
        let range = Range::for_size(size);
        let d = self.explode_mm.unwrap_or(range.default);
        (d.clamp(0.0, range.max), range)
    }

    /// Sets the explode distance in mm. 0 turns the explode off.
    pub fn set_explode(&mut self, mm: f32) {
        if mm > 0.0 {
            self.explode_mm = Some(mm);
            self.explode = true;
        } else {
            self.explode = false;
        }
    }

    /// The color of part `i` when the part colors are on and the user did
    /// not select a color: its own color, else a palette color.
    pub fn default_color(i: usize, p: &PartInfo) -> [u8; 3] {
        p.color.unwrap_or(PALETTE[i % PALETTE.len()])
    }

    /// The color of part `i` when the part colors are on.
    pub fn color_on(&self, i: usize, p: &PartInfo) -> [u8; 3] {
        (self.custom.get(&p.name).copied()).unwrap_or_else(|| View3d::default_color(i, p))
    }

    /// The color that part `i` shows. With the part colors off, a part
    /// without a color of its own is orange.
    pub fn color(&self, i: usize, p: &PartInfo) -> [u8; 3] {
        if self.part_colors {
            self.color_on(i, p)
        } else {
            p.color.unwrap_or(ORANGE)
        }
    }

    /// Sets the color of a part and turns the part colors on.
    pub fn set_color(&mut self, p: &PartInfo, color: [u8; 3]) {
        self.custom.insert(p.name.clone(), color);
        self.part_colors = true;
    }

    pub fn has_custom(&self, p: &PartInfo) -> bool {
        self.custom.contains_key(&p.name)
    }

    pub fn reset_color(&mut self, p: &PartInfo) {
        self.custom.remove(&p.name);
    }

    /// The style of each part for this frame. With `explode` false, no part
    /// moves.
    pub fn styles(&self, parts: &[PartInfo], explode: bool) -> Vec<PartStyle> {
        let offsets = if explode && self.explode {
            let (d, _) = self.explode_distance(parts);
            explode_offsets(parts, self.explode_axis, d)
        } else {
            vec![Vec3::ZERO; parts.len()]
        };
        (parts.iter().zip(offsets).enumerate())
            .map(|(i, (p, offset))| PartStyle {
                color: self.color(i, p).map(|c| c as f32 / 255.0),
                offset,
                ..PartStyle::default()
            })
            .collect()
    }
}

/// A key for a list of offsets.
fn hash_offsets(h: &mut impl Hasher, offsets: &[Vec3]) {
    for o in offsets {
        o.to_array().map(f32::to_bits).hash(h);
    }
}

impl App {
    fn offsets(styles: &[PartStyle]) -> Vec<Vec3> {
        styles.iter().map(|s| s.offset).collect()
    }

    /// The style of each part in the 3D view. The Optimize tab shows each part
    /// at its real position, so that the mesh agrees with the tool paths.
    /// It does not show the selection and the focus.
    pub(super) fn part_styles(&self) -> Vec<PartStyle> {
        let Some(a) = &self.assembly else {
            return Vec::new();
        };
        let view = self.mode != Mode::Optimize;
        let mut styles = self.view3d.styles(&a.parts, view);
        if view {
            self.view3d.sel.mark(&a.parts, &mut styles);
        }
        styles
    }

    /// The bounds of the model as the view shows it: each part with its
    /// offset. Fit and the grid use them.
    pub(super) fn view_bounds(&self) -> Option<Bounds> {
        match &self.assembly {
            Some(a) if !a.parts.is_empty() => a.bounds(&App::offsets(&self.part_styles())),
            _ => self.info.as_ref().map(|i| i.bounds),
        }
    }

    /// Turns the explode on or off. A model with fewer than 2 parts has
    /// nothing to explode.
    pub(super) fn toggle_explode(&mut self) {
        if self.assembly.as_ref().is_some_and(|a| a.parts.len() >= 2) {
            self.view3d.explode = !self.view3d.explode;
        }
    }

    /// A key that changes when the photo model changes. It is 0 for the
    /// plain mesh in the photo color: no part colors, no part moves and no
    /// section cut.
    pub(super) fn photo_model_key(&self) -> u64 {
        let Some(a) = &self.assembly else {
            return 0;
        };
        let styles = self.view3d.styles(&a.parts, true);
        let offsets = App::offsets(&styles);
        let moved = offsets.iter().any(|o| *o != Vec3::ZERO);
        let planes = self.view3d.section.planes();
        if !self.view3d.part_colors && !moved && planes.is_empty() {
            return 0;
        }
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.assembly_rev.hash(&mut h);
        hash_offsets(&mut h, &offsets);
        for p in &planes {
            p.to_array().map(f32::to_bits).hash(&mut h);
        }
        if self.view3d.part_colors {
            for s in &styles {
                s.color.map(f32::to_bits).hash(&mut h);
            }
        }
        h.finish() | 1
    }

    /// What the photo renders: the mesh, or with part colors on, moved parts
    /// or a section cut, the assembly with each part moved, in its color and
    /// cut. The moved
    /// mesh stays in a cache, so the path tracer keeps its geometry.
    pub(super) fn photo_model(&mut self) -> Option<pathtrace::Model> {
        let mesh = self.mesh.clone()?;
        if self.photo_model_key() == 0 {
            return Some(mesh.into());
        }
        let a = self.assembly.clone()?;
        let offsets = App::offsets(&self.view3d.styles(&a.parts, true));
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.assembly_rev.hash(&mut h);
        hash_offsets(&mut h, &offsets);
        let key = h.finish();
        let mesh = match &self.view3d.photo_mesh {
            Some((k, m)) if *k == key => m.clone(),
            _ => {
                let m = a.moved(&offsets);
                self.view3d.photo_mesh = Some((key, m.clone()));
                m
            }
        };
        let paint = self.view3d.part_colors.then(|| {
            let colors: Vec<[u8; 3]> = (a.parts.iter().enumerate())
                .map(|(i, p)| self.view3d.color(i, p))
                .collect();
            Arc::new(a.paint(&colors))
        });
        let clip = self.photo_clip(&offsets);
        Some(pathtrace::Model { mesh, paint, clip })
    }

    /// The "Parts" card: the part count and the part colors. The Model and
    /// the Render tabs show it.
    pub(super) fn parts_card(&mut self, ui: &mut egui::Ui) {
        let Some(a) = self.assembly.clone().filter(|a| !a.parts.is_empty()) else {
            return;
        };
        let n = a.parts.len();
        ui::card(ui, |ui| {
            ui.horizontal(|ui| {
                ui::section(ui, "Parts");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let count = if n == 1 {
                        "1 part".to_string()
                    } else {
                        format!("{n} parts")
                    };
                    ui.label(ui::dim(count));
                });
            });
            let note = if let Some(job) = &self.parts_job {
                job.note
            } else if n == 1 {
                "The model is one solid."
            } else if crate::parts::needs_render(&self.params) {
                "The parts of the cocad_part list."
            } else if a.parts.iter().any(|p| p.source > 0) {
                // The module split gives one source for each module.
                "Each solid of each module is one part."
            } else {
                "Each separate solid is one part."
            };
            ui.label(ui::dim(note));
            if let Some(p) = self.view3d.sel.selected(&a.parts) {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(ui::dim("Selected"));
                    ui.label(egui::RichText::new(&p.name).strong());
                    if self.view3d.sel.has_focus(&p.name) {
                        ui.label(ui::dim("(focus)"));
                    }
                });
                let s = p.bounds.size();
                let size = format!("{:.1} × {:.1} × {:.1} mm", s.x, s.y, s.z);
                ui.label(ui::dim(format!("{size} · {}", volume(p.volume))));
            }
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.label("Part colors");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui::toggle(ui, &mut self.view3d.part_colors)
                        .on_hover_text("Show each part in its own color");
                });
            });
            let colors =
                egui::Button::new("Colors…").min_size(Vec2::new(ui.available_width(), 34.0));
            if ui
                .add(colors)
                .on_hover_text("Select a color for each part")
                .clicked()
            {
                self.view3d.colors_open = true;
            }
        });
        ui.add_space(4.0);
    }

    /// The "Explode" card: moves the parts away from each other. The Model
    /// and the Render tabs show it. A change of the distance or the direction
    /// turns the explode on.
    pub(super) fn explode_card(&mut self, ui: &mut egui::Ui) {
        let Some(a) = self.assembly.clone().filter(|a| !a.parts.is_empty()) else {
            return;
        };
        let v = &mut self.view3d;
        ui::card(ui, |ui| {
            ui::section(ui, "Explode");
            if a.parts.len() < 2 {
                ui.label(ui::dim("Explode needs two or more parts."));
                return;
            }
            ui.label(ui::dim("Moves the parts away from the center."));
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.label("Explode");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui::toggle(ui, &mut v.explode).on_hover_text("Move the parts apart (E)");
                });
            });
            let (mut d, range) = v.explode_distance(&a.parts);
            if super::setting(ui, "Distance", &mut d, 0.0..=range.max, range.step, " mm") {
                v.explode_mm = Some(d);
                v.explode = true;
            }
            ui.label("Direction");
            let mut axis = v.explode_axis;
            if ui::segmented(ui, &mut axis, &Axis::LIST.map(|a| (a, a.label()))) {
                v.explode_axis = axis;
                v.explode = true;
            }
        });
        ui.add_space(4.0);
    }

    /// The window with the color of each part.
    pub(super) fn part_colors_window(&mut self, ctx: &egui::Context) {
        if !self.view3d.colors_open {
            return;
        }
        let Some(a) = self.assembly.clone() else {
            self.view3d.colors_open = false;
            return;
        };
        let mut close = false;
        let frame = egui::Frame::NONE
            .fill(ui::c().panel)
            .corner_radius(14)
            .inner_margin(egui::Margin::same(18))
            .stroke(egui::Stroke::new(1.0_f32, ui::c().line));
        let row_height = 30.0;
        let height = (ctx.screen_rect().height() - 220.0).clamp(row_height * 3.0, 420.0);
        let v = &mut self.view3d;
        let modal = egui::Modal::new(egui::Id::new("part_colors"))
            .frame(frame)
            .show(ctx, |ui| {
                ui.set_width(420.0);
                ui.heading("Part colors");
                ui.add_space(4.0);
                ui.label(ui::dim(
                    "The colors show in the 3D view and in the photo. The export does not change.",
                ));
                ui.add_space(10.0);
                egui::ScrollArea::vertical()
                    .max_height(height)
                    .auto_shrink([false, true])
                    .show_rows(ui, row_height, a.parts.len(), |ui, rows| {
                        for i in rows {
                            let p = &a.parts[i];
                            let row = Vec2::new(ui.available_width(), row_height);
                            let ltr = egui::Layout::left_to_right(egui::Align::Center);
                            ui.allocate_ui_with_layout(row, ltr, |ui| {
                                ui.spacing_mut().interact_size = Vec2::new(36.0, 22.0);
                                let mut color = v.color_on(i, p);
                                if ui.color_edit_button_srgb(&mut color).changed() {
                                    v.set_color(p, color);
                                }
                                ui.add_space(4.0);
                                ui.label(&p.name);
                                let rtl = egui::Layout::right_to_left(egui::Align::Center);
                                ui.with_layout(rtl, |ui| {
                                    let reset = ui::ghost("Reset", false);
                                    if ui
                                        .add_enabled(v.has_custom(p), reset)
                                        .on_hover_text("Use the default color")
                                        .clicked()
                                    {
                                        v.reset_color(p);
                                    }
                                    ui.label(ui::dim(volume(p.volume)));
                                });
                            });
                        }
                    });
                ui.add_space(12.0);
                let row = Vec2::new(ui.available_width(), 34.0);
                let rtl = egui::Layout::right_to_left(egui::Align::Center);
                ui.allocate_ui_with_layout(row, rtl, |ui| {
                    ui.spacing_mut().button_padding = Vec2::new(16.0, 7.0);
                    close = ui.add(ui::primary("Close")).clicked();
                    let any = a.parts.iter().any(|p| v.has_custom(p));
                    if ui.add_enabled(any, ui::ghost("Reset all", false)).clicked() {
                        for p in &a.parts {
                            v.reset_color(p);
                        }
                    }
                });
            });
        if close || modal.should_close() {
            self.view3d.colors_open = false;
        }
    }
}

/// A part volume for the parts list.
fn volume(mm3: f32) -> String {
    if mm3 < 1000.0 {
        format!("{mm3:.1} mm³")
    } else {
        format!("{:.2} cm³", mm3 / 1000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A box part with this center, half size and volume.
    fn part(center: Vec3, half: f32, volume: f32) -> PartInfo {
        PartInfo {
            name: String::new(),
            source: 0,
            tris: 0..0,
            edges: 0..0,
            color: None,
            bounds: Bounds {
                min: center - half,
                max: center + half,
            },
            center,
            volume,
        }
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn farthest_part_moves_the_distance() {
        // A base at the origin, a lid above it and a pin to the side.
        let parts = [
            part(Vec3::new(0.0, 0.0, 5.0), 5.0, 1000.0),
            part(Vec3::new(0.0, 0.0, 20.0), 5.0, 1000.0),
            part(Vec3::new(30.0, 0.0, 5.0), 1.0, 8.0),
        ];
        let c = part_bounds(&parts).unwrap().center();
        let off = explode_offsets(&parts, Axis::All, 40.0);
        let longest = off.iter().map(|o| o.length()).fold(0.0, f32::max);
        assert!(close(longest, 40.0));
        for (p, o) in parts.iter().zip(&off) {
            // Each part moves straight away from the center.
            let d = p.center - c;
            assert!(o.cross(d).length() < 1e-3);
            assert!(o.dot(d) >= 0.0);
        }
        // No explode: no movement.
        let zero = explode_offsets(&parts, Axis::All, 0.0);
        assert!(zero.iter().all(|o| *o == Vec3::ZERO));
        assert!(explode_offsets(&parts[..1], Axis::All, 40.0)[0] == Vec3::ZERO);
    }

    #[test]
    fn axis_keeps_one_component() {
        let parts = [
            part(Vec3::new(-10.0, 4.0, 2.0), 2.0, 64.0),
            part(Vec3::new(10.0, -4.0, 8.0), 2.0, 64.0),
            part(Vec3::new(0.0, 0.0, 30.0), 2.0, 64.0),
        ];
        for axis in [Axis::X, Axis::Y, Axis::Z] {
            let off = explode_offsets(&parts, axis, 25.0);
            let keep = axis.mask();
            let longest = off.iter().map(|o| o.length()).fold(0.0, f32::max);
            assert!(close(longest, 25.0), "{axis:?}");
            for o in &off {
                assert_eq!(*o * (Vec3::ONE - keep), Vec3::ZERO, "{axis:?}");
            }
        }
        // Along X, the parts move in opposite directions.
        let off = explode_offsets(&parts, Axis::X, 25.0);
        assert!(off[0].x < 0.0 && off[1].x > 0.0 && off[2].x == 0.0);
    }

    #[test]
    fn concentric_parts_make_a_stack() {
        // A ball in a hollow ball and a ring around them: one center.
        let parts = [
            part(Vec3::splat(10.0), 2.0, 30.0),
            part(Vec3::splat(10.0), 10.0, 900.0),
            part(Vec3::splat(10.0), 6.0, 100.0),
        ];
        let off = explode_offsets(&parts, Axis::All, 30.0);
        assert!(off.iter().all(|o| o.is_finite()));
        // The largest part stays, the smallest moves the full distance.
        assert_eq!(off[1], Vec3::ZERO);
        assert_eq!(off[2], Vec3::new(0.0, 0.0, 15.0));
        assert_eq!(off[0], Vec3::new(0.0, 0.0, 30.0));
        // Along X, the stack goes along X.
        let off = explode_offsets(&parts, Axis::X, 30.0);
        assert_eq!(off[0], Vec3::new(30.0, 0.0, 0.0));
        // Parts that differ only in Z have no X spread: a stack along X.
        let parts = [
            part(Vec3::new(0.0, 0.0, 0.0), 1.0, 8.0),
            part(Vec3::new(0.0, 0.0, 10.0), 1.0, 9.0),
        ];
        let off = explode_offsets(&parts, Axis::X, 12.0);
        assert_eq!(off, [Vec3::new(12.0, 0.0, 0.0), Vec3::ZERO]);
        // Points with no size do not give NaN.
        let parts = [part(Vec3::ZERO, 0.0, 0.0), part(Vec3::ZERO, 0.0, 0.0)];
        let off = explode_offsets(&parts, Axis::All, 5.0);
        assert!(off.iter().all(|o| o.is_finite()));
        assert_eq!(off[1], Vec3::new(0.0, 0.0, 5.0));
    }

    #[test]
    fn range_is_round() {
        let r = Range::for_size(87.0);
        assert_eq!((r.max, r.step, r.default), (200.0, 1.0, 45.0));
        let r = Range::for_size(120.0);
        assert_eq!((r.max, r.step, r.default), (250.0, 1.0, 60.0));
        let r = Range::for_size(12.0);
        assert_eq!(r.max, 25.0);
        assert!((r.step - 0.1).abs() < 1e-12);
        assert_eq!(r.default, 6.0);
        let r = Range::for_size(1000.0);
        assert_eq!((r.max, r.step, r.default), (2000.0, 10.0, 500.0));
        // Empty and broken sizes still give a range.
        for size in [0.0, -3.0, f32::NAN, f32::INFINITY] {
            let r = Range::for_size(size);
            assert!(r.max > 0.0 && r.default > 0.0 && r.default <= r.max);
        }
    }

    #[test]
    fn distance_follows_the_model_until_the_user_sets_one() {
        let parts = [
            part(Vec3::ZERO, 5.0, 1000.0),
            part(Vec3::new(77.0, 0.0, 0.0), 5.0, 1000.0),
        ];
        let mut v = View3d::default();
        assert_eq!(v.explode_distance(&parts).0, 45.0);
        assert!(
            v.styles(&parts, true)
                .iter()
                .all(|s| s.offset == Vec3::ZERO)
        );
        v.set_explode(30.0);
        assert!(v.explode);
        assert_eq!(v.explode_distance(&parts).0, 30.0);
        let moved = v.styles(&parts, true);
        assert!(close(moved[1].offset.length(), 30.0));
        // The Optimize tab: no movement.
        assert!(
            v.styles(&parts, false)
                .iter()
                .all(|s| s.offset == Vec3::ZERO)
        );
        // Above the range: the end of the range.
        v.set_explode(5000.0);
        assert_eq!(v.explode_distance(&parts).0, 200.0);
        v.set_explode(0.0);
        assert!(!v.explode);
        v.new_model();
        assert_eq!(v.explode_distance(&parts).0, 45.0);
        assert_eq!(Axis::from_name("Z"), Some(Axis::Z));
        assert_eq!(Axis::from_name("w"), None);
    }
}
