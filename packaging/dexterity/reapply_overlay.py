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


# ---- Workspace: our crates and where they sit in the layering ----------------------------------
insert("Cargo.toml", 'gridcraft-collab = { path = "crates/collab" }', 'gridcraft-pdf = { path = "crates/pdf" }\n', 'gridcraft-collab = { path = "crates/collab" }\n')
LAYERS = "xtask/src/layers.rs"
insert(
    LAYERS,
    '("collab", Class::Layer(6))',
    '    ("mcp", Class::Layer(6)),\n',
    "    // LAN co-authoring (no UI toolkit); the UI builds on it.\n"
    '    ("collab", Class::Layer(6)),\n'
    '    ("server", Class::Layer(7)),\n'
    '    ("server-manager", Class::Layer(7)),\n',
)
insert(LAYERS, '&["collab", "ui-egui"]', '&["model", "xlsx"]', ', &["collab", "ui-egui"]')

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
# The per-pane selection painting call: positional arguments before upstream #2711, a
# `SelectionPaintInput` struct since.
SELECTION_CALLS = (
    "        paint_selection(SelectionPaintInput { painter: &p, geo, sheet: sh, selection: sel, outline, tokens: t, app, rows: *rows, cols: *cols });\n",
    "        paint_selection(&p, geo, sh, sel, outline, t, app, *rows, *cols);\n",
)


def selection_call(text):
    return next((c for c in SELECTION_CALLS if c in text), None)


grid_file = next((p for p in sorted(glob.glob(os.path.join(ROOT, UI, "grid*", "*.rs"))) + [os.path.join(ROOT, UI, "grid.rs")] if os.path.exists(p) and selection_call(open(p, encoding="utf-8").read())), None)
if grid_file:
    rel = os.path.relpath(grid_file, ROOT).replace("\\", "/")
    insert(rel, "collab_ui::paint_peers(", selection_call(read(rel)), "        crate::collab_ui::paint_peers(&p, geo, sh, app, *rows, *cols);\n")
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
    "collab_ui::share_menu(app, ui)",
    "egui::Popup::menu(&share).show(|ui| {\n",
    "                    crate::collab_ui::share_menu(app, ui);\n"
    "                    ui.separator();\n",
)

# ---- Desktop app manifest: our executable name, no internet update checker --------------------
APP = "apps/gridcraft/Cargo.toml"
replace(APP, 'default-run = "gridcraft"\ndescription = "GridCraft desktop app"',
        'default-run = "DexteritySheets"\ndescription = "Dexterity Sheets desktop app (based on GridCraft)"', required=False)
replace(APP, "VERSIONINFO for gridcraft.exe", "VERSIONINFO for DexteritySheets.exe", required=False)
insert(
    APP,
    'name = "DexteritySheets"',
    "[lints]\nworkspace = true\n",
    "\n# The shipped desktop executable is `DexteritySheets` (`DexteritySheets.exe` on Windows); the\n"
    "# package keeps its name `gridcraft` (`cargo run -p gridcraft`).\n"
    '[[bin]]\nname = "DexteritySheets"\npath = "src/main.rs"\n',
)
# Earlier Dexterity Sheets builds removed upstream's craft-update; it is back (the GitHub update
# check in dex_ui/releases.rs uses its client), so a tree from those builds gets the lines again.
replace(APP, "# Dexterity Sheets: no craft-update (no update checker; the app never contacts the internet).\n", "craft-update = { workspace = true }\n", required=False)
replace("crates/ui-egui/Cargo.toml", "# Dexterity Sheets: no craft-update (no update checker; the app never contacts the internet).\n", "craft-update = { workspace = true }\n", required=False)
insert(
    APP,
    "windows-registry",
    'pollster = "1"\n',
    '# Regional settings (`regional.rs`): HKCU\\Control Panel\\International. Already in the tree.\nwindows-registry = "0.6"\n',
)

