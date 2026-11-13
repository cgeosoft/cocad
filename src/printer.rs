//! Printer presets and the editor of a print area. The Settings window uses
//! the editor for the default print area of new models (Settings > Printer).
//! The Optimize tab can use it for the area of one model.

use crate::optimize::Area;
use crate::ui;
use eframe::egui;

/// A printer with a known print area.
pub struct Preset {
    pub name: &'static str,
    pub area: Area,
}

const fn preset(name: &'static str, w: f32, d: f32, h: f32) -> Preset {
    Preset {
        name,
        area: Area { w, d, h },
    }
}

/// Width × depth × height in mm, from the spec page of each maker. See
/// "Print area presets" in `docs/optimize-tab.md`.
pub const PRESETS: [Preset; 5] = [
    preset("Prusa MK4S", 250.0, 210.0, 220.0),
    preset("Prusa CORE One", 250.0, 220.0, 270.0),
    preset("Prusa MINI+", 180.0, 180.0, 180.0),
    preset("Creality K1C", 220.0, 220.0, 250.0),
    preset("Creality Ender-3 V3", 220.0, 220.0, 250.0),
];

/// The label for an area that matches no preset.
pub const CUSTOM: &str = "Custom";

/// The limits of each print area value in mm.
pub const MIN_MM: f32 = 10.0;
pub const MAX_MM: f32 = 2000.0;

/// The first preset with exactly this area.
pub fn preset_of(area: Area) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.area == area)
}

/// The name that the dropdown shows: `picked` when that preset has this
/// area (two presets can have the same area), else the first preset with
/// this area, else "Custom".
fn shown_name(area: Area, picked: Option<&str>) -> &'static str {
    let picked = PRESETS
        .iter()
        .find(|p| Some(p.name) == picked && p.area == area);
    picked
        .or_else(|| preset_of(area))
        .map_or(CUSTOM, |p| p.name)
}

/// A dropdown of the presets and three fields: width, depth and height in
/// mm. A preset fills the fields. An edit that matches no preset shows
/// "Custom". `picked` is the name of the preset that the user selected
/// last (empty: none). The dropdown shows it when two presets have the
/// same area. The caller stores it. Returns true when the area changed.
pub fn area_editor(ui: &mut egui::Ui, area: &mut Area, picked: &mut String) -> bool {
    let before = *area;
    let shown = shown_name(*area, Some(picked.as_str()));
    egui::ComboBox::from_id_salt(ui.id().with("printer-preset"))
        .selected_text(shown)
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            for p in &PRESETS {
                let text = format!(
                    "{}   {:.0} × {:.0} × {:.0}",
                    p.name, p.area.w, p.area.d, p.area.h
                );
                if ui.selectable_label(shown == p.name, text).clicked() {
                    *area = p.area;
                    *picked = p.name.to_string();
                }
            }
        });
    ui.columns(3, |cols| {
        for (col, (label, v)) in cols.iter_mut().zip([
            ("Width", &mut area.w),
            ("Depth", &mut area.d),
            ("Height", &mut area.h),
        ]) {
            col.label(ui::dim(label));
            col.add(
                egui::DragValue::new(v)
                    .range(MIN_MM..=MAX_MM)
                    .speed(1.0)
                    .max_decimals(1)
                    .suffix(" mm"),
            );
        }
    });
    *area != before
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_of_needs_an_exact_match() {
        let mk4s = Area {
            w: 250.0,
            d: 210.0,
            h: 220.0,
        };
        assert_eq!(preset_of(mk4s).map(|p| p.name), Some("Prusa MK4S"));
        let near = Area { h: 221.0, ..mk4s };
        assert!(preset_of(near).is_none());
        for p in &PRESETS {
            assert_eq!(preset_of(p.area).unwrap().area, p.area);
        }
    }

    #[test]
    fn shown_name_keeps_the_picked_preset_of_a_shared_area() {
        let k1c = PRESETS[3].area;
        assert_eq!(shown_name(k1c, None), "Creality K1C");
        assert_eq!(
            shown_name(k1c, Some("Creality Ender-3 V3")),
            "Creality Ender-3 V3"
        );
        // A picked preset with another area does not count.
        assert_eq!(shown_name(k1c, Some("Prusa MK4S")), "Creality K1C");
        let custom = Area {
            w: 300.0,
            d: 300.0,
            h: 300.0,
        };
        assert_eq!(shown_name(custom, Some("Prusa MK4S")), CUSTOM);
    }

    #[test]
    fn presets_are_in_the_limits() {
        for p in &PRESETS {
            for v in [p.area.w, p.area.d, p.area.h] {
                assert!((MIN_MM..=MAX_MM).contains(&v), "{}", p.name);
            }
        }
    }
}
