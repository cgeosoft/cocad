//! Theme and small widgets: cards, pills, toggle switch, segmented control.

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontFamily, FontId, Margin, Pos2, Rect, RichText, Sense,
    Stroke, TextStyle, vec2,
};
use std::sync::Arc;

pub const ACCENT: Color32 = Color32::from_rgb(0xff, 0x93, 0x00);
pub const ACCENT_DIM: Color32 = Color32::from_rgb(0x8a, 0x52, 0x08);
pub const BG: Color32 = Color32::from_rgb(0x0f, 0x11, 0x14);
pub const PANEL: Color32 = Color32::from_rgb(0x15, 0x17, 0x1b);
pub const CARD: Color32 = Color32::from_rgb(0x1c, 0x1f, 0x24);
pub const INPUT: Color32 = Color32::from_rgb(0x26, 0x2a, 0x31);
pub const INPUT_HOVER: Color32 = Color32::from_rgb(0x2f, 0x34, 0x3c);
pub const LINE: Color32 = Color32::from_rgb(0x2a, 0x2e, 0x35);
pub const TEXT: Color32 = Color32::from_rgb(0xe8, 0xea, 0xed);
pub const DIM: Color32 = Color32::from_rgb(0x8d, 0x93, 0x9c);
pub const DANGER: Color32 = Color32::from_rgb(0xff, 0x6b, 0x6b);

/// System UI fonts to try, in order. The egui fonts stay as fallback for symbols.
const FONTS: &[&str] = &[
    "/usr/share/fonts/truetype/inter/Inter-Regular.ttf",
    "/usr/share/fonts/opentype/inter/Inter-Regular.otf",
    "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
    "/usr/share/fonts/opentype/cantarell/Cantarell-VF.otf",
];

pub fn theme(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    if let Some(bytes) = FONTS.iter().find_map(|p| std::fs::read(p).ok()) {
        fonts
            .font_data
            .insert("ui".into(), Arc::new(egui::FontData::from_owned(bytes)));
        fonts
            .families
            .get_mut(&FontFamily::Proportional)
            .unwrap()
            .insert(0, "ui".into());
    }
    ctx.set_fonts(fonts);

    let r = |n: u8| CornerRadius::same(n);
    let mut v = egui::Visuals::dark();
    v.panel_fill = PANEL;
    v.window_fill = CARD;
    v.window_stroke = Stroke::new(1.0_f32, LINE);
    v.window_corner_radius = r(14);
    v.menu_corner_radius = r(10);
    v.window_shadow = egui::Shadow {
        offset: [0, 10],
        blur: 30,
        spread: 0,
        color: Color32::from_black_alpha(140),
    };
    v.popup_shadow = egui::Shadow {
        offset: [0, 6],
        blur: 18,
        spread: 0,
        color: Color32::from_black_alpha(120),
    };
    v.extreme_bg_color = INPUT;
    v.faint_bg_color = CARD;
    v.override_text_color = Some(TEXT);
    v.selection.bg_fill = ACCENT_DIM;
    v.selection.stroke = Stroke::new(1.0_f32, ACCENT);
    v.hyperlink_color = ACCENT;
    v.slider_trailing_fill = true;
    v.handle_shape = egui::style::HandleShape::Circle;
    v.striped = false;

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = CARD;
    w.noninteractive.weak_bg_fill = CARD;
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32, LINE);
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    w.noninteractive.corner_radius = r(8);
    for (s, fill) in [
        (&mut w.inactive, INPUT),
        (&mut w.hovered, INPUT_HOVER),
        (&mut w.active, INPUT_HOVER),
        (&mut w.open, INPUT_HOVER),
    ] {
        s.bg_fill = fill;
        s.weak_bg_fill = fill;
        s.bg_stroke = Stroke::NONE;
        s.corner_radius = r(8);
        s.expansion = 0.0;
        s.fg_stroke = Stroke::new(1.5_f32, TEXT);
    }
    w.inactive.fg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(0xc8, 0xcc, 0xd2));
    w.hovered.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(0x44, 0x4a, 0x54));
    w.active.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    ctx.set_visuals(v);

    ctx.style_mut(|s| {
        use FontFamily::{Monospace, Proportional};
        s.text_styles = [
            (TextStyle::Small, FontId::new(12.0, Proportional)),
            (TextStyle::Body, FontId::new(14.5, Proportional)),
            (TextStyle::Button, FontId::new(14.5, Proportional)),
            (TextStyle::Heading, FontId::new(19.0, Proportional)),
            (TextStyle::Monospace, FontId::new(13.0, Monospace)),
        ]
        .into();
        let sp = &mut s.spacing;
        sp.item_spacing = vec2(10.0, 10.0);
        sp.button_padding = vec2(14.0, 7.0);
        sp.interact_size = vec2(40.0, 24.0);
        sp.combo_height = 300.0;
        sp.slider_rail_height = 5.0;
        sp.icon_width = 18.0;
        sp.icon_spacing = 8.0;
        sp.indent = 16.0;
        sp.window_margin = Margin::same(20);
        sp.menu_margin = Margin::same(8);
        sp.scroll = egui::style::ScrollStyle::floating();
        s.interaction.tooltip_delay = 0.35;
    });
}

