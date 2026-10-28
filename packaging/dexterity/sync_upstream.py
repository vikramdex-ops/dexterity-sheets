"""Merges the latest official GridCraft into Dexterity Sheets.

Run from the repository root on a clean checkout of `main` (the upstream-sync workflow does this):

    python packaging/dexterity/sync_upstream.py

Writes `sync-result.json` and GitHub step outputs:
- status: up-to-date | merged | conflict | blocked (with a reason)
- upstream: the upstream commit merged; previous: the upstream commit merged last time
- main / candidate: main before the merge, and the merge commit
- version: the new Dexterity Sheets version (when merged)

Safety checks before anything is merged (a failure → status `blocked`, a person is asked):
- the checkout must be clean;
- upstream must not have rewritten history: the commit last mirrored into our `upstream` branch
  must still be an ancestor of upstream main, so a force-push upstream can never silently drop or
  replay work;
- `main` itself must pass check_overlay.py (never build on a broken base).

Conflict policy (see packaging/dexterity/UPSTREAM.md):
1. Files Dexterity Sheets owns: keep ours.
2. Files Dexterity Sheets deleted (upstream CI, ArtCraft brand files, old icons): stay deleted.
3. Files where our only change is user-visible branding text (translations, About text…): take
   upstream's file and re-apply the branding rule. Upstream text added to these files merges
   cleanly, so they are rebranded on clean merges too.
4. Other files both sides changed: take upstream's file and re-apply Dexterity's hooks, branding
   and test updates with reapply_overlay.py. If an anchor can't be found, stop for a person.
5. Anything else (delete/modify, add/add): stop and report; the workflow opens a pull request.
Afterwards check_overlay.py verifies that every hook into upstream files is still present, other
new user-visible "GridCraft" text is listed for review, and the version becomes the next patch
number without a tag. The full CI (build, every test, the three-window end-to-end test) and the
installer smoke test must pass before anything is released.
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
    "apps/gridcraft-server-manager/*",
    "crates/ui-egui/src/collab_ui.rs",
    "packaging/dexterity/*",
    "packaging/windows/setup/*",
    # Our installer has its own identity (name, UpgradeCode, per-user scope, registry keys);
    # upstream's install-scope policy (#2704) is written for GridCraft's.
    "packaging/windows/gridcraft.wxs",
    "packaging/windows/test-desktop-shortcut.ps1",
    "docs/collaboration.md",
    "docs/readme/*",
    "site/*",
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
KEEP_UPSTREAM_NAME = re.compile(r"Based on GridCraft|GridCraft by the ArtCraft|by the ArtCraft team|gridcraft-cli|GRIDCRAFT_|storytold/gridcraft")
VISIBLE_GRIDCRAFT = re.compile(r'"[^"\n]*GridCraft[^"\n]*"')


def git(*args, check=True):
    r = subprocess.run(["git", *args], capture_output=True, text=True, encoding="utf-8")
    if check and r.returncode != 0:
        raise SystemExit(f"git {' '.join(args)} failed:\n{r.stdout}\n{r.stderr}")
    return r.stdout.strip()


def is_ancestor(a, b):
    return subprocess.run(["git", "merge-base", "--is-ancestor", a, b]).returncode == 0


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


def rebrand_file(path):
    with open(path, encoding="utf-8") as f:
        text = f.read()
    if rebrand(text) != text:
        with open(path, "w", encoding="utf-8", newline="") as f:
            f.write(rebrand(text))
    git("add", "--", path)


def branding_review(paths):
    """User-visible "GridCraft" string literals in upstream-changed UI files outside the
    branding-only list. Not changed automatically (it could be an identifier or a file name);
    listed in the sync summary and release notes for a person to look at."""
    found = []
    for path in paths:
        if not path.endswith(".rs") or not path.startswith(("crates/ui-egui/src/", "apps/gridcraft/src/")):
            continue
        if matches(path, BRANDING_ONLY) or not os.path.exists(path):
            continue
        with open(path, encoding="utf-8") as f:
            for n, line in enumerate(f, 1):
                if VISIBLE_GRIDCRAFT.search(line) and not KEEP_UPSTREAM_NAME.search(line) and not line.lstrip().startswith("//"):
                    found.append(f"{path}:{n}")
    return found


def output(**kv):
    with open("sync-result.json", "w", encoding="utf-8") as f:
        json.dump(kv, f, indent=2)
    gh = os.environ.get("GITHUB_OUTPUT")
    if gh:
        with open(gh, "a", encoding="utf-8") as f:
            for k, v in kv.items():
                text = v if isinstance(v, str) else json.dumps(v)
                f.write(f"{k}={text.replace(chr(10), ' ')}\n")
    print(json.dumps(kv, indent=2))


def bump(version: str) -> str:
    parts = [int(p) if p.isdigit() else 0 for p in version.strip().split(".")]
    while len(parts) < 3:
        parts.append(0)
    parts[2] += 1
    return ".".join(map(str, parts[:3]))


def next_free_version(current: str) -> str:
    tags = set(git("tag", "--list", "v*").split())
    version = bump(current)
    while f"v{version}" in tags:
        version = bump(version)
    return version


def main() -> int:
    if "upstream" not in git("remote").split():
        git("remote", "add", "upstream", UPSTREAM_URL)
    main_sha = git("rev-parse", "HEAD")
    if git("status", "--porcelain", "--untracked-files=no"):
        output(status="blocked", main=main_sha, reason="the checkout of main has uncommitted changes")
        return 0
    git("fetch", "--no-tags", "upstream", UPSTREAM_BRANCH)
    head = git("rev-parse", f"upstream/{UPSTREAM_BRANCH}")
    mirrored = git("rev-parse", "--verify", "--quiet", "refs/remotes/origin/upstream", check=False)
    if mirrored and not is_ancestor(mirrored, head):
        output(
            status="blocked",
            upstream=head,
            main=main_sha,
            reason=f"official GridCraft rewrote its history: {mirrored[:10]}, mirrored last time, is no longer in upstream main. Nothing was merged or mirrored.",
        )
        return 0
    if is_ancestor(head, "HEAD"):
        output(status="up-to-date", upstream=head, main=main_sha)
        return 0
    base = subprocess.run([sys.executable, "packaging/dexterity/check_overlay.py"], capture_output=True, text=True, encoding="utf-8")
    if base.returncode != 0:
        output(status="blocked", upstream=head, main=main_sha, reason="main itself fails check_overlay.py; fix main first", unresolved=base.stdout.splitlines())
        return 0
    previous = git("merge-base", "HEAD", head)
    subject = git("log", "-1", "--format=%s", head)
    merge = subprocess.run(["git", "merge", "--no-ff", "--no-commit", f"upstream/{UPSTREAM_BRANCH}"], capture_output=True, text=True, encoding="utf-8")
    unresolved = []
    reapplied = []
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
                rebrand_file(path)
            elif code == "UU":
                # Upstream changed code we also touched (often a refactor that moved it): take
                # upstream's file; reapply_overlay.py puts our hooks and branding back below.
                git("checkout", "--theirs", "--", path)
                git("add", "--", path)
                reapplied.append(path)
            else:
                unresolved.append(f"{code} {path}")
    # Upstream may add new workflows; Dexterity runs only its own.
    for wf in git("diff", "--cached", "--name-only", "--diff-filter=A").splitlines():
        if matches(wf, [".github/workflows/*"]) and not matches(wf, [".github/workflows/dexterity-*", ".github/workflows/upstream-sync.yml"]):
            git("rm", "-q", "--cached", "--", wf, check=False)
            if os.path.exists(wf):
                os.remove(wf)
    if unresolved:
        output(status="conflict", upstream=head, main=main_sha, subject=subject, unresolved=unresolved)
        return 0
    if reapplied:
        again = subprocess.run([sys.executable, "packaging/dexterity/reapply_overlay.py"], capture_output=True, text=True, encoding="utf-8")
        if again.returncode != 0:
            output(status="conflict", upstream=head, main=main_sha, subject=subject, unresolved=reapplied + again.stdout.splitlines())
            return 0
        # Re-added hook lines must look like rustfmt wrote them, or CI's format check fails.
        fmt = subprocess.run(["cargo", "fmt", "--all"], capture_output=True, text=True, encoding="utf-8")
        if fmt.returncode != 0:
            output(status="conflict", upstream=head, main=main_sha, subject=subject, unresolved=reapplied + ["cargo fmt failed:", *fmt.stderr.splitlines()[:20]])
            return 0
        git("add", "-A")
    changed = git("diff", "--name-only", previous, head).splitlines()
    for path in changed:
        if matches(path, BRANDING_ONLY) and os.path.exists(path):
            rebrand_file(path)
    hooks = subprocess.run([sys.executable, "packaging/dexterity/check_overlay.py"], capture_output=True, text=True, encoding="utf-8")
    if hooks.returncode != 0:
        output(status="conflict", upstream=head, main=main_sha, subject=subject, unresolved=hooks.stdout.splitlines())
        return 0
    with open("packaging/dexterity/VERSION", encoding="utf-8") as f:
        version = next_free_version(f.read())
    with open("packaging/dexterity/VERSION", "w", encoding="utf-8", newline="") as f:
        f.write(version)
    git("add", "packaging/dexterity/VERSION")
    git("commit", "-q", "-m", f"Sync with upstream GridCraft {head[:10]}\n\nUpstream: {subject}\nDexterity Sheets {version}")
    output(
        status="merged",
        upstream=head,
        previous=previous,
        subject=subject,
        version=version,
        main=main_sha,
        candidate=git("rev-parse", "HEAD"),
        commits=len(git("rev-list", f"{previous}..{head}").split()),
        reapplied=reapplied,
        branding_review=branding_review(changed),
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
