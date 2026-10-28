//! Result windows for the Dexterity Sheets checking tools: Compare Files, Quality Check,
//! Formula Diagnostics and Explain This Error. The engine commands do the work
//! (`crates/engine/src/cmd/dex/{compare,quality,diagnostics}.rs`); these windows show their
//! results as tables with links to the cells, filters, and (Compare) the report export.

use egui::{RichText, Ui, vec2};
use serde_json::{Value as Json, json};

use crate::SheetApp;
use crate::icons::Icon;
use crate::widgets::small_button;

#[derive(Default)]
pub struct Tools {
    /// The command and its result being shown.
    pub shown: Option<(String, Json)>,
    /// The kinds or checks hidden by the filter.
    hidden: Vec<String>,
    /// Compare Files form: the two sides and where to write the report.
    pub compare_form: bool,
    old: String,
    new: String,
    report: String,
}

/// Commands whose result opens a window here. With empty params, Compare Files opens its form.
pub fn run_command(app: &mut SheetApp, id: &str, p: &Json) -> Option<Result<Json, String>> {
    if !matches!(id, "review.compareFiles" | "data.qualityCheck" | "formulas.diagnostics" | "formulas.explainError") {
        return None;
    }
    if id == "review.compareFiles" && p.get("old").is_none() && p.get("new").is_none() {
        app.collab.tools.compare_form = true;
        if app.collab.tools.new.is_empty() {
            app.collab.tools.new = "active".into();
        }
        return Some(Ok(Json::Null));
    }
    let r = app.session.run(id, p.clone());
    if let Ok(v) = &r {
        app.collab.tools.shown = Some((id.to_string(), v.clone()));
        app.collab.tools.hidden.clear();
    }
    Some(r)
}

/// Review ▸ Compare Files and Quality Check (next to Workbook Statistics).
pub fn review_buttons(app: &mut SheetApp, ui: &mut Ui) {
    ui.vertical(|ui| {
        if small_button(
            ui,
            Icon::Calc,
            "Compare Files",
            "Compare two versions of a spreadsheet: values, formulas, formatting, sheets, rows and columns, names",
            false,
        )
        .clicked()
        {
            app.run_or_alert("review.compareFiles", json!({}));
        }
        if small_button(
            ui,
            Icon::Validation,
            "Quality Check",
            "Find duplicate numbers, blank mandatory fields, invalid dates, inconsistent values, broken or missing formulas",
            false,
        )
        .clicked()
        {
            app.run_or_alert("data.qualityCheck", json!({}));
        }
    });
}

/// Formulas ▸ Formula Auditing: Diagnostics and Explain This Error.
pub fn auditing_buttons(app: &mut SheetApp, ui: &mut Ui) {
    if small_button(
        ui,
        Icon::Validation,
        "Diagnostics",
        "Formula diagnostics: errors, broken references, inconsistent formulas, values typed over formulas, circular and external references",
        false,
    )
    .clicked()
    {
        app.run_or_alert("formulas.diagnostics", json!({}));
    }
    if small_button(
        ui,
        Icon::Calc,
        "Explain Error",
        "Explain the error in the active cell: what it means, why it happened here, how to fix it",
        false,
    )
    .clicked()
    {
        app.run_or_alert("formulas.explainError", json!({}));
    }
}

fn link(app: &mut SheetApp, ui: &mut Ui, at: &str) {
    let (sheet, cell) = match at.rsplit_once('!') {
        Some((s, c)) => (s.trim_matches('\'').to_string(), c.to_string()),
        None => (String::new(), at.to_string()),
    };
    if ui.link(at).clicked() {
        if !sheet.is_empty() {
            let _ = app.run("sheet.activate", json!({"sheet": sheet}));
        }
        if gridcraft_engine::core::CellRef::parse(&cell).is_some() {
            let _ = app.run("selection.set", json!({"range": cell}));
            app.grid.ensure_visible = true;
        }
    }
}

fn text(v: &Json, k: &str) -> String {
    match v.get(k) {
        Some(Json::String(s)) => s.clone(),
        Some(Json::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

pub fn windows(app: &mut SheetApp, ctx: &egui::Context) {
    compare_form(app, ctx);
    results(app, ctx);
}

fn compare_form(app: &mut SheetApp, ctx: &egui::Context) {
    if !app.collab.tools.compare_form {
        return;
    }
    let mut open = true;
    let mut go = false;
    egui::Window::new("Compare Files").collapsible(false).resizable(false).open(&mut open).anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0)).show(
        ctx,
        |ui| {
            ui.set_max_width(520.0);
            let t = &mut app.collab.tools;
            ui.label("Compare an earlier version (Rev A) with a later one (Rev B). Type a file path, or \"active\" for the workbook open now.");
            egui::Grid::new("dex_cmp").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
                ui.label("Earlier (Rev A)");
                ui.add(egui::TextEdit::singleline(&mut t.old).hint_text(r"D:\Jobs\Project_Tracker_Rev_A.xlsx").desired_width(380.0));
                ui.end_row();
                ui.label("Later (Rev B)");
                ui.add(egui::TextEdit::singleline(&mut t.new).hint_text("active").desired_width(380.0));
                ui.end_row();
                ui.label("Report (optional)");
                ui.add(egui::TextEdit::singleline(&mut t.report).hint_text(r"D:\Jobs\Comparison.xlsx or .csv").desired_width(380.0));
                ui.end_row();
            });
            if ui.add_enabled(!t.old.trim().is_empty() && !t.new.trim().is_empty(), egui::Button::new("Compare")).clicked() {
                go = true;
            }
        },
    );
    if go {
        let t = &app.collab.tools;
        let mut p = json!({"old": t.old.trim().trim_matches('"'), "new": t.new.trim().trim_matches('"')});
        if !t.report.trim().is_empty() {
            p["report"] = json!(t.report.trim().trim_matches('"'));
        }
        match run_command(app, "review.compareFiles", &p) {
            Some(Ok(_)) => open = false,
            Some(Err(e)) => app.message = Some(("Compare Files".into(), e)),
            None => {}
        }
    }
    if !open {
        app.collab.tools.compare_form = false;
    }
}

