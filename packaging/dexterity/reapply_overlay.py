"""Re-applies Dexterity Sheets' hooks and branding to upstream GridCraft files.

Used when an upstream merge had to take upstream's version of a file (typically after an upstream
refactor moved code around). Every step is idempotent: running it twice changes nothing more.
Anchors that are not found are reported, and the script exits 1, so a person can place that hook
by hand. Then run check_overlay.py and the tests.

    python packaging/dexterity/reapply_overlay.py
"""
import glob
import os
import re
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
problems = []


def read(path):
    with open(os.path.join(ROOT, path), encoding="utf-8") as f:
        return f.read()


def write(path, text):
    with open(os.path.join(ROOT, path), "w", encoding="utf-8", newline="\n") as f:
        f.write(text)


def insert(path, marker, anchor, text, after=True):
    """Puts `text` after (or before) `anchor` unless `marker` is already there."""
    s = read(path)
    if marker in s:
        return
    if anchor not in s:
        problems.append(f"{path}: anchor not found for [{marker}]: {anchor[:70]!r}")
        return
    s = s.replace(anchor, anchor + text if after else text + anchor, 1)
    write(path, s)


def replace(path, old, new, required=True):
    s = read(path)
    if new in s and old not in s:
        return
    if old not in s:
        if required:
            problems.append(f"{path}: text not found: {old[:70]!r}")
        return
    write(path, s.replace(old, new))


# ---- UI hooks --------------------------------------------------------------------------------
UI = "crates/ui-egui/src"
insert(f"{UI}/lib.rs", "pub mod collab_ui;", "pub mod code_editor;\n", "pub mod collab_ui;\n")
insert(
    f"{UI}/lib.rs",
    "pub collab: collab_ui::Collab,",
    "pub struct SheetApp {\n    pub session: Session,\n",
    "    /// LAN co-authoring: the server connection and the shared workbooks open here.\n"
    "    pub collab: collab_ui::Collab,\n"
    "    /// The egui context, kept so commands can start network work that wakes the window.\n"
    "    pub collab_ctx: Option<egui::Context>,\n",
)
app_new = [p for p in [f"{UI}/app.rs", f"{UI}/lib.rs"] if "SheetApp {\n            session,\n" in read(p)]
if app_new:
    insert(app_new[0], "collab: Default::default(),", "SheetApp {\n            session,\n", "            collab: Default::default(),\n            collab_ctx: None,\n")
else:
    problems.append("SheetApp constructor (`SheetApp {\\n            session,`) not found")
tick_file = next((p for p in [f"{UI}/app.rs", f"{UI}/lib.rs"] if "control::poll(self, ctx);\n" in read(p)), None)
if tick_file:
    insert(
        tick_file,
        "collab_ui::tick(self, ctx);",
        "control::poll(self, ctx);\n",
        "        if self.collab_ctx.is_none() {\n            self.collab_ctx = Some(ctx.clone());\n        }\n        crate::collab_ui::tick(self, ctx);\n",
    )
else:
    problems.append("frame loop (`control::poll(self, ctx);`) not found")
cmd_file = next((p for p in [f"{UI}/app/commands.rs", f"{UI}/lib.rs"] if os.path.exists(os.path.join(ROOT, p)) and "fn run_ui_command(&mut self, id: &str, p: &Json) -> Option<Result<Json, String>> {\n" in read(p)), None)
if cmd_file:
    insert(
        cmd_file,
        "collab_ui::run_command(self, id, p)",
        "fn run_ui_command(&mut self, id: &str, p: &Json) -> Option<Result<Json, String>> {\n",
        "        if let Some(r) = crate::collab_ui::run_command(self, id, p) {\n            return Some(r);\n        }\n",
    )
else:
    problems.append("run_ui_command not found")
