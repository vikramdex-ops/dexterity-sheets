//! Every Dexterity Sheets feature must still be reachable after an upstream GridCraft sync.
//!
//! `check_overlay.py` proves each hook's text is in the source; it cannot prove the code still
//! runs (a hook can sit in a branch upstream stopped calling). This test drives each Dexterity
//! command through the real window, the same path the ribbon, menus and keys use, so a sync that
//! leaves a feature compiled but unreachable fails CI before it reaches a release.
//!
//! Add every new Dexterity command here in the same change that adds it.

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
];

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
        if let Err(e) = r {
            if e.contains("unknown command") || e.contains("not a command") {
                broken.push(format!("{feature} ({id}): {e}"));
            }
        }
    }
    assert!(broken.is_empty(), "Dexterity features no longer reachable:\n{}", broken.join("\n"));
}
