//! Welcome tour: a modal wizard that shows the main features, one step at a
//! time. It opens by itself at the first start. Help > Welcome tour opens it
//! again. The screenshots are in `assets/tour/` (see `scripts/tour-shots.sh`).

use super::{App, card_header};
use crate::ui;
use eframe::egui::{self, RichText, Vec2};
use serde_json::json;
use std::path::{Path, PathBuf};
use zune_core::bytestream::ZCursor;
use zune_jpeg::JpegDecoder;

const TEXT_WIDTH: f32 = 300.0;
const MARGIN: f32 = 18.0;

struct Step {
    title: &'static str,
    intro: &'static str,
    points: &'static [&'static str],
    image: &'static [u8],
}

macro_rules! shot {
    ($name:literal) => {
        include_bytes!(concat!("../../assets/tour/", $name, ".jpg"))
    };
}

/// The step that also shows how to connect an MCP client.
const CONNECT: usize = 8;

const STEPS: [Step; 10] = [
    Step {
        title: "Welcome to CoCAD",
        intro: "View and change OpenSCAD models. OpenSCAD does the geometry in the background.",
        points: &[
            "Press Ctrl+O to open a .scad file, or drop it on the window.",
            "The start view and the File menu list your recent models.",
            "File > Examples opens a sample model.",
            "When the file changes on disk, the model reloads and keeps your values.",
            "Left drag orbits. Right drag pans. The wheel zooms.",
        ],
        image: shot!("1-welcome"),
    },
    Step {
        title: "Change the parameters",
        intro: "The left panel shows the Customizer parameters of the model.",
        points: &[
            "Sliders, dropdowns, switches, text and vectors are in collapsible groups.",
            "A parameter with a description comment shows the description as its label.",
            "The model builds again when you change a value.",
            "A dot marks a changed value. The arrow sets it back.",
        ],
        image: shot!("2-customizer"),
    },
    Step {
        title: "Save parameter sets",
        intro: "Keep named sets of values for one model.",
        points: &[
            "Select a set to load it. Model defaults resets all values.",
            "The disk button saves to the selected set. The gear menu makes a new set.",
            "The set that you used last loads when you open the model.",
            "The sets are in a .json file. OpenSCAD reads the same format.",
        ],
        image: shot!("3-sets"),
    },
    Step {
        title: "Check and export",
        intro: "The Model tab shows the mesh data and the export buttons.",
        points: &[
            "Check the size, volume, triangles and open edges.",
            "Back faces show in purple. Press W for the wireframe.",
            "Export binary STL, 3MF with the parameters, or STEP AP214.",
            "A model with a cocad_part list shows each part in its color and exports a multicolor 3MF.",
        ],
        image: shot!("4-export"),
    },
    Step {
        title: "See the parts",
        intro: "The Model tab finds the parts of the model. You can color them, move them apart and cut them.",
        points: &[
            "Part colors gives each part a different color. Colors… sets your own colors.",
            "Explode moves the parts apart. Press E.",
            "Section cuts the view on the X, Y and Z axes. You select the side to keep.",
            "Click a part to select it. Right click a part to focus on it or to change its color.",
        ],
        image: shot!("5-parts"),
    },
    Step {
        title: "Slice preview",
        intro: "The Optimize tab shows the paths a printer would follow. Press O to open it.",
        points: &[
            "Walls, infill and skin show as round strands at the real line width.",
            "Move the layer slider, or press Up and Down.",
            "The top card shows the print time, filament length and weight.",
            "The preview is for checks only. It writes no G-code.",
        ],
        image: shot!("6-slice"),
    },
    Step {
        title: "Photo render",
        intro: "The Render tab path-traces the current view on all CPU cores. Press R to open it.",
        points: &[
            "The image gets cleaner with each sample.",
            "Choose the plastic color, finish, studio and image size.",
            "Save PNG writes the image at its full size.",
        ],
        image: shot!("7-photo"),
    },
    Step {
        title: "Let an agent change the model",
        intro: "An AI agent can work with the open window through MCP. You see each change live.",
        points: &[
            "The agent reads the model, its parameters and its source.",
            "It changes values, builds the model, and gets the result as an image.",
            "It slices, exports, and saves photos and parameter sets.",
            "A message in the view names each change.",
            "It cannot delete a file or a parameter set.",
        ],
        image: shot!("8-agent"),
    },
    Step {
        title: "Connect an agent",
        intro: "Add CoCAD to your MCP client. The tool calls go to the open window.",
        points: &[],
        image: shot!("9-connect"),
    },
    Step {
        title: "Settings and shortcuts",
        intro: "You are ready to start.",
        points: &[
            "File > Settings sets the theme, auto save of sets, the OpenSCAD binary and your \
             privacy choices.",
            "An OpenSCAD development build with Manifold is much faster.",
            "Help > About and shortcuts lists all keys.",
            "The CLI exports, slices and makes photos without the window. Run cocad --help.",
            "Help > Welcome tour shows this tour again.",
        ],
        image: shot!("10-menu"),
    },
];