grid_file = next((p for p in sorted(glob.glob(os.path.join(ROOT, UI, "grid*", "*.rs"))) + [os.path.join(ROOT, UI, "grid.rs")] if "paint_selection(&p, geo, sh, sel, outline, t, app, *rows, *cols);" in open(p, encoding="utf-8").read()), None)
if grid_file:
    rel = os.path.relpath(grid_file, ROOT).replace("\\", "/")
    insert(rel, "collab_ui::paint_peers(", "paint_selection(&p, geo, sh, sel, outline, t, app, *rows, *cols);\n", "        crate::collab_ui::paint_peers(&p, geo, sh, app, *rows, *cols);\n")
else:
    problems.append("grid selection painting not found")

review = f"{UI}/ribbon/review.rs" if os.path.exists(os.path.join(ROOT, UI, "ribbon", "review.rs")) else f"{UI}/ribbon.rs"
insert(
    review,
    '"collab.history"',
    "fn review(app: &mut SheetApp, ui: &mut Ui) {\n",
    '    // Shared workbooks (Dexterity Sheets LAN co-authoring).\n'
    '    if big_button(ui, Icon::Share, "Shared\\nWorkbooks", "Sign in and open shared workbooks (File > Shared)", false).clicked() {\n'
    '        act(app, "collab.shared", json!({}));\n'
    "    }\n"
    '    if big_button(ui, Icon::Clock, "Edit\\nHistory", "Who changed what, and when", false).clicked() {\n'
    '        act(app, "collab.history", json!({}));\n'
    "    }\n"
    '    if big_button(ui, Icon::Book, "Versions", "Versions kept on the server", false).clicked() {\n'
    '        act(app, "collab.versions", json!({}));\n'
    "    }\n"
    "    sep(ui);\n",
)
title = f"{UI}/ribbon/title_bar.rs" if os.path.exists(os.path.join(ROOT, UI, "ribbon", "title_bar.rs")) else f"{UI}/ribbon.rs"
insert(
    title,
    "Share with the team (office server)",
    "egui::Popup::menu(&share).show(|ui| {\n",
    "                    let shared = crate::collab_ui::active_shared(app).is_some();\n"
    '                    if !shared && ui.button("Share with the team (office server)…").clicked() {\n'
    "                        if app.collab.signed_in() {\n"
    '                            app.run_or_alert("collab.share", json!({}));\n'
    "                        } else {\n"
    '                            app.run_or_alert("collab.shared", json!({}));\n'
    "                        }\n"
    "                    }\n"
    '                    if ui.button("Shared workbooks…").clicked() {\n'
    '                        app.run_or_alert("collab.shared", json!({}));\n'
    "                    }\n"
    "                    ui.separator();\n",
)

# ---- File > Shared page ----------------------------------------------------------------------
B = f"{UI}/backstage.rs"
replace(B, "    Open,\n    Info,", "    Open,\n    Shared,\n    Info,")
s = read(B)
s2 = re.sub(r"pub const ALL: \[Page; 9\] =\s*\[Page::Home, Page::New, Page::Open, Page::Info,", "pub const ALL: [Page; 10] = [Page::Home, Page::New, Page::Open, Page::Shared, Page::Info,", s)
if s2 == s and "Page::Shared, Page::Info" not in s:
    problems.append("backstage Page::ALL not found")
write(B, s2)
insert(B, 'Page::Shared => "shared",', '            Page::Open => "open",\n', '            Page::Shared => "shared",\n')
insert(B, 'Page::Shared => "Shared",', '            Page::Open => "Open",\n', '            Page::Shared => "Shared",\n')
insert(B, "Page::Shared => Icon::Share,", "            Page::Open => Icon::OpenFolder,\n", "            Page::Shared => Icon::Share,\n")
replace(B, "for page in [Page::Home, Page::New, Page::Open] {", "for page in [Page::Home, Page::New, Page::Open, Page::Shared] {")
insert(B, "crate::collab_ui::shared_page", "                    Page::Open => open_page(app, ui, &mut bs, &mut out),\n", "                    Page::Shared => crate::collab_ui::shared_page(app, ui, &mut out.close),\n")
replace(
    B,
    """        for (label, value) in
            [("Version", env!("CARGO_PKG_VERSION").to_string()), ("Platform", platform()), ("Licence", env!("CARGO_PKG_LICENSE").to_string())]
        {""",
    """        for (label, value) in [
            ("Version", gridcraft_collab::APP_VERSION.trim().to_string()),
            ("Publisher", "Dexterity Design Services Pvt. Ltd.".to_string()),
            ("Platform", platform()),
            ("Licence", env!("CARGO_PKG_LICENSE").to_string()),
            ("Based on", format!("GridCraft {} by the ArtCraft team (MIT/Apache-2.0)", env!("CARGO_PKG_VERSION"))),
        ] {""",
    required=False,
)

