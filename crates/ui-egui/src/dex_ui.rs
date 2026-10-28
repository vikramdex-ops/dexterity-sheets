//! Dexterity Sheets UI for the Excel features added after the 2026-10-28 parity audit
//! (`docs/dexterity-parity-audit.md`): Focus Cell and validation circles on the grid, and the
//! ribbon controls for Focus Cell, Remove Arrows, Auto / Clear Outline and Clear Validation
//! Circles. Upstream files only call in here (hooks listed in
//! `packaging/dexterity/check_overlay.py`), so GridCraft merges stay clean.

use egui::{Color32, Painter, Stroke, Ui, vec2};
use gridcraft_engine::core::{CellRef, RangeRef};
use gridcraft_engine::model::Sheet;
use serde_json::{Value as Json, json};

use crate::SheetApp;
use crate::grid::Geo;
use crate::icons::Icon;
use crate::widgets::{small_button, small_split_button};

/// Focus Cell's colour when none was picked: a soft amber that reads on white and dark grids.
const FOCUS_DEFAULT: Color32 = Color32::from_rgb(0xF2, 0xB7, 0x05);
/// The Focus Cell colours offered in its menu (name, `#RRGGBB`).
const FOCUS_COLORS: &[(&str, &str)] = &[("Amber", "#F2B705"), ("Green", "#3BA55C"), ("Blue", "#2F80ED"), ("Pink", "#E0529C"), ("Purple", "#8E5CD9")];
/// Validation circles are red, as in Excel.
const CIRCLE: Color32 = Color32::from_rgb(0xE0, 0x1B, 0x24);

