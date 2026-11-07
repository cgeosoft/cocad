//! Theme and small widgets: cards, pills, toggle switch, segmented control.

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontFamily, FontId, Margin, Pos2, Rect, RichText, Sense,
    Stroke, TextStyle, vec2,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub const ACCENT: Color32 = Color32::from_rgb(0xff, 0x93, 0x00);
/// Status dots: all is well.
pub const GOOD: Color32 = Color32::from_rgb(0x3f, 0xb9, 0x50);

/// Colors of one theme.
pub struct Palette {
    pub bg: Color32,
    pub panel: Color32,
    pub card: Color32,
    pub input: Color32,
    pub input_hover: Color32,
    pub line: Color32,
    pub text: Color32,
    pub dim: Color32,
    pub danger: Color32,
    pub selection: Color32,
    pub glass: Color32,
    /// Viewport gradient, top and bottom.
    pub view: [Color32; 2],
}

const DARK: Palette = Palette {
    bg: Color32::from_rgb(0x0f, 0x11, 0x14),
    panel: Color32::from_rgb(0x15, 0x17, 0x1b),
    card: Color32::from_rgb(0x1c, 0x1f, 0x24),
    input: Color32::from_rgb(0x26, 0x2a, 0x31),
    input_hover: Color32::from_rgb(0x2f, 0x34, 0x3c),
    line: Color32::from_rgb(0x2a, 0x2e, 0x35),
    text: Color32::from_rgb(0xe8, 0xea, 0xed),
    dim: Color32::from_rgb(0x8d, 0x93, 0x9c),
    danger: Color32::from_rgb(0xff, 0x6b, 0x6b),
    selection: Color32::from_rgb(0x8a, 0x52, 0x08),
    glass: Color32::from_rgba_premultiplied(0x12, 0x14, 0x17, 215),
    view: [
        Color32::from_rgb(0x26, 0x29, 0x30),
        Color32::from_rgb(0x0e, 0x0f, 0x12),
    ],
};

/// Soft light theme: mid-light greys, no pure white, so it does not glare.
const LIGHT: Palette = Palette {
    bg: Color32::from_rgb(0xd9, 0xdc, 0xe1),
    panel: Color32::from_rgb(0xe3, 0xe6, 0xea),
    card: Color32::from_rgb(0xed, 0xef, 0xf2),
    input: Color32::from_rgb(0xd6, 0xda, 0xe0),
    input_hover: Color32::from_rgb(0xcc, 0xd1, 0xd8),
    line: Color32::from_rgb(0xcb, 0xd0, 0xd7),
    text: Color32::from_rgb(0x22, 0x26, 0x2c),
    dim: Color32::from_rgb(0x5f, 0x66, 0x70),
    danger: Color32::from_rgb(0xc0, 0x36, 0x36),
    selection: Color32::from_rgb(0xf3, 0xc6, 0x8c),
    glass: Color32::from_rgba_premultiplied(0xd8, 0xdb, 0xe0, 225),
    view: [
        Color32::from_rgb(0xd2, 0xd6, 0xdc),
        Color32::from_rgb(0xa9, 0xb0, 0xba),
    ],
};

static IS_DARK: AtomicBool = AtomicBool::new(true);

pub fn is_dark() -> bool {
    IS_DARK.load(Ordering::Relaxed)
}

thread_local! {
    static FORCE_DARK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The palette of the current theme (dark inside `dark_scope`).
pub fn c() -> &'static Palette {
    if is_dark() || FORCE_DARK.with(|f| f.get()) {
        &DARK
    } else {
        &LIGHT
    }
}

/// Draws `add` with the dark theme, whatever the app theme is. For the header.
pub fn dark_scope<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.style_mut().visuals = visuals(true);
    FORCE_DARK.with(|f| f.set(true));
    let r = add(ui);
    FORCE_DARK.with(|f| f.set(false));
    r
}

/// Dark-theme colors, for parts that stay dark in both themes.
pub fn dark() -> &'static Palette {
    &DARK
}

