//! The info window of the print fixes of the Optimize tab. The info button
//! after the name of a fix opens it. It tells what the fix does, why it is
//! necessary and when to use it, with a picture of the desk dock example
//! before and after the fix. The pictures are in `assets/fixes/` (see
//! `scripts/showcase-shots.sh`).

use super::App;
use crate::ui;
use eframe::egui::{self, RichText, Vec2};

const MARGIN: f32 = 18.0;

pub struct Fix {
    /// The name of the fix, as in the Print fixes card.
    pub title: &'static str,
    what: &'static str,
    why: &'static str,
    when: &'static str,
    image: &'static [u8],
}

macro_rules! picture {
    ($name:literal) => {
        include_bytes!(concat!("../../assets/fixes/", $name, ".png"))
    };
}

/// The fixes in the order of the Print fixes card.
pub const FIXES: [Fix; 6] = [
    Fix {
        title: "Teardrop holes",
        what: "The top of each round hole with a horizontal axis gets a 45 degree point, as a teardrop. The bottom and the sides of the hole stay round.",
        why: "The top of a horizontal hole is a ceiling in the air. It sags, or it needs support. A slope of 45 degrees prints without support.",
        when: "Use it for horizontal holes for cables, screws and pins, from 1 to 40 mm in diameter. A shaft in the hole touches only the bottom and the sides. When the wall above a hole is too thin for the point, CoCAD does not change the hole.",
        image: picture!("teardrops"),
    },
    Fix {
        title: "Hole size",
        what: "Each round hole with a vertical axis, from 1 to 40 mm in diameter, gets the diameter of the model plus the value Add to diameter, 0.2 mm by default. A hole with six faces can be a pocket for a nut: it only gets the value. A blind hole does not get deeper.",
        why: "A vertical hole prints smaller than in the model. OpenSCAD makes a hole as a polygon with its corners on the circle, so the flat faces are inside the circle. The printer also makes holes smaller. A screw or a shaft then does not go in.",
        when: "Use it when screws, pins or shafts must go into vertical holes. Print a test part, measure the hole, and set the value. Set the X-Y hole compensation of the slicer to 0. Without this, the holes get two corrections.",
        image: picture!("hole-size"),
    },
    Fix {
        title: "Bridges",
        what: "The last layer under a round ceiling gets a square outline, so the bridge starts on straight edges. A hole above a bridge gets sacrificial layers, or a stepped bridge: a slot, then a square, then the hole. A blind hole that goes up gets a 45 degree cone at its end.",
        why: "Over an empty space, the printer pulls strands from one edge to the opposite edge. From a straight edge, a bridge prints well. From a round edge, the strands start in the air and sag.",
        when: "Use it for holes and pockets on the bed side, for counterbores of screw heads and nuts, and for blind holes that go up. With Sacrificial, drill the hole open after the print. With Stepped, you do no work after the print. A bridge longer than 12 mm does not change: the Check card lists it.",
        image: picture!("bridges"),
    },
    Fix {
        title: "Bed chamfer",
        what: "The edges on the bed get a 45 degree chamfer. The chamfer is a staircase of whole layers: with layers of 0.2 mm, a chamfer of 0.4 mm has two steps.",
        why: "The nozzle presses the first layers onto the bed, so they spread out. This is the elephant foot. The part is then wider at the bed than above it. Parts that go together are too tight, and the bottom edge is not clean.",
        when: "Use it on all parts that stand on a flat face, and always for parts that must fit together. Set the elephant foot compensation of the slicer to 0. Without this, the bed edges get two corrections.",
        image: picture!("bed-chamfer"),
    },
    Fix {
        title: "Brim on small islands",
        what: "Each small area of the first layer gets a brim of one layer, 5 mm wide by default. A small area is thinner than 3 mm or smaller than 25 mm². The brim keeps a gap of 1 mm from the rest of the part.",
        why: "A thin line of the first layer has little contact with the bed. It can come loose during the print.",
        when: "Use it for thin feet, thin walls on the bed and small pins that stand on the bed. After the print, cut the brim off the part.",
        image: picture!("brim"),
    },
    Fix {
        title: "Mouse ears",
        what: "Each sharp corner of a large area of the first layer gets a disk of one layer, 15 mm in diameter by default. A large area has a long side of 100 mm or more. The center of the disk is on the corner. The disks do not go into the holes of the part.",
        why: "When the plastic cools, a large flat part can lift from the bed at its corners. A disk gives the corner more area on the bed.",
        when: "Use it for large plates, lids and frames with sharp corners. After the print, cut the disks off the part. Cut them off before you join the pieces of a split part: a disk at the cut is in the way of the joint.",
        image: picture!("mouse-ears"),
    },
];

/// The index in `FIXES` of the fix with this title.
pub fn index(title: &str) -> Option<usize> {
    FIXES.iter().position(|f| f.title == title)
}

/// The info window while it is open.
pub struct FixInfo {
    /// The index in `FIXES`.
    pub index: usize,
    image: Option<(usize, egui::TextureHandle)>,
}

impl FixInfo {
    pub fn new(index: usize) -> FixInfo {
        FixInfo {
            index: index.min(FIXES.len() - 1),
            image: None,
        }
    }

    fn texture(&mut self, ctx: &egui::Context) -> Option<egui::TextureHandle> {
        if self.image.as_ref().is_none_or(|(i, _)| *i != self.index) {
            let (size, rgba) = decode(FIXES[self.index].image).ok()?;
            let options = egui::TextureOptions {
                mipmap_mode: Some(egui::TextureFilter::Linear),
                ..egui::TextureOptions::LINEAR
            };
            let image = egui::ColorImage::from_rgba_unmultiplied(size, &rgba);
            let tex = ctx.load_texture("fix-info", image, options);
            self.image = Some((self.index, tex));
        }
        self.image.as_ref().map(|(_, t)| t.clone())
    }
}