fn parse_hex(s: &str) -> Option<Color32> {
    let h = s.trim_start_matches('#');
    let v = u32::from_str_radix(h, 16).ok().filter(|_| h.len() == 6)?;
    Some(Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

/// Paints Focus Cell (the active cell's row and column, under the selection) and the validation
/// circles on one quadrant of the grid. Called from the grid painter before the selection.
pub fn paint(p: &Painter, geo: &Geo, sh: &Sheet, app: &SheetApp, rows: (u32, u32), cols: (u32, u32)) {
    let dex = &app.session.dex;
    let Some(d) = app.session.active() else { return };
    if d.wb.active().is_none_or(|a| a.name != sh.name) {
        return;
    }
    if dex.focus_cell {
        let c = dex.focus_color.as_deref().and_then(parse_hex).unwrap_or(FOCUS_DEFAULT).gamma_multiply(0.22);
        let at = d.selection.active;
        let band = |r: RangeRef| {
            if r.start.row <= r.end.row && r.start.col <= r.end.col {
                p.rect_filled(geo.range_rect(sh, r), 0.0, c);
            }
        };
        if at.row >= rows.0 && at.row <= rows.1 {
            if at.col > cols.0 {
                band(RangeRef::new(CellRef::new(at.row, cols.0), CellRef::new(at.row, at.col.saturating_sub(1).min(cols.1))));
            }
            if at.col < cols.1 {
                band(RangeRef::new(CellRef::new(at.row, (at.col + 1).max(cols.0)), CellRef::new(at.row, cols.1)));
            }
        }
        if at.col >= cols.0 && at.col <= cols.1 {
            if at.row > rows.0 {
                band(RangeRef::new(CellRef::new(rows.0, at.col), CellRef::new(at.row.saturating_sub(1).min(rows.1), at.col)));
            }
            if at.row < rows.1 {
                band(RangeRef::new(CellRef::new((at.row + 1).max(rows.0), at.col), CellRef::new(rows.1, at.col)));
            }
        }
    }
    if dex.circles.is_empty() {
        return;
    }
    for c in gridcraft_engine::cmd::dex::validation_circles(&app.session) {
        if c.row < rows.0 || c.row > rows.1 || c.col < cols.0 || c.col > cols.1 {
            continue;
        }
        let r = geo.cell_rect(sh, c).expand2(vec2(4.0, 2.0) * geo.z.max(0.5));
        p.add(egui::Shape::ellipse_stroke(r.center(), r.size() / 2.0, Stroke::new(1.5, CIRCLE)));
    }
}

/// View ▸ Show ▸ Focus Cell: the button toggles it, the arrow picks its colour.
pub fn focus_cell_button(app: &mut SheetApp, ui: &mut Ui) {
    let on = app.session.dex.focus_cell;
    let (main, arrow) = small_split_button(
        ui,
        // The check mark shows it is on (an icon: not every interface font has a ✓ glyph).
        if on { Icon::Check } else { Icon::Gridlines },
        "Focus Cell",
        "Focus Cell: highlight the active cell's row and column",
    );
    if main.clicked() {
        app.run_or_alert("view.focusCell", json!({}));
    }
    egui::Popup::menu(&arrow).show(|ui| {
        let lang = app.ui.language;
        if ui.button(lang.tr(if on { "Turn Off Focus Cell" } else { "Turn On Focus Cell" })).clicked() {
            app.run_or_alert("view.focusCell", json!({"on": !on}));
        }
        ui.separator();
        for (name, hex) in FOCUS_COLORS {
            let col = parse_hex(hex).unwrap_or(FOCUS_DEFAULT);
            let r = ui.horizontal(|ui| {
                let (sw, _) = ui.allocate_exact_size(vec2(14.0, 14.0), egui::Sense::hover());
                ui.painter().rect_filled(sw, 2.0, col);
                ui.button(lang.tr(name))
            });
            if r.inner.clicked() {
                app.run_or_alert("view.focusCell", json!({"color": hex}));
            }
        }
    });
}

/// Formulas ▸ Formula Auditing ▸ Remove Arrows, with Remove Precedent / Dependent Arrows.
pub fn remove_arrows_button(app: &mut SheetApp, ui: &mut Ui) {
    let (main, arrow) = small_split_button(ui, Icon::Close, "Remove Arrows", "Remove Arrows");
    if main.clicked() {
        app.run_or_alert("formulas.removeArrows", json!({}));
    }
    egui::Popup::menu(&arrow).show(|ui| {
        let lang = app.ui.language;
        for (label, id) in [
            ("Remove Arrows", "formulas.removeArrows"),
            ("Remove Precedent Arrows", "formulas.removePrecedentArrows"),
            ("Remove Dependent Arrows", "formulas.removeDependentArrows"),
        ] {
            if ui.button(lang.tr(label)).clicked() {
                app.run_or_alert(id, json!({}));
            }
        }
    });
}

/// Data ▸ Outline: Auto Outline and Clear Outline, next to Group and Ungroup.
pub fn outline_buttons(app: &mut SheetApp, ui: &mut Ui) {
    ui.vertical(|ui| {
        if small_button(ui, Icon::Group, "Auto Outline", "Auto Outline: group the detail next to summary formulas", false).clicked() {
            app.run_or_alert("data.autoOutline", json!({}));
        }
        if small_button(ui, Icon::Ungroup, "Clear Outline", "Clear Outline: remove every outline group on the sheet", false).clicked() {
            app.run_or_alert("data.clearOutline", json!({}));
        }
    });
}

/// Front-end side of commands whose effect is on screen: the trace arrows live in the grid.
/// Returns `None` for every other command.
pub fn run_command(app: &mut SheetApp, id: &str, _: &Json) -> Option<Result<Json, String>> {
    let precedents = |app: &SheetApp| {
        app.grid.trace.as_ref().is_some_and(|t| t.items.as_array().and_then(|a| a.first()).is_some_and(|i| i.get("range").is_some()))
    };
    let keep = match id {
        "formulas.removeArrows" => false,
        "formulas.removePrecedentArrows" => !precedents(app),
        "formulas.removeDependentArrows" => precedents(app),
        _ => return None,
    };
    if !keep {
        app.grid.trace = None;
    }
    // The engine command still runs, so it is journaled and recorded like any other.
    Some(app.session.run(id, json!({})))
}
