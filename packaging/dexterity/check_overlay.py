"""Checks that every Dexterity Sheets hook into upstream GridCraft files is still in place.

Dexterity Sheets keeps its own code in its own files (crates/collab, apps/gridcraft-server,
crates/ui-egui/src/collab_ui.rs, packaging/dexterity). Upstream files carry only small hook lines
that call into them. A merge from upstream can silently drop a hook (for example when a
conflicted file is taken from upstream), which would compile but lose a feature, so the sync and
CI workflows run this check and fail if any marker is missing.

Usage: python packaging/dexterity/check_overlay.py [repo root]
"""
import glob
import os
import re
import sys

# (file, marker that must be present, what it does)
HOOKS = [
    ("Cargo.toml", 'gridcraft-collab = { path = "crates/collab" }', "workspace dependency on the collab crate"),
    ("xtask/src/layers.rs", '("collab", Class::Layer(6))', "layering rule for collab"),
    ("xtask/src/layers.rs", '&["collab", "ui-egui"]', "ui-egui may use collab"),
    ("xtask/src/layers.rs", '("server-manager", Class::Layer(7))', "layering rule for the Server Manager"),
    ("CLAUDE.md", "RULES.md", "agents are pointed at RULES.md"),
    ("AGENTS.md", "RULES.md", "agents are pointed at RULES.md (upstream's own instructions live here)"),
    ("crates/xlsx/src/array_formula.rs", "mod dex;", "the Dexterity addition to the legacy/dynamic formula choice is compiled in"),
    ("crates/xlsx/src/array_formula.rs", "dex::reference_function_used_as_value(&expr)", "IFS, SWITCH, INDIRECT and CELL used as values are saved with array evaluation (no `@` in Excel)"),
    ("crates/ui-egui/src/keytips/keytips.json", '"target": "commandSearch"', "Alt, Q opens the command search"),
    ("packaging/dexterity/GRIDCRAFT_COMMIT", "", "the official GridCraft commit this tree was last synced with"),
    ("crates/engine/src/{lib.rs,session.rs,session/*.rs}", "pub fn document_mut(&mut self, uid: u64)", "engine accessor used to apply remote edits"),
    ("crates/ui-egui/Cargo.toml", "gridcraft-collab = { workspace = true }", "UI depends on collab"),
    ("crates/ui-egui/src/lib.rs", "pub mod collab_ui;", "collab UI module"),
    ("crates/ui-egui/src/{lib.rs,app.rs,state.rs}", "pub collab: collab_ui::Collab,", "collab state on the app"),
    ("crates/ui-egui/src/{lib.rs,app.rs,app/*.rs}", "collab_ui::tick(self, ctx);", "per-frame sync"),
    ("crates/ui-egui/src/{lib.rs,app.rs,app/*.rs}", "collab_ui::run_command(self, id, p)", "collab commands (sign in, share, save version, updates)"),
    ("crates/ui-egui/src/{backstage.rs,backstage/*.rs}", "Page::Shared => crate::collab_ui::shared_page", "File > Shared page"),
    ("crates/ui-egui/src/{backstage.rs,backstage/*.rs}", "gridcraft_collab::GRIDCRAFT_RELEASE", "File > Account names the GridCraft release this build contains"),
    ("crates/ui-egui/src/{panes.rs,panes/*.rs}", '"history" => crate::collab_ui::history_pane', "Edit History pane"),
    ("crates/ui-egui/src/{panes.rs,panes/*.rs}", '"versions" => crate::collab_ui::versions_pane', "Versions pane"),
    ("crates/ui-egui/src/{grid.rs,grid/*.rs}", "crate::collab_ui::paint_peers(", "other people's selections on the grid"),
    ("crates/ui-egui/src/{tabs.rs,tabs/*.rs}", "crate::collab_ui::status(app, ui, &t);", "status bar: shared state and last editor"),
    ("crates/ui-egui/src/{tabs.rs,tabs/*.rs}", "crate::collab_ui::update_badge(app, ui, &t);", "status bar: update available"),
    ("crates/ui-egui/src/{ribbon.rs,ribbon/*.rs}", '"collab.history"', "Review tab: Edit History"),
    ("crates/ui-egui/src/{ribbon.rs,ribbon/*.rs}", "collab_ui::share_menu(app, ui)", "title-bar Share menu"),
    ("apps/gridcraft/Cargo.toml", 'name = "DexteritySheets"', "executable name"),
    ("apps/gridcraft/src/main.rs", "regional::apply();", "Windows regional settings (dates, grouping) like Excel"),
    ("apps/gridcraft/src/main.rs", "mod regional;", "the regional-settings module is compiled in"),
    ("apps/gridcraft/src/main.rs", "dexterity-sheets-macos-512.png", "the macOS window icon is ours (the GridCraft one is not in the tree)"),
    ("crates/ui-egui/src/updates_ui.rs", "pub const APP: craft_update::App", "the update check looks at the Dexterity Sheets releases, never GridCraft's"),
    ("packaging/windows/gridcraft.wxs", 'UpgradeCode="FE2AD491-38E3-4E1B-8441-FDF652AB8443"', "installer identity (never change)"),
    ("packaging/windows/gridcraft.wxs", 'Scope="perUser"', "per-user install so updates need no admin"),
    # Excel parity additions (docs/dexterity-parity-audit.md): commands in cmd/dex.rs, UI in dex_ui.rs.
    ("crates/engine/src/cmd.rs", "pub mod dex;", "Dexterity commands module"),
    ("crates/engine/src/cmd.rs", "dex::specs().into_iter().filter(", "Dexterity commands registered (upstream's own id wins)"),
    ("crates/engine/src/{lib.rs,session.rs,session/*.rs}", "pub dex: cmd::dex::DexState,", "Dexterity command state on the session"),
    ("crates/engine/src/{lib.rs,session.rs,session/*.rs}", "cmd::dex::after_command(self, id, &params, &result, dex_before);", "Repeat and validation circles see every command"),
    ("crates/engine/tests/it/command_sequence_fuzz.rs", '("edit.repeat", DEX)', "fuzzer keeps Dexterity commands out of its random pool (sequences match upstream)"),
    ("crates/ui-egui/src/lib.rs", "pub mod dex_ui;", "Dexterity UI module"),
    ("crates/ui-egui/tests/it/main.rs", "mod dex_parity;", "UI tests of the Dexterity additions"),
    ("crates/ui-egui/src/{dialogs.rs,dialogs/*.rs}", "crate::dex_ui::new_rule_button(app, ui);", "Rules Manager: New Rule…"),
    ("crates/ui-egui/src/{dialogs.rs,dialogs/*.rs}", "crate::dex_ui::mention::field(ui, &mut s, d.name == \"comment\"", "New Comment: @mention suggestions"),
    ("crates/ui-egui/src/{panes.rs,panes/*.rs}", "crate::dex_ui::mention::field(ui, &mut draft, c.threaded", "Comments pane replies: @mention suggestions"),
    ("crates/ui-egui/src/{dialogs.rs,dialogs/*.rs}", "crate::dex_ui::result_view(", "result dialogs (Statistics, Accessibility, Error Checking, Evaluate Formula) readable, not raw JSON"),
    ("crates/ui-egui/src/{lib.rs,app.rs,app/*.rs}", "crate::dex_ui::run_command(self, id, p)", "Remove Arrows variants clear the trace arrows"),
    ("crates/ui-egui/src/{lib.rs,app.rs,app/*.rs}", "crate::dex_ui::dialog_alias(name)", "engine dialog requests under other names (New Formatting Rule)"),
    ("crates/ui-egui/src/{grid.rs,grid/*.rs}", "crate::dex_ui::paint(&p, geo, sh, app, *rows, *cols);", "Focus Cell and validation circles on the grid"),
    ("crates/ui-egui/src/{grid.rs,grid/*.rs}", '"edit.repeat"', "F4 repeats the last action"),
    ("crates/ui-egui/src/{ribbon.rs,ribbon/*.rs}", '("edit.repeat", json!({}))', "Ctrl+Y repeats when there is nothing to redo"),
    ("crates/ui-egui/src/{ribbon.rs,ribbon/*.rs}", "crate::dex_ui::remove_arrows_button(app, ui);", "Formulas: Remove Arrows menu"),
    ("crates/ui-egui/src/{ribbon.rs,ribbon/*.rs}", '"data.clearValidationCircles"', "Data Validation menu: Clear Validation Circles"),
    ("crates/ui-egui/src/{ribbon.rs,ribbon/*.rs}", "crate::dex_ui::outline_buttons(app, ui);", "Data: Auto Outline and Clear Outline"),
    ("crates/ui-egui/src/{ribbon.rs,ribbon/*.rs}", "crate::dex_ui::focus_cell_button(app, ui);", "View: Focus Cell"),
    ("crates/xlsx/src/fmla.rs", "pub(crate) struct TableNames", "bare table names written as Table[] (Excel reads a bare name as #NAME?)"),
    ("crates/xlsx/src/fmla.rs", "Expr::Name(n) => is_table(n)", "the formula writer's fast path still rewrites bare table names"),
    ("crates/xlsx/src/fmla.rs", "Expr::Name(s.table)", "Table[] reads back as the bare table name it was written from"),
    ("crates/xlsx/src/write.rs", "crate::fmla::TableNames::of(wb)", "the workbook writer knows the table names, so bare ones are written as Table[]"),
    # Regional settings in number display (crates/numfmt/src/dex.rs): the only Dexterity lines in upstream's renderer.
    ("crates/numfmt/src/lib.rs", "mod dex;", "the regional-settings module of number formats is compiled in"),
    ("crates/numfmt/src/lib.rs", "dex::as_typed(|| fmt.format_number(n, sys))", "TEXT() takes its format text as typed (`/` stays `/`)"),
    ("crates/numfmt/src/lib.rs", "dex::general)", "General shows the locale's decimal point"),
    ("crates/numfmt/src/parse.rs", "Tok::Date(_) | Tok::Fill(_) | Tok::Slash", "`/` in a date code stays the date separator"),
    ("crates/numfmt/src/render.rs", "crate::dex::general(", "a General section shows the locale's decimal point"),
    ("crates/numfmt/src/render.rs", "text.push(crate::dex::ctx().decimal)", "fractions of a second use the locale's decimal point"),
    ("crates/numfmt/src/render.rs", "Tok::Slash => out.text.push(crate::dex::ctx().date_sep)", "dates show the locale's date separator"),
    ("crates/numfmt/src/render.rs", "dex.grouping.separator_at(p)", "digits group the locale's way (3;2 in en-IN)"),
    ("crates/numfmt/src/render.rs", "parts.buf.push(dex.thousands)", "the locale's thousands separator"),
    ("crates/numfmt/src/render.rs", "crate::dex::ctx().grouping.separator_at(n - i)", "grouping when a section has no integer placeholder"),
    ("crates/numfmt/src/render.rs", "crate::dex::ctx().decimal.encode_utf8(", "numbers show the locale's decimal point"),
    ("apps/gridcraft/src/main.rs", "gridcraft_ui_egui::dex_ui::recovery::clean_exit(&mut self.0);", "a normal exit removes the crash-recovery copies"),
    ("apps/gridcraft/src/main.rs", "gridcraft_ui_egui::dex_ui::enable_desktop();", "crash recovery, links and notifications run in the desktop app only (never in tests)"),
    ("crates/ui-egui/src/{dialogs.rs,dialogs/*.rs}", "crate::dex_ui::releases::about_details(ui);", "About shows the build, update channel and repository"),
    ("crates/ui-egui/src/{lib.rs,app.rs,app/*.rs}", "crate::dex_ui::share::open_link(self, path)", "dexterity:// links open shared spreadsheets"),
    ("crates/ui-egui/src/{ribbon.rs,ribbon/*.rs}", "crate::dex_ui::tools::auditing_buttons(app, ui)", "Formulas: Diagnostics and Explain Error"),
    ("crates/ui-egui/src/{ribbon.rs,ribbon/*.rs}", "crate::dex_ui::tools::review_buttons(app, ui);", "Review: Compare Files and Quality Check"),
    ("crates/ui-egui/src/backstage.rs", "crate::dex_ui::templates::new_page(app, ui, &mut out.close);", "File › New: office and personal templates"),
    ("crates/ui-egui/src/{ribbon.rs,ribbon/*.rs}", "crate::dex_ui::releases::help_buttons(app, ui);", "Automate: What's New, Report a Problem, Roll Back, Recover Unsaved Work"),
]

