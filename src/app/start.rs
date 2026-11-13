//! Start page: shows when no model is open. It replaces the side panels and
//! the 3D view, which have nothing to show without a model.

use super::{App, SettingsPage, thumb, tour};
use crate::{examples, ui};
use eframe::egui::{self, Align2, Color32, FontId, RichText, Sense, Vec2, vec2};
use std::path::{Path, PathBuf};

/// The source of a new model: a valid model with Customizer parameters, so
/// that the first build shows a shape and the left panel shows controls.
pub const TEMPLATE: &str = r#"// New CoCAD model.
// Change the values in the left panel, or edit this file in a text editor.
// CoCAD builds the model again when the file changes on disk.

/* [Size] */
// Width (mm)
width = 40; // [5:1:200]
// Depth (mm)
depth = 30; // [5:1:200]
// Height (mm)
height = 12; // [1:1:100]
// Corner radius (mm)
radius = 3; // [0:0.5:10]

/* [Hidden] */
$fn = 48;

r = min(radius, width / 2 - 0.01, depth / 2 - 0.01);
if (r > 0) {
    hull()
        for (x = [r, width - r], y = [r, depth - r])
            translate([x, y, 0]) cylinder(r = r, h = height);
} else {
    cube([width, depth, height]);
}
"#;

/// Writes a new model with the template source. With `source`, writes that
/// text. Adds `.scad` when the name has no extension.
pub fn write_new(path: &Path, source: Option<&str>) -> std::io::Result<PathBuf> {
    let path = if path.extension().is_none() {
        path.with_extension("scad")
    } else {
        path.to_path_buf()
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, source.unwrap_or(TEMPLATE))?;
    Ok(path)
}

/// Most recent models on the start page.
const RECENT_MAX: usize = 8;
/// Width of the two columns and the gap between them.
const LEFT_W: f32 = 320.0;
const GAP: f32 = 20.0;
const RIGHT_W: f32 = 400.0;

impl App {
    /// Asks for a file name, writes the template and opens the new model.
    pub(super) fn new_model(&mut self) {
        let mut d = rfd::FileDialog::new()
            .add_filter("OpenSCAD", &["scad"])
            .set_file_name("model.scad")
            .set_title("New model");
        if let Some(dir) = self
            .file
            .as_ref()
            .and_then(|f| f.parent())
            .or_else(|| self.recent.first().and_then(|f| f.parent()))
        {
            d = d.set_directory(dir);
        }
        let Some(path) = d.save_file() else {
            return;
        };
        match write_new(&path, None) {
            Ok(path) => self.open(path),
            Err(e) => self.error = Some(format!("cannot write {}: {e}", path.display())),
        }
    }

