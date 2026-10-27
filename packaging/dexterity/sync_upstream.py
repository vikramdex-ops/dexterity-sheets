"""Merges the latest official GridCraft into Dexterity Sheets.

Run from the repository root on a clean checkout of `main` (the upstream-sync workflow does this):

    python packaging/dexterity/sync_upstream.py

Writes `sync-result.json` and GitHub step outputs:
- status: up-to-date | merged | conflict
- upstream: the upstream commit merged
- version: the new Dexterity Sheets version (when merged)

Conflict policy (see packaging/dexterity/UPSTREAM.md):
1. Files Dexterity Sheets owns: keep ours.
2. Files Dexterity Sheets deleted (upstream CI, ArtCraft brand files, old icons): stay deleted.
3. Files where our only change is user-visible branding text (translations, About text…): take
   upstream's file and re-apply the branding rule.
4. Anything else: stop and report; the workflow opens a pull request for a person.
Afterwards check_overlay.py verifies that every hook into upstream files is still present.
"""
import fnmatch
import json
import os
import re
import subprocess
import sys

UPSTREAM_URL = "https://github.com/storytold/gridcraft.git"
UPSTREAM_BRANCH = "main"

OURS = [
    "crates/collab/*",
    "apps/gridcraft-server/*",
    "crates/ui-egui/src/collab_ui.rs",
    "packaging/dexterity/*",
    "docs/collaboration.md",
    ".github/workflows/dexterity-*",
    ".github/workflows/upstream-sync.yml",
    "README.md",
    "ATTRIBUTION.md",
    "NOTICE",
    "assets/app-icon/*",
    "crates/ui-egui/src/updates_ui.rs",
    "docs/images/*",
    ".gitattributes",
]
DELETED_BY_US = [
    ".github/workflows/*",
    ".github/dependabot.yml",
    "docs/brand/*",
]
# Files whose only Dexterity change is branding text.
BRANDING_ONLY = [
    "crates/ui-egui/src/i18n/*.rs",
    "crates/ui-egui/src/credits.rs",
    "crates/ui-egui/src/dialogs.rs",
    "crates/ui-egui/src/formula_bar.rs",
    "crates/ui-egui/src/craft_fonts_policy.rs",
    "crates/ui-egui/tests/it/file_menu.rs",
    "apps/gridcraft/src/native_menu.rs",
    "apps/gridcraft/src/print.rs",
    "crates/xlsx/src/write.rs",
]
KEEP_UPSTREAM_NAME = re.compile(r"Based on GridCraft|GridCraft by the ArtCraft|gridcraft-cli|GRIDCRAFT_|storytold/gridcraft")


def git(*args, check=True):
    r = subprocess.run(["git", *args], capture_output=True, text=True, encoding="utf-8")
    if check and r.returncode != 0:
        raise SystemExit(f"git {' '.join(args)} failed:\n{r.stdout}\n{r.stderr}")
    return r.stdout.strip()


def matches(path, patterns):
    return any(fnmatch.fnmatch(path, p) for p in patterns)


def rebrand(text: str) -> str:
    """User-visible "GridCraft" → "Dexterity Sheets" inside string literals, leaving code
    identifiers, the attribution line and CLI/env names alone."""
    out = []
    for line in text.splitlines(keepends=True):
        if "GridCraft" in line and '"' in line and not KEEP_UPSTREAM_NAME.search(line):
            line = re.sub(r'"([^"\n]*)"', lambda m: '"' + m.group(1).replace("GridCraft", "Dexterity Sheets") + '"', line)
        out.append(line)
    return "".join(out)


def output(**kv):
    with open("sync-result.json", "w", encoding="utf-8") as f:
        json.dump(kv, f, indent=2)
    gh = os.environ.get("GITHUB_OUTPUT")
    if gh:
        with open(gh, "a", encoding="utf-8") as f:
            for k, v in kv.items():
                f.write(f"{k}={v if isinstance(v, str) else json.dumps(v)}\n")
    print(json.dumps(kv, indent=2))


def bump(version: str) -> str:
    parts = [int(p) if p.isdigit() else 0 for p in version.strip().split(".")]
    while len(parts) < 3:
        parts.append(0)
    parts[2] += 1
    return ".".join(map(str, parts[:3]))


def main() -> int:
    if "upstream" not in git("remote").split():
        git("remote", "add", "upstream", UPSTREAM_URL)
    git("fetch", "--no-tags", "upstream", UPSTREAM_BRANCH)
    head = git("rev-parse", f"upstream/{UPSTREAM_BRANCH}")
    if subprocess.run(["git", "merge-base", "--is-ancestor", head, "HEAD"]).returncode == 0:
        output(status="up-to-date", upstream=head)
        return 0
    subject = git("log", "-1", "--format=%s", head)
    merge = subprocess.run(["git", "merge", "--no-ff", "--no-commit", f"upstream/{UPSTREAM_BRANCH}"], capture_output=True, text=True, encoding="utf-8")
    unresolved = []
    if merge.returncode != 0:
        status_lines = git("status", "--porcelain").splitlines()
        conflicted = [(l[:2], l[3:].strip().strip('"')) for l in status_lines if l[:2] in ("UU", "AA", "DU", "UD", "AU", "UA", "DD")]
        for code, path in conflicted:
            if code in ("UD", "DD") or (code == "DU" and matches(path, DELETED_BY_US)):
                # We deleted it (or both did): it stays deleted.
                git("rm", "-q", "--", path, check=False)
            elif matches(path, OURS):
                git("checkout", "--ours", "--", path)
                git("add", "--", path)
            elif matches(path, BRANDING_ONLY) and code == "UU":
                git("checkout", "--theirs", "--", path)
                with open(path, encoding="utf-8") as f:
                    text = f.read()
                with open(path, "w", encoding="utf-8", newline="") as f:
                    f.write(rebrand(text))
                git("add", "--", path)
            else:
                unresolved.append(f"{code} {path}")
    # Upstream may add new workflows; Dexterity runs only its own.
    for wf in git("diff", "--cached", "--name-only", "--diff-filter=A").splitlines():
        if matches(wf, [".github/workflows/*"]) and not matches(wf, [".github/workflows/dexterity-*", ".github/workflows/upstream-sync.yml"]):
            git("rm", "-q", "--cached", "--", wf, check=False)
            if os.path.exists(wf):
                os.remove(wf)
    if unresolved:
        output(status="conflict", upstream=head, subject=subject, unresolved=unresolved)
        return 0
    hooks = subprocess.run([sys.executable, "packaging/dexterity/check_overlay.py"], capture_output=True, text=True, encoding="utf-8")
    if hooks.returncode != 0:
        output(status="conflict", upstream=head, subject=subject, unresolved=hooks.stdout.splitlines())
        return 0
    with open("packaging/dexterity/VERSION", encoding="utf-8") as f:
        version = bump(f.read())
    with open("packaging/dexterity/VERSION", "w", encoding="utf-8", newline="") as f:
        f.write(version)
    git("add", "packaging/dexterity/VERSION")
    git("commit", "-q", "-m", f"Sync with upstream GridCraft {head[:10]}\n\nUpstream: {subject}\nDexterity Sheets {version}")
    output(status="merged", upstream=head, subject=subject, version=version)
    return 0


if __name__ == "__main__":
    sys.exit(main())
