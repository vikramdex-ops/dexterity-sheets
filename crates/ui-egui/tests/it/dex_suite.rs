//! Dexterity Sheets' office-suite integrations (`dex_ui::suite`): links to a cell (`?at=`),
//! Share to Connect, Notes (Slate or the pane), Find in Sleuth, Open in Project Opener and links
//! in cells to the office apps. Which apps are installed is set by the test, never looked up.

use egui_kittest::kittest::Queryable as _;
use gridcraft_engine::Session;
use gridcraft_ui_egui::SheetApp;
use gridcraft_ui_egui::dex_ui::suite::{self, Installed, Project};
use serde_json::json;

type Harness = egui_kittest::Harness<'static, SheetApp>;

/// A line list: headers in row 1, two rows of values.
fn book() -> Session {
    let mut s = Session::new();
    s.new_workbook();
    s.execute(
        "range.setValues",
        json!({"range": "A1", "values": [["Line", "Size", "Spec", "Status"], ["100-P-001", "6\"", "A1", "Issued"], ["P12513", "", "B2", "Hold"]]}),
    )
    .unwrap();
    s
}

fn all_apps(dir: &std::path::Path) -> Installed {
    Installed {
        connect: true,
        sleuth: true,
        datum: true,
        project_opener: true,
        slate: Some(dir.join("slate.exe")),
        workspace: Some(dir.join("workspace.json")),
    }
}

/// A folder of its own under the system temp folder.
fn temp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("dexsheets-suite-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn harness(session: Session, apps: Installed) -> Harness {
    let mut h = crate::support::harness(session, 4);
    suite::set_installed(h.state_mut(), apps);
    h.run_steps(2);
    h
}

/// Saves the active workbook as `<dir>/Line List.xlsx` (only its path matters here).
fn saved_in(h: &mut Harness, dir: &std::path::Path) -> std::path::PathBuf {
    let p = dir.join("Line List.xlsx");
    h.state_mut().session.active_mut().unwrap().path = Some(p.display().to_string());
    p
}

fn select(h: &mut Harness, cell: &str) {
    h.state_mut().run("selection.set", json!({"cell": cell})).unwrap();
    h.run_steps(2);
}

// ---- Links -----------------------------------------------------------------------------------

#[test]
fn cell_links_encode_the_sheet_and_read_back_ranges_and_quoted_names() {
    let l = suite::link_at("PC-SERVER:47810", "line-list-3f2a", "Line List", "B4");
    assert_eq!(l, "dexterity://open/PC-SERVER:47810/line-list-3f2a?at=Line%20List!B4");
    assert_eq!(suite::parse_link(&l), Some(("PC-SERVER:47810".into(), "line-list-3f2a".into(), Some(("Line List".into(), "B4".into())))));
    // A sheet with a quote and an exclamation mark, and a range.
    let l = suite::link_at("s:1", "b", "O'Brien! & Co", "A1:C9");
    assert_eq!(l, "dexterity://open/s:1/b?at=O%27Brien%21%20%26%20Co!A1:C9");
    assert_eq!(suite::parse_link(&l).and_then(|x| x.2), Some(("O'Brien! & Co".into(), "A1:C9".into())));
    // Excel's way of writing it, typed or pasted by a person.
    assert_eq!(suite::parse_at("'Line List'!$b$4"), Some(("Line List".into(), "$B$4".into())));
    assert_eq!(suite::parse_at("'O''Brien'!A1"), Some(("O'Brien".into(), "A1".into())));
    assert_eq!(suite::parse_at("%27Line%20List%27!B4"), Some(("Line List".into(), "B4".into())));
    // Damaged places: the workbook still opens, without a cell.
    for bad in ["Sheet1!B4;rm", "%ZZ!B4", "Sheet1!", "!B4", "Sheet1!A1:B2:C3", "Bad[Name]!A1", "Sheet1!ZZZZZZ1"] {
        assert_eq!(suite::parse_at(bad), None, "{bad}");
        let l = format!("dexterity://open/s:1/b?at={bad}");
        assert_eq!(suite::parse_link(&l), Some(("s:1".into(), "b".into(), None)), "{bad}");
    }
    assert_eq!(suite::parse_link("dexterity://open/s:1/b"), Some(("s:1".into(), "b".into(), None)));
    assert_eq!(suite::parse_link("https://example.com/open/s/b"), None);
}