    /// The start page in the full window area below the menu bar.
    pub(super) fn start_page(&mut self, ui: &mut egui::Ui) {
        let rect = ui.max_rect();
        let [top, bottom] = ui::c().view;
        let mut bg = egui::Mesh::default();
        bg.colored_vertex(rect.left_top(), top);
        bg.colored_vertex(rect.right_top(), top);
        bg.colored_vertex(rect.right_bottom(), bottom);
        bg.colored_vertex(rect.left_bottom(), bottom);
        bg.add_triangle(0, 1, 2);
        bg.add_triangle(0, 2, 3);
        ui.painter().add(egui::Shape::mesh(bg));

        // Two columns on a wide window, one column on a narrow window.
        let wide = rect.width() >= LEFT_W + GAP + RIGHT_W + 64.0;
        let width = if wide {
            LEFT_W + GAP + RIGHT_W
        } else {
            (rect.width() - 48.0).clamp(280.0, RIGHT_W)
        };
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let side = ((ui.available_width() - width) / 2.0).max(0.0);
                // Center the content when it is lower than the window.
                let pad = ((rect.height() - self.start_height) / 2.0).max(32.0);
                ui.add_space(pad);
                let start = ui.cursor().top();
                ui.horizontal(|ui| {
                    ui.add_space(side);
                    ui.vertical(|ui| {
                        ui.set_width(width);
                        self.start_header(ui);
                        ui.add_space(22.0);
                        if wide {
                            ui.horizontal_top(|ui| {
                                ui.spacing_mut().item_spacing.x = GAP;
                                ui.vertical(|ui| {
                                    ui.set_width(LEFT_W);
                                    self.start_actions(ui);
                                });
                                ui.vertical(|ui| {
                                    ui.set_width(RIGHT_W);
                                    self.start_lists(ui);
                                });
                            });
                        } else {
                            self.start_actions(ui);
                            ui.add_space(12.0);
                            self.start_lists(ui);
                        }
                        ui.add_space(18.0);
                        ui.label(
                            ui::dim(
                                "You can also drop a .scad file on the window. \
                                 Help > Welcome tour shows the main features.",
                            )
                            .size(12.5),
                        );
                    });
                });
                self.start_height = ui.cursor().top() - start;
                ui.add_space(pad);
            });
    }

    fn start_header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::hover());
            ui::paint_app_mark(ui.painter(), r);
            ui.add_space(6.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Max), |ui| {
                    ui.label(RichText::new("CoCAD").size(24.0).strong());
                    ui.add_space(2.0);
                    ui.label(ui::dim("by cgeosoft"));
                });
                ui.label(ui::dim("Make, change and print OpenSCAD models."));
            });
        });
        if let Some(e) = &self.error {
            ui.add_space(10.0);
            ui.label(RichText::new(e).color(ui::c().danger));
        }
    }

    /// Left column: new, open and the agent card.
    fn start_actions(&mut self, ui: &mut egui::Ui) {
        ui::card(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 6.0;
            ui::section(ui, "Start");
            ui.add_space(4.0);
            if action_row(
                ui,
                "New model…",
                "A small model with parameters",
                "Ctrl+N",
                true,
            )
            .clicked()
            {
                self.new_model();
            }
            if action_row(
                ui,
                "Open model…",
                "A .scad file on this computer",
                "Ctrl+O",
                false,
            )
            .clicked()
            {
                self.pick_file();
            }
        });
        ui.add_space(12.0);
        let running = self.server.is_some();
        ui::card(ui, |ui| {
            let (color, state) = if running {
                (ui::GOOD, "MCP on")
            } else {
                (ui::c().dim, "MCP off")
            };
            super::card_header(ui, "Work with an agent", color, state);
            ui.add_space(6.0);
            ui.label(
                "An AI agent can write a new model and change it in this window. \
                 You see each change live.",
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let button = |text: &str| {
                    egui::Button::new(text.to_string())
                        .fill(ui::c().input)
                        .corner_radius(8)
                        .min_size(vec2(0.0, 30.0))
                };
                if ui.add(button("Connect an agent")).clicked() {
                    self.tour = Some(tour::Tour::connect());
                }
                if !running && ui.add(button("Turn on MCP")).clicked() {
                    self.open_settings();
                    self.settings_page = SettingsPage::Mcp;
                }
            });
        });
    }

    /// Right column: one card with two tabs, recent models and examples.
    /// Without recent models (often the first start), Examples is the tab
    /// that shows first.
    fn start_lists(&mut self, ui: &mut egui::Ui) {
        let mut tab = self.start_tab.unwrap_or(if self.recent.is_empty() {
            StartTab::Examples
        } else {
            StartTab::Recent
        });
        let mut pick = None;
        let mut example = None;
        ui::card(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            if ui::segmented(
                ui,
                &mut tab,
                &[
                    (StartTab::Recent, "Recent"),
                    (StartTab::Examples, "Examples"),
                ],
            ) {
                self.start_tab = Some(tab);
            }
            ui.add_space(10.0);
            // The card keeps its height when the tab changes.
            let rows = self.recent.len().clamp(1, RECENT_MAX) as f32;
            let examples = examples::MENU.len().div_ceil(2) as f32;
            ui.set_min_height(
                ui.cursor().top() - ui.max_rect().top()
                    + (rows * RECENT_ROW).max(examples * EXAMPLE_ROW),
            );
            match tab {
                StartTab::Recent if self.recent.is_empty() => {
                    ui.add_space(6.0);
                    ui.label(ui::dim(
                        "No recent models yet. The models that you open show here.",
                    ));
                }
                StartTab::Recent => {
                    for path in self.recent.iter().take(RECENT_MAX) {
                        let card = self.start_cache.card(ui.ctx(), path, None);
                        if recent_row(ui, path, &card) {
                            pick = Some(path.clone());
                        }
                    }
                }
                StartTab::Examples => {
                    let w = (ui.available_width() - 4.0) / 2.0;
                    for pair in examples::MENU.chunks(2) {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            for (label, file) in pair {
                                let path = examples::path(file);
                                let card = path.as_deref().map(|p| {
                                    self.start_cache.card(ui.ctx(), p, examples::thumb(file))
                                });
                                let texture = card.as_ref().and_then(|c| c.texture);
                                if example_row(ui, label, w, texture) {
                                    example = Some(*file);
                                }
                            }
                        });
                    }
                }
            }
        });
        if let Some(path) = pick {
            self.open(path);
        }
        if let Some(file) = example {
            self.open_example(file);
        }
    }
}

/// The tabs of the start page card.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum StartTab {
    Recent,
    Examples,
}

/// Row heights of the two tabs.
const RECENT_ROW: f32 = 56.0;
const EXAMPLE_ROW: f32 = 48.0;

