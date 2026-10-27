"""Keeps the README, the landing page and the release notes true to what is actually shipped
(RULES.md §3). Run in CI on every push and pull request.

Fails when:
  1. a user-visible change has no entry under "## Unreleased" in packaging/dexterity/CHANGES.md
     (unless a commit in the range says [no-user-change] with a reason; upstream syncs are
     described by the release notes automatically and are exempt);
  2. README.md, site/index.html or release_notes.py names a release file that the release does
     not produce (an old bundle, a renamed installer);
  3. an image or relative link in README.md or site/index.html points at nothing;
  4. text shown in the app or the Server Manager says "coming soon", "not implemented",
     "placeholder", "lorem ipsum" or "TBD".

Run:  python packaging/dexterity/docs_check.py [BASE_REF]
      BASE_REF (default: origin/main's merge base, or HEAD~1 on main) is where the range starts.
"""
import os
import re
import subprocess
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
problems = []

# What every release publishes (dexterity-release.yml checks the same list before publishing).
RELEASE_FILES = {
    "DexteritySheets-Setup.exe",
    "dexterity-sheets-<version>-windows-x64.msi",
    "dexterity-sheets-<version>-windows-x64-portable.zip",
    "DexSheetsServer-Setup.exe",
    "DexSheetsServer-<version>-x64.msi",
    "SHA256SUMS",
}
# Programs the installers put on a PC: fine to name, they are not release files.
PROGRAMS = {"DexSheetsServer.exe", "DexSheetsServerManager.exe", "DexteritySheets.exe"}
# Code whose change a user can notice.
USER_VISIBLE = re.compile(
    r"^(crates/ui-egui/src/|crates/engine/src/cmd/|crates/collab/src/(?!tests)|apps/(?!gridcraft-web/)[^/]+/src/|packaging/windows/[^/]+\.wxs$|packaging/windows/setup/main\.rs$)"
)
PLACEHOLDER = re.compile(r'"[^"\n]*\b(coming soon|not (yet )?implemented|placeholder|lorem ipsum|TBD)\b[^"\n]*"', re.I)


def git(*args):
    r = subprocess.run(["git", *args], cwd=ROOT, capture_output=True, text=True, encoding="utf-8", errors="replace")
    return r.stdout.strip() if r.returncode == 0 else ""


def read(rel):
    with open(os.path.join(ROOT, rel), encoding="utf-8") as f:
        return f.read()


def check_changelog(base):
    if not base:
        print("changelog: no base to compare with; skipped")
        return
    files = [f for f in git("diff", "--name-only", f"{base}...HEAD").splitlines() if f]
    visible = [f for f in files if USER_VISIBLE.match(f) and not f.endswith(("_test.rs", "/tests.rs"))]
    log = git("log", "--format=%B%x00", f"{base}..HEAD")
    if not visible:
        print("changelog: no user-visible change")
        return
    if "packaging/dexterity/CHANGES.md" in files:
        diff = git("diff", f"{base}...HEAD", "--", "packaging/dexterity/CHANGES.md")
        if any(l.startswith("+") and not l.startswith("+++") and l[1:].strip() for l in diff.splitlines()):
            print(f"changelog: {len(visible)} user-visible file(s), CHANGES.md updated")
            return
    if "[no-user-change]" in log:
        print("changelog: user-visible files changed, commit says [no-user-change]")
        return
    if re.search(r"^Sync with upstream GridCraft ", log, re.M):
        print("changelog: upstream sync (described in the release notes)")
        return
    problems.append(
        "packaging/dexterity/CHANGES.md: these user-visible files changed but nothing was added under '## Unreleased' "
        "(or say [no-user-change] and why in the commit message):\n    " + "\n    ".join(visible[:20])
    )


def normalise(name):
    return re.sub(r"\d+\.\d+\.\d+|\{version\}|\$v|\$\{?VERSION\}?|X\.Y\.Z", "<version>", name)


def check_release_files():
    pattern = re.compile(r"(DexteritySheets-Setup\.exe|DexSheetsServer[-\w.<>{}$]*\.(?:zip|msi|exe)|dexterity-sheets-[-\w.<>{}$]*\.(?:msi|zip))")
    for rel in ("README.md", "site/index.html", "packaging/dexterity/release_notes.py", "docs/collaboration.md"):
        for m in pattern.finditer(read(rel)):
            name = normalise(m.group(1))
            if name not in RELEASE_FILES and name not in PROGRAMS:
                problems.append(f"{rel}: names `{m.group(1)}`, which no release produces (it ships: {', '.join(sorted(RELEASE_FILES))})")


def check_links():
    readme = read("README.md")
    targets = re.findall(r'(?:src|href)="([^"#?]+)"', readme) + re.findall(r"\]\(([^)#?\s]+)\)", readme)
    for t in targets:
        if re.match(r"^(https?:|mailto:|\.\./\.\./)", t):
            continue
        if not os.path.exists(os.path.join(ROOT, t)):
            problems.append(f"README.md: `{t}` does not exist")
    site = read("site/index.html")
    # The pages workflow serves site/ with docs/images as images/ and docs/readme as readme/.
    served = {"images/": "docs/images/", "readme/": "docs/readme/", "icon.svg": "assets/app-icon/dexterity-sheets.svg", "icon-1024.png": "assets/app-icon/dexterity-sheets-1024.png"}
    for t in re.findall(r'(?:src|href)="([^"#?]+)"', site):
        if re.match(r"^(https?:|mailto:)", t) or "{{" in t:
            continue
        path = next((v + t[len(k):] if k.endswith("/") else v for k, v in served.items() if t.startswith(k)), "site/" + t)
        if not os.path.exists(os.path.join(ROOT, path)):
            problems.append(f"site/index.html: `{t}` does not exist (looked for {path})")


def check_placeholders():
    for top in ("crates/ui-egui/src", "apps/gridcraft-server-manager/src", "apps/gridcraft/src"):
        for d, _, files in os.walk(os.path.join(ROOT, top)):
            for f in files:
                if not f.endswith(".rs") or f in ("tests.rs",):
                    continue
                path = os.path.join(d, f)
                text = open(path, encoding="utf-8").read()
                # Unit tests at the end of a file are not shown to anyone.
                text = text.split("#[cfg(test)]")[0]
                for m in PLACEHOLDER.finditer(text):
                    line = text.count("\n", 0, m.start()) + 1
                    problems.append(f"{os.path.relpath(path, ROOT)}:{line}: shipped text {m.group(0)[:80]}")


def main():
    sys.stdout.reconfigure(encoding="utf-8")
    base = sys.argv[1] if len(sys.argv) > 1 else ""
    if not base:
        head_is_main = git("rev-parse", "HEAD") == git("rev-parse", "origin/main")
        base = git("rev-parse", "HEAD~1") if head_is_main else git("merge-base", "HEAD", "origin/main")
    check_changelog(base)
    check_release_files()
    check_links()
    check_placeholders()
    if problems:
        print("\nDocs are out of step with the product (RULES.md §3):")
        for p in problems:
            print("  - " + p)
        sys.exit(1)
    print("docs check: README, landing page, release notes and changelog are in step")


if __name__ == "__main__":
    main()
