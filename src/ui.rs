//! Theme and small widgets: cards, pills, toggle switch, segmented control.

use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontFamily, FontId, Margin, Pos2, Rect, RichText, Sense,
    Stroke, TextStyle, vec2,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub const ACCENT: Color32 = Color32::from_rgb(0xff, 0x93, 0x00);
/// The side margin of the right panel.
pub const PANEL_PAD: i8 = 18;
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
        sp.scroll = scroll_style();
        s.interaction.tooltip_delay = 0.35;
    });
}

/// Slim scroll bars without a track. The bar is a thin line while the
/// pointer is on the scroll area, and becomes a little wider on the bar.
fn scroll_style() -> egui::style::ScrollStyle {
    egui::style::ScrollStyle {
        bar_width: 6.0,
        floating_width: 2.0,
        active_background_opacity: 0.0,
        interact_background_opacity: 0.0,
        ..egui::style::ScrollStyle::floating()
    }
}

/// Space between a scroll bar and the edge of a panel that has a resize
/// handle or a border at that edge. Set it in `spacing.scroll.bar_outer_margin`.
pub const BAR_INSET: f32 = 6.0;

/// Space at the right of the content of a scroll area for the scroll bar.
const BAR_ROOM: i8 = 8;

/// Content of a scroll area that has no side padding: the content stops
/// before the scroll bar, so the bar does not cover it.
pub fn bar_room<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::NONE
        .inner_margin(Margin {
            right: BAR_ROOM,
            ..Margin::ZERO
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
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

/// A vertical scroll area that fills the space left. The content gets
/// `pad` on the left, the right and the bottom, so the scroll bar is in the
/// padding at the edge of the panel, not on the content. A soft shadow
/// shows at the top or the bottom when more content is above or below.
pub fn scroll<R>(ui: &mut egui::Ui, pad: i8, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let out = egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // The bottom margin keeps the last card off the edge of the panel.
            egui::Frame::NONE
                .inner_margin(Margin {
                    left: pad,
                    right: pad,
                    top: 0,
                    bottom: pad,
                })
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    add(ui)
                })
                .inner
        });
    let view = out.inner_rect;
    let top = out.state.offset.y;
    let below = out.content_size.y - view.height() - top;
    edge_shadow(ui, view, true, (top / 24.0).clamp(0.0, 1.0));
    edge_shadow(ui, view, false, (below / 24.0).clamp(0.0, 1.0));
    out.inner
}

/// A shadow at the top or the bottom edge of `rect`. `strength` from 0 to
/// 1 fades it in as the content scrolls.
fn edge_shadow(ui: &egui::Ui, rect: Rect, top: bool, strength: f32) {
    if strength <= 0.0 {
        return;
    }
    const HEIGHT: f32 = 14.0;
    let alpha = if is_dark() { 110.0 } else { 45.0 };
    let dark = Color32::from_black_alpha((alpha * strength) as u8);
    let clear = Color32::TRANSPARENT;
    let (y0, y1) = if top {
        (rect.top(), rect.top() + HEIGHT)
    } else {
        (rect.bottom(), rect.bottom() - HEIGHT)
    };
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(egui::pos2(rect.left(), y0), dark);
    mesh.colored_vertex(egui::pos2(rect.right(), y0), dark);
    mesh.colored_vertex(egui::pos2(rect.right(), y1), clear);
    mesh.colored_vertex(egui::pos2(rect.left(), y1), clear);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    ui.painter().add(egui::Shape::mesh(mesh));
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
    let (rect, resp, _) = fold_row(ui, title, open);
    if edited > 0 {
        pill(
            ui.painter(),
            egui::pos2(rect.right() - 26.0, rect.center().y),
            Align2::RIGHT_CENTER,
            &format!("{edited} edited"),
            FontId::proportional(11.0),
            ACCENT,
            Color32::from_rgba_unmultiplied(0xff, 0x93, 0x00, 30),
        );
    }
    resp
}

