//! Dexterity Sheets additions to the command set: Excel features the 2026-10-28 parity audit
//! found missing (`docs/dexterity-parity-audit.md`). They live in this one file, with their
//! state in [`DexState`], so upstream GridCraft merges stay clean; `cmd.rs` and `lib.rs` hold
//! only the hook lines that `packaging/dexterity/check_overlay.py` checks.

use std::collections::BTreeSet;

use gridcraft_core::{CellRef, RangeRef};
use serde_json::{Value as Json, json};

use super::*;
use crate::outline;

mod compare;
mod diagnostics;
mod quality;

/// Excel draws at most this many validation circles at a time.
pub const MAX_CIRCLES: usize = 255;
/// Auto Outline reads at most this many formulas on a sheet.
const AUTO_OUTLINE_FORMULAS: usize = 200_000;

/// Session state the Dexterity commands keep (front ends read it to draw).
#[derive(Clone, Debug, Default)]
pub struct DexState {
    /// The last top-level command that changed a workbook and can be repeated (F4 / Ctrl+Y),
    /// with the parameters that name where it ran removed, so it runs on the selection.
    pub repeat: Option<(String, Json)>,
    /// View ▸ Focus Cell: the active cell's row and column are highlighted.
    pub focus_cell: bool,
    /// Focus Cell colour as `#RRGGBB`; `None` = the front end's default.
    pub focus_color: Option<String>,
    /// Data ▸ Circle Invalid Data: (document uid, sheet index, cell) for each circle.
    pub circles: Vec<(u64, usize, CellRef)>,
    /// How many times each command ran (ids only, never parameters), for the office server's
    /// feature-adoption report; the desktop app takes and resets it about once an hour.
    pub usage: std::collections::BTreeMap<String, u64>,
}

/// Parameters that say where a command ran. Repeat drops them so the command runs on the
/// selection, as Excel's Repeat does.
const PLACE_KEYS: &[&str] = &["range", "ranges", "cell", "at", "rows", "cols", "sheet", "target"];

/// Commands Repeat never repeats: navigation, files, windows, history and recording, and
/// Repeat itself.
fn repeatable(id: &str) -> bool {
    const NEVER_PREFIX: &[&str] = &["selection.", "file.", "window.", "history.", "automate.", "document.", "app.", "collab.", "view."];
    const NEVER: &[&str] = &["edit.undo", "edit.redo", "edit.repeat", "edit.copy", "edit.cut", "sheet.activate", "sheet.next", "sheet.previous"];
    !NEVER.contains(&id) && !NEVER_PREFIX.iter().any(|p| id.starts_with(p))
}

