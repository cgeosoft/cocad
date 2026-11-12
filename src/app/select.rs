//! Select a part in the 3D view, the part menu, the focus and the color
//! window of one part.
//!
//! - A click selects the part under the mouse. A click on the background
//!   clears the selection. A right click on a part selects it and opens the
//!   part menu: "Focus" and "Change color…".
//! - Focus: the camera turns around the part, and the other parts are
//!   transparent. While a part has the focus, a click goes through the
//!   transparent parts to the parts that are not transparent.
//! - Selection and focus use the part name, as the custom colors do. A new
//!   render keeps them when a part with that name is there when the parts
//!   are done.
//!
//! The selection and the focus show in the 3D view of the Model and Photo
//! tabs only. The Slice tab and the path traced photo do not show them.

use super::{App, Camera, Mode};
use crate::assembly::PartInfo;
use crate::mesh::Bounds;
use crate::render::PartStyle;
use crate::ui;
use eframe::egui::{self, Color32, Pos2, Rect, Vec2};
use glam::Vec3;

/// The transparency of the other parts while a part has the focus.
const GHOST: f32 = 0.15;
/// The transparency of a selected part that is not in focus, while a part
/// has the focus. A click on a transparent part then shows.
const GHOST_SELECTED: f32 = 0.35;
/// The width of the content of the color window.
const COLOR_WIDTH: f32 = 300.0;

/// The selected part, the part in focus, the part menu and the color window.
#[derive(Default)]
pub(super) struct Selection {
    /// The selected part, by name.
    part: Option<String>,
    /// The part in focus, by name.
    focus: Option<String>,
    /// The center of the part in focus in the last frame, with its offset.
    /// When the part moves (explode, new render), the camera target moves
    /// with it.
    focus_at: Option<Vec3>,
    menu: Option<Menu>,
    color: Option<ColorWindow>,
    /// The left button went down on the view while the part menu was
    /// open. That click only closes the menu.
    swallow: bool,
}

/// The part menu at the point of a right click.
#[derive(Clone)]
struct Menu {
    at: Pos2,
    /// The pass of the right click. In that pass, a click outside the
    /// menu does not close it.
    pass: u64,
    /// The part under the mouse. `None`: the background, while a part has
    /// the focus.
    part: Option<String>,
}

/// The color window of one part.
struct ColorWindow {
    part: String,
    /// The first position of the window. After the first frame, the user
    /// can move the window.
    at: Option<Pos2>,
}

/// The index of the part with this name.
fn find(parts: &[PartInfo], name: Option<&str>) -> Option<usize> {
    let name = name?;
    parts.iter().position(|p| p.name == name)
}

impl Selection {
    /// Highlights the selected part and the part in focus. While a part
    /// has the focus, the other parts are transparent.
    pub fn mark(&self, parts: &[PartInfo], styles: &mut [PartStyle]) {
        let focus = find(parts, self.focus.as_deref());
        let part = find(parts, self.part.as_deref());
        for (i, s) in styles.iter_mut().enumerate().take(parts.len()) {
            s.highlight |= Some(i) == part || Some(i) == focus;
            if focus.is_some_and(|f| f != i) {
                let ghost = if Some(i) == part {
                    GHOST_SELECTED
                } else {
                    GHOST
                };
                s.alpha = s.alpha.min(ghost);
            }
        }
    }

    /// The selected part.
    pub fn selected<'a>(&self, parts: &'a [PartInfo]) -> Option<&'a PartInfo> {
        find(parts, self.part.as_deref()).map(|i| &parts[i])
    }

    /// True when the part with this name has the focus.
    pub fn has_focus(&self, name: &str) -> bool {
        self.focus.as_deref() == Some(name)
    }

    /// Drops the names that are not in `parts`. Call it only with the
    /// final parts of a render: a render shows the solids first, and the
    /// named parts can replace them a moment later.
    fn keep(&mut self, parts: &[PartInfo]) {
        let gone = |name: &Option<String>| name.is_some() && find(parts, name.as_deref()).is_none();
        if gone(&self.part) {
            self.part = None;
        }
        if gone(&self.focus) {
            self.focus = None;
            self.focus_at = None;
        }
        if self
            .color
            .as_ref()
            .is_some_and(|c| find(parts, Some(c.part.as_str())).is_none())
        {
            self.color = None;
        }
    }
}