/// Clickable card title with a chevron, as `group_header`, and a short
/// dim `note` at the right, for example a value of a closed card. When
/// `info` is not empty, an info icon after the title shows it on hover.
pub fn fold_header(
    ui: &mut egui::Ui,
    title: &str,
    open: bool,
    note: &str,
    info: &str,
) -> egui::Response {
    let (rect, resp, mut title_w) = fold_row(ui, title, open);
    if !info.is_empty() {
        let center = egui::pos2(rect.left() + title_w + 13.0, rect.center().y);
        let hit = Rect::from_center_size(center, vec2(20.0, 20.0));
        // Click sense, so that a click on the icon does not fold the card.
        let icon = ui.interact(hit, resp.id.with("info"), Sense::click());
        let color = if icon.hovered() { c().text } else { c().dim };
        let p = ui.painter();
        p.circle_stroke(center, 6.5, Stroke::new(1.2_f32, color));
        p.text(
            center + vec2(0.0, 0.5),
            Align2::CENTER_CENTER,
            "i",
            FontId::proportional(10.0),
            color,
        );
        icon.on_hover_ui(|ui| {
            ui.set_max_width(260.0);
            ui.label(info);
        });
        title_w += 26.0;
    }
    if !note.is_empty() {
        let max = (rect.width() - title_w - 34.0).max(0.0);
        let galley = one_line(ui, note, FontId::proportional(12.5), max);
        let at = egui::pos2(
            rect.right() - 22.0 - galley.size().x,
            rect.center().y - galley.size().y / 2.0,
        );
        ui.painter().galley(at, galley, c().dim);
    }
    resp
}

