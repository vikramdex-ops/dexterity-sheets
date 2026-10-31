"""packaging/dexterity/CHANGES.md: the one place a user-visible change is written down. The
release notes, the README's "What's new" and the landing page all read it from here.

    python packaging/dexterity/changes.py roll <version>   # Unreleased -> "## <version> · <date>"
    python packaging/dexterity/changes.py show <version>   # the items for that version
"""
import datetime
import os
import re
import sys

PATH = os.path.join(os.path.dirname(os.path.abspath(__file__)), "CHANGES.md")


def read():
    with open(PATH, encoding="utf-8") as f:
        return f.read()


def section(text, title_re):
    """The `- ` items under the first `## ` heading matching `title_re`."""
    m = re.search(r"^## " + title_re + r"[^\n]*\n(.*?)(?=^## |\Z)", text, re.M | re.S)
    if not m:
        return []
    return [l.rstrip() for l in m.group(1).splitlines() if l.startswith("- ")]


def items(version):
    """What changed in `version`: its own section once released, else Unreleased."""
    text = read()
    return section(text, re.escape(version) + r"\b") or section(text, "Unreleased")


def roll(version):
    """Moves the Unreleased items under a new heading for `version`. Idempotent."""
    text = read()
    if re.search(r"^## " + re.escape(version) + r"\b", text, re.M):
        return False
    unreleased = section(text, "Unreleased")
    if not unreleased:
        # An automatic upstream sync with nothing of Dexterity's own still gets a section, so
        # What's New after the update is not empty (the release notes list GridCraft's changes).
        unreleased = ["- Brings in the latest fixes and features of official GridCraft (listed in the release notes on GitHub). Nothing else changed in Dexterity Sheets."]
    date = datetime.date.today().strftime("%d %B %Y").lstrip("0")
    head = re.search(r"^## Unreleased[^\n]*\n(.*?)(?=^## |\Z)", text, re.M | re.S)
    new = "## Unreleased\n\n## " + f"{version} · {date}\n\n" + "\n".join(unreleased) + "\n\n"
    text = text[: head.start()] + new + text[head.end():]
    with open(PATH, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)
    return True


if __name__ == "__main__":
    sys.stdout.reconfigure(encoding="utf-8")
    cmd, version = sys.argv[1], sys.argv[2]
    if cmd == "roll":
        print("rolled" if roll(version) else "nothing to roll")
    else:
        print("\n".join(items(version)))