pub struct Tour {
    step: usize,
    image: Option<(usize, egui::TextureHandle)>,
    /// This program, for the MCP client config.
    binary: String,
    /// The text that was copied last, for the button label.
    copied: Option<String>,
}

impl Tour {
    pub fn new() -> Tour {
        Tour {
            step: 0,
            image: None,
            binary: binary_path(std::env::current_exe().ok()),
            copied: None,
        }
    }

    fn texture(&mut self, ctx: &egui::Context) -> Option<egui::TextureHandle> {
        if self.image.as_ref().is_none_or(|(s, _)| *s != self.step) {
            let (size, rgb) = decode(STEPS[self.step].image).ok()?;
            let options = egui::TextureOptions {
                mipmap_mode: Some(egui::TextureFilter::Linear),
                ..egui::TextureOptions::LINEAR
            };
            let image = egui::ColorImage::from_rgb(size, &rgb);
            let tex = ctx.load_texture("tour", image, options);
            self.image = Some((self.step, tex));
        }
        self.image.as_ref().map(|(_, t)| t.clone())
    }
}

/// True at the first start for this user. A later start finds the marker file,
/// also when the user closed the app in the middle of the tour.
pub fn first_start() -> bool {
    crate::openscad::config_dir().is_some_and(|d| claim(&d.join("tour_seen")))
}

fn claim(marker: &Path) -> bool {
    if marker.exists() {
        return false;
    }
    if let Some(dir) = marker.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(marker, "");
    true
}

fn binary_path(exe: Option<PathBuf>) -> String {
    exe.map_or("cocad".into(), |p| p.display().to_string())
}

fn client_config(binary: &str) -> String {
    let config = json!({
        "mcpServers": { "cocad": { "command": binary, "args": ["mcp"] } }
    });
    serde_json::to_string_pretty(&config).unwrap_or_default()
}

fn claude_code_command(binary: &str) -> String {
    format!("claude mcp add cocad -- \"{binary}\" mcp")
}

fn decode(jpeg: &[u8]) -> Result<([usize; 2], Vec<u8>), String> {
    let mut decoder = JpegDecoder::new(ZCursor::new(jpeg));
    let rgb = decoder.decode().map_err(|e| e.to_string())?;
    let (w, h) = decoder.dimensions().ok_or("no JPEG header")?;
    Ok(([w, h], rgb))
}