# ---- Excel parity additions: cmd/dex.rs and dex_ui.rs (docs/dexterity-parity-audit.md) ---------
insert("crates/engine/src/cmd.rs", "pub mod dex;", "pub mod data;\n", "pub mod dex;\n")
insert(
    "crates/engine/src/cmd.rs",
    "dex::specs().into_iter().filter(",
    "        v.extend(print::specs());\n",
    "        // Dexterity Sheets additions; an id upstream GridCraft registers itself is left to upstream's.\n"
    "        let dex: Vec<CommandSpec> = dex::specs().into_iter().filter(|d| !v.iter().any(|c| c.id == d.id)).collect();\n"
    "        v.extend(dex);\n",
)
insert(
    "crates/engine/src/lib.rs",
    "pub dex: cmd::dex::DexState,",
    "    pub comments_pane: bool,\n",
    "    /// Dexterity Sheets command state (Repeat, Focus Cell, validation circles).\n    pub dex: cmd::dex::DexState,\n",
)
insert(
    "crates/engine/src/lib.rs",
    "pub fn document_mut(&mut self, uid: u64)",
    "    pub fn active_mut(&mut self) -> Option<&mut DocState> {\n",
    "    /// The open workbook with this [`DocState::uid`], for changes that don't go through a command\n"
    "    /// (LAN co-authoring applies other people's edits this way).\n"
    "    pub fn document_mut(&mut self, uid: u64) -> Option<&mut DocState> {\n"
    "        self.docs.iter_mut().find(|d| d.uid == uid)\n"
    "    }\n",
    after=False,
)
insert(
    "crates/engine/src/lib.rs",
    "let dex_before =",
    "        let before = self.active().map(|d| (d.uid, d.wb.clone(), d.selection.clone()));\n",
    "        let dex_before = self.active().map(|d| (d.uid, d.revision));\n",
)
insert(
    "crates/engine/src/lib.rs",
    "cmd::dex::after_command(",
    "            self.bound_tail(uid);\n        }\n",
    "        if self.depth == 0 {\n            cmd::dex::after_command(self, id, &params, &result, dex_before);\n        }\n",
)
FUZZ = "crates/engine/tests/it/command_sequence_fuzz.rs"
s = read(FUZZ)
if '("edit.repeat", DEX)' not in s:
    dex_src = read("crates/engine/src/cmd/dex.rs") + "".join(
        read(os.path.relpath(p, ROOT)) for p in sorted(glob.glob(os.path.join(ROOT, "crates", "engine", "src", "cmd", "dex", "*.rs")))
    )
    dex_ids = re.findall(r'cmd!\(\s*(?:noundo |query )?"([A-Za-z.]+)"', dex_src)
    s2 = s.replace(
        "const EXCLUDED: &[(&str, &str)] = &[\n",
        "/// Dexterity Sheets keeps its own commands out of the random pool so the sequences stay the same\n"
        "/// as upstream GridCraft's, whose CI vets them.\n"
        'const DEX: &str = "Dexterity Sheets command, tested in cmd/dex.rs; kept out so sequences match upstream";\n'
        "const EXCLUDED: &[(&str, &str)] = &[\n"
        "    // Dexterity Sheets commands (crates/engine/src/cmd/dex.rs, tested there).\n"
        + "".join(f'    ("{i}", DEX),\n' for i in dex_ids),
        1,
    )
    if s2 == s:
        problems.append(f"{FUZZ}: EXCLUDED list not found")
    else:
        write(FUZZ, s2)
