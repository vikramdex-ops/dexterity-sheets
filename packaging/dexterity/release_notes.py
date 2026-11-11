"""Writes the release notes of a Dexterity Sheets release, in words a non-specialist can follow.

    python packaging/dexterity/release_notes.py <version> <out.md> [--checks]

Facts only: which files to download, what changed for users (packaging/dexterity/CHANGES.md),
the GridCraft changes this release brings in (upstream pull requests merged since the previous
release, with links), and, when CHANGES.md has nothing, Dexterity's own commits. `--checks` adds the list of automatic checks the release passed; the release workflow
passes it only after they ran.
"""
import datetime
import os
import re
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import changes  # noqa: E402

UPSTREAM_REPO = "https://github.com/storytold/gridcraft"
REPO = "https://github.com/" + os.environ.get("GITHUB_REPOSITORY", "vikramdex-ops/dexterity-sheets")
HERE = "packaging/dexterity"
SKIP = re.compile(r"^(Sync with upstream GridCraft|Sync conflict:|Docs: refresh|Excel watch:|Merge )")


def git(*args):
    r = subprocess.run(["git", *args], capture_output=True, text=True, encoding="utf-8", errors="replace")
    return r.stdout.strip() if r.returncode == 0 else ""


def upstream_at(rev):
    """The official GridCraft commit contained in `rev`. The sync records it in GRIDCRAFT_COMMIT;
    for releases older than that file it is the upstream side of the newest sync merge. Nothing
    here needs a branch or a remote that mirrors upstream."""
    sha = git("show", f"{rev}:{HERE}/GRIDCRAFT_COMMIT")
    if re.fullmatch(r"[0-9a-f]{40}", sha):
        return sha
    parents = git("log", "-1", "--merges", "--grep=^Sync with upstream GridCraft", "--format=%P", rev).split()
    return parents[1] if len(parents) > 1 else ""


def previous_tag(version):
    for tag in git("tag", "--merged", "HEAD", "--list", "v*", "--sort=-v:refname").split():
        # Our own releases are 1.0.0 and later; `v0.*` tags in a clone are official GridCraft's.
        if tag != f"v{version}" and not tag.startswith("v0."):
            return tag
    return None


def gridcraft_version(rev=None):
    """The GridCraft release in this tree (GRIDCRAFT_RELEASE, written by the sync), else the
    workspace version in Cargo.toml."""
    text = git("show", f"{rev}:{HERE}/GRIDCRAFT_RELEASE") if rev else ""
    if not rev and os.path.exists(f"{HERE}/GRIDCRAFT_RELEASE"):
        with open(f"{HERE}/GRIDCRAFT_RELEASE", encoding="utf-8") as f:
            text = f.read().strip()
    if text:
        return text
    cargo = git("show", f"{rev}:Cargo.toml") if rev else open("Cargo.toml", encoding="utf-8").read()
    m = re.search(r'\[workspace\.package\][^\[]*?version\s*=\s*"([^"]+)"', cargo, re.S)
    return m.group(1) if m else "?"


def upstream_changes(old, new):
    """Upstream's own history between the GridCraft commit in the previous release and the one in
    this release, first-parent only: one line per merged pull request or direct commit."""
    if not new or new == old:
        return []
    rng = f"{old}..{new}" if old else f"{new}~20..{new}"
    out = []
    for rec in git("log", "--first-parent", "--format=%H%x1f%s%x1f%b%x1e", rng).split("\x1e"):
        parts = rec.strip().split("\x1f")
        if len(parts) < 2:
            continue
        sha, subject, body = parts[0], parts[1], parts[2] if len(parts) > 2 else ""
        m = re.match(r"Merge pull request #(\d+) from \S+", subject)
        squash = re.search(r"\(#(\d+)\)\s*$", subject)
        if m:
            title = next((l.strip() for l in body.splitlines() if l.strip()), subject)
            out.append(f"- {title} ([GridCraft #{m.group(1)}]({UPSTREAM_REPO}/pull/{m.group(1)}))")
        elif squash:
            title = re.sub(r"(\s*\(#\d+\))+$", "", subject)
            out.append(f"- {title} ([GridCraft #{squash.group(1)}]({UPSTREAM_REPO}/pull/{squash.group(1)}))")
        else:
            out.append(f"- {subject} ([{sha[:8]}]({UPSTREAM_REPO}/commit/{sha}))")
    return out