impl App {
    pub(super) fn tour_window(&mut self, ctx: &egui::Context) {
        let running = self.server.is_some();
        let Some(tour) = &mut self.tour else {
            return;
        };
        let last = STEPS.len() - 1;
        let mut step = tour.step;
        let mut close = false;
        let focused = ctx.memory(|m| m.focused().is_some());
        ctx.input(|i| {
            close |= i.key_pressed(egui::Key::Escape);
            if i.key_pressed(egui::Key::ArrowLeft) {
                step = step.saturating_sub(1);
            }
            if i.key_pressed(egui::Key::ArrowRight) {
                step = (step + 1).min(last);
            }
            if i.key_pressed(egui::Key::Enter) && !focused {
                close |= step == last;
                step = (step + 1).min(last);
            }
        });
        let texture = tour.texture(ctx);

        let screen = ctx.screen_rect().size();
        let width = (screen.x - 48.0).clamp(520.0, 940.0) - 2.0 * MARGIN;
        let image_width = width - TEXT_WIDTH - 20.0;
        let height = (image_width * 10.0 / 16.0).min((screen.y - 190.0).max(120.0));
        let frame = egui::Frame::NONE
            .fill(ui::c().panel)
            .corner_radius(14)
            .inner_margin(egui::Margin::same(MARGIN as i8))
            .stroke(egui::Stroke::new(1.0_f32, ui::c().line));
        let current = &STEPS[tour.step];
        egui::Modal::new(egui::Id::new("tour"))
            .frame(frame)
            .show(ctx, |ui| {
                ui.set_width(width);
                ui.horizontal(|ui| {
                    ui.heading(current.title);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let x = RichText::new("×").size(20.0).color(ui::c().dim);
                        close |= ui
                            .add(egui::Button::new(x).frame(false))
                            .on_hover_text("Close (Esc)")
                            .clicked();
                        let count = format!("Step {} of {}", tour.step + 1, STEPS.len());
                        ui.label(ui::dim(count).size(12.5));
                    });
                });
                ui.add_space(12.0);
                ui.horizontal_top(|ui| {
                    let (rect, _) = ui
                        .allocate_exact_size(Vec2::new(image_width, height), egui::Sense::hover());
                    if let Some(tex) = &texture {
                        let fit = Vec2::new(16.0, 10.0) * (image_width / 16.0).min(height / 10.0);
                        let rect = egui::Rect::from_center_size(rect.center(), fit);
                        let image = egui::Image::new(tex)
                            .fit_to_exact_size(fit)
                            .corner_radius(8);
                        ui.put(rect, image);
                    }
                    ui.add_space(20.0);
                    ui.vertical(|ui| {
                        ui.set_width(TEXT_WIDTH);
                        egui::ScrollArea::vertical()
                            .id_salt(tour.step)
                            .max_height(height)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.spacing_mut().item_spacing.y = 8.0;
                                ui.label(current.intro);
                                ui.add_space(4.0);
                                for point in current.points {
                                    bullet(ui, point);
                                }
                                if tour.step == CONNECT {
                                    connect(ui, running, &tour.binary, &mut tour.copied);
                                }
                            });
                    });
                });
                ui.add_space(14.0);
                let row = Vec2::new(ui.available_width(), 34.0);
                let rtl = egui::Layout::right_to_left(egui::Align::Center);
                ui.allocate_ui_with_layout(row, rtl, |ui| {
                    ui.spacing_mut().button_padding = Vec2::new(16.0, 7.0);
                    if tour.step == last {
                        close |= ui.add(ui::primary("Done")).clicked();
                    } else if ui.add(ui::primary("Next")).clicked() {
                        step = tour.step + 1;
                    }
                    if ui
                        .add_enabled(tour.step > 0, ui::ghost("Back", false))
                        .clicked()
                    {
                        step = tour.step - 1;
                    }
                    if tour.step != last {
                        close |= ui.add(ui::ghost("Skip", false)).clicked();
                    }
                    let ltr = egui::Layout::left_to_right(egui::Align::Center);
                    ui.with_layout(ltr, |ui| {
                        if let Some(n) = dots(ui, tour.step) {
                            step = n;
                        }
                    });
                });
            });
        if close {
            self.tour = None;
        } else if let Some(tour) = &mut self.tour {
            if step != tour.step {
                tour.copied = None;
            }
            tour.step = step;
        }
    }
}