insert(f"{UI}/lib.rs", "pub mod dex_ui;", "pub mod collab_ui;\n", "pub mod dex_ui;\n")
insert("crates/ui-egui/tests/it/main.rs", "mod dex_parity;", "mod control_timeout;\n", "mod dex_parity;\n")
# @mentions in comments: suggestions in the New Comment field and in replies.
replace(
    f"{UI}/dialogs/show.rs",
    '                            ui.add(egui::TextEdit::multiline(&mut s).desired_rows(4).desired_width(320.0)).labelled_by(label.id)\n',
    '                            crate::dex_ui::mention::field(ui, &mut s, d.name == "comment", |ui, s| {\n                                ui.add(egui::TextEdit::multiline(s).desired_rows(4).desired_width(320.0))\n                            })\n                            .labelled_by(label.id)\n',
    required=False,
)
replace(
    f"{UI}/panes.rs",
    '                        let r = ui.add(egui::TextEdit::singleline(&mut draft).hint_text("Reply…").desired_width(ui.available_width()));\n',
    '                        let r = crate::dex_ui::mention::field(ui, &mut draft, c.threaded, |ui, s| {\n                            ui.add(egui::TextEdit::singleline(s).hint_text("Reply…").desired_width(ui.available_width()))\n                        });\n',
    required=False,
)
insert(
    f"{UI}/dialogs/cond_format_rules.rs",
    "crate::dex_ui::new_rule_button(app, ui);",
    "pub(super) fn manage_rules(app: &mut SheetApp, ui: &mut egui::Ui) {\n",
    "    crate::dex_ui::new_rule_button(app, ui);\n",
)
replace(
    f"{UI}/dialogs/show.rs",
    "                    egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {\n"
    "                        ui.monospace(serde_json::to_string_pretty(&r.unwrap_or(Json::Null)).unwrap_or_default());\n"
    "                    });\n",
    "                    // Dexterity Sheets: readable tables and cell links instead of the raw result.\n"
    "                    crate::dex_ui::result_view(app, ui, &d.name, &r.unwrap_or(Json::Null));\n",
    required=False,
)
insert(
    f"{UI}/app/commands.rs",
    "crate::dex_ui::dialog_alias(name)",
    "    pub fn open_dialog(&mut self, name: &str, params: Json) {\n",
    "        if let Some((alias, p)) = crate::dex_ui::dialog_alias(name) {\n            return self.open_dialog(alias, p);\n        }\n",
)
insert(
    f"{UI}/app/commands.rs",
    "crate::dex_ui::run_command(self, id, p)",
    "        if let Some(r) = crate::collab_ui::run_command(self, id, p) {\n            return Some(r);\n        }\n",
    "        if let Some(r) = crate::dex_ui::run_command(self, id, p) {\n            return Some(r);\n        }\n",
)
insert(
    f"{UI}/backstage.rs",
    "dex_ui::templates::new_page(",
    "    new_tiles(app, ui, out);\n",
    "    crate::dex_ui::templates::new_page(app, ui, &mut out.close);\n",
)
insert(
    f"{UI}/ribbon/formulas.rs",
    "dex_ui::tools::auditing_buttons(",
    "        crate::dex_ui::remove_arrows_button(app, ui);\n    });\n",
    "    ui.vertical(|ui| crate::dex_ui::tools::auditing_buttons(app, ui));\n",
)
insert(
    f"{UI}/ribbon/review.rs",
    "dex_ui::tools::review_buttons(",
    '        app.open_dialog("accessibility", json!({}));\n    }\n',
    "    crate::dex_ui::tools::review_buttons(app, ui);\n",
)
insert(
    f"{UI}/app/files.rs",
    "dex_ui::share::open_link(",
    "    pub fn open_path(&mut self, path: &str) {\n",
    "        // Dexterity Sheets: a dexterity:// link to a shared spreadsheet, not a file.\n        if crate::dex_ui::share::open_link(self, path) {\n            return;\n        }\n",
)
insert(
    f"{UI}/dialogs/about.rs",
    "dex_ui::releases::about_details(",
    '            ui.label(format!("Version {}", gridcraft_collab::APP_VERSION.trim()));\n',
    "            crate::dex_ui::releases::about_details(ui);\n",
)
MAIN = "apps/gridcraft/src/main.rs"
insert(MAIN, "mod regional;", "mod print;\n", "mod regional;\n")
insert(
    MAIN,
    "regional::apply();",
    "    let mut session = Session::new();\n",
    "    // Regional settings (date order and separator, digit grouping) before any workbook is read.\n    regional::apply();\n",
    after=False,
)
replace(MAIN, '"../../../assets/app-icon/gridcraft-macos-512.png"', '"../../../assets/app-icon/dexterity-sheets-macos-512.png"', required=False)
replace(MAIN, '.map(|c| c.join("gridcraft"))', '.map(|c| c.join("dexterity-sheets"))', required=False)
replace(MAIN, "//! GridCraft desktop app.\n", "//! Dexterity Sheets desktop app (GridCraft-based; the binary is `DexteritySheets`).\n", required=False)
replace(MAIN, "//! Usage: `gridcraft [", "//! Usage: `DexteritySheets [", required=False)
for p in glob.glob(os.path.join(ROOT, UI, "ribbon", "*.rs")):
    rel = os.path.relpath(p, ROOT).replace("\\", "/")
    if '"app.checkForUpdates"' in read(rel):
        insert(
            rel,
            "crate::dex_ui::releases::help_buttons(app, ui);",
            '        app.run_or_alert("app.checkForUpdates", json!({}));\n    }\n',
            "    crate::dex_ui::releases::help_buttons(app, ui);\n",
        )