/// Decodes a PNG with 8 bits for each channel: the size and the RGBA pixels.
fn decode(bytes: &[u8]) -> Result<([usize; 2], Vec<u8>), String> {
    let mut dec = png::Decoder::new(bytes);
    dec.set_transformations(png::Transformations::EXPAND);
    let mut reader = dec.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    if info.bit_depth != png::BitDepth::Eight {
        return Err(format!("{:?} bits", info.bit_depth));
    }
    buf.truncate(info.buffer_size());
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|&[r, g, b]| [r, g, b, 255])
            .collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::GrayscaleAlpha => buf
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|&[g, a]| [g, g, g, a])
            .collect(),
        other => return Err(format!("{other:?}")),
    };
    Ok(([info.width as usize, info.height as usize], rgba))
}

impl App {
    pub(super) fn fix_info_window(&mut self, ctx: &egui::Context) {
        let Some(info) = &mut self.fix_info else {
            return;
        };
        let last = FIXES.len() - 1;
        let mut index = info.index;
        let mut close = false;
        ctx.input(|i| {
            close |= i.key_pressed(egui::Key::Escape);
            if i.key_pressed(egui::Key::ArrowLeft) {
                index = index.saturating_sub(1);
            }
            if i.key_pressed(egui::Key::ArrowRight) {
                index = (index + 1).min(last);
            }
        });
        let texture = info.texture(ctx);
        let fix = &FIXES[info.index];

        let screen = ctx.screen_rect().size();
        let width = (screen.x - 48.0).clamp(420.0, 780.0) - 2.0 * MARGIN;
        let image_height = texture.as_ref().map_or(0.0, |t| {
            let [w, h] = t.size();
            (width * h as f32 / w as f32).min(screen.y * 0.45)
        });
        let text_height = (screen.y - image_height - 200.0).max(120.0);
        let frame = egui::Frame::NONE
            .fill(ui::c().panel)
            .corner_radius(14)
            .inner_margin(egui::Margin::same(MARGIN as i8))
            .stroke(egui::Stroke::new(1.0_f32, ui::c().line));
        egui::Modal::new(egui::Id::new("fix_info"))
            .frame(frame)
            .show(ctx, |ui| {
                ui.set_width(width);
                ui.horizontal(|ui| {
                    ui.heading(fix.title);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let x = RichText::new("×").size(20.0).color(ui::c().dim);
                        close |= ui
                            .add(egui::Button::new(x).frame(false))
                            .on_hover_text("Close (Esc)")
                            .clicked();
                        let count = format!("Print fix {} of {}", info.index + 1, FIXES.len());
                        ui.label(ui::dim(count).size(12.5));
                    });
                });
                ui.add_space(12.0);
                if let Some(tex) = &texture {
                    let [w, h] = tex.size();
                    let fit = Vec2::new(w as f32, h as f32)
                        * (width / w as f32).min(image_height / h as f32);
                    let (rect, _) = ui
                        .allocate_exact_size(Vec2::new(width, image_height), egui::Sense::hover());
                    let rect = egui::Rect::from_center_size(rect.center(), fit);
                    let image = egui::Image::new(tex)
                        .fit_to_exact_size(fit)
                        .corner_radius(8);
                    ui.put(rect, image);
                    ui.add_space(12.0);
                }
                egui::ScrollArea::vertical()
                    .id_salt(info.index)
                    .max_height(text_height)
                    .show(ui, |ui| {
                        ui.set_width(width);
                        ui.spacing_mut().item_spacing.y = 4.0;
                        for (head, text) in [
                            ("What it does", fix.what),
                            ("Why", fix.why),
                            ("When to use it", fix.when),
                        ] {
                            ui.label(ui::dim(head).size(12.5).strong());
                            ui.add(egui::Label::new(text).wrap());
                            ui.add_space(8.0);
                        }
                    });
                ui.add_space(8.0);
                let row = Vec2::new(ui.available_width(), 34.0);
                let rtl = egui::Layout::right_to_left(egui::Align::Center);
                ui.allocate_ui_with_layout(row, rtl, |ui| {
                    ui.spacing_mut().button_padding = Vec2::new(16.0, 7.0);
                    close |= ui.add(ui::primary("Close")).clicked();
                    if ui
                        .add_enabled(info.index < last, ui::ghost("Next", false))
                        .clicked()
                    {
                        index = info.index + 1;
                    }
                    if ui
                        .add_enabled(info.index > 0, ui::ghost("Back", false))
                        .clicked()
                    {
                        index = info.index - 1;
                    }
                });
            });
        if close {
            self.fix_info = None;
        } else if let Some(info) = &mut self.fix_info {
            info.index = index;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_fix_has_a_picture_that_decodes() {
        for f in &FIXES {
            let ([w, h], rgba) = decode(f.image).unwrap_or_else(|e| panic!("{}: {e}", f.title));
            assert_eq!(rgba.len(), w * h * 4, "{}", f.title);
            // Small enough for any GPU texture, and not tiny.
            assert!(
                (600..=2048).contains(&w) && h >= 200,
                "{}: {w} x {h}",
                f.title
            );
        }
    }

    #[test]
    fn index_finds_the_card_names() {
        for (k, f) in FIXES.iter().enumerate() {
            assert_eq!(index(f.title), Some(k));
        }
        assert_eq!(index("No such fix"), None);
    }
}