impl Camera {
    /// The ray through the point `ndc` of the view (-1 to 1, y up), from
    /// the near plane: a start point and a unit direction. It inverts the
    /// view-projection matrix that the renderer uses.
    fn ray(&self, aspect: f32, ndc: glam::Vec2) -> (Vec3, Vec3) {
        let inv = self.matrix(aspect).inverse();
        let near = inv.project_point3(ndc.extend(-1.0));
        let far = inv.project_point3(ndc.extend(1.0));
        (near, (far - near).normalize_or_zero())
    }

    /// True when all corners of the box are in the view.
    fn sees(&self, b: &Bounds, aspect: f32) -> bool {
        let m = self.matrix(aspect);
        (0..8).all(|k| {
            let pick = |bit: usize, lo: f32, hi: f32| if k & bit == 0 { lo } else { hi };
            let p = Vec3::new(
                pick(1, b.min.x, b.max.x),
                pick(2, b.min.y, b.max.y),
                pick(4, b.min.z, b.max.z),
            );
            let c = m * p.extend(1.0);
            c.w > 0.0 && (c.truncate() / c.w).abs().cmple(Vec3::ONE).all()
        })
    }
}

impl App {
    /// The part under the point `pos` of the 3D view `rect`.
    fn part_at(&self, rect: Rect, pos: Pos2) -> Option<usize> {
        let a = self.assembly.as_ref()?;
        let aspect = rect.width() / rect.height().max(1.0);
        let ndc = glam::Vec2::new(
            (pos.x - rect.left()) / rect.width().max(1.0) * 2.0 - 1.0,
            1.0 - (pos.y - rect.top()) / rect.height().max(1.0) * 2.0,
        );
        let (origin, dir) = self.cam.ray(aspect, ndc);
        let styles = self.part_styles();
        let offsets: Vec<Vec3> = styles.iter().map(|s| s.offset).collect();
        let ghost: Vec<bool> = styles.iter().map(|s| s.alpha < 1.0).collect();
        let planes = self.view3d.clip_planes();
        a.pick(origin, dir, &offsets, &planes, &ghost)
            .map(|(i, _)| i)
    }

    fn part_name(&self, i: usize) -> Option<String> {
        Some(self.assembly.as_ref()?.parts.get(i)?.name.clone())
    }

    /// The bounds of the part in focus, with its offset.
    pub(super) fn focus_bounds(&self) -> Option<Bounds> {
        let a = self.assembly.as_ref()?;
        let i = find(&a.parts, self.view3d.sel.focus.as_deref())?;
        let offset = self.part_styles().get(i).map_or(Vec3::ZERO, |s| s.offset);
        let b = a.parts[i].bounds;
        Some(Bounds {
            min: b.min + offset,
            max: b.max + offset,
        })
    }

    /// Click and right click in the 3D view `rect`. A drag does not select.
    pub(super) fn select_input(&mut self, resp: &egui::Response, rect: Rect) {
        if self.mode == Mode::Slice || self.assembly.is_none() {
            return;
        }
        if resp.hovered() && resp.ctx.input(|i| i.pointer.primary_pressed()) {
            self.view3d.sel.swallow = self.view3d.sel.menu.is_some();
        }
        let Some(pos) = resp.interact_pointer_pos() else {
            return;
        };
        if resp.clicked() {
            if std::mem::take(&mut self.view3d.sel.swallow) {
                return;
            }
            let hit = self.part_at(rect, pos);
            self.view3d.sel.part = hit.and_then(|i| self.part_name(i));
        } else if resp.secondary_clicked() {
            let hit = self.part_at(rect, pos).and_then(|i| self.part_name(i));
            let pass = resp.ctx.cumulative_pass_nr();
            let sel = &mut self.view3d.sel;
            if hit.is_some() {
                sel.part = hit.clone();
            }
            sel.menu = (hit.is_some() || sel.focus.is_some()).then_some(Menu {
                at: pos,
                pass,
                part: hit,
            });
        }
    }