/// Called by [`Session::execute`] after every top-level command. `before` is the active
/// document's (uid, revision) before it ran: a new revision means it made an undo step.
pub fn after_command(s: &mut Session, id: &str, params: &Json, result: &Result<Json>, before: Option<(u64, u64)>) {
    let Ok(value) = result else { return };
    // Bounded by the command catalogue (unknown ids fail before they get here).
    if s.dex.usage.len() < 4096 || s.dex.usage.contains_key(id) {
        let n = s.dex.usage.entry(id.to_string()).or_insert(0);
        *n = n.saturating_add(1);
    }
    if id == "data.circleInvalid" {
        if let Some(d) = s.active() {
            let (uid, sheet) = (d.uid, d.wb.active_sheet());
            let cells: Vec<CellRef> = value
                .get("invalid")
                .and_then(Json::as_array)
                .map(|a| a.iter().filter_map(Json::as_str).filter_map(CellRef::parse).take(MAX_CIRCLES).collect())
                .unwrap_or_default();
            s.dex.circles.retain(|(u, sh, _)| !(*u == uid && *sh == sheet));
            s.dex.circles.extend(cells.into_iter().map(|c| (uid, sheet, c)));
        }
        return;
    }
    let changed = before.is_some_and(|(uid, rev)| s.active().is_some_and(|d| d.uid == uid && d.revision != rev));
    if changed && repeatable(id) {
        let kept = match params {
            Json::Object(o) => {
                Json::Object(o.iter().filter(|(k, _)| !PLACE_KEYS.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect())
            }
            other => other.clone(),
        };
        s.dex.repeat = Some((id.to_string(), journal_params(&kept)));
    }
}

/// The validation circles to draw on the active sheet: cells circled by Circle Invalid Data whose
/// value still breaks their rule (a corrected cell loses its circle, as in Excel).
pub fn validation_circles(s: &Session) -> Vec<CellRef> {
    let Some(d) = s.active() else { return Vec::new() };
    let sheet = d.wb.active_sheet();
    let Some(sh) = d.wb.sheet(sheet) else { return Vec::new() };
    s.dex
        .circles
        .iter()
        .filter(|(u, si, _)| *u == d.uid && *si == sheet)
        .map(|(_, _, c)| *c)
        .filter(|c| {
            let text = sh.input_text(*c);
            !text.is_empty() && super::data::check_validation(&d.wb, sheet, *c, &text).is_some()
        })
        .collect()
}

pub fn specs() -> Vec<CommandSpec> {
    let mut v = vec![
        cmd!(
            "edit.repeat",
            "Repeat",
            ["Home", "Undo"],
            Some("F4"),
            "{} → {command}: runs the last command that changed the workbook again, on the selection",
            can_repeat,
            repeat
        ),
        cmd!(noundo "data.clearValidationCircles", "Clear Validation Circles", ["Data", "Data Tools", "Data Validation"], None, "{} → {cleared}", has_doc, clear_circles),
        cmd!(
            "data.autoOutline",
            "Auto Outline",
            ["Data", "Outline", "Group"],
            None,
            "{sheet?, by?: rows|cols|both (default both)} → {rows, cols}: groups the detail above summary formulas (SUM(B2:B9) in B10) and left of summary columns",
            has_doc,
            auto_outline
        ),
        cmd!(
            "data.clearOutline",
            "Clear Outline",
            ["Data", "Outline", "Ungroup"],
            None,
            "{sheet?, rows?: \"2:9\", cols?: \"B:D\"} (whole sheet without them) → {rows, cols}",
            has_doc,
            clear_outline
        ),
        cmd!(noundo "formulas.removePrecedentArrows", "Remove Precedent Arrows", ["Formulas", "Formula Auditing", "Remove Arrows"], None, "{} → {removed: \"precedents\"}", has_doc, |_, _| Ok(json!({"removed": "precedents"}))),
        cmd!(noundo "formulas.removeDependentArrows", "Remove Dependent Arrows", ["Formulas", "Formula Auditing", "Remove Arrows"], None, "{} → {removed: \"dependents\"}", has_doc, |_, _| Ok(json!({"removed": "dependents"}))),
        cmd!(noundo "view.focusCell", "Focus Cell", ["View", "Show"], None, "{on?: bool (toggles without it), color?: \"#RRGGBB\"} → {on, color}", always, focus_cell),
        cmd!(
            query "file.recoverUnsaved",
            "Recover Unsaved Work",
            ["File", "Info", "Manage Workbook"],
            None,
            "{action?: list|recover|openSaved|compare|discard|discardAll, index?} — the desktop app's copies of unsaved work left by a crash (the app window handles it)",
            always,
            |_, _| Ok(json!({"items": [], "note": "recovery copies are kept by the desktop app"}))
        ),
        cmd!(
            query "file.newFromTemplate",
            "New from Template",
            ["File", "New"],
            None,
            "{id? (office template), personal? (name)} — a new, unsaved spreadsheet with the template's content; without either, the Templates window (the desktop app)",
            always,
            |_, _| Err(bad("file.newFromTemplate", "templates are kept by the desktop app and the office server"))
        ),
        cmd!(
            query "file.saveAsTemplate",
            "Save as Template",
            ["File", "Save As"],
            None,
            "{name?, office?: bool (administrators: the office library), category?, description?, mandatory?} — the open workbook as a template (the desktop app)",
            has_doc,
            |_, _| Err(bad("file.saveAsTemplate", "templates are kept by the desktop app and the office server"))
        ),
        cmd!(
            query "automate.rules",
            "Automation Rules",
            ["Automate", "Automation"],
            None,
            "{rules?: [{name, enabled, trigger: {kind: schedule, every: daily|weekly, day, time} | {kind: change, column, equals}, actions: [{kind: notify, people} | {kind: qualityCheck} | {kind: overdue, dateColumn, statusColumn, done} | {kind: report} | {kind: stamp, column}]}], run?: rule id} — a shared workbook's rules, run by the office server (the desktop app)",
            has_doc,
            |_, _| Err(bad("automate.rules", "automation rules belong to shared workbooks on the office server (the desktop app)"))
        ),
        cmd!(query "help.whatsNew", "What's New", ["Help"], None, "{} — this version's release notes, with Report a Problem below (the app window shows them)", always, |_, _| Ok(json!({}))),
        cmd!(
            query "help.reportProblem",
            "Report a Problem",
            ["Help"],
            None,
            "{features?: [opening|saving|formulas|formatting|importExport|sharing|notifications|startup|performance|other], description?, expected?, actual?, severity?: low|medium|high|blocking, afterUpdate?, rolledBack?, logs?: bool, send?: bool} — a problem report to the office server's administrator (the app window sends it)",
            always,
            |_, _| Err(bad("help.reportProblem", "problem reports are sent by the desktop app, signed in to the office server"))
        ),
        cmd!(
            query "help.rollBack",
            "Roll Back to an Earlier Version",
            ["Help"],
            None,
            "{version?} — installs an earlier Dexterity Sheets from the office server (lists them without `version`); workbooks are not touched",
            always,
            |_, _| Err(bad("help.rollBack", "rolling back is done by the desktop app, signed in to the office server"))
        ),
        cmd!(
            query "help.updateChannel",
            "Update Channel",
            ["Help"],
            None,
            "{channel?: stable|testing} → {channel}: which versions the office server offers this PC",
            always,
            |_, _| Err(bad("help.updateChannel", "the update channel is the desktop app's setting"))
        ),
        cmd!(noundo "review.showChanges", "Show Changes", ["Review", "Changes"], None, "{} — opens the shared workbook's edit history in the app window (Review ▸ Edit History)", has_doc, show_changes),
    ];
    v.extend(compare::specs());
    v.extend(diagnostics::specs());
    v.extend(quality::specs());
    v
}

fn can_repeat(s: &Session) -> std::result::Result<(), String> {
    has_doc(s)?;
    if s.dex.repeat.is_some() { Ok(()) } else { Err("nothing to repeat".into()) }
}

fn repeat(s: &mut Session, _: &Json) -> Result<Json> {
    let Some((id, params)) = s.dex.repeat.clone() else { return Err(bad("edit.repeat", "nothing to repeat")) };
    // One undo step, named after the command repeated ("Bold"), as Excel names it.
    let label = find_command(&id).map_or("Repeat", |c| c.label.trim_end_matches('…'));
    let r = s.grouped(label, |s| s.execute(&id, params))?;
    Ok(json!({"command": id, "result": r}))
}

fn clear_circles(s: &mut Session, _: &Json) -> Result<Json> {
    let d = s.doc()?;
    let (uid, sheet) = (d.uid, d.wb.active_sheet());
    let before = s.dex.circles.len();
    s.dex.circles.retain(|(u, sh, _)| !(*u == uid && *sh == sheet));
    Ok(json!({"cleared": before - s.dex.circles.len()}))
}

fn focus_cell(s: &mut Session, p: &Json) -> Result<Json> {
    if let Some(c) = str_param(p, "color") {
        let hex = c.trim_start_matches('#');
        if hex.len() != 6 || !hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return Err(bad("view.focusCell", "color must be #RRGGBB"));
        }
        s.dex.focus_color = Some(format!("#{}", hex.to_ascii_uppercase()));
        // Picking a colour turns the highlight on, as in Excel's colour menu.
        s.dex.focus_cell = bool_param(p, "on").unwrap_or(true);
    } else {
        s.dex.focus_cell = bool_param(p, "on").unwrap_or(!s.dex.focus_cell);
    }
    Ok(json!({"on": s.dex.focus_cell, "color": s.dex.focus_color}))
}