/// The thumbnail of a model in a square at the left of `row`, or an empty
/// square when the model has none yet. Returns the right edge of the square.
fn thumb_box(ui: &egui::Ui, row: egui::Rect, texture: Option<&egui::TextureHandle>) -> f32 {
    let side = row.height() - 8.0;
    let rect = egui::Rect::from_min_size(row.min + vec2(4.0, 4.0), Vec2::splat(side));
    let p = ui.painter();
    p.rect_filled(rect, 6.0, ui::c().view[1]);
    if let Some(t) = texture {
        let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        p.image(t.id(), rect, uv, Color32::WHITE);
    }
    rect.right()
}

/// One example in the Examples tab: the thumbnail and the name.
fn example_row(
    ui: &mut egui::Ui,
    label: &str,
    width: f32,
    texture: Option<&egui::TextureHandle>,
) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, EXAMPLE_ROW), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 8.0, ui::c().input);
    }
    let x = thumb_box(ui, rect, texture);
    ui.painter().text(
        egui::pos2(x + 10.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(14.5),
        ui::c().text,
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

/// A large row with a title, a second line and a shortcut at the right.
/// `primary` gives the row the accent color.
fn action_row(
    ui: &mut egui::Ui,
    title: &str,
    detail: &str,
    shortcut: &str,
    primary: bool,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 52.0), Sense::click());
    let hot = resp.hovered();
    let (fill, fg, sub) = if primary {
        let fill = if hot {
            ui::ACCENT.gamma_multiply(0.88)
        } else {
            ui::ACCENT
        };
        (fill, Color32::BLACK, Color32::from_black_alpha(170))
    } else {
        let fill = if hot {
            ui::c().input_hover
        } else {
            ui::c().input
        };
        (fill, ui::c().text, ui::c().dim)
    };
    let p = ui.painter();
    p.rect_filled(rect, 10.0, fill);
    let x = rect.left() + 14.0;
    p.text(
        egui::pos2(x, rect.center().y - 9.0),
        Align2::LEFT_CENTER,
        title,
        FontId::proportional(15.0),
        fg,
    );
    p.text(
        egui::pos2(x, rect.center().y + 10.0),
        Align2::LEFT_CENTER,
        detail,
        FontId::proportional(12.5),
        sub,
    );
    p.text(
        rect.right_center() - vec2(14.0, 0.0),
        Align2::RIGHT_CENTER,
        shortcut,
        FontId::monospace(11.5),
        sub,
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Row for a recent model: the thumbnail, the title and the description.
/// Without a title, the row shows the file name. Without a description, it
/// shows the folder. A missing file cannot be picked.
fn recent_row(ui: &mut egui::Ui, path: &Path, card: &thumb::Card) -> bool {
    let exists = path.exists();
    let sense = if exists {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), RECENT_ROW), sense);
    if exists && resp.hovered() {
        ui.painter().rect_filled(rect, 8.0, ui::c().input);
    }
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let dir = path
        .parent()
        .map(|d| d.display().to_string())
        .unwrap_or_default();
    let (fg, sub) = if exists {
        (ui::c().text, ui::c().dim)
    } else {
        (ui::c().dim, ui::c().dim.gamma_multiply(0.7))
    };
    let x = thumb_box(ui, rect, card.texture) + 10.0;
    let max = rect.right() - 10.0 - x;
    let p = ui.painter();
    let title = one_line(
        ui,
        card.title.unwrap_or(&name),
        FontId::proportional(14.5),
        max,
    );
    p.galley(
        egui::pos2(x, rect.center().y - 2.0 - title.size().y),
        title,
        fg,
    );
    let dir = if !exists {
        format!("Not found: {dir}")
    } else if let Some(d) = card.description {
        // The first line of the description.
        d.lines().next().unwrap_or_default().to_string()
    } else {
        dir
    };
    let folder = one_line(ui, &dir, FontId::proportional(12.0), max);
    p.galley(egui::pos2(x, rect.center().y + 1.0), folder, sub);
    let resp = resp.on_hover_text(path.display().to_string());
    if exists {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
    } else {
        false
    }
}

/// One line of text, cut at `max` with an ellipsis.
fn one_line(ui: &egui::Ui, text: &str, font: FontId, max: f32) -> std::sync::Arc<egui::Galley> {
    let mut job =
        egui::text::LayoutJob::simple_singleline(text.to_owned(), font, Color32::PLACEHOLDER);
    job.wrap = egui::text::TextWrapping::truncate_at_width(max);
    ui.fonts(|f| f.layout_job(job))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_has_parameters() {
        let params = crate::params::parse(TEMPLATE);
        let names: Vec<_> = params.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["width", "depth", "height", "radius"]);
    }

    #[test]
    fn write_new_adds_the_extension() {
        let dir = std::env::temp_dir().join(format!("cocad-new-{}", std::process::id()));
        let path = write_new(&dir.join("box"), None).unwrap();
        assert_eq!(path, dir.join("box.scad"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), TEMPLATE);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
