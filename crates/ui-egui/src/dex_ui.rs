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

/// Set by the desktop app's `main` only: the side effects that belong to a person's own PC (crash
/// recovery copies, settings files, the local port links arrive on, Windows notifications) run
/// only then, so tests and other hosts of `SheetApp` never touch the user's folders or desktop.
static DESKTOP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The desktop app is running (called once from `apps/gridcraft`'s `main`).
pub fn enable_desktop() {
    DESKTOP.store(true, std::sync::atomic::Ordering::Relaxed);
}

pub(crate) fn desktop() -> bool {
    DESKTOP.load(std::sync::atomic::Ordering::Relaxed)
}

pub mod automation;
pub mod recovery;
pub mod releases;
pub mod share;
pub mod templates;
pub mod tools;
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
pub fn run_command(app: &mut SheetApp, id: &str, p: &Json) -> Option<Result<Json, String>> {
    match id {
        // The engine command is the catalogue entry; the copies and the dialog are the window's.
        "file.recoverUnsaved" => return Some(recovery::run_command(app, p)),
        "automate.rules" => return Some(automation::run_command(app, p)),
        "file.newFromTemplate" | "file.saveAsTemplate" => return templates::run_command(app, id, p),
        "review.compareFiles" | "data.qualityCheck" | "formulas.diagnostics" | "formulas.explainError" => return tools::run_command(app, id, p),
        "help.whatsNew" | "help.reportProblem" | "help.rollBack" | "help.updateChannel" => return releases::run_command(app, id, p),
        // The panes these open are UI state the ribbon buttons set; from the command search, a
        // key tip or the control channel the command alone left nothing on screen.
        "formulas.watchWindow" => {
            let r = app.session.run(id, p.clone());
            if r.is_ok() {
                app.grid.pane = Some("watch".into());
            }
            return Some(r);
        }
        "review.showComments" => {
            let r = app.session.run(id, p.clone());
            if app.session.comments_pane {
                app.grid.pane = None;
            }
            return Some(r);
        }
        _ => {}
    }
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

/// Dialogs the engine asks for under another name than the UI's: Conditional Formatting run with
/// no rule (from the command search or a key tip) asks for "newFormattingRule", which is the New
/// Rule dialog the ribbon's Conditional Formatting ▸ New Rule… opens.
pub fn dialog_alias(name: &str) -> Option<(&'static str, Json)> {
    match name {
        "newFormattingRule" => Some(("cfQuick", json!({"type": "expression", "title": "New Formatting Rule"}))),
        _ => None,
    }
}

/// A worksheet value from a command result as Excel shows it in a dialog.
fn value_text(v: &Json) -> String {
    match v {
        Json::Null => "(empty)".into(),
        Json::Bool(b) => if *b { "TRUE" } else { "FALSE" }.into(),
        Json::Number(n) => n.to_string(),
        Json::String(s) => format!("\"{s}\""),
        Json::Array(rows) => format!("{{array, {} row(s)}}", rows.len()),
        Json::Object(o) => o.get("error").and_then(Json::as_str).unwrap_or("?").to_string(),
    }
}

/// A sheet cell link that selects the cell when clicked.
fn cell_link(app: &mut SheetApp, ui: &mut Ui, sheet: &str, cell: &str) {
    let label = if sheet.is_empty() { cell.to_string() } else { format!("{sheet}!{cell}") };
    if ui.link(label).clicked() {
        if !sheet.is_empty() {
            let _ = app.run("sheet.activate", json!({"sheet": sheet}));
        }
        let _ = app.run("selection.set", json!({"range": cell}));
        app.grid.ensure_visible = true;
    }
}

/// Readable body of the result dialogs (Workbook Statistics, Accessibility, Error Checking,
/// Evaluate Formula, the comments list) in place of the command's raw JSON.
pub fn result_view(app: &mut SheetApp, ui: &mut Ui, name: &str, r: &Json) {
    let lang = app.ui.language;
    let get = |o: &Json, k: &str| {
        o.get(k).map(|v| {
            if let Some(s) = v.as_str() {
                s.to_string()
            } else if v.is_null() {
                "—".into()
            } else {
                v.to_string()
            }
        })
    };
    egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| match name {
        "statistics" => {
            let sheet = r.get("sheet").cloned().unwrap_or_default();
            let wb = r.get("workbook").cloned().unwrap_or_default();
            ui.strong(format!("{}: {}", lang.tr("Current sheet"), get(&sheet, "name").unwrap_or_default()));
            egui::Grid::new("dex_stats_sheet").num_columns(2).striped(true).show(ui, |ui| {
                for (k, label) in [
                    ("endOfSheet", "End of sheet"),
                    ("cellsWithData", "Cells with data"),
                    ("tables", "Tables"),
                    ("formulas", "Formulas"),
                    ("charts", "Charts"),
                    ("comments", "Comments"),
                ] {
                    ui.label(lang.tr(label));
                    ui.label(get(&sheet, k).unwrap_or_default());
                    ui.end_row();
                }
            });
            ui.add_space(8.0);
            ui.strong(lang.tr("Workbook"));
            egui::Grid::new("dex_stats_book").num_columns(2).striped(true).show(ui, |ui| {
                for (k, label) in [
                    ("sheets", "Sheets"),
                    ("cellsWithData", "Cells with data"),
                    ("tables", "Tables"),
                    ("charts", "Charts"),
                    ("names", "Defined names"),
                ] {
                    ui.label(lang.tr(label));
                    ui.label(get(&wb, k).unwrap_or_default());
                    ui.end_row();
                }
            });
        }
        "accessibility" => {
            let issues = r.as_array().cloned().unwrap_or_default();
            if issues.is_empty() {
                ui.label(lang.tr("No accessibility issues found."));
            }
            for i in &issues {
                let what = [get(i, "object"), i.get("count").map(|c| format!("{c}"))].into_iter().flatten().collect::<Vec<_>>().join(", ");
                let sheet = get(i, "sheet").unwrap_or_default();
                let issue = get(i, "issue").unwrap_or_default();
                ui.label(if what.is_empty() { format!("{sheet}: {issue}") } else { format!("{sheet}: {issue} ({what})") });
            }
        }
        "errorChecking" => {
            let errors = r.get("errors").and_then(Json::as_array).cloned().unwrap_or_default();
            let circular = r.get("circular").and_then(Json::as_array).cloned().unwrap_or_default();
            if errors.is_empty() && circular.is_empty() {
                ui.label(lang.tr("No errors found in this workbook."));
            }
            if !errors.is_empty() {
                egui::Grid::new("dex_errors").num_columns(3).striped(true).show(ui, |ui| {
                    ui.strong(lang.tr("Cell"));
                    ui.strong(lang.tr("Error"));
                    ui.strong(lang.tr("Formula"));
                    ui.end_row();
                    for e in &errors {
                        cell_link(app, ui, &get(e, "sheet").unwrap_or_default(), &get(e, "cell").unwrap_or_default());
                        ui.label(get(e, "error").unwrap_or_default());
                        ui.monospace(get(e, "formula").unwrap_or_default());
                        ui.end_row();
                    }
                });
            }
            if !circular.is_empty() {
                ui.add_space(6.0);
                ui.strong(lang.tr("Circular references"));
                for c in circular.iter().filter_map(Json::as_str) {
                    let (sheet, cell) = c.rsplit_once('!').map_or(("", c), |(s, c)| (s.trim_matches('\''), c));
                    cell_link(app, ui, sheet, cell);
                }
            }
        }
        "evaluateFormula" if r.get("formula").is_none() => {
            ui.label(lang.tr("Select a cell that contains a formula, then choose Evaluate Formula."));
        }
        "evaluateFormula" => {
            ui.monospace(get(r, "formula").unwrap_or_default());
            ui.add_space(4.0);
            egui::Grid::new("dex_eval").num_columns(2).striped(true).show(ui, |ui| {
                ui.strong(lang.tr("Part"));
                ui.strong(lang.tr("Value"));
                ui.end_row();
                for st in r.get("steps").and_then(Json::as_array).cloned().unwrap_or_default() {
                    ui.monospace(get(&st, "expression").unwrap_or_default());
                    ui.label(value_text(st.get("value").unwrap_or(&Json::Null)));
                    ui.end_row();
                }
            });
            ui.add_space(4.0);
            ui.strong(format!("{} {}", lang.tr("Result:"), value_text(r.get("result").unwrap_or(&Json::Null))));
        }
        _ => {
            // The comments list: cell, author and text.
            let list = r.as_array().cloned().unwrap_or_default();
            if list.is_empty() {
                ui.label(lang.tr("No comments on this sheet."));
            }
            for c in &list {
                ui.horizontal_wrapped(|ui| {
                    cell_link(app, ui, "", &get(c, "cell").unwrap_or_default());
                    ui.strong(get(c, "author").unwrap_or_default());
                    ui.label(get(c, "text").unwrap_or_default());
                });
            }
        }
    });
}

/// Conditional Formatting Rules Manager: New Rule…, as in Excel (the list alone could only reorder
/// and delete rules).
pub fn new_rule_button(app: &mut SheetApp, ui: &mut Ui) {
    if ui.button(app.ui.language.tr("New Rule…")).clicked() {
        app.open_dialog("cfQuick", json!({"type": "expression", "title": "New Formatting Rule"}));
    }
    ui.separator();
}
