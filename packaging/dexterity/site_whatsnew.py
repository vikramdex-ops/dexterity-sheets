"""Fills the landing page's {{WHATSNEW}} with this version's items from CHANGES.md: the same list
as the release notes and the README, so the three never disagree.

    python packaging/dexterity/site_whatsnew.py _site/index.html <version>
"""
import html
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import changes  # noqa: E402

REPO = "https://github.com/" + os.environ.get("GITHUB_REPOSITORY", "vikramdex-ops/dexterity-sheets")


def to_html(item):
    text = html.escape(item[2:].strip(), quote=False)
    text = re.sub(r"\*\*(.+?)\*\*", r"<b>\1</b>", text)
    return re.sub(r"`(.+?)`", r"<code>\1</code>", text)


def main():
    page, version = sys.argv[1], sys.argv[2]
    with open(page, encoding="utf-8") as f:
        text = f.read()
    items = changes.items(version)
    lis = [f"<li>{to_html(i)}</li>" for i in items[:8]]
    lis.append(f'<li><a href="{REPO}/releases/tag/v{version}">Everything in {version}</a>, including what came from official GridCraft</li>')
    text = text.replace("{{WHATSNEW}}", "\n          ".join(lis))
    with open(page, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)
    print(f"{page}: {len(items)} item(s) for {version}")


if __name__ == "__main__":
    main()
