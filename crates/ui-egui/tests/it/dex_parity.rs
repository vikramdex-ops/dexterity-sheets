//! Dexterity Sheets Excel-parity additions driven through the real UI (keys and commands):
//! F4 / Ctrl+Y repeat, the Remove Arrows variants, Focus Cell, validation circles and Show Changes.

use egui::{Event, Key, Modifiers};
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