def own_changes(prev, upstream):
    """Dexterity's own commits since the previous release: everything that is not upstream's."""
    rng = f"{prev}..HEAD" if prev else "HEAD~30..HEAD"
    args = ["log", "--no-merges", "--format=%s", rng]
    if upstream:
        args.append(f"^{upstream}")
    return [f"- {s}" for s in git(*args).splitlines() if s and not SKIP.match(s)]


def main():
    sys.stdout.reconfigure(encoding="utf-8")
    version, out = sys.argv[1], sys.argv[2]
    prev = previous_tag(version)
    new_upstream = upstream_at("HEAD")
    old_upstream = upstream_at(prev) if prev else ""
    ups = upstream_changes(old_upstream, new_upstream)
    own = own_changes(prev, new_upstream)
    news = changes.items(version)
    # The same date as CHANGES.md's heading (the jobs can straddle midnight UTC).
    date = changes.date(version) or datetime.date.today().strftime("%d %B %Y").lstrip("0")
    gridcraft, was = gridcraft_version(), gridcraft_version(prev) if prev else ""
    moved = f" (the previous release was based on {was})" if was and was not in ("?", gridcraft) else ""
    lines = [
        f"**Dexterity Sheets {version}** · {date} · based on GridCraft {gridcraft}{moved}",
        "",
        "### Already using Dexterity Sheets?",
        f"Nothing to download. Within about 30 minutes the status bar shows **Update to {version}**: click it and the app updates itself and reopens.",
        "",
        "### New install",
        "| File | Who it's for |",
        "|---|---|",
        "| **DexteritySheets-Setup.exe** | Everyone: download, double-click, done. No administrator rights needed. |",
        f"| `dexterity-sheets-{version}-windows-x64.msi` | IT: the same installer for deployment tools (`msiexec /i … /qn`). |",
        f"| `dexterity-sheets-{version}-windows-x64-portable.zip` | Run without installing (USB stick, test PC). |",
        f"| `DexSheetsServer-Setup.exe` | The office server, for IT, once per office: installs the server as a Windows service and the Server Manager (people, bulk import, updates, backups) ([guide]({REPO}/blob/main/docs/collaboration.md)). |",
        f"| `DexSheetsServer-{version}-x64.msi` | The same server installer for deployment tools. |",
        "| `SHA256SUMS` | Checksums of every file above. |",
        "",
        "Windows may say *\"Windows protected your PC\"* because the installer is not code-signed yet: click **More info → Run anyway**.",
        "",
    ]
    if news:
        lines += ["### What's new in Dexterity Sheets", *news, ""]
    if ups:
        lines += [f"### New from official GridCraft ({len(ups)})", *ups[:60]]
        if len(ups) > 60:
            lines.append(f"- … and {len(ups) - 60} more")
        if old_upstream:
            lines += ["", f"Every GridCraft change in this release: [{old_upstream[:10]}…{new_upstream[:10]}]({UPSTREAM_REPO}/compare/{old_upstream}...{new_upstream})"]
        lines.append("")
    if own and not news:
        lines += ["### Dexterity Sheets changes", *own[:40], ""]
    if prev:
        lines += [f"All changes: [{prev}…v{version}]({REPO}/compare/{prev}...v{version})", ""]
    if "--checks" in sys.argv:
        lines += [
            "### Checked automatically before release",
            "- Dexterity's hooks into GridCraft intact; code formatting and lint clean",
            "- Every engine, formula, file-format and collaboration test",
            "- Three app windows editing one shared workbook live over the network (end to end)",
            "- DexteritySheets-Setup.exe and the .msi each installed, started, calculated a formula and uninstalled cleanly",
            "- DexSheetsServer-Setup.exe and its .msi each installed a running Windows service (firewall for the office network, data folder locked to administrators), opened in the Server Manager, and uninstalled keeping the data",
            "- The README, landing page and release notes checked against what this release ships",
            "",
        ]
    os.makedirs(os.path.dirname(os.path.abspath(out)), exist_ok=True)
    with open(out, "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(lines))
    print("\n".join(lines))


if __name__ == "__main__":
    main()