fn bullet(ui: &mut egui::Ui, text: &str) {
    ui.horizontal_top(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(12.0, 16.0), egui::Sense::hover());
        let dot = egui::pos2(rect.left() + 3.0, rect.center().y);
        ui.painter().circle_filled(dot, 2.5, ui::ACCENT);
        ui.add(egui::Label::new(text).wrap());
    });
}

/// Step indicator. Returns the step of the dot that the user clicked.
fn dots(ui: &mut egui::Ui, current: usize) -> Option<usize> {
    let mut picked = None;
    ui.spacing_mut().item_spacing.x = 2.0;
    for (n, step) in STEPS.iter().enumerate() {
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(14.0), egui::Sense::click());
        let response = response.on_hover_text(step.title);
        let color = match (n == current, response.hovered()) {
            (true, _) => ui::ACCENT,
            (false, true) => ui::c().text,
            (false, false) => ui::c().line,
        };
        ui.painter().circle_filled(rect.center(), 4.0, color);
        if response.clicked() {
            picked = Some(n);
        }
    }
    picked
}

/// MCP state, and the texts to paste in a client.
fn connect(ui: &mut egui::Ui, running: bool, binary: &str, copied: &mut Option<String>) {
    let (color, state) = if running {
        (ui::GOOD, "Running")
    } else {
        (ui::c().dim, "Off")
    };
    card_header(ui, "MCP server", color, state);
    if !running {
        ui.label(ui::dim("Turn it on in File > Settings."));
    }
    ui.add_space(2.0);
    code_block(
        ui,
        "Claude Desktop and other clients",
        &client_config(binary),
        copied,
    );
    code_block(ui, "Claude Code", &claude_code_command(binary), copied);
}

fn code_block(ui: &mut egui::Ui, title: &str, code: &str, copied: &mut Option<String>) {
    ui.horizontal(|ui| {
        ui.label(ui::dim(title).size(12.5));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let done = copied.as_deref() == Some(code);
            let label = if done { "Copied" } else { "Copy" };
            if ui.add(ui::ghost(label, done)).clicked() {
                ui.ctx().copy_text(code.to_string());
                *copied = Some(code.to_string());
            }
        });
    });
    egui::Frame::NONE
        .fill(ui::c().card)
        .corner_radius(8)
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let text = RichText::new(code).monospace().size(11.5);
            ui.add(egui::Label::new(text).wrap().selectable(true));
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screenshots_decode_at_the_same_shape() {
        for step in &STEPS {
            let ([w, h], rgb) = decode(step.image).unwrap();
            assert_eq!(w * 10, h * 16, "{}", step.title);
            assert_eq!(rgb.len(), w * h * 3, "{}", step.title);
        }
    }

    #[test]
    fn connect_step_is_about_mcp() {
        assert_eq!(STEPS[CONNECT].title, "Connect an agent");
        assert!(STEPS[CONNECT].points.is_empty());
    }

    #[test]
    fn marker_is_claimed_once() {
        let dir = std::env::temp_dir().join(format!("scad-tour-{}", std::process::id()));
        let marker = dir.join("sub").join("tour_seen");
        assert!(claim(&marker));
        assert!(!claim(&marker));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn client_config_escapes_the_path() {
        let text = client_config("C:\\Apps\\cocad.exe");
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        let server = &v["mcpServers"]["cocad"];
        assert_eq!(server["command"], "C:\\Apps\\cocad.exe");
        assert_eq!(server["args"][0], "mcp");
    }

    #[test]
    fn claude_code_command_quotes_the_path() {
        assert_eq!(
            claude_code_command("/Apps/CoCAD.app/cocad"),
            "claude mcp add cocad -- \"/Apps/CoCAD.app/cocad\" mcp"
        );
    }
}