# Strings that must not come back into the shipped app's code from upstream.
FORBIDDEN = [
    ("packaging/windows/package.ps1", "check-install-policy.ps1') -MsiPath", "GridCraft's install-identity check (fails on our installer)"),
    ("crates/ui-egui/src/credits.rs", "ui.hyperlink_to(c.name(", "contributor names as web links (an internal LAN tool opens no websites; reapply_overlay.py makes them plain text)"),
]


def expand(root, path):
    m = re.search(r"\{([^}]*)\}", path)
    options = [path[: m.start()] + o + path[m.end() :] for o in m.group(1).split(",")] if m else [path]
    out = []
    for o in options:
        out.extend(f for f in glob.glob(os.path.join(root, o)) if os.path.isfile(f))
    return out


def unruled() -> int:
    """`--unruled`: the files official GridCraft owns that differ here, with how much and how
    many markers above cover the file. A file with changes and no marker merges only until
    upstream rewrites the same lines (see UPSTREAM.md): each is a candidate for a hook."""
    import subprocess

    sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
    import sync_upstream as sync  # noqa: E402

    with open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "GRIDCRAFT_COMMIT"), encoding="utf-8") as f:
        base = f.read().strip()
    out = subprocess.run(["git", "diff", "--numstat", "--diff-filter=M", base, "HEAD"], capture_output=True, text=True, encoding="utf-8")
    print(f"Changed here in files official GridCraft owns (against {base[:10]}): lines added, removed, markers, file")
    for row in out.stdout.splitlines():
        added, removed, path = row.split("\t")
        if sync.matches(path, sync.OURS) or path == "Cargo.lock" or path.startswith(("assets/", "docs/images/")) or path in sync.GENERATED:
            continue
        markers = len(sync.hook_markers(path))
        note = "branding text" if sync.rebranded(path) and not markers else ("" if markers else "NO MARKER")
        print(f"{added:>5} {removed:>5} {markers:>3}  {path}  {note}")
    return out.returncode