/// System UI fonts to try, in order. The egui fonts stay as fallback for symbols.
const FONTS: &[&str] = &[
    "/usr/share/fonts/truetype/inter/Inter-Regular.ttf",
    "/usr/share/fonts/opentype/inter/Inter-Regular.otf",
    "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
    "/usr/share/fonts/opentype/cantarell/Cantarell-VF.otf",
];

/// Loads the UI font. Call once.
pub fn fonts(ctx: &egui::Context) {
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
}

/// Applies the dark or light theme.
pub fn apply(ctx: &egui::Context, dark: bool) {
    IS_DARK.store(dark, Ordering::Relaxed);
    ctx.set_theme(if dark {
        egui::ThemePreference::Dark
    } else {
        egui::ThemePreference::Light
    });
    ctx.set_visuals(visuals(dark));
    style(ctx);
}

/// Widget colors for one theme.
pub fn visuals(dark: bool) -> egui::Visuals {
    let p = if dark { &DARK } else { &LIGHT };
    let r = |n: u8| CornerRadius::same(n);
    let mut v = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    v.panel_fill = p.panel;
    v.window_fill = p.card;
    v.window_stroke = Stroke::new(1.0_f32, p.line);
    v.window_corner_radius = r(14);
    v.menu_corner_radius = r(10);
    v.window_shadow = egui::Shadow {
        offset: [0, 10],
        blur: 30,
        spread: 0,
        color: Color32::from_black_alpha(if dark { 140 } else { 40 }),
    };
    v.popup_shadow = egui::Shadow {
        offset: [0, 6],
        blur: 18,
        spread: 0,
        color: Color32::from_black_alpha(if dark { 120 } else { 35 }),
    };
    v.extreme_bg_color = p.input;
    v.faint_bg_color = p.card;
    v.override_text_color = Some(p.text);
    v.selection.bg_fill = p.selection;
    v.selection.stroke = Stroke::new(1.0_f32, ACCENT);
    v.hyperlink_color = ACCENT;
    v.slider_trailing_fill = true;
    v.handle_shape = egui::style::HandleShape::Circle;
    v.striped = false;

    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.card;
    w.noninteractive.weak_bg_fill = p.card;
    w.noninteractive.bg_stroke = Stroke::new(1.0_f32, p.line);
    w.noninteractive.fg_stroke = Stroke::new(1.0_f32, p.text);
    w.noninteractive.corner_radius = r(8);
    for (s, fill) in [
        (&mut w.inactive, p.input),
        (&mut w.hovered, p.input_hover),
        (&mut w.active, p.input_hover),
        (&mut w.open, p.input_hover),
    ] {
        s.bg_fill = fill;
        s.weak_bg_fill = fill;
        s.bg_stroke = Stroke::NONE;
        s.corner_radius = r(8);
        s.expansion = 0.0;
        s.fg_stroke = Stroke::new(1.5_f32, p.text);
    }
    w.inactive.fg_stroke = Stroke::new(
        1.0_f32,
        if dark {
            Color32::from_rgb(0xc8, 0xcc, 0xd2)
        } else {
            Color32::from_rgb(0x3a, 0x40, 0x48)
        },
    );
    w.hovered.bg_stroke = Stroke::new(
        1.0_f32,
        if dark {
            Color32::from_rgb(0x44, 0x4a, 0x54)
        } else {
            Color32::from_rgb(0xc4, 0xc9, 0xd1)
        },
    );
    w.active.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    v
}

fn style(ctx: &egui::Context) {
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
        .fill(c().card)
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
            .color(c().dim)
            .extra_letter_spacing(1.2),
    );
}