/// The list itself lives in the app window (the Edit History pane, fed by the office server);
/// the app opens it and runs this so the step is journaled. Headless callers get where it is.
fn show_changes(_: &mut Session, _: &Json) -> Result<Json> {
    Ok(json!({"pane": "history", "note": "Show Changes lists who changed which cell in a shared workbook, in the app window's Edit History pane"}))
}

/// The detail spans of the sheet's summary formulas, as `(first, last)` lines. A summary formula
/// reads cells of its own column (rows; its own row for columns) that end on the line just
/// before it (or start just after it, when summaries sit above the detail); its detail runs from
/// the first line it reads to the last. A summary that reads other summaries (a grand total
/// `=B4+B7`) takes in their detail too, which nests the outline one level deeper.
fn summary_spans(s: &Session, sheet: usize, rows: bool) -> Result<BTreeSet<(u32, u32)>> {
    use std::collections::BTreeMap;
    let d = s.doc()?;
    let Some(sh) = d.wb.sheet(sheet) else { return Err(EngineError::NoDocument) };
    let before = outline::summary_before(sh, rows);
    // Summary line → the lines it reads directly.
    let mut direct: BTreeMap<u32, (u32, u32)> = BTreeMap::new();
    for (at, _) in sh.cells.iter().filter(|(_, c)| c.formula.is_some()).take(AUTO_OUTLINE_FORMULAS) {
        let (line, cross) = if rows { (at.row, at.col) } else { (at.col, at.row) };
        let mut span: Option<(u32, u32)> = None;
        for a in d.calc.graph.precedents_of((sheet, at)) {
            let r: RangeRef = a.range;
            // A whole-column or whole-row reference is not a block of detail.
            let whole = if rows { r.is_full_cols() } else { r.is_full_rows() };
            let (lo, hi, cross_lo, cross_hi) =
                if rows { (r.start.row, r.end.row, r.start.col, r.end.col) } else { (r.start.col, r.end.col, r.start.row, r.end.row) };
            let on_side = if before { lo > line } else { hi < line };
            if a.sheet != sheet || whole || cross < cross_lo || cross > cross_hi || hi < lo || !on_side {
                continue;
            }
            span = Some(span.map_or((lo, hi), |(a, b)| (a.min(lo), b.max(hi))));
        }
        let Some((lo, hi)) = span else { continue };
        let next_to = if before { lo.checked_sub(1) == Some(line) } else { hi.checked_add(1) == Some(line) };
        if next_to {
            let e = direct.entry(line).or_insert((lo, hi));
            *e = (e.0.min(lo), e.1.max(hi));
        }
    }
    // Take in the detail of the summaries a summary reads, nearest first so each one is final.
    let mut full: BTreeMap<u32, (u32, u32)> = BTreeMap::new();
    let lines: Vec<u32> = if before { direct.keys().rev().copied().collect() } else { direct.keys().copied().collect() };
    for line in lines {
        let Some(&(mut lo, mut hi)) = direct.get(&line) else { continue };
        for (_, &(a, b)) in full.range(lo..=hi) {
            lo = lo.min(a);
            hi = hi.max(b);
        }
        full.insert(line, (lo, hi));
    }
    Ok(full.into_values().collect())
}