def main() -> int:
    if "--unruled" in sys.argv:
        return unruled()
    root = sys.argv[1] if len(sys.argv) > 1 else os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
    missing = []
    for path, marker, what in HOOKS:
        # A path may name alternatives ({a.rs,a/*.rs}) so an upstream refactor that splits a file
        # into a module doesn't count as a lost hook.
        texts = [open(f, encoding="utf-8").read() for f in expand(root, path)]
        if not texts:
            missing.append(f"{path}: file missing ({what})")
        elif not any(marker in t for t in texts):
            missing.append(f"{path}: hook missing: {what}  [{marker}]")
    for path, bad, what in FORBIDDEN:
        try:
            text = open(os.path.join(root, path), encoding="utf-8").read()
        except OSError:
            continue
        if bad in text:
            missing.append(f"{path}: came back from upstream: {what}  [{bad}]")
    # Not a failure: a Dexterity command that upstream now has itself gives way to upstream's at
    # run time (cmd.rs), but the Dexterity copy and its UI should then be removed.
    try:
        dex = open(os.path.join(root, "crates/engine/src/cmd/dex.rs"), encoding="utf-8").read()
        ids = re.findall(r'cmd!\((?:noundo |query )?"([A-Za-z.]+)"', dex)
        theirs = ""
        for f in glob.glob(os.path.join(root, "crates/engine/src/cmd/**/*.rs"), recursive=True):
            if not f.endswith("dex.rs"):
                theirs += open(f, encoding="utf-8").read()
        for i in ids:
            if re.search(r'cmd!\(\s*(?:noundo |query )?"' + re.escape(i) + '"', theirs):
                print(f"NOTE: upstream GridCraft now registers {i}; its version is used. Remove the Dexterity copy from cmd/dex.rs and dex_ui.rs.")
    except OSError:
        pass
    for m in missing:
        print("OVERLAY:", m)
    if missing:
        print(f"{len(missing)} Dexterity Sheets hook problem(s); see packaging/dexterity/UPSTREAM.md")
        return 1
    print(f"all {len(HOOKS)} Dexterity Sheets hooks present")
    return 0


if __name__ == "__main__":
    sys.exit(main())