    /// Keeps the selection up to date for this frame: drops names that the
    /// final parts do not have, and moves the camera target with the part
    /// in focus. Call it before the camera matrix of the frame.
    pub(super) fn update_selection(&mut self) {
        if let Some(a) = self.assembly.clone()
            && self.job.is_none()
            && self.parts_job.is_none()
        {
            self.view3d.sel.keep(&a.parts);
        }
        let at = self.focus_bounds().map(|b| b.center());
        let sel = &mut self.view3d.sel;
        if let (Some(now), Some(before)) = (at, sel.focus_at)
            && now != before
        {
            self.cam.target += now - before;
        }
        if at.is_some() || sel.focus.is_none() {
            sel.focus_at = at;
        }
    }

    /// Gives the focus to part `i`. The camera turns around the center of
    /// the part. The distance stays when all of the part is in the view.
    fn focus_on(&mut self, i: usize, aspect: f32) {
        let Some(name) = self.part_name(i) else {
            return;
        };
        self.view3d.sel.part = Some(name.clone());
        self.view3d.sel.focus = Some(name);
        let Some(b) = self.focus_bounds() else {
            return;
        };
        self.cam.target = b.center();
        if !self.cam.sees(&b, aspect) {
            self.cam.fit(&b);
        }
        self.view3d.sel.focus_at = Some(b.center());
    }

    /// Ends the focus. The camera target goes back to the center of the
    /// model. The direction and the distance stay.
    fn leave_focus(&mut self) {
        let sel = &mut self.view3d.sel;
        sel.focus = None;
        sel.focus_at = None;
        if let Some(b) = self.view_bounds() {
            self.cam.target = b.center();
        }
    }