fn auto_outline(s: &mut Session, p: &Json) -> Result<Json> {
    let si = target_sheet(s, p)?;
    let (do_rows, do_cols) = match str_param(p, "by") {
        Some("rows") => (true, false),
        Some("cols" | "columns") => (false, true),
        None | Some("both") => (true, true),
        Some(other) => return Err(bad("data.autoOutline", format!("by: rows, cols or both, not {other:?}"))),
    };
    let row_spans = if do_rows { summary_spans(s, si, true)? } else { BTreeSet::new() };
    let col_spans = if do_cols { summary_spans(s, si, false)? } else { BTreeSet::new() };
    if row_spans.is_empty() && col_spans.is_empty() {
        return Err(bad("data.autoOutline", "Cannot create an outline: no summary formulas sit next to the rows or columns they total"));
    }
    let (nr, nc) = (row_spans.len(), col_spans.len());
    edit(s, |cx| {
        let sh = cx.sheet_mut(si)?;
        for (spans, map) in [(&row_spans, &mut sh.rows), (&col_spans, &mut sh.cols)] {
            if spans.is_empty() {
                continue;
            }
            // Auto Outline replaces the outline it builds over, as Excel's does.
            for e in map.values_mut() {
                e.outline = 0;
                e.collapsed = false;
            }
            for &(a, b) in spans {
                for i in a..=b.min(a.saturating_add(1_048_575)) {
                    let e = map.entry(i).or_default();
                    e.outline = e.outline.saturating_add(1).min(outline::MAX_LEVEL);
                }
            }
        }
        cx.structural = true;
        Ok(json!({"rows": nr, "cols": nc}))
    })
}

