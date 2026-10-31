//! Dexterity Sheets Excel-parity additions driven through the real UI (keys and commands):
//! F4 / Ctrl+Y repeat, the Remove Arrows variants, Focus Cell, validation circles and Show Changes.

use egui::{Event, Key, Modifiers};
use egui_kittest::kittest::Queryable as _;
use gridcraft_engine::Session;
use gridcraft_engine::core::CellRef;
use gridcraft_ui_egui::SheetApp;
use serde_json::json;

type Harness = egui_kittest::Harness<'static, SheetApp>;

const CTRL: Modifiers = Modifiers { alt: false, ctrl: true, shift: false, mac_cmd: false, command: true };

fn harness(session: Session) -> Harness {
    crate::support::harness(session, 4)
}

fn book() -> Session {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("range.setValues", json!({"range": "A1", "values": [[1, 2], [3, 4], [5, 6]]})).unwrap();
    s
}

fn key(h: &mut Harness, k: Key, m: Modifiers) {
    h.input_mut().events.push(Event::Key { key: k, physical_key: Some(k), pressed: true, repeat: false, modifiers: m });
    h.input_mut().events.push(Event::Key { key: k, physical_key: Some(k), pressed: false, repeat: false, modifiers: m });
    h.run_steps(2);
}

fn bold(h: &Harness, a1: &str) -> bool {
    let d = h.state().session.active().unwrap();
    let sh = d.wb.active().unwrap();
    d.wb.styles.get(sh.style_id(CellRef::parse(a1).unwrap())).font.bold
}

fn select(h: &mut Harness, range: &str) {
    h.state_mut().session.run("selection.set", json!({"range": range})).unwrap();
    h.run_steps(2);
}

#[test]
fn f4_and_ctrl_y_repeat_the_last_action_on_the_selection() {
    let mut h = harness(book());
    select(&mut h, "A1");
    key(&mut h, Key::B, CTRL);
    assert!(bold(&h, "A1"));
    select(&mut h, "B2");
    key(&mut h, Key::F4, Modifiers::NONE);
    assert!(bold(&h, "B2"), "F4 repeats Bold on the new selection");
    // Ctrl+Y with nothing to redo repeats too.
    select(&mut h, "A3");
    key(&mut h, Key::Y, CTRL);
    assert!(bold(&h, "A3"), "Ctrl+Y repeats when there is nothing to redo");
    // With something to redo, Ctrl+Y redoes instead.
    key(&mut h, Key::Z, CTRL);
    assert!(!bold(&h, "A3"));
    key(&mut h, Key::Y, CTRL);
    assert!(bold(&h, "A3"), "Ctrl+Y redoes the undone repeat");
}

#[test]
fn remove_arrow_variants_clear_only_their_kind() {
    let mut s = book();
    s.execute("cell.set", json!({"cell": "C1", "input": "=A1+B1"})).unwrap();
    s.execute("selection.set", json!({"range": "C1"})).unwrap();
    let mut h = harness(s);
    let r = h.state_mut().run("formulas.tracePrecedents", json!({})).unwrap();
    h.state_mut().grid.trace = gridcraft_ui_egui::grid::Trace::new(h.state(), r);
    h.state_mut().run("formulas.removeDependentArrows", json!({})).unwrap();
    assert!(h.state().grid.trace.is_some(), "dependent arrows leave precedent arrows");
    h.state_mut().run("formulas.removePrecedentArrows", json!({})).unwrap();
    assert!(h.state().grid.trace.is_none());
}

#[test]
fn focus_cell_validation_circles_and_show_changes_render() {
    let mut s = book();
    s.execute("data.validation", json!({"range": "A1:A3", "type": "whole", "operator": "lessOrEqual", "formula1": "3"})).unwrap();
    let mut h = harness(s);
    h.state_mut().run("view.focusCell", json!({"color": "#3BA55C"})).unwrap();
    let r = h.state_mut().run("data.circleInvalid", json!({})).unwrap();
    assert_eq!(r["invalid"], json!(["A3"]));
    assert_eq!(h.state().session.dex.circles.len(), 1);
    h.run_steps(2);
    assert!(h.state().session.dex.focus_cell);
    h.state_mut().run("data.clearValidationCircles", json!({})).unwrap();
    assert!(h.state().session.dex.circles.is_empty());
    // Show Changes opens the Edit History pane, which explains when the workbook isn't shared.
    h.state_mut().run("review.showChanges", json!({})).unwrap();
    h.run_steps(2);
    assert_eq!(h.state().grid.pane.as_deref(), Some("history"));
}

#[test]
fn conditional_formatting_with_no_rule_opens_new_formatting_rule() {
    let mut h = harness(book());
    h.state_mut().run_or_alert("home.conditionalFormat", json!({}));
    h.run_steps(2);
    let d = h.state().dialog.as_ref().map(|d| d.name().to_string());
    assert_eq!(d.as_deref(), Some("cfQuick"), "the request for newFormattingRule must open the New Rule dialog");
}

