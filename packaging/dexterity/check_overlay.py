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
    ("crates/engine/src/{lib.rs,session.rs,session/*.rs}", "pub fn document_mut(&mut self, uid: u64)", "engine accessor used to apply remote edits"),
    ("crates/ui-egui/Cargo.toml", "gridcraft-collab = { workspace = true }", "UI depends on collab"),
    ("crates/ui-egui/src/lib.rs", "pub mod collab_ui;", "collab UI module"),
    ("crates/ui-egui/src/{lib.rs,app.rs,state.rs}", "pub collab: collab_ui::Collab,", "collab state on the app"),
    ("crates/ui-egui/src/{lib.rs,app.rs,app/*.rs}", "collab_ui::tick(self, ctx);", "per-frame sync"),
    ("crates/ui-egui/src/{lib.rs,app.rs,app/*.rs}", "collab_ui::run_command(self, id, p)", "collab commands (sign in, share, save version, updates)"),
    ("crates/ui-egui/src/{backstage.rs,backstage/*.rs}", "Page::Shared => crate::collab_ui::shared_page", "File > Shared page"),
    ("crates/ui-egui/src/{panes.rs,panes/*.rs}", '"history" => crate::collab_ui::history_pane', "Edit History pane"),
    ("crates/ui-egui/src/{panes.rs,panes/*.rs}", '"versions" => crate::collab_ui::versions_pane', "Versions pane"),
    ("crates/ui-egui/src/{grid.rs,grid/*.rs}", "crate::collab_ui::paint_peers(", "other people's selections on the grid"),
    ("crates/ui-egui/src/{tabs.rs,tabs/*.rs}", "crate::collab_ui::status(app, ui, &t);", "status bar: shared state and last editor"),
    ("crates/ui-egui/src/{tabs.rs,tabs/*.rs}", "crate::collab_ui::update_badge(app, ui, &t);", "status bar: update available"),
    ("crates/ui-egui/src/{ribbon.rs,ribbon/*.rs}", '"collab.history"', "Review tab: Edit History"),
    ("crates/ui-egui/src/{ribbon.rs,ribbon/*.rs}", "collab_ui::share_menu(app, ui)", "title-bar Share menu"),
    ("apps/gridcraft/Cargo.toml", 'name = "DexteritySheets"', "executable name"),
    ("apps/gridcraft/src/main.rs", "regional::apply();", "Windows regional settings (dates, grouping) like Excel"),
    ("packaging/windows/gridcraft.wxs", 'UpgradeCode="FE2AD491-38E3-4E1B-8441-FDF652AB8443"', "installer identity (never change)"),
    ("packaging/windows/gridcraft.wxs", 'Scope="perUser"', "per-user install so updates need no admin"),
]

# Strings that must not come back into the shipped app's code from upstream.
FORBIDDEN = [
    ("crates/ui-egui/Cargo.toml", "craft-update = {", "upstream internet update checker"),
    ("apps/gridcraft/Cargo.toml", "craft-update = {", "upstream internet update checker"),
]


def expand(root, path):
    m = re.search(r"\{([^}]*)\}", path)
    options = [path[: m.start()] + o + path[m.end() :] for o in m.group(1).split(",")] if m else [path]
    out = []
    for o in options:
        out.extend(f for f in glob.glob(os.path.join(root, o)) if os.path.isfile(f))
    return out


def main() -> int:
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
    for m in missing:
        print("OVERLAY:", m)
    if missing:
        print(f"{len(missing)} Dexterity Sheets hook problem(s); see packaging/dexterity/UPSTREAM.md")
        return 1
    print(f"all {len(HOOKS)} Dexterity Sheets hooks present")
    return 0


if __name__ == "__main__":
    sys.exit(main())