fn clear_outline(s: &mut Session, p: &Json) -> Result<Json> {
    let si = target_sheet(s, p)?;
    let rows = str_param(p, "rows").and_then(parse_lines).map(|r| (r.start.row, r.end.row));
    let cols = str_param(p, "cols").and_then(parse_lines).map(|r| (r.start.col, r.end.col));
    let whole = rows.is_none() && cols.is_none();
    edit(s, |cx| {
        let sh = cx.sheet_mut(si)?;
        let mut cleared = [0usize; 2];
        for (k, (span, map)) in [(rows, &mut sh.rows), (cols, &mut sh.cols)].into_iter().enumerate() {
            if !whole && span.is_none() {
                continue;
            }
            let (a, b) = span.unwrap_or((0, u32::MAX));
            for (_, e) in map.range_mut(a..=b) {
                if e.outline > 0 || e.collapsed {
                    // Lines hidden by a collapsed group come back, as when ungrouping.
                    if e.outline > 0 {
                        e.hidden = false;
                    }
                    e.outline = 0;
                    e.collapsed = false;
                    cleared[k] += 1;
                }
            }
        }
        cx.structural = true;
        Ok(json!({"rows": cleared[0], "cols": cleared[1]}))
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::Session;

    fn session() -> Session {
        let mut s = Session::default();
        s.execute("file.new", json!({})).unwrap();
        s
    }

    fn text(s: &mut Session, cell: &str) -> String {
        s.execute("cell.get", json!({"cell": cell})).unwrap()["text"].as_str().unwrap_or_default().to_string()
    }

    #[test]
    fn repeat_runs_the_last_change_on_the_new_selection() {
        let mut s = session();
        assert!(s.execute("edit.repeat", json!({})).is_err(), "nothing to repeat yet");
        s.execute("home.bold", json!({"range": "A1"})).unwrap();
        s.execute("selection.set", json!({"range": "C3:D4"})).unwrap();
        let r = s.execute("edit.repeat", json!({})).unwrap();
        assert_eq!(r["command"], "home.bold");
        let bold = |s: &Session, a1: &str| {
            let d = s.doc().unwrap();
            let sh = d.wb.active().unwrap();
            d.wb.styles.get(sh.style_id(gridcraft_core::CellRef::parse(a1).unwrap())).font.bold
        };
        assert!(bold(&s, "A1") && bold(&s, "C3") && bold(&s, "D4"));
        // Undo takes back the repeated step only.
        s.execute("edit.undo", json!({})).unwrap();
        assert!(bold(&s, "A1") && !bold(&s, "C3"));
    }

    #[test]
    fn repeat_types_the_last_entry_and_ignores_navigation() {
        let mut s = session();
        s.execute("cell.set", json!({"cell": "A1", "input": "Tee 4\""})).unwrap();
        s.execute("selection.set", json!({"range": "B2"})).unwrap();
        s.execute("view.gridlines", json!({})).unwrap();
        s.execute("edit.repeat", json!({})).unwrap();
        assert_eq!(text(&mut s, "B2"), "Tee 4\"");
        // Undo is not repeated, and Repeat keeps the step it had.
        s.execute("edit.undo", json!({})).unwrap();
        s.execute("selection.set", json!({"range": "C5"})).unwrap();
        s.execute("edit.repeat", json!({})).unwrap();
        assert_eq!(text(&mut s, "C5"), "Tee 4\"");
    }

    #[test]
    fn circles_are_kept_until_cleared_or_corrected() {
        let mut s = session();
        s.execute("cell.set", json!({"cell": "A1", "input": "5"})).unwrap();
        s.execute("cell.set", json!({"cell": "A2", "input": "50"})).unwrap();
        s.execute("cell.set", json!({"cell": "A3", "input": "70"})).unwrap();
        s.execute("data.validation", json!({"range": "A1:A3", "type": "whole", "operator": "between", "formula1": "1", "formula2": "10"})).unwrap();
        let r = s.execute("data.circleInvalid", json!({})).unwrap();
        assert_eq!(r["invalid"], json!(["A2", "A3"]));
        let circles = |s: &Session| super::validation_circles(s).iter().map(|c| c.a1()).collect::<Vec<_>>();
        assert_eq!(circles(&s), ["A2", "A3"]);
        // A corrected value loses its circle.
        s.execute("cell.set", json!({"cell": "A2", "input": "7"})).unwrap();
        assert_eq!(circles(&s), ["A3"]);
        assert_eq!(s.execute("data.clearValidationCircles", json!({})).unwrap()["cleared"], 2);
        assert!(circles(&s).is_empty());
    }

    #[test]
    fn auto_outline_groups_detail_above_totals_and_clear_outline_removes_it() {
        let mut s = session();
        for (cell, input) in [("B2", "1"), ("B3", "2"), ("B4", "=SUM(B2:B3)"), ("B5", "4"), ("B6", "5"), ("B7", "=SUM(B5:B6)"), ("B8", "=B4+B7")] {
            s.execute("cell.set", json!({"cell": cell, "input": input})).unwrap();
        }
        let r = s.execute("data.autoOutline", json!({"by": "rows"})).unwrap();
        // Two subtotals and a grand total over both: two levels, as Excel builds them.
        assert_eq!(r["rows"], 3);
        let level = |s: &Session, row: u32| s.doc().unwrap().wb.active().unwrap().rows.get(&row).map_or(0, |e| e.outline);
        let levels: Vec<u8> = (0..8).map(|row| level(&s, row)).collect();
        assert_eq!(levels, [0, 2, 2, 1, 2, 2, 1, 0]);
        s.execute("data.hideDetail", json!({"rows": "2:3"})).unwrap();
        let r = s.execute("data.clearOutline", json!({})).unwrap();
        assert_eq!(r["rows"], 6, "every outlined row, the collapsed summary among them");
        assert!((0..8).all(|row| level(&s, row) == 0));
        assert!(!s.doc().unwrap().wb.active().unwrap().rows.values().any(|e| e.hidden || e.collapsed));
        // Nothing to outline is an error, as in Excel.
        let mut empty = session();
        assert!(empty.execute("data.autoOutline", json!({})).is_err());
    }

    #[test]
    fn focus_cell_toggles_and_takes_a_colour() {
        let mut s = session();
        assert_eq!(s.execute("view.focusCell", json!({})).unwrap()["on"], true);
        assert_eq!(s.execute("view.focusCell", json!({})).unwrap()["on"], false);
        let r = s.execute("view.focusCell", json!({"color": "#ffcc00"})).unwrap();
        assert_eq!(r, json!({"on": true, "color": "#FFCC00"}));
        assert!(s.execute("view.focusCell", json!({"color": "yellow"})).is_err());
    }

    #[test]
    fn show_changes_says_where_it_opens_headless() {
        let mut s = session();
        let r = s.execute("review.showChanges", json!({})).unwrap();
        assert_eq!(r["pane"], "history");
    }
}