insert(
    MAIN,
    "dex_ui::enable_desktop(",
    "    let mut app = SheetApp::new(session, services());\n",
    "    // Dexterity Sheets: crash recovery, links and notifications belong to the desktop app only.\n    gridcraft_ui_egui::dex_ui::enable_desktop();\n",
)
insert(
    "apps/gridcraft/src/main.rs",
    "dex_ui::recovery::clean_exit(",
    "    fn on_exit(&mut self) {\n        print::finish_pending_jobs();\n",
    "        // Dexterity Sheets: a normal exit leaves no crash-recovery copies behind.\n        gridcraft_ui_egui::dex_ui::recovery::clean_exit(&mut self.0);\n",
)
insert(
    f"{UI}/grid/show.rs",
    "crate::dex_ui::paint(",
    selection_call(read(f"{UI}/grid/show.rs")) or SELECTION_CALLS[0],
    "        crate::dex_ui::paint(&p, geo, sh, app, *rows, *cols);\n",
    after=False,
)
replace(
    f"{UI}/grid/keyboard.rs",
    "                    Key::F4 if !cmd => {}\n",
    "                    // Repeat the last action (Dexterity Sheets); nothing happens when there is none.\n"
    "                    Key::F4 if !cmd => {\n                        let _ = app.run(\"edit.repeat\", json!({}));\n                    }\n",
    required=False,
)
insert(
    f"{UI}/ribbon/shortcuts.rs",
    '("edit.repeat", json!({}))',
    '        Key::Y => Some(("edit.redo", json!({}))),\n',
    "        // Ctrl+Y redoes, or repeats the last action when there is nothing to redo (Dexterity Sheets).\n"
    '        Key::Y if app.session.active().is_some_and(|d| d.redo.is_empty()) => Some(("edit.repeat", json!({}))),\n',
    after=False,
)
replace(
    f"{UI}/ribbon/formulas.rs",
    '        if small_button(ui, Icon::Close, "Remove Arrows", "Remove Arrows", false).clicked() {\n            app.grid.trace = None;\n        }\n',
    "        crate::dex_ui::remove_arrows_button(app, ui);\n",
    required=False,
)
insert(
    f"{UI}/ribbon/data.rs",
    '"data.clearValidationCircles"',
    '                ("Circle Invalid Data", "data.circleInvalid", json!({})),\n',
    '                ("Clear Validation Circles", "data.clearValidationCircles", json!({})),\n',
)
insert(
    f"{UI}/ribbon/data.rs",
    "crate::dex_ui::outline_buttons(app, ui);",
    '            app.open_dialog("outlineSettings", json!({}));\n        }\n    });\n',
    "    crate::dex_ui::outline_buttons(app, ui);\n",
)
insert(
    f"{UI}/ribbon/view.rs",
    "crate::dex_ui::focus_cell_button(app, ui);",
    '            act(app, "view.headings", json!({"on": h}));\n        }\n    });\n',
    "    ui.add_space(6.0);\n    crate::dex_ui::focus_cell_button(app, ui);\n",
)

