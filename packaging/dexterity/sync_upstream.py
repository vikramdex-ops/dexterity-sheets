"""Merges the latest official GridCraft into Dexterity Sheets.

    python packaging/dexterity/sync_upstream.py              # merge upstream main into this checkout
    python packaging/dexterity/sync_upstream.py --continue   # finish after conflicts were fixed by hand
    python packaging/dexterity/sync_upstream.py --auto --park   # what the upstream-sync workflow runs

Run from the repository root on a clean checkout of `main`. Writes `sync-result.json` and GitHub
step outputs:
- status: up-to-date | merged | conflict | waiting | blocked (with a reason)
- upstream: the upstream commit merged; previous: the upstream commit merged last time
- main / candidate: main before the merge, and the commit to test and release
- version: the new Dexterity Sheets version; gridcraft: the GridCraft release it contains

Nothing here needs a person, an AI service or a secret for what a rule can decide. What a rule
cannot decide is never guessed: it is left with conflict markers for a person (status `conflict`).

Safety checks before anything is merged (a failure → status `blocked`):
- the checkout must be clean;
- upstream must not have rewritten history: the commit recorded in GRIDCRAFT_COMMIT (the one
  merged last time) must still be in upstream main, so a force-push upstream can never silently
  drop or replay work;
- `main` itself must pass check_overlay.py (never build on a broken base).

Conflict policy (see packaging/dexterity/UPSTREAM.md):
1. Files Dexterity Sheets owns: keep ours (one we removed stays removed).
2. Files Dexterity Sheets deleted (upstream CI, ArtCraft brand files) or upstream deleted: stay
   deleted.
3. Generated files (the key tip table): upstream's copy, then regenerated from the merged source.
4. Files where our only change is user-visible branding text (translations, About text…): take
   upstream's file and re-apply the branding rule.
5. Any other file both sides changed: merged hunk by hunk. Where the two sides touch the same
   lines upstream's lines are taken, every other Dexterity change in the file is kept, and
   reapply_overlay.py puts Dexterity's hooks back. Then every Dexterity line that sat in a
   conflicting hunk must be present again. If one is not, the file gets its conflict markers back
   and the sync stops: nothing of ours is ever dropped silently.
6. Anything else (a file we deleted that is on no list, a binary both sides changed): stop.

Afterwards check_overlay.py verifies every hook, Cargo.lock is completed, new user-visible
"GridCraft" text is listed for review, GRIDCRAFT_COMMIT / GRIDCRAFT_RELEASE record what was
merged, CHANGES.md says what came from GridCraft (so What's New, the README and the landing page
show it), and the version goes up: the minor number when GridCraft made a new minor release, the
patch number otherwise. The full CI and the installer smoke tests must pass before anything is
released.

When it stops with conflicts (`--park`): the half-merged tree is committed, markers included, for
the branch `sync/pending`. Fix the marked files there (in the browser, locally, or by an agent)
and push; the next run (`--auto`) finishes the merge from that branch instead of starting again.
"""
import argparse
import fnmatch
import json
import os
import re
import subprocess
import sys
import tempfile

UPSTREAM_URL = "https://github.com/storytold/gridcraft.git"
UPSTREAM_BRANCH = "main"
HERE = "packaging/dexterity"
RECORD = f"{HERE}/GRIDCRAFT_COMMIT"
RELEASE = f"{HERE}/GRIDCRAFT_RELEASE"
STATE = f"{HERE}/sync-state.json"
CHANGES = f"{HERE}/CHANGES.md"
PENDING = "sync/pending"
CANDIDATE = "sync/candidate"
PARKED = "Sync conflict:"
MERGED = "Sync with upstream GridCraft"
TAGS = "refs/gridcraft/tags"

