"""Brings the README and its pictures up to date with a release. Run by the release workflow after
it publishes; safe to run by hand.

    python packaging/dexterity/update_docs.py [--media <dir>] [--notes <release notes .md>]

1. Pictures: every PNG in <dir> (fresh screenshots of the released build) replaces the one in
   docs/images only if it really looks different (see CHANGED_SHARE, or a different size), so
   a clock in a screenshot never produces a commit. <dir>/demo.webp replaces
   docs/readme/demo.webp when its frames differ.
2. README: fills the blocks between `<!-- dex:NAME -->` and `<!-- /dex:NAME -->` markers:
   - version: "1.2.3"
   - release: one line, version · date · GridCraft base
   - whatsnew: the release's GridCraft changes (from the notes), at most 6, plus a link
   Text outside the markers is never touched.
Prints what changed; exit code 0 either way. Needs Pillow for step 1.
"""
import datetime
import glob
import os
import re
import shutil
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
REPO = "https://github.com/" + os.environ.get("GITHUB_REPOSITORY", "vikramdex-ops/dexterity-sheets")
# A picture counts as changed when more than 0.08% of its pixels differ by more than 40 levels.
# Re-captures of an unchanged build differ only in clock text (about 0.02%); a new ribbon button
# or a moved panel is well above it.
PIXEL_DELTA, CHANGED_SHARE = 40, 0.0008


def arg(name):
    return sys.argv[sys.argv.index(name) + 1] if name in sys.argv else None


def looks_different(a, b):
    from PIL import Image, ImageChops

    with Image.open(a) as x, Image.open(b) as y:
        frames = (getattr(x, "n_frames", 1), getattr(y, "n_frames", 1))
        if x.size != y.size or frames[0] != frames[1]:
            return True
        for i in range(0, frames[0], max(1, frames[0] // 8)):
            x.seek(i)
            y.seek(i)
            hist = ImageChops.difference(x.convert("RGB"), y.convert("RGB")).convert("L").histogram()
            if sum(hist[PIXEL_DELTA:]) / max(1, sum(hist)) > CHANGED_SHARE:
                return True
    return False


def refresh_media(src):
    changed = []
    targets = [(p, os.path.join(ROOT, "docs", "images", os.path.basename(p))) for p in glob.glob(os.path.join(src, "*.png"))]
    demo = os.path.join(src, "demo.webp")
    if os.path.exists(demo):
        targets.append((demo, os.path.join(ROOT, "docs", "readme", "demo.webp")))
    for new, old in sorted(targets):
        if not os.path.exists(old) or looks_different(new, old):
            os.makedirs(os.path.dirname(old), exist_ok=True)
            shutil.copyfile(new, old)
            changed.append(os.path.relpath(old, ROOT))
    return changed


def fill(text, name, body):
    pattern = re.compile(rf"(<!-- dex:{name} -->)(.*?)(<!-- /dex:{name} -->)", re.S)
    return pattern.sub(lambda m: m.group(1) + body + m.group(3), text)


def gridcraft_version():
    with open(os.path.join(ROOT, "Cargo.toml"), encoding="utf-8") as f:
        m = re.search(r'\[workspace\.package\][^\[]*?version\s*=\s*"([^"]+)"', f.read(), re.S)
    return m.group(1) if m else "?"


def whatsnew(notes, version):
    items = []
    if notes and os.path.exists(notes):
        with open(notes, encoding="utf-8") as f:
            section = re.search(r"### New from official GridCraft[^\n]*\n(.*?)(?:\n\n|\Z)", f.read(), re.S)
        if section:
            items = [l for l in section.group(1).splitlines() if l.startswith("- ")]
    link = f"[Everything in {version}]({REPO}/releases/tag/v{version})"
    if not items:
        return f"\n{link}\n"
    more = f" and {len(items) - 6} more" if len(items) > 6 else ""
    return "\n" + "\n".join(items[:6]) + f"\n\n{link}{more}.\n"


def update_readme(notes):
    path = os.path.join(ROOT, "README.md")
    with open(os.path.join(ROOT, "packaging", "dexterity", "VERSION"), encoding="utf-8") as f:
        version = f.read().strip()
    with open(path, encoding="utf-8") as f:
        before = f.read()
    date = datetime.date.today().strftime("%d %B %Y").lstrip("0")
    text = fill(before, "version", version)
    text = fill(text, "release", f"Version <b>{version}</b> · {date} · based on GridCraft {gridcraft_version()}")
    text = fill(text, "whatsnew", whatsnew(notes, version))
    if text != before:
        with open(path, "w", encoding="utf-8", newline="\n") as f:
            f.write(text)
        return ["README.md"]
    return []


def main():
    sys.stdout.reconfigure(encoding="utf-8")
    changed = []
    if arg("--media"):
        changed += refresh_media(arg("--media"))
    changed += update_readme(arg("--notes"))
    print("\n".join(changed) if changed else "nothing to update")


if __name__ == "__main__":
    main()