/// Clickable group title with a chevron and a count of edited values.
pub fn group_header(ui: &mut egui::Ui, title: &str, open: bool, edited: usize) -> egui::Response {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 20.0), Sense::click());
    let p = ui.painter();
    let color = if resp.hovered() { c().text } else { c().dim };
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
    RichText::new(text).color(c().dim).size(12.5)
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
    c().glass
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
    let off = if resp.hovered() {
        c().input_hover
    } else {
        c().input
    };
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
        .fill(c().input)
        .corner_radius(10)
        .inner_margin(Margin::same(3))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.horizontal(|ui| {
                for (v, label) in options {
                    let sel = *value == *v;
                    let text =
                        RichText::new(*label).color(if sel { Color32::BLACK } else { c().dim });
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

/// Vertical slider for `0..count`, drawn centred in `rect`. Bottom is 0.
/// Drag or click to set; the mouse wheel over it steps one value.
pub fn vertical_slider(ui: &mut egui::Ui, rect: Rect, value: usize, count: usize) -> usize {
    let max = count.saturating_sub(1);
    let mut value = value.min(max);
    let resp = ui.interact(rect, ui.id().with("vslider"), Sense::click_and_drag());
    let track = rect.shrink2(vec2(0.0, 10.0));
    if let Some(p) = resp
        .interact_pointer_pos()
        .filter(|_| resp.is_pointer_button_down_on())
    {
        let t = ((track.bottom() - p.y) / track.height()).clamp(0.0, 1.0);
        value = (t * max as f32).round() as usize;
    }
    if resp.hovered() {
        let wheel = ui.input(|i| i.raw_scroll_delta.y);
        if wheel > 0.0 {
            value = (value + 1).min(max);
        } else if wheel < 0.0 {
            value = value.saturating_sub(1);
        }
    }
    let x = rect.center().x;
    let t = if max == 0 {
        1.0
    } else {
        value as f32 / max as f32
    };
    let y = track.bottom() - t * track.height();
    let p = ui.painter();
    p.rect_filled(
        Rect::from_min_max(
            egui::pos2(x - 3.0, track.top()),
            egui::pos2(x + 3.0, track.bottom()),
        ),
        3.0,
        c().input,
    );
    p.rect_filled(
        Rect::from_min_max(egui::pos2(x - 3.0, y), egui::pos2(x + 3.0, track.bottom())),
        3.0,
        ACCENT,
    );
    let active = resp.hovered() || resp.dragged();
    p.circle_filled(
        egui::pos2(x, y),
        if active { 10.0 } else { 9.0 },
        Color32::WHITE,
    );
    p.circle_stroke(egui::pos2(x, y), 9.0, Stroke::new(2.0_f32, ACCENT));
    value
}

/// Filled accent button.
pub fn primary(text: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.to_string()).color(Color32::BLACK))
        .fill(ACCENT)
        .corner_radius(8)
}

/// Borderless icon-ish button for toolbars.
pub fn ghost(text: &str, active: bool) -> egui::Button<'static> {
    let color = if active { ACCENT } else { c().text };
    egui::Button::new(RichText::new(text.to_string()).color(color))
        .fill(if active {
            Color32::from_rgba_unmultiplied(0xff, 0x93, 0x00, 28)
        } else {
            Color32::TRANSPARENT
        })
        .corner_radius(8)
}

fn one_line(ui: &egui::Ui, text: &str, font: FontId, max: f32) -> Arc<egui::Galley> {
    let mut job =
        egui::text::LayoutJob::simple_singleline(text.to_owned(), font, Color32::PLACEHOLDER);
    job.wrap = egui::text::TextWrapping::truncate_at_width(max);
    ui.fonts(|f| f.layout_job(job))
}

fn accent_tint() -> Color32 {
    Color32::from_rgba_unmultiplied(0xff, 0x93, 0x00, 42)
}

/// Row of a side navigation list, with an optional state dot at the right.
pub fn nav_item(
    ui: &mut egui::Ui,
    label: &str,
    active: bool,
    dot: Option<Color32>,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
    let fill = if active {
        accent_tint()
    } else if resp.hovered() {
        c().card
    } else {
        Color32::TRANSPARENT
    };
    let fg = if active || resp.hovered() {
        c().text
    } else {
        c().dim
    };
    let p = ui.painter();
    p.rect_filled(rect, 8.0, fill);
    p.text(
        rect.left_center() + vec2(10.0, 0.0),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(14.5),
        fg,
    );
    if let Some(color) = dot {
        p.circle_filled(rect.right_center() - vec2(12.0, 0.0), 4.0, color);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Menu bar button. `open` marks the menu that is showing.
pub fn menu_trigger(ui: &mut egui::Ui, text: &str, open: bool) -> egui::Response {
    let galley = one_line(ui, text, FontId::monospace(12.5), f32::INFINITY);
    let size = vec2(galley.size().x + 20.0, 24.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let (fg, bg) = if open {
        (Color32::from_rgb(0xff, 0xc4, 0x73), accent_tint())
    } else if resp.hovered() {
        (Color32::WHITE, c().input_hover)
    } else {
        (c().text, Color32::TRANSPARENT)
    };
    let p = ui.painter();
    p.rect_filled(rect, 6.0, bg);
    p.galley(rect.center() - galley.size() / 2.0, galley, fg);
    resp
}

/// Dropdown surface of the menu bar.
pub fn menu_frame<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::NONE
        .fill(c().panel)
        .stroke(Stroke::new(1.0_f32, c().input_hover))
        .corner_radius(10)
        .inner_margin(Margin::same(6))
        .shadow(ui.visuals().popup_shadow)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.set_width(320.0);
            add(ui)
        })
        .inner
}