OURS = [
    "crates/collab/*",
    "apps/gridcraft-server/*",
    "apps/gridcraft-server-manager/*",
    "crates/ui-egui/src/collab_ui.rs",
    "crates/ui-egui/src/dex_ui.rs",
    "crates/ui-egui/src/dex_ui/*",
    "crates/engine/src/cmd/dex.rs",
    "crates/engine/src/cmd/dex/*",
    "crates/xlsx/src/array_formula/dex.rs",
    "packaging/dexterity/*",
    "packaging/windows/setup/*",
    # Our installer has its own identity (name, UpgradeCode, per-user scope, registry keys);
    # upstream's install-scope policy (#2704) is written for GridCraft's.
    "packaging/windows/gridcraft.wxs",
    "packaging/windows/server.wxs",
    "packaging/windows/test-desktop-shortcut.ps1",
    "docs/collaboration.md",
    "docs/dexterity-*.md",
    "docs/readme/*",
    "site/*",
    ".github/workflows/dexterity-*",
    ".github/workflows/upstream-sync.yml",
    "README.md",
    "RULES.md",
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
# Files outside the UI and app sources whose user-visible "GridCraft" text is rebranded after
# every merge (reapply_overlay.py rebrands crates/ui-egui/src and apps/gridcraft/src itself).
# Being on this list never excuses another Dexterity change in the file: every file both sides
# changed goes through the same hunk-by-hunk merge and the same check for dropped lines. (The
# files here were once taken from upstream whole; that dropped the `Table[]` hook in write.rs.)
BRANDING_ONLY = [
    "crates/ui-egui/src/i18n/*.rs",
    "crates/ui-egui/src/credits.rs",
    "crates/ui-egui/src/formula_bar.rs",
    "crates/ui-egui/src/craft_fonts_policy.rs",
    "crates/ui-egui/tests/it/file_menu.rs",
    "apps/gridcraft/src/native_menu.rs",
    "apps/gridcraft/src/print.rs",
    "crates/xlsx/src/write.rs",
]
# Upstream's own documents (checked after OURS, so the README and Dexterity's documents are not
# among them): upstream's wording wins where both sides changed the same lines.
UPSTREAM_DOCS = ["*.md"]
# Generated files: upstream's copy is taken and the generator runs again on the merged source.
# file -> the generator's key in GENERATORS.
GENERATED = {
    "crates/ui-egui/src/keytips/keytip_table/generated.rs": "keytips",
    "crates/ui-egui/tests/it/keytips_generated.rs": "keytips",
}
GENERATORS = {
    # (what its input is, the command)
    "keytips": ("crates/ui-egui/src/keytips/keytips.json", ["cargo", "run", "--quiet", "--package", "xtask", "--", "keytips"]),
}
KEEP_UPSTREAM_NAME = re.compile(r"Based on GridCraft|GridCraft by the ArtCraft|by the ArtCraft team|gridcraft-cli|GRIDCRAFT_|storytold/gridcraft")
VISIBLE_GRIDCRAFT = re.compile(r'"[^"\n]*GridCraft[^"\n]*"')
# Upstream commit subjects that are not something a person using the Windows app notices; the
# release notes still list every change.
NOT_FOR_USERS = re.compile(r"^(merge\b|release \d|(ci|chore|docs?|tests?|build|refactor|contributors?|packaging|style|revert|bump|deps?)\b[^:]*:)", re.I)
OTHER_PLATFORMS = re.compile(r"\b(macOS|Macs?|Linux|FreeBSD|BSD|Flatpak|Nix|wasm|gridcraft-web|AppKit|Wayland|X11|Homebrew|AppImage|Landlock|seccomp)\b", re.I)


def git(*args, check=True):
    r = subprocess.run(["git", *args], capture_output=True, text=True, encoding="utf-8", errors="replace")
    if check and r.returncode != 0:
        raise SystemExit(f"git {' '.join(args)} failed:\n{r.stdout}\n{r.stderr}")
    return r.stdout.strip()


def run(cmd):
    return subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8", errors="replace")


def is_ancestor(a, b):
    return subprocess.run(["git", "merge-base", "--is-ancestor", a, b], capture_output=True).returncode == 0


def exists(ref):
    return bool(git("rev-parse", "--verify", "--quiet", ref, check=False))


def matches(path, patterns):
    return any(fnmatch.fnmatch(path, p) for p in patterns)


def read(path):
    with open(path, encoding="utf-8") as f:
        return f.read()


def write(path, text):
    with open(path, "w", encoding="utf-8", newline="") as f:
        f.write(text)


def conflicts():
    """Every unmerged path with its two-letter status (UU, AA, DU, UD, AU, UA, DD)."""
    out = []
    for line in git("status", "--porcelain", "-z").split("\0"):
        if line[:2] in ("UU", "AA", "DU", "UD", "AU", "UA", "DD"):
            out.append((line[:2], line[3:]))
    return out


def has_stage(path, stage):
    """Whether the index holds a version of `path` for `stage` (2 = ours, 3 = upstream)."""
    return any(l.split()[2] == str(stage) for l in git("ls-files", "-u", "-z", "--", path).split("\0") if l.strip())


def stage_bytes(path, stage):
    r = subprocess.run(["git", "show", f":{stage}:{path}"], capture_output=True)
    return r.stdout if r.returncode == 0 else None


def remove(path):
    try:
        os.remove(path)
    except OSError:
        pass


# ---- Hunk-level merge ---------------------------------------------------------------------------


def merge_file(path, *flags):
    """`git merge-file` over the three versions of `path` in the index. Returns the merged text
    (with `--theirs`: upstream's lines where both sides changed the same ones; with `--diff3`:
    conflict markers showing ours, the common ancestor and upstream's), or None for a file that
    is not text."""
    with tempfile.TemporaryDirectory() as d:
        files = []
        for stage, name in ((2, "ours"), (1, "base"), (3, "theirs")):
            data = stage_bytes(path, stage)
            if data is None:
                if stage != 1:
                    return None
                data = b""  # both sides added the file: no common ancestor
            if b"\0" in data:
                return None
            p = os.path.join(d, name)
            with open(p, "wb") as f:
                f.write(data)
            files.append(p)
        r = subprocess.run(
            ["git", "merge-file", "-p", *flags, "-L", "Dexterity Sheets", "-L", "before", "-L", "official GridCraft", *files],
            capture_output=True,
        )
        if r.returncode < 0 or r.returncode > 127:
            return None
        try:
            return r.stdout.decode("utf-8")
        except UnicodeDecodeError:
            return None


def conflict_hunks(text):
    """The conflicting regions of a `--diff3` merge: our lines, the ancestor's and upstream's."""
    hunks, cur, side = [], None, None
    for line in text.splitlines():
        if line.startswith("<<<<<<< "):
            cur, side = {"ours": [], "base": [], "theirs": []}, "ours"
        elif side == "ours" and line.startswith("||||||| "):
            side = "base"
        elif side in ("ours", "base") and line == "=======":
            side = "theirs"
        elif side == "theirs" and line.startswith(">>>>>>> "):
            hunks.append(cur)
            cur, side = None, None
        elif side:
            cur[side].append(line)
    return hunks


def norm(line):
    return " ".join(line.split())


def significant(line):
    """A line whose loss would matter: code or data, not a blank, a lone bracket or a comment."""
    s = line.strip()
    return bool(re.search(r"[A-Za-z0-9]", s)) and not s.startswith("//")


def rebranded(path):
    """Whether the sync re-applies the branding rule to `path` (so a line of ours that is only a
    rebranded upstream line comes back by itself, in upstream's newest wording)."""
    if matches(path, BRANDING_ONLY):
        return True
    ui = path.endswith(".rs") and path.startswith(("crates/ui-egui/src/", "apps/gridcraft/src/"))
    return ui and "/tests" not in path and "/i18n/" not in path and not path.endswith("tests.rs")


def at_risk(path, hunks):
    """Dexterity's part of the conflicting hunks: lines we added (which taking upstream's hunk
    drops) and lines we removed (which it brings back)."""
    added, removed = [], []
    for h in hunks:
        base = {norm(l) for l in h["base"]}
        ours = {norm(l) for l in h["ours"]}
        theirs = {norm(l) for l in h["theirs"]}
        branded = {norm(rebrand(l + "\n")) for l in h["base"]} if rebranded(path) else set()
        added += [norm(l) for l in h["ours"] if significant(l) and norm(l) not in base | theirs | branded]
        removed += [norm(l) for l in h["base"] if significant(l) and norm(l) not in ours and norm(l) in theirs]
    return sorted(set(added)), sorted(set(removed))


def count_lines(text):
    counts = {}
    for l in text.splitlines():
        counts[norm(l)] = counts.get(norm(l), 0) + 1
    return counts


def resolve_by_hunk(path):
    """Merges `path` taking upstream's lines only where both sides changed the same ones. Returns
    what must be checked afterwards, or None when the file cannot be merged as text."""
    marked = merge_file(path, "--diff3")
    taken = merge_file(path, "--theirs")
    ours = stage_bytes(path, 2)
    if marked is None or taken is None or ours is None:
        return None
    added, removed = at_risk(path, conflict_hunks(marked))
    had = count_lines(ours.decode("utf-8", errors="replace"))
    write(path, taken)
    git("add", "--", path)
    # How often each of those lines stood in our version: the merged file must have each added
    # line at least as often, and each removed line no more often.
    return {"marked": marked, "added": {l: had.get(l, 0) for l in added}, "removed": {l: had.get(l, 0) for l in removed}}


def hook_markers(path):
    """The markers check_overlay.py requires in `path`: the essence of each Dexterity hook there."""
    sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
    import check_overlay  # noqa: E402

    out = []
    for hooked, marker, _what in check_overlay.HOOKS:
        m = re.search(r"\{([^}]*)\}", hooked)
        options = [hooked[: m.start()] + o + hooked[m.end() :] for o in m.group(1).split(",")] if m else [hooked]
        if marker and matches(path, options):
            out.append(norm(marker))
    return out


def lost_lines(path, check):
    """Dexterity lines from the conflicting hunks that are not in the file now, and lines
    Dexterity had removed that are in it again. A line that carried a hook is not lost when the
    hook's marker is in the file: reapply_overlay.py put it back in the shape upstream's new code
    needs, which need not be the old line."""
    try:
        text = read(path)
    except (OSError, UnicodeDecodeError):
        return ["(the file cannot be read)"]
    final = count_lines(text)
    flat = norm(text)
    placed = [m for m in hook_markers(path) if m in flat]
    gone = [f"missing: {l}" for l, n in check["added"].items() if final.get(l, 0) < n and not any(m in l for m in placed)]
    back = [f"back again: {l}" for l, n in check["removed"].items() if final.get(l, 0) > n]
    return gone + back


def marked_files():
    """Tracked files that still hold conflict markers."""
    r = run(["git", "grep", "-l", "-I", "-E", "^(<<<<<<< |>>>>>>> )"])
    return [l for l in r.stdout.splitlines() if l.strip()]


def generators_needed():
    """Generators whose source or output the merge in the index touches."""
    touched = set(git("diff", "--cached", "--name-only", "HEAD", check=False).splitlines())
    return sorted({k for k, (src, _cmd) in GENERATORS.items() if src in touched} | {k for f, k in GENERATED.items() if f in touched})


# ---- Branding -----------------------------------------------------------------------------------


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
    text = read(path)
    if rebrand(text) != text:
        write(path, rebrand(text))
    git("add", "--", path)


def branding_review(paths):
    """User-visible "GridCraft" string literals in upstream-changed UI files outside the
    branding-only list. Not changed automatically (it could be an identifier or a file name);
    listed in the sync summary for a person to look at."""
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


# ---- What was merged ----------------------------------------------------------------------------


def recorded_upstream(rev="HEAD"):
    """The upstream commit `rev` was last synced with: GRIDCRAFT_COMMIT, or for trees older than
    that file, where our history and upstream's meet."""
    r = run(["git", "show", f"{rev}:{RECORD}"])
    sha = r.stdout.strip() if r.returncode == 0 else ""
    return sha if re.fullmatch(r"[0-9a-f]{40}", sha) else None


def gridcraft_release(commit, head):
    """The newest official GridCraft release contained in `commit`: a release counts once the
    point of upstream main it was cut from is in `commit`. None when no release tag is known."""
    best = None
    for ref in git("for-each-ref", "--format=%(refname)", TAGS).splitlines():
        m = re.fullmatch(re.escape(TAGS) + r"/v(\d+)\.(\d+)\.(\d+)", ref)
        if not m:
            continue
        cut = git("merge-base", ref, head, check=False)
        if cut and is_ancestor(cut, commit):
            best = max(best, tuple(map(int, m.groups()))) if best else tuple(map(int, m.groups()))
    return ".".join(map(str, best)) if best else None


def cargo_version(rev):
    m = re.search(r'\[workspace\.package\][^\[]*?version\s*=\s*"([^"]+)"', git("show", f"{rev}:Cargo.toml", check=False), re.S)
    return m.group(1) if m else None


def upstream_subjects(previous, head):
    """One line per change on upstream main between the two commits, newest first: the title of
    each merged pull request, or the commit's own subject."""
    out = []
    for rec in git("log", "--first-parent", "--format=%s%x1f%b%x1e", f"{previous}..{head}").split("\x1e"):
        subject, _, body = rec.strip().partition("\x1f")
        if not subject:
            continue
        if re.match(r"Merge pull request #\d+ from \S+", subject):
            subject = next((l.strip() for l in body.splitlines() if l.strip()), subject)
        subject = re.sub(r"(\s*\(#\d+\))+$", "", subject).strip()
        if subject and subject not in out:
            out.append(subject)
    return out


def note_changes(previous, head, old_release, new_release):
    """Says in CHANGES.md (Unreleased) what this sync brings from official GridCraft, so the app's
    What's New, the README and the landing page show it. Titles are upstream's own words."""
    subjects = upstream_subjects(previous, head)
    shown = [s for s in subjects if not NOT_FOR_USERS.match(s) and not OTHER_PLATFORMS.search(s)][:8]
    count = len(subjects)
    if new_release and new_release != old_release:
        was = f" (was {old_release})" if old_release else ""
        lines = [f"- **Now built on GridCraft {new_release}**{was}: {count} changes from official GridCraft are in this version. The newest are below; the release notes on GitHub list them all."]
    else:
        on = f" (still GridCraft {new_release})" if new_release else ""
        lines = [f"- {count} more fixes and improvements from official GridCraft{on}. The newest are below; the release notes on GitHub list them all."]
    lines += [f"- From GridCraft: {s}" for s in shown]
    text = read(CHANGES)
    m = re.search(r"^## Unreleased[^\n]*\n(.*?)(?=^## |\Z)", text, re.M | re.S)
    if not m:
        return
    body = m.group(1).rstrip("\n")
    new = [l for l in lines if l not in body]
    if not new:
        return
    body = (body + "\n" if body.strip() else "\n") + "\n".join(new) + "\n\n"
    write(CHANGES, text[: m.start(1)] + body + text[m.end(1):])
    git("add", "--", CHANGES)


# ---- Result and version -------------------------------------------------------------------------


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
    return 0


def bump(version: str, part: str) -> str:
    parts = [int(p) if p.isdigit() else 0 for p in version.strip().split(".")]
    while len(parts) < 3:
        parts.append(0)
    if part == "minor":
        parts[1], parts[2] = parts[1] + 1, 0
    else:
        parts[2] += 1
    return ".".join(map(str, parts[:3]))


def next_free_version(current: str, part: str) -> str:
    tags = set(git("tag", "--list", "v*").split())
    version = bump(current, part)
    while f"v{version}" in tags:
        version = bump(version, "patch")
    return version


def minor_of(release):
    return tuple(release.split(".")[:2]) if release else None


# ---- The merge ----------------------------------------------------------------------------------


def fetch_upstream():
    if "upstream" not in git("remote").split():
        git("remote", "add", "upstream", UPSTREAM_URL)
    git("fetch", "--no-tags", "upstream", UPSTREAM_BRANCH)
    # Upstream's release tags, kept apart from our own `v*` tags (never pushed).
    git("fetch", "--no-tags", "upstream", f"+refs/tags/v*:{TAGS}/v*", check=False)
    return git("rev-parse", f"upstream/{UPSTREAM_BRANCH}")


def stop(state, unresolved, park, details=None):
    """Conflicts a rule cannot settle. With `park`, the tree is committed as it is (markers
    included) so it can be pushed to `sync/pending` for a person."""
    state = dict(state, unresolved=unresolved, details=details or {})
    result = dict(status="conflict", upstream=state["upstream"], main=state["main"], subject=state.get("subject", ""), unresolved=unresolved, details=details or {})
    if park:
        saved = {k: v for k, v in state.items() if k != "checks"}
        write(STATE, json.dumps(saved, indent=2) + "\n")
        git("add", "-A")
        files = "\n".join(f"- {u}" for u in unresolved)
        git("commit", "-q", "--no-verify", "-m", f"{PARKED} GridCraft {state['upstream'][:10]} needs {len(unresolved)} file(s) settled\n\n{files}\n\nFix the files that hold conflict markers on this branch and push: the upstream sync finishes the merge.")
        result["parked"] = git("rev-parse", "HEAD")
    return output(**result)


def finish(state, park, unresolved=()):
    """Everything after git's own merge: hooks, the proof that nothing of ours was dropped,
    generated files, format, lock, branding, checks, the record of what was merged, the
    changelog, the version and the commit."""
    head, previous, main_sha, subject = state["upstream"], state["previous"], state["main"], state.get("subject", "")
    checks = state.get("checks", {})
    unresolved = list(unresolved)
    again = run([sys.executable, f"{HERE}/reapply_overlay.py"])
    if again.returncode != 0:
        unresolved += [l for l in again.stdout.splitlines() if l.startswith("REAPPLY:")] or again.stdout.splitlines()[-5:]
    # Nothing of ours may be dropped silently: a Dexterity line from a conflicting hunk that the
    # overlay did not put back sends the file to a person, with both sides shown. This runs before
    # any stop, so a parked merge never holds a quietly shortened file.
    lost = {p: lost_lines(p, c) for p, c in checks.items()}
    lost = {p: l for p, l in lost.items() if l}
    for p in lost:
        write(p, checks[p]["marked"])
    unresolved += [f"UU {p} (Dexterity lines in a conflicting hunk: {len(l)})" for p, l in lost.items()]
    if unresolved:
        return stop(state, unresolved, park, lost)
    flags = os.environ.get("DEXSHEETS_CARGO_FLAGS", "").split()
    for key in state.get("regenerate", []):
        cmd = GENERATORS[key][1]
        cmd = cmd[: cmd.index("--")] + flags + cmd[cmd.index("--") :]
        gen = run(cmd)
        if gen.returncode != 0:
            return stop(state, [f"{' '.join(cmd)} failed:", *gen.stderr.splitlines()[-20:]], park)
    # Re-added hook lines must look like rustfmt wrote them, or CI's format check fails.
    fmt = run(["cargo", "fmt", "--all"])
    if fmt.returncode != 0:
        return stop(state, ["cargo fmt failed:", *fmt.stderr.splitlines()[:20]], park)
    git("add", "-A")
    if state.get("relock") or state.get("resumed") or git("diff", "--name-only", previous, head, "--", "Cargo.lock", "Cargo.toml", "*/Cargo.toml", "*/*/Cargo.toml"):
        # After the overlay (it adds dependencies of ours to upstream manifests): upstream's
        # Cargo.lock has no entries for our crates' own dependencies, and a dependency upstream
        # bumped must land in the lock too; the CI build runs --locked. `--workspace` touches
        # only what the manifests require.
        lock = run(["cargo", "update", "--workspace", "--quiet"])
        if lock.returncode != 0:
            return stop(state, ["Cargo.lock could not be brought up to date:", *lock.stderr.splitlines()[:20]], park)
        git("add", "--", "Cargo.lock")
    upstream_changed = git("diff", "--name-only", previous, head).splitlines()
    for path in upstream_changed:
        if matches(path, BRANDING_ONLY) and os.path.exists(path):
            rebrand_file(path)
    left = marked_files()
    if left:
        return stop(state, [f"UU {p} (conflict markers)" for p in left], park)
    hooks = run([sys.executable, f"{HERE}/check_overlay.py"])
    if hooks.returncode != 0:
        return stop(state, [l for l in hooks.stdout.splitlines() if l.startswith("OVERLAY:")], park)
    old_release = gridcraft_release(previous, head) or cargo_version(previous)
    new_release = gridcraft_release(head, head) or cargo_version(head)
    version = read(f"{HERE}/VERSION").strip()
    if not os.path.exists(RECORD) or read(RECORD).strip() != head:
        write(RECORD, head + "\n")
        write(RELEASE, (new_release or "") + "\n")
        note_changes(previous, head, old_release, new_release)
        version = next_free_version(version, "minor" if minor_of(new_release) != minor_of(old_release) else "patch")
        write(f"{HERE}/VERSION", version)
    remove(STATE)
    git("add", "-A")
    release = f"GridCraft {new_release}" if new_release else "GridCraft"
    git("commit", "-q", "--allow-empty", "-m", f"{MERGED} {head[:10]}\n\nUpstream: {subject}\n{release}\nDexterity Sheets {version}")
    return output(
        status="merged",
        upstream=head,
        previous=previous,
        subject=subject,
        version=version,
        gridcraft=new_release or "",
        main=main_sha,
        candidate=git("rev-parse", "HEAD"),
        commits=len(git("rev-list", f"{previous}..{head}").split()),
        reapplied=state.get("reapplied", []),
        branding_review=branding_review(upstream_changed),
    )


def fresh(park):
    """Merges upstream main into the checked-out main."""
    main_sha = git("rev-parse", "HEAD")
    if git("status", "--porcelain", "--untracked-files=no"):
        return output(status="blocked", main=main_sha, reason="the checkout of main has uncommitted changes")
    head = fetch_upstream()
    last = recorded_upstream()
    if last and not is_ancestor(last, head):
        return output(
            status="blocked",
            upstream=head,
            main=main_sha,
            reason=f"official GridCraft rewrote its history: {last[:10]}, merged last time, is no longer in upstream main. Nothing was merged.",
        )
    if is_ancestor(head, "HEAD"):
        return output(status="up-to-date", upstream=head, main=main_sha)
    base = run([sys.executable, f"{HERE}/check_overlay.py"])
    if base.returncode != 0:
        return output(status="blocked", upstream=head, main=main_sha, reason="main itself fails check_overlay.py; fix main first", unresolved=base.stdout.splitlines())
    previous = git("merge-base", "HEAD", head)
    state = {"upstream": head, "previous": previous, "main": main_sha, "subject": git("log", "-1", "--format=%s", head), "reapplied": [], "regenerate": [], "checks": {}}
    merge = run(["git", "merge", "--no-ff", "--no-commit", f"upstream/{UPSTREAM_BRANCH}"])
    unresolved = []
    if merge.returncode != 0:
        for code, path in conflicts():
            ours, theirs = has_stage(path, 2), has_stage(path, 3)
            if matches(path, OURS):
                # Our file: ours wins, and a file we removed or renamed (the old icon names)
                # stays removed even when upstream keeps editing its copy.
                if ours:
                    git("checkout", "--ours", "--", path)
                    git("add", "--", path)
                else:
                    git("rm", "-q", "--cached", "--", path, check=False)
                    remove(path)
            elif matches(path, DELETED_BY_US) or not (ours or theirs):
                # We dropped it on purpose, or both sides deleted it: it stays deleted.
                git("rm", "-q", "--cached", "--", path, check=False)
                remove(path)
            elif not theirs:
                # Upstream deleted (or moved) a file Dexterity had changed: git only reports this
                # when our copy differs from the common ancestor. Deleting it would drop those
                # changes without a word (the locale's built-in formats went this way when
                # upstream moved numfmt's table to xlsx), so a person gives them a new home and
                # removes the file. `--continue` waits until it is gone.
                git("add", "--", path)
                unresolved.append(f"{code} {path} (changed here; removed or moved upstream. Move Dexterity's changes to where the code now lives, then delete this file)")
                state.setdefault("removed_upstream", []).append(path)
            elif not ours:
                # Upstream changed a file we deleted and that is not on any list: a person decides.
                unresolved.append(f"{code} {path} (deleted here, changed upstream)")
            elif path in GENERATED:
                git("checkout", "--theirs", "--", path)
                git("add", "--", path)
                state["regenerate"].append(GENERATED[path])
            elif matches(path, UPSTREAM_DOCS):
                # Upstream's own documents: its wording where both sides changed the same lines.
                # Dexterity's notes belong in docs/dexterity-*.md, which upstream never touches.
                taken = merge_file(path, "--theirs")
                if taken is None:
                    unresolved.append(f"{code} {path} (both changed; not a text file)")
                else:
                    write(path, taken)
                    git("add", "--", path)
                    state["reapplied"].append(path)
            elif path == "Cargo.lock":
                # Upstream's lock, then the entries our own crates need are added back in finish().
                git("checkout", "--theirs", "--", path)
                git("add", "--", path)
                state["relock"] = True
            else:
                check = resolve_by_hunk(path)
                if check is None:
                    unresolved.append(f"{code} {path} (both changed; not a text file)")
                else:
                    state["checks"][path] = check
                    state["reapplied"].append(path)
    # Upstream may add new workflows; Dexterity runs only its own.
    for wf in git("diff", "--cached", "--name-only", "--diff-filter=A").splitlines():
        if matches(wf, [".github/workflows/*"]) and not matches(wf, [".github/workflows/dexterity-*", ".github/workflows/upstream-sync.yml"]):
            git("rm", "-q", "--cached", "--", wf, check=False)
            remove(wf)
    state["regenerate"] = sorted(set(state["regenerate"]) | set(generators_needed()))
    return finish(state, park, unresolved)


def resume(park):
    """Finishes a merge whose conflicts a person (or an agent) settled: on a `sync/pending`
    checkout, or in a checkout where the merge is still open."""
    open_merge = exists("MERGE_HEAD")
    if os.path.exists(STATE):
        state = json.loads(read(STATE))
    elif open_merge:
        head = git("rev-parse", "MERGE_HEAD")
        state = {"upstream": head, "previous": git("merge-base", "HEAD", head), "main": git("rev-parse", "HEAD"), "subject": git("log", "-1", "--format=%s", head)}
    else:
        # A finished candidate that failed its tests and was fixed on top.
        head = recorded_upstream()
        if not head:
            return output(status="blocked", reason=f"nothing to continue: no {STATE}, no open merge and no {RECORD}")
        state = {"upstream": head, "previous": head, "main": git("rev-parse", "HEAD"), "subject": git("log", "-1", "--format=%s", head)}
    state["resumed"] = True
    state.pop("unresolved", None)
    state.pop("details", None)
    left = [p for _code, p in conflicts()] + marked_files() + [p for p in state.get("removed_upstream", []) if os.path.exists(p)]
    if left:
        return output(
            status="waiting",
            upstream=state["upstream"],
            main=state["main"],
            unresolved=sorted(set(left)),
            reason="these files still hold conflict markers, or were removed upstream and still exist here",
        )
    if open_merge:
        git("add", "-A")
        state["regenerate"] = sorted(set(state.get("regenerate", [])) | set(generators_needed()))
    elif exists("refs/remotes/origin/main"):
        # main may have moved since the merge was parked (the daily Excel watch, a person's push):
        # bring it in, so the result can still fast-forward main.
        main_sha = git("rev-parse", "refs/remotes/origin/main")
        if not is_ancestor(main_sha, "HEAD"):
            merged = run(["git", "merge", "--no-edit", "refs/remotes/origin/main"])
            if merged.returncode != 0:
                git("merge", "--abort", check=False)
                return output(status="conflict", upstream=state["upstream"], main=main_sha, unresolved=["main changed since the merge was parked and no longer merges with it"], reason=merged.stdout[-400:])
        state["main"] = main_sha
    if not git("for-each-ref", "--count=1", TAGS):
        fetch_upstream()
    # Never parked again from here: a commit of ours on top would hide the person's work from
    # `--auto`. What still stops it is reported, and the branch stays as they left it.
    return finish(state, False)


def auto(park):
    """What the workflow runs: finish a parked merge someone has worked on, else merge afresh."""
    listed = git("ls-remote", "--heads", "origin", PENDING, check=False)
    if listed:
        git("fetch", "--no-tags", "origin", f"+refs/heads/{PENDING}:refs/remotes/origin/{PENDING}", "+refs/heads/main:refs/remotes/origin/main")
        pending = git("rev-parse", f"refs/remotes/origin/{PENDING}")
        if not is_ancestor(pending, "refs/remotes/origin/main"):
            subject = git("log", "-1", "--format=%s", pending)
            parked_state = run(["git", "show", f"{pending}:{STATE}"])
            if subject.startswith(PARKED):
                # Nobody has touched the parked merge. Start again when upstream has moved on (the
                # conflict may be gone); otherwise keep waiting without making a new commit.
                head = fetch_upstream()
                upstream = json.loads(parked_state.stdout).get("upstream") if parked_state.returncode == 0 else None
                if upstream == head and is_ancestor(git("rev-parse", "HEAD"), pending):
                    unresolved = json.loads(parked_state.stdout).get("unresolved", [])
                    return output(status="waiting", upstream=head, main=git("rev-parse", "HEAD"), unresolved=unresolved, reason=f"the merge parked on {PENDING} is still waiting for its conflicts to be settled")
                return fresh(park)
            candidate = git("rev-parse", "--verify", "--quiet", f"refs/remotes/origin/{CANDIDATE}", check=False)
            if candidate and is_ancestor(pending, candidate):
                return output(status="waiting", upstream=recorded_upstream(pending) or "", main=git("rev-parse", "HEAD"), unresolved=[], reason=f"{PENDING} was already tried as a candidate and failed its checks; push a fix to it")
            git("checkout", "-q", "--detach", pending)
            return resume(park)
    return fresh(park)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--continue", dest="resume", action="store_true", help="finish a merge whose conflicts were settled by hand")
    ap.add_argument("--auto", action="store_true", help=f"finish a merge waiting on {PENDING} when someone has worked on it, else merge afresh")
    ap.add_argument("--park", action="store_true", help=f"on conflicts, commit the half-merged tree (markers included) for {PENDING}")
    args = ap.parse_args()
    if args.resume:
        return resume(args.park)
    if args.auto:
        return auto(args.park)
    return fresh(args.park)


if __name__ == "__main__":
    sys.exit(main())