# ---- UI test: File › Account shows Dexterity Sheets' own version -------------------------------
replace(
    "crates/ui-egui/tests/it/file_menu.rs",
    '    h.get_by_label(env!("CARGO_PKG_VERSION"));\n    h.get_by_label(env!("CARGO_PKG_LICENSE"));',
    "    // Dexterity Sheets shows its own version (packaging/dexterity/VERSION), not GridCraft's.\n"
    '    h.get_by_label(gridcraft_collab::APP_VERSION.trim());\n    h.get_by_label(env!("CARGO_PKG_LICENSE"));',
    required=False,
)

# ---- XLSX formulas: the fast path must still spell bare table names `Table[]` ------------------
insert(
    "crates/xlsx/src/fmla.rs",
    "Expr::Name(n) => is_table(n)",
    "            Expr::Struct(_) => true,\n",
    "            // Dexterity Sheets: a bare table name is written `Table[]` (see [`TableNames`]).\n"
    "            Expr::Name(n) => is_table(n),\n",
)
# The arm sits wherever upstream strips the `_xlpm.` prefixes of a formula read from a file: in
# `from_file` itself at first, in `strip_param_prefixes` since upstream split that out.
FMLA = "crates/xlsx/src/fmla.rs"
for indent in ("        ", "            "):
    if f"{indent}Expr::Call(n, args) => Expr::Call(strip_param_prefix(n), args),\n" in read(FMLA) or "Expr::Name(s.table)" in read(FMLA):
        insert(
            FMLA,
            "Expr::Name(s.table)",
            f"{indent}Expr::Call(n, args) => Expr::Call(strip_param_prefix(n), args),\n",
            f"{indent}// Dexterity Sheets: `Table[]` is how a bare table name is spelled in a file (see\n"
            f"{indent}// [`TableNames`]); it reads back as the bare name, the same range, so a formula keeps\n"
            f"{indent}// the text it was typed with. Tables and defined names share one namespace.\n"
            f"{indent}Expr::Struct(s) if !s.table.is_empty() && s.specifiers.is_empty() && s.col_start.is_none() && s.col_end.is_none() => {{\n"
            f"{indent}    Expr::Name(s.table)\n"
            f"{indent}}}\n",
        )
        break
else:
    problems.append(f"{FMLA}: where `_xlpm.` prefixes are stripped (`Expr::Call(n, args) => Expr::Call(strip_param_prefix(n), args),`) not found for [Expr::Name(s.table)]")
insert(
    "crates/xlsx/src/write.rs",
    "crate::fmla::TableNames::of(wb)",
    "    check_sheet_names(wb)?;\n",
    "    // Formulas written from here on spell this workbook's bare table names `Table[]`.\n    let _tables = crate::fmla::TableNames::of(wb);\n",
)