# ---- About dialog: no ArtCraft links, keep the attribution -----------------------------------
about = f"{UI}/dialogs/about.rs" if os.path.exists(os.path.join(ROOT, UI, "dialogs", "about.rs")) else f"{UI}/dialogs.rs"
replace(
    about,
    """            ui.label("A clean-room, open-source, Rust-native spreadsheet. Part of ArtCraft.");
            ui.hyperlink_to("getartcraft.com/apps/gridcraft", "https://getartcraft.com/apps/gridcraft");
            ui.hyperlink_to("Join the ArtCraft Discord", "https://discord.gg/artcraft");""",
    """            ui.label("Internal spreadsheet of Dexterity Design Services Pvt. Ltd.");
            ui.add_space(4.0);
            // Attribution required by the upstream licence (MIT OR Apache-2.0): keep it.
            ui.label("Based on GridCraft by the ArtCraft team (MIT/Apache-2.0).");""",
    required=False,
)
replace(about, 'ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));', 'ui.label(format!("Version {}", gridcraft_collab::APP_VERSION.trim()));', required=False)

# ---- Check for Updates goes to the office server ----------------------------------------------
for p in glob.glob(os.path.join(ROOT, UI, "ribbon", "*.rs")) + [os.path.join(ROOT, UI, "ribbon.rs")]:
    if os.path.exists(p):
        rel = os.path.relpath(p, ROOT).replace("\\", "/")
        replace(rel, '"Look for a newer GridCraft release"', '"Look for a newer Dexterity Sheets on the office server"', required=False)

# ---- Branding text: "GridCraft" in user-visible string literals -------------------------------
KEEP = re.compile(r"Based on GridCraft|GridCraft by the ArtCraft|GridCraft \{\} by|gridcraft-cli|GRIDCRAFT_|storytold/gridcraft|craft-fonts|ai\.storyteller")


def rebrand_text(text):
    out = []
    for line in text.splitlines(keepends=True):
        stripped = line.lstrip()
        if "GridCraft" in line and '"' in line and not KEEP.search(line) and not stripped.startswith("//"):
            line = re.sub(r'"([^"\n]*)"', lambda m: '"' + m.group(1).replace("GridCraft", "Dexterity Sheets") + '"', line)
        out.append(line)
    return "".join(out)


for pattern in [f"{UI}/**/*.rs", "apps/gridcraft/src/**/*.rs"]:
    for p in glob.glob(os.path.join(ROOT, pattern), recursive=True):
        if os.sep + "tests" + os.sep in p or p.endswith("tests.rs") or os.sep + "i18n" + os.sep in p:
            continue
        with open(p, encoding="utf-8") as f:
            t = f.read()
        n = rebrand_text(t)
        if n != t:
            with open(p, "w", encoding="utf-8", newline="\n") as f:
                f.write(n)

# ---- Engine tests the Excel-parity fixes changed ---------------------------------------------
E = "crates/engine/src"
edit_tests = f"{E}/cmd/edit/tests.rs" if os.path.exists(os.path.join(ROOT, E, "cmd", "edit", "tests.rs")) else f"{E}/cmd/edit.rs"
replace(edit_tests, "    // The default is unchanged: the dates swap, the thousands separator is a decimal point.\n",
        "    // The default is unchanged: the thousands separator is a decimal point. A dotted date is\n    // text where `.` isn't the date separator, as in Excel (en-US).\n", required=False)