/// A row with an upper-case title and a chevron: down when open, right
/// when closed. Returns the row, its response and the title width.
fn fold_row(ui: &mut egui::Ui, title: &str, open: bool) -> (Rect, egui::Response, f32) {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 20.0), Sense::click());
    let p = ui.painter();
    let color = if resp.hovered() { c().text } else { c().dim };
    let galley = p.layout_no_wrap(title.to_uppercase(), FontId::proportional(11.5), color);
    let title_w = galley.size().x;
    p.galley(
        egui::pos2(rect.left(), rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
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
    (
        rect,
        resp.on_hover_cursor(egui::CursorIcon::PointingHand),
        title_w,
    )
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
            // Narrower options when the row does not fit.
            let n = options.len().max(1) as f32;
            let w = ((ui.available_width() - 2.0 * (n - 1.0)) / n).min(76.0);
            ui.horizontal(|ui| {
                for (v, label) in options {
                    let sel = *value == *v;
                    let text =
                        RichText::new(*label).color(if sel { Color32::BLACK } else { c().dim });
                    let b = egui::Button::new(text)
                        .fill(if sel { ACCENT } else { Color32::TRANSPARENT })
                        .corner_radius(8)
                        .min_size(vec2(w, 28.0));
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

/// A button group: a wide accent button and a narrow accent button with
/// an arrow up, for a menu that opens above it. Returns (main, arrow).
pub fn split_button(
    ui: &mut egui::Ui,
    text: &str,
    main_enabled: bool,
    menu_enabled: bool,
) -> (egui::Response, egui::Response) {
    const H: f32 = 34.0;
    const ARROW_W: f32 = 38.0;
    const GAP: f32 = 2.0;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = GAP;
        let main_w = (ui.available_width() - ARROW_W - GAP).max(40.0);
        let main = ui.add_enabled(
            main_enabled,
            egui::Button::new(RichText::new(text.to_string()).color(Color32::BLACK))
                .fill(ACCENT)
                .corner_radius(egui::CornerRadius {
                    nw: 8,
                    sw: 8,
                    ne: 2,
                    se: 2,
                })
                .min_size(vec2(main_w, H)),
        );
        let arrow = ui.add_enabled(
            menu_enabled,
            egui::Button::new("")
                .fill(ACCENT)
                .corner_radius(egui::CornerRadius {
                    nw: 2,
                    sw: 2,
                    ne: 8,
                    se: 8,
                })
                .min_size(vec2(ARROW_W, H)),
        );
        let c = arrow.rect.center();
        let color = if menu_enabled {
            Color32::BLACK
        } else {
            Color32::from_black_alpha(100)
        };
        ui.painter().add(egui::Shape::line(
            vec![c + vec2(-5.0, 2.5), c + vec2(0.0, -2.5), c + vec2(5.0, 2.5)],
            Stroke::new(1.8_f32, color),
        ));
        (main, arrow)
    })
    .inner
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

/// The CoCAD mark, as in packaging/cocad.svg without the tile: a cube with a
/// teardrop hole, tip up, in the left face. The teardrop is the hole shape
/// that prints without support (Optimize tab).
pub fn paint_app_mark(p: &egui::Painter, rect: egui::Rect) {
    // The cube in the SVG fills x 26..102 and y 22..106 of a 128 grid.
    let k = rect.height().min(rect.width() * 84.0 / 76.0) / 84.0;
    let at = |x: f32, y: f32| rect.center() + k * vec2(x - 64.0, y - 64.0);
    let top = [
        at(64.0, 22.0),
        at(102.0, 43.0),
        at(64.0, 64.0),
        at(26.0, 43.0),
    ];
    let left = [
        at(26.0, 43.0),
        at(64.0, 64.0),
        at(64.0, 106.0),
        at(26.0, 85.0),
    ];
    p.add(egui::Shape::convex_polygon(
        top.to_vec(),
        ACCENT.gamma_multiply(0.45),
        Stroke::NONE,
    ));
    p.add(egui::Shape::convex_polygon(
        left.to_vec(),
        ACCENT,
        Stroke::NONE,
    ));
    // The teardrop: a circle of radius 9 at (45, 74.4) and a tip r·√2 above
    // its center. The shear puts it in the plane of the left face, which
    // falls 21 units over 38.
    let face = |x: f32, y: f32| at(x, y + (x - 45.0) * 21.0 / 38.0);
    let mut drop = vec![face(45.0, 74.4 - 9.0 * std::f32::consts::SQRT_2)];
    const STEPS: usize = 24;
    for i in 0..=STEPS {
        let a = (-45.0 + 270.0 * i as f32 / STEPS as f32).to_radians();
        drop.push(face(45.0 + 9.0 * a.cos(), 74.4 + 9.0 * a.sin()));
    }
    p.add(egui::Shape::convex_polygon(drop, c().bg, Stroke::NONE));
    let s = Stroke::new((6.0 * k).max(1.2), ACCENT);
    let outline = [
        at(64.0, 22.0),
        at(102.0, 43.0),
        at(102.0, 85.0),
        at(64.0, 106.0),
        at(26.0, 85.0),
        at(26.0, 43.0),
    ];
    p.add(egui::Shape::closed_line(outline.to_vec(), s));
    p.line(vec![at(26.0, 43.0), at(64.0, 64.0), at(102.0, 43.0)], s);
    p.line_segment([at(64.0, 64.0), at(64.0, 106.0)], s);
}

/// Line icons for `icon_button`.
#[derive(Clone, Copy)]
pub enum Icon {
    /// A floppy disk.
    Save,
    /// A gear: more actions.
    Gear,
    /// An arrow that turns back.
    Reset,
    /// Two chevrons that point to each other.
    Collapse,
    /// Two chevrons that point away from each other.
    Expand,
    /// An "i" in a circle: more information.
    Info,
}

/// Draws `icon` on a 16 × 16 grid around `center`.
fn paint_icon(p: &egui::Painter, icon: Icon, center: Pos2, color: Color32) {
    let o = center - vec2(8.0, 8.0);
    let pts = |xy: &[(f32, f32)]| xy.iter().map(|&(x, y)| o + vec2(x, y)).collect::<Vec<_>>();
    let s = Stroke::new(1.5_f32, color);
    let line = |xy: &[(f32, f32)]| {
        p.add(egui::Shape::line(pts(xy), s));
    };
    let closed = |xy: &[(f32, f32)]| {
        p.add(egui::Shape::closed_line(pts(xy), s));
    };
    match icon {
        Icon::Save => {
            closed(&[
                (2.25, 2.25),
                (11.0, 2.25),
                (13.75, 5.0),
                (13.75, 13.75),
                (2.25, 13.75),
            ]);
            line(&[(5.0, 2.25), (5.0, 5.75), (10.0, 5.75), (10.0, 2.25)]);
            line(&[(4.75, 13.75), (4.75, 9.5), (11.25, 9.5), (11.25, 13.75)]);
        }
        Icon::Gear => {
            // Six teeth: each tooth has two points on the outer circle and
            // two on the inner circle.
            let mut outline = Vec::new();
            for i in 0..6 {
                let a = i as f32 * std::f32::consts::FRAC_PI_3;
                for (r, da) in [(5.0, -0.36), (7.0, -0.2), (7.0, 0.2), (5.0, 0.36)] {
                    let t: f32 = a + da;
                    outline.push(center + vec2(t.cos(), t.sin()) * r);
                }
            }
            p.add(egui::Shape::closed_line(outline, s));
            p.circle_stroke(center, 2.2, s);
        }
        Icon::Reset => {
            // Three quarters of a circle from the top, clockwise, and an
            // arrow head at the top that points back.
            let c = center + vec2(0.0, 0.5);
            let arc: Vec<Pos2> = (0..=18)
                .map(|i| {
                    let t =
                        -std::f32::consts::FRAC_PI_2 + i as f32 / 18.0 * 1.5 * std::f32::consts::PI;
                    c + vec2(t.cos(), t.sin()) * 5.25
                })
                .collect();
            p.add(egui::Shape::line(arc, s));
            line(&[(9.75, 1.0), (7.25, 3.25), (9.75, 5.5)]);
        }
        Icon::Collapse => {
            line(&[(4.0, 2.5), (8.0, 6.0), (12.0, 2.5)]);
            line(&[(4.0, 13.5), (8.0, 10.0), (12.0, 13.5)]);
        }
        Icon::Expand => {
            line(&[(4.0, 6.0), (8.0, 2.5), (12.0, 6.0)]);
            line(&[(4.0, 10.0), (8.0, 13.5), (12.0, 10.0)]);
        }
        Icon::Info => {
            p.circle_stroke(center, 6.25, s);
            p.circle_filled(o + vec2(8.0, 4.9), 1.0, color);
            line(&[(8.0, 7.25), (8.0, 11.5)]);
        }
    }
}

/// Background and foreground of a filled button in its current state.
fn button_colors(ui: &egui::Ui, resp: &egui::Response, active: bool) -> (Color32, Color32) {
    let enabled = ui.is_enabled();
    let fill = if active {
        accent_tint()
    } else if enabled && resp.hovered() {
        c().input_hover
    } else {
        c().input
    };
    let fg = if !enabled {
        c().dim.gamma_multiply(0.6)
    } else if active {
        ACCENT
    } else {
        c().text
    };
    (fill, fg)
}

/// Filled square button with a line icon. `active` shows it in the accent
/// color.
pub fn icon_button(ui: &mut egui::Ui, icon: Icon, size: f32, active: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    if ui.is_rect_visible(rect) {
        let (fill, fg) = button_colors(ui, &resp, active);
        let p = ui.painter();
        p.rect_filled(rect, 8.0, fill);
        paint_icon(p, icon, rect.center(), fg);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
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
    menu_frame_width(ui, 320.0, add)
}

/// Menu surface with this width, for example a context menu.
pub fn menu_frame_width<R>(
    ui: &mut egui::Ui,
    width: f32,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    egui::Frame::NONE
        .fill(c().panel)
        .stroke(Stroke::new(1.0_f32, c().input_hover))
        .corner_radius(10)
        .inner_margin(Margin::same(6))
        .shadow(ui.visuals().popup_shadow)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.set_width(width);
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

/// Full-width row of classic tabs; returns true when the value changed. Each
/// option has a value, a label and the index (in characters) of the letter
/// of its key. The tooltip shows that key. The selected tab has
/// the panel color and no bottom edge, so it joins the panel below it. Put
/// the row in a strip with the `bg` color, with no bottom margin.
pub fn tabs<T: PartialEq + Copy>(
    ui: &mut egui::Ui,
    value: &mut T,
    options: &[(T, &str, Option<usize>)],
) -> bool {
    const GAP: f32 = 4.0;
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::hover());
    let n = options.len() as f32;
    let w = (rect.width() - GAP * (n - 1.0)) / n;
    let line = Stroke::new(1.0_f32, c().line);
    let base = rect.bottom() - 0.5;
    // The line below the tabs goes to the edges of the strip.
    ui.painter().hline(ui.clip_rect().x_range(), base, line);
    let top = CornerRadius {
        nw: 8,
        ne: 8,
        sw: 0,
        se: 0,
    };
    let mut changed = false;
    for (i, (v, label, key)) in options.iter().enumerate() {
        let cell = Rect::from_min_size(
            rect.min + vec2((w + GAP) * i as f32, 0.0),
            vec2(w, rect.height()),
        );
        let resp = ui.interact(cell, ui.id().with(("tab", i)), Sense::click());
        let sel = *value == *v;
        if resp.clicked() && !sel {
            *value = *v;
            changed = true;
        }
        let p = ui.painter();
        if sel {
            // The accent fills the tab, and the face covers all but a band
            // at the top, so the band follows the round corners.
            p.rect_filled(cell, top, ACCENT);
            let face = Rect::from_min_max(
                egui::pos2(cell.left(), cell.top() + 2.5),
                egui::pos2(cell.right(), cell.bottom() + 1.0),
            );
            p.rect_filled(face, top, c().panel);
            let sides = (cell.top() + 8.0)..=base;
            p.vline(cell.left() + 0.5, sides.clone(), line);
            p.vline(cell.right() - 0.5, sides, line);
        } else {
            let t = if resp.hovered() { 0.85 } else { 0.45 };
            let cell = cell.with_min_y(cell.top() + 3.0);
            let fill = c().bg.lerp_to_gamma(c().input, t);
            p.rect_filled(cell.with_max_y(cell.bottom() - 1.0), top, fill);
        }
        let fg = if sel || resp.hovered() {
            c().text
        } else {
            c().dim
        };
        let font = FontId::proportional(14.5);
        let galley = ui.fonts(|f| f.layout_no_wrap(label.to_string(), font, fg));
        let center = cell.center() + vec2(0.0, if sel { 0.0 } else { 1.5 });
        ui.painter()
            .galley(center - galley.size() / 2.0, galley, fg);
        let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
        if let Some(k) = key.and_then(|k| label.chars().nth(k)) {
            resp.on_hover_text(format!("Key {}", k.to_ascii_uppercase()));
        }
    }
    changed
}