    /// Esc: closes the part menu or the color window. Else it clears the
    /// selection and ends the focus. Other windows that close on Esc go
    /// first.
    pub(super) fn select_escape(&mut self, ctx: &egui::Context) {
        if !ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            return;
        }
        let sel = &mut self.view3d.sel;
        if sel.menu.take().is_some() || sel.color.take().is_some() {
            return;
        }
        if self.menu.is_some()
            || self.cfg_edit.is_some()
            || self.view3d.colors_open
            || self.show_changes
        {
            return;
        }
        self.view3d.sel.part = None;
        if self.view3d.sel.focus.is_some() {
            self.leave_focus();
        }
    }

    /// The part menu at the point of the right click. A click on an item or
    /// outside the menu closes it.
    pub(super) fn part_menu(&mut self, ctx: &egui::Context, rect: Rect) {
        let Some(menu) = self.view3d.sel.menu.clone() else {
            return;
        };
        let Some(a) = self.assembly.clone() else {
            self.view3d.sel.menu = None;
            return;
        };
        let part = find(&a.parts, menu.part.as_deref());
        let focus = find(&a.parts, self.view3d.sel.focus.as_deref());
        let valid = match menu.part {
            Some(_) => part.is_some(),
            None => focus.is_some(),
        };
        if self.mode == Mode::Slice || !valid {
            self.view3d.sel.menu = None;
            return;
        }
        enum Act {
            Focus(usize),
            Leave,
            Color(String),
        }
        let mut act = None;
        let area = egui::Area::new(egui::Id::new("part_menu"))
            .order(egui::Order::Foreground)
            .fixed_pos(menu.at)
            .show(ctx, |ui| {
                ui::dark_scope(ui, |ui| {
                    ui::menu_frame_width(ui, 220.0, |ui| {
                        let Some(i) = part.or(focus) else {
                            return;
                        };
                        let p = &a.parts[i];
                        ui::menu_header(ui, &p.name);
                        let focused = Some(i) == focus;
                        if focused {
                            if ui::menu_item(ui, "Leave focus", "Esc", true, false).clicked() {
                                act = Some(Act::Leave);
                            }
                        } else if ui::menu_item(ui, "Focus", "", true, false)
                            .on_hover_text("Turn the view around this part")
                            .clicked()
                        {
                            act = Some(Act::Focus(i));
                        }
                        if part.is_some()
                            && ui::menu_item(ui, "Change color…", "", true, false).clicked()
                        {
                            act = Some(Act::Color(p.name.clone()));
                        }
                    })
                })
            });
        let (pos, pressed) = ctx.input(|i| (i.pointer.interact_pos(), i.pointer.any_pressed()));
        let outside = pressed
            && menu.pass != ctx.cumulative_pass_nr()
            && !pos.is_some_and(|p| area.response.rect.contains(p));
        if act.is_some() || outside {
            self.view3d.sel.menu = None;
        }
        match act {
            Some(Act::Focus(i)) => self.focus_on(i, rect.width() / rect.height().max(1.0)),
            Some(Act::Leave) => self.leave_focus(),
            Some(Act::Color(part)) => {
                // The top right corner of the view, so that the window does
                // not cover the part. The window has a 16 px margin on each
                // side.
                let width = COLOR_WIDTH + 2.0 * 16.0;
                let at = egui::pos2(rect.right() - width - 16.0, rect.top() + 16.0);
                self.view3d.sel.color = Some(ColorWindow {
                    part,
                    at: Some(at.max(rect.left_top())),
                });
            }
            None => {}
        }
    }

    /// The color window of one part. It does not block the view, so the
    /// view shows each change at once.
    pub(super) fn part_color_window(&mut self, ctx: &egui::Context) {
        let Some(w) = &mut self.view3d.sel.color else {
            return;
        };
        let at = w.at.take();
        let name = w.part.clone();
        let Some((a, i)) = (self.assembly.clone())
            .and_then(|a| find(&a.parts, Some(name.as_str())).map(|i| (a, i)))
        else {
            // While a render runs, the part can be missing for a moment.
            return;
        };
        let p = &a.parts[i];
        let v = &mut self.view3d;
        let mut close = false;
        let frame = egui::Frame::NONE
            .fill(ui::c().panel)
            .corner_radius(14)
            .inner_margin(egui::Margin::same(16))
            .stroke(egui::Stroke::new(1.0_f32, ui::c().line))
            .shadow(ctx.style().visuals.popup_shadow);
        let mut window = egui::Window::new("Part color")
            .id(egui::Id::new("part_color"))
            .title_bar(false)
            .resizable(false)
            .collapsible(false)
            .frame(frame);
        if let Some(at) = at {
            window = window.current_pos(at);
        }
        window.show(ctx, |ui| {
            ui.set_width(COLOR_WIDTH);
            ui::section(ui, "Part color");
            ui.add_space(2.0);
            ui.label(egui::RichText::new(&p.name).strong());
            ui.add_space(8.0);
            let [r, g, b] = v.color_on(i, p);
            let mut color = Color32::from_rgb(r, g, b);
            let alpha = egui::color_picker::Alpha::Opaque;
            // The color field and the hue bar use the width of the window.
            ui.spacing_mut().slider_width = ui.available_width();
            if egui::color_picker::color_picker_color32(ui, &mut color, alpha) {
                v.set_color(p, [color.r(), color.g(), color.b()]);
            }
            ui.add_space(10.0);
            let row = Vec2::new(ui.available_width(), 30.0);
            let rtl = egui::Layout::right_to_left(egui::Align::Center);
            ui.allocate_ui_with_layout(row, rtl, |ui| {
                ui.spacing_mut().button_padding = Vec2::new(14.0, 6.0);
                close = ui.add(ui::primary("Close")).clicked();
                if ui
                    .add_enabled(v.has_custom(p), ui::ghost("Reset", false))
                    .on_hover_text("Use the default color")
                    .clicked()
                {
                    v.reset_color(p);
                }
            });
        });
        if close {
            self.view3d.sel.color = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(name: &str) -> PartInfo {
        PartInfo {
            name: name.into(),
            source: 0,
            tris: 0..0,
            edges: 0..0,
            color: None,
            bounds: Bounds {
                min: Vec3::ZERO,
                max: Vec3::ONE,
            },
            center: Vec3::splat(0.5),
            volume: 1.0,
        }
    }

    #[test]
    fn focus_makes_the_other_parts_transparent() {
        let parts = [part("a"), part("b"), part("c")];
        let mut sel = Selection::default();
        let mut s = vec![PartStyle::default(); 3];
        sel.mark(&parts, &mut s);
        assert!(s.iter().all(|s| s.alpha == 1.0 && !s.highlight));
        sel.part = Some("c".into());
        sel.focus = Some("b".into());
        sel.mark(&parts, &mut s);
        assert_eq!(
            s.iter().map(|s| s.highlight).collect::<Vec<_>>(),
            [false, true, true]
        );
        let alpha: Vec<f32> = s.iter().map(|s| s.alpha).collect();
        assert_eq!(alpha, [GHOST, 1.0, GHOST_SELECTED]);
        // A focus name that the parts do not have changes nothing.
        let mut s = vec![PartStyle::default(); 3];
        sel.focus = Some("x".into());
        sel.part = None;
        sel.mark(&parts, &mut s);
        assert!(s.iter().all(|s| s.alpha == 1.0 && !s.highlight));
    }

    #[test]
    fn keep_drops_missing_names() {
        let mut sel = Selection {
            part: Some("a".into()),
            focus: Some("b".into()),
            focus_at: Some(Vec3::ONE),
            ..Selection::default()
        };
        sel.keep(&[part("a"), part("b")]);
        assert_eq!(sel.part.as_deref(), Some("a"));
        assert_eq!(sel.focus.as_deref(), Some("b"));
        sel.keep(&[part("b")]);
        assert_eq!(sel.part, None);
        assert!(sel.has_focus("b"));
        sel.keep(&[part("a")]);
        assert_eq!(sel.focus, None);
        assert_eq!(sel.focus_at, None);
    }

    #[test]
    fn ray_goes_through_the_cursor() {
        let cam = Camera {
            yaw: -0.9,
            pitch: 0.55,
            dist: 200.0,
            target: Vec3::new(10.0, -5.0, 3.0),
        };
        let aspect = 1.6;
        // The center of the view: from the eye to the target.
        let (o, d) = cam.ray(aspect, glam::Vec2::ZERO);
        let to_target = (cam.target - cam.eye()).normalize();
        assert!(d.dot(to_target) > 0.99999);
        assert!((o - cam.eye()).cross(to_target).length() < 1e-2);
        // A point off the center: the ray goes through the point that the
        // matrix puts there.
        let p = Vec3::new(40.0, 20.0, -10.0);
        let c = cam.matrix(aspect).project_point3(p);
        let (o, d) = cam.ray(aspect, glam::Vec2::new(c.x, c.y));
        let miss = (p - o).cross(d).length();
        assert!(miss < 1e-2, "{miss}");
        // A box at the target is in the view; a box far to the side is not.
        let b = Bounds {
            min: cam.target - 5.0,
            max: cam.target + 5.0,
        };
        assert!(cam.sees(&b, aspect));
        let side = Bounds {
            min: b.min + Vec3::new(500.0, 0.0, 0.0),
            max: b.max + Vec3::new(500.0, 0.0, 0.0),
        };
        assert!(!cam.sees(&side, aspect));
    }
}