/// Menu row: check mark gutter, label, and dim text on the right.
pub fn menu_item(
    ui: &mut egui::Ui,
    label: &str,
    right: &str,
    enabled: bool,
    checked: bool,
) -> egui::Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 29.0), sense);
    let hot = enabled && resp.hovered();
    if hot {
        ui.painter().rect_filled(rect, 6.0, accent_tint());
    }
    let fg = if !enabled {
        c().dim.gamma_multiply(0.7)
    } else if hot {
        Color32::WHITE
    } else {
        c().text
    };
    let inner = rect.width() - 36.0;
    let side = one_line(ui, right, FontId::monospace(11.0), inner * 0.4);
    let main = one_line(
        ui,
        label,
        FontId::monospace(12.5),
        inner - side.size().x - 12.0,
    );
    let y = rect.center().y;
    let p = ui.painter();
    if checked {
        let at = egui::pos2(rect.left() + 12.0, y);
        p.add(egui::Shape::line(
            vec![
                at + vec2(-4.0, 0.0),
                at + vec2(-1.0, 3.5),
                at + vec2(5.0, -4.0),
            ],
            Stroke::new(1.8_f32, ACCENT),
        ));
    }
    p.galley(
        egui::pos2(rect.left() + 26.0, y - main.size().y / 2.0),
        main,
        fg,
    );
    p.galley(
        egui::pos2(rect.right() - 10.0 - side.size().x, y - side.size().y / 2.0),
        side,
        c().dim,
    );
    if enabled {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp
    }
}

/// Small title above a group of menu rows.
pub fn menu_header(ui: &mut egui::Ui, text: &str) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
    let p = ui.painter();
    let g = p.layout_no_wrap(text.to_uppercase(), FontId::monospace(10.5), c().dim);
    p.galley(
        egui::pos2(rect.left() + 10.0, rect.bottom() - g.size().y - 4.0),
        g,
        c().dim,
    );
}

pub fn menu_separator(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 11.0), Sense::hover());
    ui.painter().hline(
        rect.x_range().shrink(4.0),
        rect.center().y,
        Stroke::new(1.0_f32, c().line),
    );
}

/// Full-width tab selector; returns true when the value changed.
pub fn tabs<T: PartialEq + Copy>(ui: &mut egui::Ui, value: &mut T, options: &[(T, &str)]) -> bool {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
    ui.painter().rect_filled(rect, 11.0, c().input);
    let inner = rect.shrink(4.0);
    let w = inner.width() / options.len() as f32;
    let mut changed = false;
    for (i, (v, label)) in options.iter().enumerate() {
        let cell =
            Rect::from_min_size(inner.min + vec2(w * i as f32, 0.0), vec2(w, inner.height()));
        let resp = ui.interact(cell, ui.id().with(("tab", i)), Sense::click());
        let sel = *value == *v;
        if resp.clicked() && !sel {
            *value = *v;
            changed = true;
        }
        let fg = if sel {
            Color32::BLACK
        } else if resp.hovered() {
            c().text
        } else {
            c().dim
        };
        let p = ui.painter();
        if sel {
            p.rect_filled(cell, 8.0, ACCENT);
        }
        p.text(
            cell.center(),
            Align2::CENTER_CENTER,
            label,
            FontId::proportional(14.5),
            fg,
        );
        resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    }
    changed
}