# ---- Number formats: regional settings (crates/numfmt/src/dex.rs) ---------------------------------
# Upstream's renderer stays as upstream writes it; these calls are the only Dexterity lines in it.
NUMFMT = "crates/numfmt/src"
insert(f"{NUMFMT}/lib.rs", "mod dex;", "mod decimal;\n", "mod dex;\n")
replace(
    f"{NUMFMT}/lib.rs",
    "            text: format_general_fit(n, GENERAL_WIDTH).unwrap_or_else(|| OVERFLOW.into()),\n",
    "            text: format_general_fit(n, GENERAL_WIDTH).map_or_else(|| OVERFLOW.into(), dex::general),\n",
    required=False,
)
replace(
    f"{NUMFMT}/lib.rs",
    "    let (f, overflow) = fmt.format_number(n, sys);\n",
    "    let (f, overflow) = dex::as_typed(|| fmt.format_number(n, sys));\n",
    required=False,
)
replace(
    f"{NUMFMT}/parse.rs",
    "            Tok::Date(_) | Tok::Fill(_) => out.push(t.clone()),\n",
    "            // `/` is the date separator: shown as the locale's (an escaped `\\/` is a literal).\n"
    "            Tok::Date(_) | Tok::Fill(_) | Tok::Slash => out.push(t.clone()),\n",
    required=False,
)
RENDER = f"{NUMFMT}/render.rs"
replace(
    RENDER,
    '            Tok::General => out.push(&format_general_fit(v, width).unwrap_or_else(|| "#".repeat(width.clamp(1, 11)))),\n',
    '            Tok::General => out.push(&crate::dex::general(format_general_fit(v, width).unwrap_or_else(|| "#".repeat(width.clamp(1, 11))))),\n',
    required=False,
)
replace(RENDER, "                        text.push('.');\n", "                        text.push(crate::dex::ctx().decimal);\n", required=False)
insert(
    RENDER,
    "Tok::Slash => out.text.push(crate::dex::ctx().date_sep)",
    "                    DatePart::AP(a, p) => text.push(if hour >= 12 { p } else { a }),\n                }\n            }\n",
    "            // Dexterity Sheets: the date separator (`\\/` and `\"/\"` are literal and stay `/`).\n"
    "            Tok::Slash => out.text.push(crate::dex::ctx().date_sep),\n",
)
replace(
    RENDER,
    "    let sep = |p: usize| thousands && p > 0 && p.is_multiple_of(3);\n",
    "    let dex = crate::dex::ctx();\n    let sep = |p: usize| thousands && dex.grouping.separator_at(p);\n",
    required=False,
)
replace(RENDER, "parts.buf.push(',');", "parts.buf.push(dex.thousands);", required=False)
replace(
    RENDER,
    "        if i > 0 && (n - i).is_multiple_of(3) {\n            s.push(',');\n",
    "        if i > 0 && crate::dex::ctx().grouping.separator_at(n - i) {\n            s.push(crate::dex::ctx().thousands);\n",
    required=False,
)
replace(RENDER, '            parts.set_piece(p, ".");\n', "            parts.set_piece(p, crate::dex::ctx().decimal.encode_utf8(&mut [0; 4]));\n", required=False)

# ---- Windows packaging: upstream's install-policy check is for GridCraft's identity -----------
s = read("packaging/windows/package.ps1")
s2 = re.sub(r"# Check the compiled scope, path and registration contract before signing\.\r?\n& \(Join-Path \$PSScriptRoot 'check-install-policy\.ps1'\) -MsiPath \$Msi\r?\n",
            "# Dexterity Sheets: no check-install-policy.ps1 (it checks GridCraft's install identity, not ours).\n", s)
if s2 != s:
    write("packaging/windows/package.ps1", s2)

# ---- XLSX formulas: IFS, SWITCH, INDIRECT and CELL used as values keep array evaluation ---------
ARRAY = "crates/xlsx/src/array_formula.rs"
insert(ARRAY, "mod dex;", "use crate::FunctionInfo;\n", "\n// Dexterity Sheets: array_formula/dex.rs (formulas Excel would show with a spurious `@`).\nmod dex;\n")
insert(
    ARRAY,
    "dex::reference_function_used_as_value(",
    "    expr.walk(&mut |e| explicit |= matches!(e, Expr::Unary(UnOp::At, _)));\n",
    "    // Dexterity Sheets: IFS, SWITCH, INDIRECT or CELL used as a value is saved with array\n"
    "    // evaluation, as Excel saves its own, so Excel does not show it as `=@IFS(…)`.\n"
    "    if !explicit && dex::reference_function_used_as_value(&expr) {\n"
    "        return Export::Dynamic;\n"
    "    }\n",
)
replace(
    "crates/xlsx/src/tests/array_export.rs",
    '            ("SQRT(INDIRECT(\\"A1\\"))", false),\n',
    "            // Dexterity Sheets: saved with array evaluation, or Excel shows `SQRT(@INDIRECT(\"A1\"))`\n"
    "            // (array_formula/dex.rs).\n"
    '            ("SQRT(INDIRECT(\\"A1\\"))", true),\n',
    required=False,
)

# ---- Key tips: Alt, Q opens the command search --------------------------------------------------
replace(
    f"{UI}/keytips/keytips.json",
    '   "seq": "Q",\n   "kind": "command",\n   "target": "ui.search",\n   "pending": true,\n   "excel": "Search / Tell Me",\n   "note": "Alt+Q search box does not exist yet (issue #49)"\n',
    '   "seq": "Q",\n   "kind": "dialog",\n   "target": "commandSearch",\n   "excel": "Search / Tell Me",\n   "note": "Dexterity Sheets: Alt, Q opens the command search (the title bar\'s search icon)"\n',
    required=False,
)

