"""End-to-end crash recovery with the real desktop app.

Types into a new workbook, waits for the recovery copy, kills the process (as a crash or a power
cut would), starts the app again and checks that Recover Unsaved Work offers the workbook and
brings back what was typed; then checks that a normal exit leaves nothing to recover.

Run:  py -3 -I packaging\\dexterity\\tests\\recovery_e2e.py [path\\to\\target\\release]
"""
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time

REL = os.path.abspath(sys.argv[1] if len(sys.argv) > 1 else os.path.join("target", "release"))
APP = next((os.path.join(REL, n) for n in ("DexteritySheets.exe", "gridcraft.exe") if os.path.exists(os.path.join(REL, n))), None)
results = []


def check(name, ok, detail=""):
    results.append((name, bool(ok), detail))
    print(("PASS " if ok else "FAIL ") + name + (f"  [{detail}]" if detail else ""), flush=True)


class App:
    def __init__(self, port, env):
        token = "recovery-e2e-token-abcdef"
        self.proc = subprocess.Popen([APP, "--control", str(port)], env=dict(env, GRIDCRAFT_CONTROL_TOKEN=token), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        deadline = time.time() + 60
        self.sock = None
        while time.time() < deadline:
            try:
                self.sock = socket.create_connection(("127.0.0.1", port), timeout=30)
                break
            except OSError:
                time.sleep(0.5)
        if not self.sock:
            raise RuntimeError("control port never opened")
        self.f = self.sock.makefile("rw", encoding="utf-8", newline="\n")
        self.n = 0
        self.call("auth", {"token": token})

    def call(self, method, params=None):
        self.n += 1
        self.f.write(json.dumps({"id": self.n, "method": method, "params": params or {}}) + "\n")
        self.f.flush()
        r = json.loads(self.f.readline())
        if not r.get("ok"):
            raise RuntimeError(f"{method} {params}: {r.get('error')}")
        return r.get("result")

    def run(self, command, **params):
        return self.call("engine.execute", {"command": command, "params": params})


def main():
    if not APP:
        print("build first: app missing in", REL)
        sys.exit(2)
    rec = tempfile.mkdtemp(prefix="dexsheets-recovery-e2e-")
    # The apps are the real desktop build: their settings, templates and recovery copies must
    # land in fresh folders, never in the developer's own %LOCALAPPDATA% and %APPDATA%.
    local = os.path.join(rec, "LocalAppData")
    roaming = os.path.join(rec, "AppData")
    os.makedirs(local)
    os.makedirs(roaming)
    env = dict(os.environ, DEXSHEETS_RECOVERY_DIR=os.path.join(rec, "Recovery"), LOCALAPPDATA=local, APPDATA=roaming)
    try:
        a = App(7961, env)
        a.run("cell.set", cell="A1", input="typed before the crash 7Q")
        a.run("cell.set", cell="B2", input="=6*7")
        def copies():
            """The copies kept so far: {json path: cell count} (a copy follows the edits within ~10 s)."""
            out = {}
            for root, _, fs in os.walk(env["DEXSHEETS_RECOVERY_DIR"]):
                for f in fs:
                    if f.endswith(".json"):
                        try:
                            with open(os.path.join(root, f), encoding="utf-8") as h:
                                out[os.path.join(root, f)] = json.load(h).get("cells", 0)
                        except (OSError, ValueError):
                            pass
            return out

        def kept():
            return list(copies().values())
        end = time.time() + 40
        while time.time() < end and 2 not in kept():
            time.sleep(0.5)
        check("a recovery copy is kept, and follows the edits, while there are unsaved changes", 2 in kept(), str(kept()))
        a.proc.kill()
        a.proc.wait()

        b = App(7962, env)
        items = b.run("file.recoverUnsaved", action="list")["items"]
        check("the next start offers the unsaved workbook", len(items) == 1, json.dumps(items))
        old_copies = set(copies())
        b.run("file.recoverUnsaved", action="recover", index=0)
        docs = [d["title"] for d in b.call("ui.inspect")["documents"]]
        a1 = b.call("cell.get", {"cell": "A1"})
        b2 = b.call("cell.get", {"cell": "B2"})
        check("recover brings back what was typed", isinstance(a1, dict) and a1.get("value") == "typed before the crash 7Q" and b2.get("value") == 42, f"{docs} A1={a1 and a1.get('value')} B2={b2 and b2.get('value')}")
        check("the recovered workbook is new and unsaved (the file is not touched)", any("(Recovered)" in t for t in docs), str(docs))
        # The old copy may still be there, or this run's own copy may already have replaced it
        # (a copy is written within seconds of recovering); what must never be seen is no copy.
        now = set(copies())
        check("the crashed run's copy stays until this run has its own", bool(now) and (old_copies <= now or len(now - old_copies) == 1), str(copies()))
        items = b.run("file.recoverUnsaved", action="list")["items"]
        check("a copy already recovered is not offered again", items == [], json.dumps(items))
        end = time.time() + 40
        while time.time() < end and (old_copies & set(copies()) or len(copies()) != 1):
            time.sleep(0.5)
        check("once this run's copy of the recovered workbook lands, the old copy goes", not (old_copies & set(copies())) and len(copies()) == 1, str(copies()))
        try:
            b.run("review.compareFiles")
            check("Compare Files with nothing named is an error, not a window", False, "no error")
        except RuntimeError as e:
            check("Compare Files with nothing named is an error, not a window", "show: true" in str(e), str(e))
        # A normal exit after discarding: nothing to recover next time.
        b.run("file.close", force=True, document=True)
        b.call("app.quit", {})
        try:
            b.proc.wait(timeout=30)
        except subprocess.TimeoutExpired:
            b.proc.kill()
        c = App(7963, env)
        items = c.run("file.recoverUnsaved", action="list")["items"]
        check("a normal exit leaves nothing to recover", items == [], json.dumps(items))
        c.proc.kill()
    finally:
        shutil.rmtree(rec, ignore_errors=True)
    passed = sum(1 for _, ok, _ in results if ok)
    print(f"\n{passed}/{len(results)} checks passed")
    sys.exit(0 if passed == len(results) else 1)


if __name__ == "__main__":
    main()
