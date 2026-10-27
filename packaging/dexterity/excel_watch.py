"""Watches Microsoft Excel's public release notes (Microsoft 365 Current Channel) and turns every
new Excel version into a tracked task (RULES.md §2.2).

    python packaging/dexterity/excel_watch.py [--dry-run] [--baseline]

Reads https://learn.microsoft.com/en-us/officeupdates/current-channel, finds versions not yet in
packaging/dexterity/excel-releases.json, and for each one with Excel items writes an issue body
(new features to implement, fixes to check) to excel-watch/<version>.md, noting which items the
parity audit (docs/dexterity-parity-audit.md) already mentions. The workflow opens the issues and
commits the updated state file. --baseline records every version on the page without reporting
any (first run). Only public documentation is read: no Microsoft code, assets or wording are
copied into the product (clean-room).
"""
import html
import json
import os
import re
import sys
import urllib.request

URL = "https://learn.microsoft.com/en-us/officeupdates/current-channel"
ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
STATE = os.path.join(ROOT, "packaging", "dexterity", "excel-releases.json")
AUDIT = os.path.join(ROOT, "docs", "dexterity-parity-audit.md")
OUT = os.path.join(ROOT, "excel-watch")
KINDS = {"feature updates": "feature", "resolved issues": "fix", "security updates": "security"}


def text(fragment):
    return html.unescape(re.sub(r"<[^>]+>", "", fragment)).replace("\xa0", " ").strip()


def parse(page):
    """[(id, title, [(kind, item)])] for every version, newest first: only Excel's items."""
    versions = []
    kind, app = None, None
    for m in re.finditer(r'<h2[^>]*id="([^"]+)"[^>]*>(.*?)</h2>|<h3[^>]*>(.*?)</h3>|<li>(.*?)</li>', page, re.S):
        if m.group(1):
            title = text(m.group(2))
            if title.lower().startswith("version "):
                versions.append((m.group(1), title, []))
            kind, app = None, None
        elif m.group(3) is not None:
            h = text(m.group(3)).lower()
            if h in KINDS:
                kind, app = KINDS[h], None
            else:
                app = h
        elif m.group(4) is not None and versions and app == "excel":
            versions[-1][2].append((kind or "fix", text(m.group(4))))
    return versions


def load_state():
    try:
        with open(STATE, encoding="utf-8") as f:
            return json.load(f)
    except (OSError, ValueError):
        return {"source": URL, "seen": []}


def tracked(item, audit):
    words = re.findall(r"[a-z0-9]+", item.lower())
    key = " ".join(words[:5])
    return bool(key) and key in " ".join(re.findall(r"[a-z0-9]+", audit.lower()))


def issue_body(title, items, audit):
    feats = [i for k, i in items if k == "feature"]
    fixes = [i for k, i in items if k != "feature"]
    lines = [
        f"Microsoft released **Excel {title}** (Microsoft 365 Current Channel). [Release notes]({URL}).",
        "",
        "Per RULES.md §2.2, every item below ends as **implemented** (link the PR), **already there** "
        "(with the evidence) or **not applicable** (with the reason), and `docs/dexterity-parity-audit.md` "
        "is updated. Clean-room: implement from the public description and black-box observation only.",
        "",
    ]
    if feats:
        lines += ["### New in Excel: implement in Dexterity Sheets", ""]
        lines += [f"- [ ] {i}" + ("  _(mentioned in the parity audit)_" if tracked(i, audit) else "") for i in feats]
        lines.append("")
    if fixes:
        lines += ["### Fixed in Excel: check Dexterity Sheets doesn't have the same problem", ""]
        lines += [f"- [ ] {i}" for i in fixes]
        lines.append("")
    return "\n".join(lines)


def main():
    sys.stdout.reconfigure(encoding="utf-8")
    dry = "--dry-run" in sys.argv
    baseline = "--baseline" in sys.argv
    req = urllib.request.Request(URL, headers={"User-Agent": "dexterity-sheets-excel-watch"})
    with urllib.request.urlopen(req, timeout=60) as r:
        page = r.read().decode("utf-8", "replace")
    versions = parse(page)
    if not versions:
        print("::error::no versions found: the release notes page changed its layout; update excel_watch.py")
        sys.exit(1)
    state = load_state()
    seen = set(state.get("seen", []))
    new = [v for v in versions if v[0] not in seen]
    audit = open(AUDIT, encoding="utf-8").read() if os.path.exists(AUDIT) else ""
    reported = 0
    if not baseline:
        os.makedirs(OUT, exist_ok=True)
        for vid, title, items in reversed(new):
            print(f"{title}: {len(items)} Excel item(s)")
            if not items:
                continue
            body = issue_body(title, items, audit)
            if dry:
                print(body)
            else:
                with open(os.path.join(OUT, f"{vid}.md"), "w", encoding="utf-8", newline="\n") as f:
                    f.write(f"Excel {title}\n{body}")
            reported += 1
    if not dry:
        state["seen"] = sorted(seen | {v[0] for v in versions})
        state["source"] = URL
        with open(STATE, "w", encoding="utf-8", newline="\n") as f:
            json.dump(state, f, indent=1)
            f.write("\n")
    print(f"{len(versions)} versions on the page, {len(new)} new, {reported} with Excel items to track")


if __name__ == "__main__":
    main()