replace(edit_tests, 'assert_eq!(get(&mut s, "A1")["text"], "5/4/2026", "dd.mm.yyyy read as mm.dd.yyyy");',
        'assert_eq!(get(&mut s, "A1")["value"], json!("05.04.2026"), "dotted dates stay text");', required=False)
io_tests = f"{E}/io/tests.rs" if os.path.exists(os.path.join(ROOT, E, "io", "tests.rs")) else f"{E}/io.rs"
s = read(io_tests)
s = re.sub(r'#\[cfg\(not\(target_arch = "wasm32"\)\)\]\n(#\[test\]\n[^\n]*\n(?:[^\n]*\n){0,12}?[^\n]*std::os::unix::fs::symlink)', r"// Unix symlinks; the test doesn't compile on Windows.\n#[cfg(unix)]\n\1", s, count=1)
pairs = [
    ('save_bytes(&wb, "cut list.csv").unwrap()).unwrap(), CUT_LIST);', 'save_bytes(&wb, "cut list.csv").unwrap()).unwrap(), bom(CUT_LIST));'),
    ('save_bytes(&wb, "copy.CSV").unwrap()).unwrap(), CUT_LIST);', 'save_bytes(&wb, "copy.CSV").unwrap()).unwrap(), bom(CUT_LIST));'),
    ('save_bytes(&wb, "plain.csv").unwrap()).unwrap(), commas);', 'save_bytes(&wb, "plain.csv").unwrap()).unwrap(), bom(commas));'),
    ('save_bytes(&wb, "tabs.csv").unwrap()).unwrap(), tabs);\n}', 'save_bytes(&wb, "tabs.csv").unwrap()).unwrap(), bom(tabs));\n    // Reopening a saved file reads the same cells (the mark is not part of A1).\n    let (again, _) = open_bytes("cut list.csv", &save_bytes(&wb, "tabs.csv").unwrap()).unwrap();\n    assert_eq!(String::from_utf8(save_bytes(&again, "tabs.csv").unwrap()).unwrap(), bom(tabs));\n}'),
    ('assert_eq!(save_bytes(&wb, "new.csv").unwrap(), b"a,1\\r\\n");', 'assert_eq!(save_bytes(&wb, "new.csv").unwrap(), b"\\xEF\\xBB\\xBFa,1\\r\\n");'),
    ('.starts_with("1,511,184,"));', '.starts_with("\\u{FEFF}1,511,184,"));'),
    ('assert_eq!(save_bytes(&tsv, "list.csv").unwrap(), b"a,b\\r\\n");', 'assert_eq!(save_bytes(&tsv, "list.csv").unwrap(), b"\\xEF\\xBB\\xBFa,b\\r\\n");'),
    ('assert_eq!(save_bytes(&tsv, "list.tsv").unwrap(), b"a\\tb\\r\\n");', 'assert_eq!(save_bytes(&tsv, "list.tsv").unwrap(), b"\\xEF\\xBB\\xBFa\\tb\\r\\n");'),
    ('assert_eq!(csv, "2026-01-01,', 'assert_eq!(csv, "\\u{FEFF}2026-01-01,'),
    ('assert!(tsv.starts_with("2026-01-01\\t', 'assert!(tsv.starts_with("\\u{FEFF}2026-01-01\\t'),
    ('format!("124412140710{separator}', 'format!("\\u{FEFF}124412140710{separator}'),
]
for old, new in pairs:
    if new not in s and old in s:
        s = s.replace(old, new, 1)
if "let bom = |s: &str|" not in s and "bom(CUT_LIST)" in s:
    s = s.replace(
        "fn csv_save_keeps_the_separator_it_was_opened_with() {\n",
        "fn csv_save_keeps_the_separator_it_was_opened_with() {\n    // Saved text is UTF-8 with a byte order mark (Excel's CSV UTF-8); the separator is kept.\n    let bom = |s: &str| format!(\"\\u{FEFF}{s}\");\n",
        1,
    )
write(io_tests, s)

for p in problems:
    print("REAPPLY:", p)
print("overlay re-applied" if not problems else f"{len(problems)} problem(s)")
sys.exit(1 if problems else 0)
