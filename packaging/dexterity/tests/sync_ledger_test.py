"""Regression test for the sync's whole-tree ledger (sync_upstream.overlay_losses).

A Dexterity line in a file official GridCraft owns must be reported when a merged tree no longer
has it, even where git merged the file cleanly; a line listed in overlay-retired.txt, or one whose
hook marker is in the file, is not.

Run: python packaging/dexterity/tests/sync_ledger_test.py
"""
import os
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, HERE)
import sync_upstream as s  # noqa: E402


def sh(*args, cwd):
    return subprocess.run(args, cwd=cwd, check=True, capture_output=True, text=True, encoding="utf-8").stdout.strip()


def put(root, path, text):
    full = os.path.join(root, path)
    os.makedirs(os.path.dirname(full), exist_ok=True)
    with open(full, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)


def main() -> int:
    with tempfile.TemporaryDirectory(prefix="ledger-") as root:
        sh("git", "init", "-q", cwd=root)
        sh("git", "config", "user.email", "test@example.invalid", cwd=root)
        sh("git", "config", "user.name", "test", cwd=root)
        put(root, "crates/core/src/parse.rs", "fn parse() {\n    upstream_step();\n}\n")
        sh("git", "add", "-A", cwd=root)
        sh("git", "commit", "-qm", "base", cwd=root)
        base = sh("git", "rev-parse", "HEAD", cwd=root)
        put(root, "crates/core/src/parse.rs", "fn parse() {\n    upstream_step();\n    dex_regional_dates(&mut s);\n}\n")
        sh("git", "commit", "-qam", "ours", cwd=root)
        ours = sh("git", "rev-parse", "HEAD", cwd=root)

        old = os.getcwd()
        os.chdir(root)
        failures = []
        try:
            if "crates/core/src/parse.rs" not in s.overlay_ledger(base, ours):
                failures.append("the ledger does not list a Dexterity line in an upstream file")
            if s.overlay_losses(base, ours):
                failures.append("a tree that kept every line reports a loss")
            # A merge that rewrote the function and dropped our line.
            put(root, "crates/core/src/parse.rs", "fn parse() {\n    upstream_step_v2();\n}\n")
            lost = s.overlay_losses(base, ours)
            if "crates/core/src/parse.rs" not in lost:
                failures.append("a dropped Dexterity line is not reported")
            # A person retired it on purpose.
            put(root, s.RETIRED, "# test\ncrates/core/src/parse.rs: dex_regional_dates(&mut s);\n")
            if s.overlay_losses(base, ours):
                failures.append("a retired line is still reported")
        finally:
            os.chdir(old)
    for f in failures:
        print("FAIL:", f)
    print("ledger test:", "FAILED" if failures else "ok")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