#[test]
fn connect_and_sleuth_addresses_are_percent_encoded() {
    let u = suite::connect_share("Line List \u{2014} Lines", "Size: 6\"\nSpec: A1 & B", "dexterity://open/s:1/b?at=Lines!B4");
    assert_eq!(
        u,
        "dexconnect://share?from=Dexterity%20Sheets&title=Line%20List%20%E2%80%94%20Lines&text=Size%3A%206%22%0ASpec%3A%20A1%20%26%20B&link=dexterity%3A%2F%2Fopen%2Fs%3A1%2Fb%3Fat%3DLines%21B4"
    );
    assert!(!suite::connect_share("t", "", "").contains("text="), "empty parts are left out");
    assert_eq!(suite::sleuth_search("  pump skid #2 "), "sleuth://search?q=pump%20skid%20%232");
    let long = "x".repeat(500);
    assert_eq!(suite::sleuth_search(&long).len(), "sleuth://search?q=".len() + 200, "Sleuth reads 200 characters");
}

#[test]
fn only_the_office_apps_schemes_count_as_suite_links() {
    for ok in ["sleuth://search?q=a", "DATUM://c/elbow-90", "projectopener://open/abc", "dexconnect://share?text=a", "dexterity://open/s:1/b"] {
        assert!(suite::suite_scheme(ok).is_some(), "{ok}");
    }
    for bad in
        ["https://example.com", "javascript:alert(1)", "file:///C:/x", "sleuthx://a", "sleuth:", "sle\nuth://a", "ms-excel:ofe|u|x", "C:\\x.exe"]
    {
        assert_eq!(suite::suite_scheme(bad), None, "{bad}");
    }
}

#[test]
fn a_project_resolves_by_number_or_name_and_never_by_guess() {
    let ws = br#"{"projects": [
        {"id": "a1b2c3d4e5f6", "name": "P12513 As-built isometrics", "job_number": ""},
        {"id": "0f0f0f0f0f0f", "name": "Line List Review", "job_number": "J-77"},
        {"id": "111111111111", "name": "P9 Pumps", "job_number": ""},
        {"id": "222222222222", "name": "P9 Piping", "job_number": ""},
        {"name": "no id"}, 7
    ]}"#;
    let projects = suite::read_projects(ws);
    assert_eq!(projects.len(), 4);
    let id = |t: &str| suite::resolve_project(&projects, t).map(|p: &Project| p.id.clone());
    assert_eq!(id("P12513").as_deref(), Some("a1b2c3d4e5f6"), "the one name that starts with the number");
    assert_eq!(id("p12513 as-built isometrics").as_deref(), Some("a1b2c3d4e5f6"));
    assert_eq!(id("j-77").as_deref(), Some("0f0f0f0f0f0f"));
    assert_eq!(id("P9"), None, "two projects fit: none is guessed");
    assert_eq!(id("P125"), None, "a part of a word is not a match");
    assert_eq!(id(""), None);
    assert!(suite::read_projects(b"not json").is_empty());
}

// ---- Commands through the window -------------------------------------------------------------

#[test]
fn nothing_shows_and_every_app_command_says_why_when_no_app_is_installed() {
    let dir = temp_dir("none");
    let mut h = harness(book(), Installed::default());
    saved_in(&mut h, &dir);
    let apps = h.state_mut().run("suite.apps", json!({})).unwrap();
    assert_eq!(apps, json!({"connect": false, "sleuth": false, "datum": false, "projectOpener": false, "slate": false}));
    for (id, why) in [
        ("suite.shareToConnect", "Connect"),
        ("suite.discussRow", "Connect"),
        ("suite.findInSleuth", "Sleuth"),
        ("suite.openInProjectOpener", "Project Opener"),
    ] {
        let e = h.state_mut().run(id, json!({})).unwrap_err();
        assert!(e.contains(why) && e.contains("not installed"), "{id}: {e}");
    }
    assert!(h.state().collab.suite.opened.is_empty(), "nothing was handed to Windows");
}