fn results(app: &mut SheetApp, ctx: &egui::Context) {
    let Some((id, r)) = app.collab.tools.shown.clone() else { return };
    let title = match id.as_str() {
        "review.compareFiles" => format!("Compare: {} with {}", text(&r, "old"), text(&r, "new")),
        "data.qualityCheck" => "Spreadsheet Quality Check".into(),
        "formulas.diagnostics" => "Formula Diagnostics".into(),
        _ => "Explain This Error".into(),
    };
    let mut open = true;
    egui::Window::new(title).collapsible(false).open(&mut open).default_width(640.0).show(ctx, |ui| {
        if id == "formulas.explainError" {
            explain(app, ui, &r);
            return;
        }
        let (rows_key, kind_key, place) = if id == "review.compareFiles" { ("changes", "kind", "cell") } else { ("issues", "check", "cell") };
        let counts: Vec<(String, u64)> = r
            .get("counts")
            .and_then(Json::as_object)
            .map(|o| o.iter().map(|(k, v)| (k.clone(), v.as_u64().unwrap_or(0))).collect())
            .unwrap_or_default();
        ui.horizontal_wrapped(|ui| {
            ui.label("Show:");
            for (k, n) in &counts {
                let mut on = !app.collab.tools.hidden.contains(k);
                if ui.checkbox(&mut on, format!("{k} ({n})")).changed() {
                    app.collab.tools.hidden.retain(|x| x != k);
                    if !on {
                        app.collab.tools.hidden.push(k.clone());
                    }
                }
            }
        });
        if r.get("truncated").and_then(Json::as_bool) == Some(true) {
            ui.label(
                RichText::new(format!(
                    "Showing the first {} of {} found.",
                    r.get(rows_key).and_then(Json::as_array).map_or(0, Vec::len),
                    text(&r, "total")
                ))
                .weak(),
            );
        }
        if let Some(note) = r.get("note").and_then(Json::as_str) {
            ui.label(RichText::new(note).weak());
        }
        if let Some(path) = r.get("report").and_then(Json::as_str) {
            ui.label(format!("Report written to {path}"));
        }
        let rows: Vec<Json> = r.get(rows_key).and_then(Json::as_array).cloned().unwrap_or_default();
        let hidden = app.collab.tools.hidden.clone();
        let rows: Vec<&Json> = rows.iter().filter(|x| !hidden.contains(&text(x, kind_key))).collect();
        if rows.is_empty() {
            ui.label(if id == "review.compareFiles" {
                "No differences: the two versions hold the same content."
            } else {
                "Nothing found: no issues in what was checked."
            });
            return;
        }
        egui::ScrollArea::vertical().max_height(380.0).show(ui, |ui| {
            egui::Grid::new("dex_tool_rows").num_columns(4).striped(true).spacing(vec2(12.0, 4.0)).show(ui, |ui| {
                if id == "review.compareFiles" {
                    for h in ["Kind", "Where", "Previous", "New"] {
                        ui.strong(h);
                    }
                } else {
                    for h in ["Check", "Where", "Issue", "Suggested action"] {
                        ui.strong(h);
                    }
                }
                ui.end_row();
                for row in rows.into_iter().take(5000) {
                    ui.label(text(row, kind_key));
                    let at = if id == "review.compareFiles" {
                        let (s, c) = (text(row, "sheet"), text(row, place));
                        if s.is_empty() { c } else { format!("{s}!{c}") }
                    } else {
                        text(row, place)
                    };
                    if at.is_empty() {
                        ui.label("—");
                    } else {
                        link(app, ui, &at);
                    }
                    if id == "review.compareFiles" {
                        ui.label(text(row, "old"));
                        ui.label(text(row, "new"));
                    } else {
                        ui.label(text(row, "message"));
                        let s = text(row, "suggestion");
                        ui.label(if s.is_empty() { text(row, "detail") } else { s });
                    }
                    ui.end_row();
                }
            });
        });
    });
    if !open {
        app.collab.tools.shown = None;
    }
}

fn explain(app: &mut SheetApp, ui: &mut Ui, r: &Json) {
    if r.get("error").is_none_or(Json::is_null) {
        ui.label(format!("{} has no error (its value is {}).", text(r, "cell"), text(r, "value")));
        return;
    }
    ui.heading(format!("{}  {}", text(r, "error"), text(r, "title")));
    ui.horizontal(|ui| {
        let at = format!("{}!{}", text(r, "sheet"), text(r, "cell"));
        link(app, ui, &at);
        ui.monospace(text(r, "formula"));
    });
    ui.label(text(r, "explanation"));
    for (k, label) in [("evidence", "In this formula"), ("likelyCauses", "Likely causes"), ("fixes", "How to fix it")] {
        let items: Vec<String> =
            r.get(k).and_then(Json::as_array).map(|a| a.iter().filter_map(Json::as_str).map(str::to_string).collect()).unwrap_or_default();
        if items.is_empty() {
            continue;
        }
        ui.add_space(4.0);
        ui.strong(label);
        for i in items {
            ui.label(format!("- {i}"));
        }
    }
}