/// Rounded surface with padding.
pub fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::NONE
        .fill(CARD)
        .corner_radius(12)
        .inner_margin(Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// Small upper-case section title.
pub fn section(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text.to_uppercase())
            .size(11.5)
            .color(DIM)
            .extra_letter_spacing(1.2),
    );
}

/// Clickable group title with a chevron and a count of edited values.
pub fn group_header(ui: &mut egui::Ui, title: &str, open: bool, edited: usize) -> egui::Response {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 20.0), Sense::click());
    let p = ui.painter();
    let color = if resp.hovered() { TEXT } else { DIM };
    let galley = p.layout_no_wrap(title.to_uppercase(), FontId::proportional(11.5), color);
    p.galley(
        egui::pos2(rect.left(), rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
    if edited > 0 {
        pill(
            p,
            egui::pos2(rect.right() - 26.0, rect.center().y),
            Align2::RIGHT_CENTER,
            &format!("{edited} edited"),
            FontId::proportional(11.0),
            ACCENT,
            Color32::from_rgba_unmultiplied(0xff, 0x93, 0x00, 30),
        );
    }
    // Chevron: down when open, right when closed.
    let c = egui::pos2(rect.right() - 8.0, rect.center().y);
    let pts = if open {
        vec![
            c + vec2(-5.0, -2.5),
            c + vec2(0.0, 2.5),
            c + vec2(5.0, -2.5),
        ]
    } else {
        vec![
            c + vec2(-2.5, -5.0),
            c + vec2(2.5, 0.0),
            c + vec2(-2.5, 5.0),
        ]
    };
    p.add(egui::Shape::line(pts, Stroke::new(1.6_f32, color)));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn dim(text: impl Into<String>) -> RichText {
    RichText::new(text).color(DIM).size(12.5)
}

/// Text on a rounded translucent background, drawn straight on a painter.
pub fn pill(
    painter: &egui::Painter,
    anchor: Pos2,
    align: Align2,
    text: &str,
    font: FontId,
    fg: Color32,
    bg: Color32,
) -> Rect {
    let galley = painter.layout_no_wrap(text.to_owned(), font, fg);
    let pad = vec2(14.0, 9.0);
    let rect = align.anchor_size(anchor, galley.size() + pad * 2.0);
    painter.rect_filled(rect, 10.0, bg);
    painter.galley(rect.min + pad, galley, fg);
    rect
}

pub fn glass() -> Color32 {
    Color32::from_rgba_unmultiplied(0x15, 0x17, 0x1b, 215)
}

/// iOS-style switch.
pub fn toggle(ui: &mut egui::Ui, on: &mut bool) -> egui::Response {
    let size = vec2(40.0, 22.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool_responsive(resp.id, *on);
    let off = if resp.hovered() { INPUT_HOVER } else { INPUT };
    let fill = lerp_color(off, ACCENT, t);
    let p = ui.painter();
    p.rect_filled(rect, 11.0, fill);
    let x = egui::lerp((rect.left() + 11.0)..=(rect.right() - 11.0), t);
    p.circle_filled(egui::pos2(x, rect.center().y), 8.0, Color32::WHITE);
    resp
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// A row of options in one rounded track; returns true when the value changed.
pub fn segmented<T: PartialEq + Copy>(
    ui: &mut egui::Ui,
    value: &mut T,
    options: &[(T, &str)],
) -> bool {
    let mut changed = false;
    egui::Frame::NONE
        .fill(INPUT)
        .corner_radius(10)
        .inner_margin(Margin::same(3))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.horizontal(|ui| {
                for (v, label) in options {
                    let sel = *value == *v;
                    let text = RichText::new(*label).color(if sel { Color32::BLACK } else { DIM });
                    let b = egui::Button::new(text)
                        .fill(if sel { ACCENT } else { Color32::TRANSPARENT })
                        .corner_radius(8)
                        .min_size(vec2(76.0, 28.0));
                    if ui.add(b).clicked() && !sel {
                        *value = *v;
                        changed = true;
                    }
                }
            });
        });
    changed
}

/// Filled accent button.
pub fn primary(text: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.to_string()).color(Color32::BLACK))
        .fill(ACCENT)
        .corner_radius(8)
}

/// Borderless icon-ish button for toolbars.
pub fn ghost(text: &str, active: bool) -> egui::Button<'static> {
    let color = if active { ACCENT } else { TEXT };
    egui::Button::new(RichText::new(text.to_string()).color(color))
        .fill(if active {
            Color32::from_rgba_unmultiplied(0xff, 0x93, 0x00, 28)
        } else {
            Color32::TRANSPARENT
        })
        .corner_radius(8)
}