#[test]
fn result_dialogs_read_as_text_not_json() {
    let mut s = book();
    s.execute("cell.set", json!({"cell": "C1", "input": "=1/0"})).unwrap();
    let mut h = harness(s);
    h.state_mut().open_dialog("statistics", json!({}));
    h.run_steps(2);
    assert!(h.query_all_by_label("Cells with data").count() >= 2, "sheet and workbook tables");
    h.state_mut().open_dialog("errorChecking", json!({}));
    h.run_steps(2);
    h.get_by_label("#DIV/0!");
    h.state_mut().open_dialog("accessibility", json!({}));
    h.run_steps(2);
    assert!(h.query_by_label("[]").is_none(), "no raw JSON");
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("dexsheets-it-{tag}-{}-{}", std::process::id(), gridcraft_ui_egui::now_ms() as i64));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn workbook_with(a1: &str) -> gridcraft_engine::model::Workbook {
    let mut s = Session::new();
    s.new_workbook();
    s.execute("cell.set", json!({"cell": "A1", "input": a1})).unwrap();
    (*s.active().unwrap().wb).clone()
}

fn xlsx_file(dir: &std::path::Path, name: &str, a1: &str) -> String {
    let path = dir.join(name);
    std::fs::write(&path, gridcraft_engine::io::save_bytes(&workbook_with(a1), name).unwrap()).unwrap();
    path.to_string_lossy().into_owned()
}