#[test]
fn share_to_connect_sends_the_shared_cell_link_or_says_the_file_is_local() {
    let dir = temp_dir("connect");
    let mut h = harness(book(), all_apps(&dir));
    let file = saved_in(&mut h, &dir);
    select(&mut h, "C2");
    let r = h.state_mut().run("suite.discussRow", json!({})).unwrap();
    let url = r["url"].as_str().unwrap().to_string();
    assert!(url.starts_with("dexconnect://share?from=Dexterity%20Sheets&title=Line%20List%20%E2%80%94%20Sheet1&text="), "{url}");
    // The row as header: value pairs, then where it is (a local file has no cell link).
    let text = suite::decode(url.split("&text=").nth(1).unwrap().split('&').next().unwrap()).unwrap();
    assert!(text.starts_with("Line: 100-P-001\nSize: 6\"\nSpec: A1\nStatus: Issued"), "{text}");
    assert!(text.contains("Local file (not shared), Sheet1!C2"), "{text}");
    let link = suite::decode(url.split("&link=").nth(1).unwrap()).unwrap();
    assert_eq!(link, file.display().to_string());
    // Share to Connect… (the title-bar menu) carries no row, just the link.
    let r = h.state_mut().run("suite.shareToConnect", json!({})).unwrap();
    assert!(!r["url"].as_str().unwrap().contains("Line%3A"), "no row values");
    assert_eq!(h.state().collab.suite.opened.len(), 2);
    // A new workbook nobody saved has no place to link to, and says so.
    let mut h = harness(book(), all_apps(&dir));
    let e = h.state_mut().run("suite.copyCellLink", json!({})).unwrap_err();
    assert!(e.contains("Save or share"), "{e}");
}

#[test]
fn copy_link_to_this_cell_names_the_file_and_the_cell_when_not_shared() {
    let dir = temp_dir("copy");
    let mut h = harness(book(), all_apps(&dir));
    let file = saved_in(&mut h, &dir);
    h.state_mut().run("selection.set", json!({"range": "B2:C3"})).unwrap();
    let r = h.state_mut().run("suite.copyCellLink", json!({})).unwrap();
    assert_eq!(r, json!({"link": format!("{} (Sheet1!B2:C3)", file.display()), "cellLink": false}));
}