# ---- Agents read RULES.md first ---------------------------------------------------------------
# Upstream keeps its agent instructions in AGENTS.md (CLAUDE.md points at it); both get the line.
POINTER = (
    "> **Dexterity Sheets:** read [`RULES.md`](RULES.md) first. Its rules (share from anywhere, no placeholders, performance budgets, docs and `CHANGES.md` in the same change, Excel and GridCraft kept current) bind every change here.\n"
    "> Where the rules below (written for official GridCraft's own repositories) and `RULES.md` differ, for example on how `main` changes, on CI and on releases, `RULES.md` wins in this repository.\n\n"
)
for doc in ("CLAUDE.md", "AGENTS.md"):
    if os.path.exists(os.path.join(ROOT, doc)) and "RULES.md" not in read(doc):
        write(doc, POINTER + read(doc))

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
            ("Based on", format!("GridCraft {} by the ArtCraft team (MIT/Apache-2.0)", gridcraft_collab::GRIDCRAFT_RELEASE.trim())),
        ] {""",
    required=False,
)
# Builds up to 1.4.1 named the workspace version here, which lags behind the release on main.
replace(
    B,
    'format!("GridCraft {} by the ArtCraft team (MIT/Apache-2.0)", env!("CARGO_PKG_VERSION"))',
    'format!("GridCraft {} by the ArtCraft team (MIT/Apache-2.0)", gridcraft_collab::GRIDCRAFT_RELEASE.trim())',
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

# ---- Contributors: names as plain text (an internal LAN tool opens no websites) ---------------
replace(f"{UI}/credits.rs", 'ui.hyperlink_to(c.name(v.names), format!("https://github.com/{}", c.login))', "ui.label(c.name(v.names))", required=False)

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
# Upstream's range.setValues tests (GridCraft 0.15, #2255) take `05.04.2026` for a date. Here it
# is text, as above, so it is not among the conversions reported, and the date that survives the
# round trip is one typed with the separator.
replace(
    edit_tests,
    '    assert_eq!(r["convertedCount"], 4, "{r}");\n'
    '    assert_eq!(r["converted"][0], json!({"cell": "A1", "input": "05.04.2026", "stored": "5/4/2026"}), "{r}");\n'
    '    assert_eq!(r["converted"][1], json!({"cell": "B1", "input": "02/11/2026", "stored": "2/11/2026"}), "{r}");\n'
    '    assert_eq!(r["converted"][2], json!({"cell": "C1", "input": "2.500", "stored": "2.5"}), "{r}");\n'
    '    assert_eq!(r["converted"][3], json!({"cell": "D1", "input": "4.25 %", "stored": "4.25%"}), "{r}");\n',
    "    // Dexterity Sheets: a dotted date is text where `.` isn't the date separator (en-US), as in\n"
    "    // Excel, so A1 is stored as typed and is not a conversion.\n"
    '    assert_eq!(r["convertedCount"], 3, "{r}");\n'
    '    assert_eq!(r["converted"][0], json!({"cell": "B1", "input": "02/11/2026", "stored": "2/11/2026"}), "{r}");\n'
    '    assert_eq!(r["converted"][1], json!({"cell": "C1", "input": "2.500", "stored": "2.5"}), "{r}");\n'
    '    assert_eq!(r["converted"][2], json!({"cell": "D1", "input": "4.25 %", "stored": "4.25%"}), "{r}");\n',
    required=False,
)
replace(
    edit_tests,
    '    s.execute("range.setValues", json!({"range": "A3", "values": [["05.04.2026"]]})).unwrap();\n',
    "    // Dexterity Sheets: typed with the date separator (a dotted date stays text under en-US).\n"
    '    s.execute("range.setValues", json!({"range": "A3", "values": [["5/4/2026"]]})).unwrap();\n',
    required=False,
)
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