#[test]
fn recover_unsaved_compare_opens_the_compare_window_and_recover_keeps_the_copy() {
    use gridcraft_ui_egui::dex_ui::recovery::{ItemMeta, Recovery};
    let base = temp_dir("recovery");
    let saved = xlsx_file(&base, "Tracker.xlsx", "saved");
    // A run that crashed with unsaved work in Tracker.xlsx.
    let mut crashed = Recovery::with_base(base.clone());
    let meta = ItemMeta { title: "Tracker.xlsx".into(), path: Some(saved.clone()), saved_at: 1, cells: 1 };
    crashed.write_copy(5, &workbook_with("unsaved 7Q"), &meta).unwrap();
    crashed.clean_exit(&[(5, std::sync::Arc::new(workbook_with("unsaved 7Q")), meta)]);
    drop(crashed);

    let mut h = harness(book());
    h.state_mut().collab.recovery = Recovery::with_base(base.clone());
    let r = h.state_mut().run("file.recoverUnsaved", json!({"action": "list"})).unwrap();
    assert_eq!(r["items"].as_array().map(Vec::len), Some(1), "{r}");
    assert!(!h.state().collab.recovery.show, "a programmatic list opens no window");
    h.state_mut().run("file.recoverUnsaved", json!({"action": "compare", "index": 0})).unwrap();
    h.run_steps(2);
    let shown = h.state().collab.tools.shown.as_ref().map(|(id, r)| (id.clone(), r["counts"]["value"].as_u64()));
    assert_eq!(shown, Some(("review.compareFiles".into(), Some(1))), "Compare opens the Compare window with the one changed value");

    let copy = h.state().collab.recovery.found[0].file.clone();
    h.state_mut().run("file.recoverUnsaved", json!({"action": "recover", "index": 0})).unwrap();
    assert!(h.state().session.documents().iter().any(|d| d.title.contains("(Recovered)")));
    assert!(copy.is_file(), "the copy stays until this run has copied the recovered workbook itself");
    assert_eq!(h.state().collab.recovery.pending_files(), vec![copy.clone()]);
    let r = h.state_mut().run("file.recoverUnsaved", json!({"action": "list"})).unwrap();
    assert_eq!(r["items"].as_array().map(Vec::len), Some(0), "a copy already recovered is not offered again: {r}");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn compare_files_with_nothing_named_is_an_error_unless_shown() {
    let mut h = harness(book());
    let e = h.state_mut().run("review.compareFiles", json!({})).unwrap_err();
    assert!(e.contains("show: true"), "{e}");
    assert!(!h.state().collab.tools.compare_form, "a programmatic call opens no window");
    h.state_mut().run("review.compareFiles", json!({"show": true})).unwrap();
    assert!(h.state().collab.tools.compare_form);
}

#[test]
fn compare_form_browse_buttons_pick_the_files() {
    let base = temp_dir("browse");
    let old = xlsx_file(&base, "Rev_A.xlsx", "a");
    let new = xlsx_file(&base, "Rev_B.xlsx", "b");
    let picks = std::sync::Arc::new(std::sync::Mutex::new(vec![new.clone(), old.clone()]));
    let p = picks.clone();
    let services = gridcraft_ui_egui::Services { pick_open: Some(Box::new(move || p.lock().unwrap().pop())), ..Default::default() };
    let mut h = crate::support::services_harness(services, 4);
    h.state_mut().run("review.compareFiles", json!({"show": true})).unwrap();
    h.run_steps(2);
    for _ in 0..2 {
        let browse: Vec<_> = h.query_all_by_label("Browse…").collect();
        assert_eq!(browse.len(), 2, "one Browse… button per side");
        drop(browse);
        let n = picks.lock().unwrap().len();
        h.query_all_by_label("Browse…").nth(2 - n).unwrap().click();
        h.run_steps(2);
    }
    assert!(picks.lock().unwrap().is_empty(), "both Browse… buttons asked the file picker");
    h.get_by_role_and_label(egui::accesskit::Role::Button, "Compare").click();
    h.run_steps(2);
    let shown = h.state().collab.tools.shown.as_ref().map(|(id, r)| (id.clone(), r["counts"]["value"].as_u64()));
    assert_eq!(shown, Some(("review.compareFiles".into(), Some(1))), "the picked files were compared: {:?}", h.state().message);
    assert!(!h.state().collab.tools.compare_form);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn new_from_template_with_nothing_named_lists_without_opening_the_window() {
    let mut h = harness(book());
    h.state_mut().collab.templates.set_personal_dir(temp_dir("tpl-list"));
    let r = h.state_mut().run("file.newFromTemplate", json!({})).unwrap();
    assert_eq!(r["personal"], json!([]));
    assert!(!h.state().collab.templates.window, "a programmatic call opens no window");
    h.state_mut().run("file.newFromTemplate", json!({"show": true})).unwrap();
    assert!(h.state().collab.templates.window);
    let e = h.state_mut().run("automate.rules", json!({})).unwrap_err();
    assert!(e.contains("share"), "{e}");
    assert!(!h.state().collab.automation.window);
}

#[test]
fn a_personal_template_is_not_replaced_without_overwrite() {
    let dir = temp_dir("tpl-save");
    let mut h = harness(book());
    h.state_mut().collab.templates.set_personal_dir(dir.clone());
    h.state_mut().run("file.saveAsTemplate", json!({"name": "Tracker"})).unwrap();
    assert!(dir.join("Tracker.xlsx").is_file());
    let e = h.state_mut().run("file.saveAsTemplate", json!({"name": "Tracker"})).unwrap_err();
    assert!(e.contains("overwrite: true"), "{e}");
    h.state_mut().run("file.saveAsTemplate", json!({"name": "Tracker", "overwrite": true})).unwrap();
    let r = h.state_mut().run("file.newFromTemplate", json!({})).unwrap();
    assert_eq!(r["personal"], json!(["Tracker"]));
    h.state_mut().run("file.newFromTemplate", json!({"personal": "tracker"})).unwrap();
    assert!(h.state().session.documents().iter().any(|d| d.title == "tracker1"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn creating_from_an_office_template_waits_for_the_server_and_says_when_it_does_not_answer() {
    use gridcraft_collab::proto::{self, TemplateInfo};
    use gridcraft_ui_egui::dex_ui::templates::{WAIT_MS, on_template};
    let mut h = harness(book());
    let info = TemplateInfo { id: "t1".into(), name: "Tracker".into(), ..TemplateInfo::default() };
    h.state_mut().collab.templates.list = vec![info.clone()];
    h.state_mut().collab.templates.window = true;
    h.run_steps(2);
    h.get_by_label("Tracker").click();
    h.run_steps(2);
    let now = gridcraft_ui_egui::monotonic_ms();
    h.state_mut().collab.templates.wait_for("t1".into(), now);
    h.run_steps(2);
    assert!(h.query_by_label("Getting the template from the office server…").is_some(), "the window says it is waiting");
    assert!(h.state().collab.templates.window, "the window stays open while waiting");
    // The template arrives: the spreadsheet is made and the window closes.
    let bytes = gridcraft_engine::io::save_bytes(&workbook_with("from template"), "t.xlsx").unwrap();
    on_template(h.state_mut(), info, &proto::b64(&bytes), &proto::sha256_hex(&bytes));
    h.run_steps(2);
    assert!(h.state().session.documents().iter().any(|d| d.title == "Tracker1"));
    assert!(!h.state().collab.templates.window);
    assert_eq!(h.state().collab.templates.waiting(), None);
    // The server never answers: after WAIT_MS the window says so instead of spinning for ever.
    h.state_mut().collab.templates.window = true;
    h.state_mut().collab.templates.wait_for("t1".into(), gridcraft_ui_egui::monotonic_ms() - WAIT_MS - 1.0);
    h.run_steps(2);
    assert_eq!(h.state().collab.templates.waiting(), None);
    let e = h.state().collab.templates.error().unwrap_or_default().to_string();
    assert!(e.contains("did not send the template"), "{e}");
    assert!(h.query_by_label_contains("did not send the template").is_some());
}

#[test]
fn watch_window_and_show_comments_open_their_panes_from_any_entry_point() {
    let mut h = harness(book());
    h.state_mut().run_or_alert("formulas.watchWindow", json!({}));
    h.run_steps(2);
    assert_eq!(h.state().grid.pane.as_deref(), Some("watch"));
    h.state_mut().run_or_alert("review.showComments", json!({"on": true}));
    h.run_steps(2);
    assert!(h.state().session.comments_pane, "Comments stays open in place of the Watch Window");
    assert_eq!(h.state().grid.pane, None);
}
