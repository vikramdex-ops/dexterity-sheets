//! Every Dexterity Sheets feature must still be reachable after an upstream GridCraft sync.
//!
//! `check_overlay.py` proves each hook's text is in the source; it cannot prove the code still
//! runs (a hook can sit in a branch upstream stopped calling). This test drives each Dexterity
//! command through the real window, the same path the ribbon, menus and keys use, so a sync that
//! leaves a feature compiled but unreachable fails CI before it reaches a release.
//!
//! Add every new Dexterity command here in the same change that adds it.

use egui_kittest::kittest::Queryable;
use gridcraft_engine::Session;
use serde_json::json;

/// Dexterity commands that are safe to run with no parameters and no server: they open a page,
/// a pane or a dialog, or toggle a view. (feature, command id)
const REACHABLE: &[(&str, &str)] = &[
    ("File › Shared page", "collab.shared"),
    ("Edit history pane", "collab.history"),
    ("Review › Show Changes", "review.showChanges"),
    ("Versions pane", "collab.versions"),
    ("Notification settings", "collab.notifications"),
    ("Focus Cell", "view.focusCell"),
    ("What's New", "help.whatsNew"),
    ("Remove precedent arrows", "formulas.removePrecedentArrows"),
    ("Remove dependent arrows", "formulas.removeDependentArrows"),
    ("Clear validation circles", "data.clearValidationCircles"),
    ("People panel", "collab.people"),
    ("Assigned to Me panel", "collab.assigned"),
    ("Assign To dialog", "collab.assign"),
    ("Mark an assignment done", "collab.setAssignment"),
    ("Connect quick panel", "collab.connectPanel"),
    ("Discuss in Connect", "collab.discussInConnect"),
];

fn person(user: &str, display: &str, online: bool, books: &[(&str, &str)]) -> gridcraft_collab::proto::PersonInfo {
    gridcraft_collab::proto::PersonInfo {
        user: user.into(),
        account: user.to_uppercase(),
        display: display.into(),
        color: [0x1f, 0x77, 0xb4],
        role: "editor".into(),
        online,
        books: books.iter().map(|(a, b)| ((*a).to_string(), (*b).to_string())).collect(),
    }
}

/// The People panel shows the server's list; that list is also who can be @mentioned or picked.
#[test]
fn dex_people_panel_shows_the_server_list() {
    let mut h = app();
    let people = vec![person(r"dds\anita", "Anita Rao", true, &[("line-1", "Line list 100")]), person(r"dds\rahul", "Rahul", false, &[])];
    gridcraft_ui_egui::dex_ui::people::on_people(h.state_mut(), people);
    assert_eq!(h.state().collab.users.iter().map(|u| u.display.as_str()).collect::<Vec<_>>(), vec!["Anita Rao", "Rahul"]);
    let listed = h.state_mut().run("collab.people", json!({})).expect("people");
    assert_eq!(listed[0]["books"], json!(["Line list 100"]));
    assert_eq!(listed[1]["online"], json!(false));
    assert!(h.state().collab.people.panel);
    h.run_steps(3);
    // Not signed in here: the window says so instead of listing anyone.
    assert!(h.query_by_label_contains("Connect to the office server (File › Shared) to see your colleagues.").is_some());
    h.state_mut().run("collab.people", json!({"show": false})).expect("close");
    assert!(!h.state().collab.people.panel);
}

/// Assign To, Assigned to Me and Discuss in Connect need a shared spreadsheet and say so.
#[test]
fn dex_assignment_commands_need_a_shared_spreadsheet() {
    let mut h = app();
    let e = h.state_mut().run("collab.assign", json!({"user": r"dds\anita"})).expect_err("not shared");
    assert!(e.contains("Share the spreadsheet first"), "{e}");
    let e = h.state_mut().run("collab.discussInConnect", json!({})).expect_err("not shared");
    assert!(e.contains("Share the spreadsheet first"), "{e}");
    assert!(h.state().collab.people.last_launch.is_none(), "nothing was handed to Windows");
    let e = h.state_mut().run("collab.setAssignment", json!({})).expect_err("needs the item");
    assert!(e.contains("book and id"), "{e}");
    h.state_mut().run("collab.assigned", json!({})).expect("assigned");
    assert!(h.state().collab.people.assigned_panel);
    h.run_steps(3);
    assert!(h.query_by_label_contains("to see what is assigned to you").is_some());
}

/// The Connect button sits at the left of the status bar and opens the quick panel; its count is
/// the open assignments.
#[test]
fn dex_connect_button_opens_the_quick_panel() {
    let mut h = app();
    h.run_steps(2);
    h.get_by_label("Connect").click();
    h.run_steps(3);
    assert!(h.state().collab.people.connect_panel, "the status-bar button opens the panel");
    assert!(h.query_by_label_contains("No mentions or assignments yet.").is_some());
    let item = gridcraft_collab::proto::Assignment { id: "a1".into(), status: "open".into(), ..Default::default() };
    gridcraft_ui_egui::dex_ui::people::on_assigned(h.state_mut(), vec![item]);
    h.run_steps(2);
    assert!(h.query_by_label("Connect · 1").is_some(), "the open assignment is counted");
    h.state_mut().run("collab.connectPanel", json!({"on": false})).expect("close");
    assert!(!h.state().collab.people.connect_panel);
}

/// A mention or an assignment notice is in the panel with Go to cell, and its Windows
/// notification goes to the cell.
#[test]
fn dex_mentions_and_assignments_reach_the_connect_panel() {
    let mut h = app();
    let n = gridcraft_collab::proto::Notice {
        kind: "assigned".into(),
        by: "Vikram".into(),
        name: "Line list 100".into(),
        book: "line-1".into(),
        sheet: "Line List".into(),
        cell: "B4".into(),
        text: "Check the rating".into(),
        ..Default::default()
    };
    gridcraft_ui_egui::dex_ui::share::notice(h.state_mut(), n);
    h.state_mut().run("collab.connectPanel", json!({"on": true})).expect("open");
    h.run_steps(3);
    assert!(h.query_by_label("Vikram assigned you").is_some());
    assert!(h.query_by_label("Go to cell").is_some());
    let kinds = gridcraft_ui_egui::dex_ui::share::kinds();
    assert!(kinds.contains_key("assigned") && kinds.contains_key("assignmentDone"), "people can turn them off");
}

fn app() -> egui_kittest::Harness<'static, gridcraft_ui_egui::SheetApp> {
    let mut s = Session::new();
    s.new_workbook();
    crate::support::harness(s, 4)
}

#[test]
fn every_dexterity_command_is_reachable_through_the_window() {
    let mut h = app();
    // The detector below must recognise a command the window does not know.
    let bogus = h.state_mut().run("dex.noSuchFeature", json!({})).expect_err("a made-up id must fail");
    assert!(bogus.contains("unknown command"), "unexpected error text for an unknown id: {bogus}");
    let mut broken = Vec::new();
    for (feature, id) in REACHABLE {
        let r = h.state_mut().run(id, json!({}));
        h.run_steps(2);
        if let Err(e) = r
            && (e.contains("unknown command") || e.contains("not a command"))
        {
            broken.push(format!("{feature} ({id}): {e}"));
        }
    }
    assert!(broken.is_empty(), "Dexterity features no longer reachable:\n{}", broken.join("\n"));
}