#[test]
fn sleuth_and_project_opener_open_from_the_cell_text() {
    let dir = temp_dir("lookup");
    std::fs::write(dir.join("workspace.json"), br#"{"projects": [{"id": "a1b2c3d4e5f6", "name": "P12513 As-built isometrics"}]}"#).unwrap();
    let mut h = harness(book(), all_apps(&dir));
    saved_in(&mut h, &dir);
    select(&mut h, "A2");
    let r = h.state_mut().run("suite.findInSleuth", json!({})).unwrap();
    assert_eq!(r["url"], "sleuth://search?q=100-P-001");
    select(&mut h, "A3");
    let r = h.state_mut().run("suite.openInProjectOpener", json!({})).unwrap();
    assert_eq!(r, json!({"url": "projectopener://open/a1b2c3d4e5f6", "project": "P12513 As-built isometrics"}));
    select(&mut h, "B3");
    assert!(h.state_mut().run("suite.findInSleuth", json!({})).unwrap_err().contains("empty"));
    select(&mut h, "D2");
    assert!(h.state_mut().run("suite.openInProjectOpener", json!({})).unwrap_err().contains("No single project"));
}

#[test]
fn links_in_cells_open_only_for_the_office_apps() {
    let dir = temp_dir("links");
    let mut h = harness(book(), all_apps(&dir));
    h.state_mut().run("insert.link", json!({"cell": "E2", "target": "sleuth://search?q=pump"})).unwrap();
    h.state_mut().run("cell.set", json!({"cell": "E3", "input": "=HYPERLINK(\"datum://c/elbow-90\",\"Elbow\")"})).unwrap();
    select(&mut h, "E2");
    assert_eq!(h.state_mut().run("suite.openLink", json!({})).unwrap()["url"], "sleuth://search?q=pump");
    select(&mut h, "E3");
    assert_eq!(h.state_mut().run("suite.openLink", json!({})).unwrap()["url"], "datum://c/elbow-90");
    for bad in ["javascript:alert(1)", "https://example.com", "file:///C:/Windows/calc.exe", "ms-excel:ofe|u|x"] {
        assert!(h.state_mut().run("suite.openLink", json!({"url": bad})).is_err(), "{bad}");
    }
    assert_eq!(h.state().collab.suite.opened, ["sleuth://search?q=pump", "datum://c/elbow-90"]);
}

#[test]
fn notes_open_in_slate_beside_the_workbook_and_a_row_note_is_appended() {
    let dir = temp_dir("slate");
    let mut h = harness(book(), all_apps(&dir));
    saved_in(&mut h, &dir);
    let notes = dir.join("Line List.notes.md");
    let r = h.state_mut().run("suite.notes", json!({})).unwrap();
    assert_eq!(r, json!({"path": notes.display().to_string(), "in": "slate"}));
    assert_eq!(std::fs::read_to_string(&notes).unwrap(), "# Notes: Line List\n\n", "Slate opens only a file that is there");
    select(&mut h, "B2");
    h.state_mut().run("suite.noteForRow", json!({})).unwrap();
    let text = std::fs::read_to_string(&notes).unwrap();
    assert!(text.starts_with("# Notes: Line List\n\n\n## Sheet1!B2 ("), "{text}");
    assert!(text.contains("- Line: 100-P-001\n- Size: 6\"\n- Spec: A1\n- Status: Issued\n"), "{text}");
    assert!(text.contains(&format!("In {}, Sheet1!B2", dir.join("Line List.xlsx").display())), "{text}");
    assert_eq!(h.state().collab.suite.opened.iter().filter(|o| o.starts_with("slate:")).count(), 2);
    assert!(h.state().collab.suite.notes.is_none(), "no pane when Slate has them");
}

#[test]
fn without_slate_the_notes_pane_edits_and_never_overwrites_another_programs_change() {
    let dir = temp_dir("pane");
    let mut apps = all_apps(&dir);
    apps.slate = None;
    let mut h = harness(book(), apps);
    saved_in(&mut h, &dir);
    let notes = dir.join("Line List.notes.md");
    assert_eq!(h.state_mut().run("suite.notes", json!({})).unwrap()["in"], "pane");
    h.run_steps(2);
    h.get_by_label("Notes: Line List");
    h.state_mut().run("suite.notesPane", json!({"text": "# Notes: Line List\n\nCheck the spec breaks.\n", "save": true})).unwrap();
    assert_eq!(std::fs::read_to_string(&notes).unwrap(), "# Notes: Line List\n\nCheck the spec breaks.\n");
    // A row note goes into the pane's unsaved text, not under it.
    h.state_mut().run("suite.notesPane", json!({"text": "draft"})).unwrap();
    select(&mut h, "A3");
    h.state_mut().run("suite.noteForRow", json!({})).unwrap();
    let pane = h.state_mut().run("suite.notesPane", json!({})).unwrap();
    assert!(pane["text"].as_str().unwrap().starts_with("draft\n## Sheet1!A3 ("), "{pane}");
    assert_eq!(pane["saved"], false);
    // Someone changes the file in another program: saving refuses and keeps both.
    std::fs::write(&notes, "changed elsewhere").unwrap();
    let e = h.state_mut().run("suite.notesPane", json!({"save": true})).unwrap_err();
    assert!(e.contains("Another program changed"), "{e}");
    assert_eq!(std::fs::read_to_string(&notes).unwrap(), "changed elsewhere");
    h.run_steps(2);
    h.get_by_label("Reload");
    h.state_mut().run("suite.notesPane", json!({"close": true})).unwrap();
    assert!(h.state().collab.suite.notes.is_none());
}

#[test]
fn the_cell_menu_and_the_notes_button_show_only_what_is_installed() {
    let dir = temp_dir("menu");
    std::fs::write(dir.join("workspace.json"), br#"{"projects": [{"id": "a1b2c3d4e5f6", "name": "P12513 As-built isometrics"}]}"#).unwrap();
    let mut h = harness(book(), all_apps(&dir));
    saved_in(&mut h, &dir);
    select(&mut h, "A3");
    assert!(h.query_all_by_label("Notes").count() >= 1, "the Notes button");
    h.state_mut().grid.context_menu = Some(egui::pos2(300.0, 300.0));
    h.run_steps(3);
    for label in [
        "Copy Link to This Cell",
        "Discuss This Row in Connect…",
        "Add Note in Slate for This Row",
        "Find \u{201c}P12513\u{201d} in Sleuth",
        "Open in Project Opener",
    ] {
        h.get_by_label(label);
    }
    h.get_by_label("Find \u{201c}P12513\u{201d} in Sleuth").click();
    h.run_steps(3);
    assert_eq!(h.state().collab.suite.opened.last().map(String::as_str), Some("sleuth://search?q=P12513"));
    // Nothing installed: only the items that need no other app.
    let mut h = harness(book(), Installed::default());
    saved_in(&mut h, &dir);
    select(&mut h, "A3");
    h.state_mut().grid.context_menu = Some(egui::pos2(300.0, 300.0));
    h.run_steps(3);
    h.get_by_label("Copy Link to This Cell");
    h.get_by_label("Add Note for This Row");
    for absent in ["Discuss This Row in Connect…", "Open in Project Opener", "Find \u{201c}P12513\u{201d} in Sleuth"] {
        assert!(h.query_by_label(absent).is_none(), "{absent}");
    }
}
