"""End-to-end LAN co-authoring test with real Dexterity Sheets windows.

Starts a DexSheetsServer with a throw-away data folder, three desktop app instances (each with
its own control port), signs three different people in, and drives real editing through the
control channel: live propagation, conflicting edits, formatting, row inserts, undo, who-edited
attribution, a server crash with offline edits, and persistence after restart.

Run:  py -3 -I D:\\GridCraft\\tests\\lan_e2e.py [path\\to\\target\\release]
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
SERVER = os.path.join(REL, "DexSheetsServer.exe")
PORT = 47900
OUT = os.environ.get("E2E_OUT", os.path.join(os.getcwd(), "tests", "out"))
results = []


def check(name, ok, detail=""):
    results.append((name, bool(ok), detail))
    print(("PASS " if ok else "FAIL ") + name + (f"  [{detail}]" if detail else ""), flush=True)


class App:
    def __init__(self, port, token, who):
        self.port, self.token, self.who = port, token, who
        env = dict(os.environ, GRIDCRAFT_CONTROL_TOKEN=token)
        self.proc = subprocess.Popen([APP, "--control", str(port)], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        self.sock = None
        self.n = 0
        deadline = time.time() + 60
        while time.time() < deadline:
            try:
                self.sock = socket.create_connection(("127.0.0.1", port), timeout=30)
                break
            except OSError:
                time.sleep(0.5)
        if not self.sock:
            raise RuntimeError(f"{who}: control port {port} never opened")
        self.f = self.sock.makefile("rw", encoding="utf-8", newline="\n")
        self.call("auth", {"token": token}, raw=True)

    def call(self, method, params=None, raw=False):
        self.n += 1
        self.f.write(json.dumps({"id": self.n, "method": method, "params": params or {}}) + "\n")
        self.f.flush()
        line = self.f.readline()
        r = json.loads(line)
        if not r.get("ok"):
            raise RuntimeError(f"{self.who} {method} {params}: {r.get('error')}")
        return r.get("result")

    def run(self, command, **params):
        return self.call("engine.execute", {"command": command, "params": params})

    def cell(self, a1):
        r = self.call("cell.get", {"cell": a1})
        return r

    def value(self, a1):
        r = self.cell(a1)
        return r.get("value") if isinstance(r, dict) else r

    def status(self):
        return self.run("collab.status")

    def shot(self, path):
        for _ in range(10):
            try:
                return self.call("ui.screenshot", {"path": path})
            except RuntimeError:
                time.sleep(0.5)
        return None

    def close(self):
        try:
            self.proc.kill()
        except Exception:
            pass


def wait(pred, timeout=15, step=0.05):
    end = time.time() + timeout
    last = None
    while time.time() < end:
        try:
            last = pred()
            if last:
                return last
        except Exception as e:  # keep polling through transient states
            last = e
        time.sleep(step)
    return None


def start_server(data):
    return subprocess.Popen([SERVER, "--data", data, "--port", str(PORT), "--name", "E2E test server"], stdout=subprocess.DEVNULL, stderr=open(os.path.join(data, "stderr.txt"), "a"))


def main():
    if not APP or not os.path.exists(SERVER):
        print("build first: app or server exe missing in", REL)
        sys.exit(2)
    os.makedirs(OUT, exist_ok=True)
    data = tempfile.mkdtemp(prefix="dexsheets-e2e-")
    for u, d, p, r in [("vikram", "Vikram", "pass-vik-1", "admin"), ("anita", "Anita", "pass-ani-2", "editor"), ("rahul", "Rahul", "pass-rah-3", "editor")]:
        subprocess.run([SERVER, "--data", data, "user", "add", u, "--display", d, "--password", p, "--role", r], check=True, capture_output=True)
    srv = start_server(data)
    time.sleep(1.0)
    apps = []
    try:
        a = App(7981, "e2e-token-aaaaaaaaaaaa", "vikram")
        b = App(7982, "e2e-token-bbbbbbbbbbbb", "anita")
        c = App(7983, "e2e-token-cccccccccccc", "rahul")
        apps = [a, b, c]
        server = f"127.0.0.1:{PORT}"

        # Wrong password is refused.
        a.run("collab.signIn", server=server, user="vikram", password="wrong-password")
        st = wait(lambda: (lambda s: s if s["error"] else None)(a.status()))
        check("wrong password refused", st and "Wrong user name or password" in (st["error"] or ""), st and st["error"])

        for app, pw in [(a, "pass-vik-1"), (b, "pass-ani-2"), (c, "pass-rah-3")]:
            app.run("collab.signIn", server=server, user=app.who, password=pw)
        ok = all(wait(lambda app=app: app.status()["status"] == "Online") for app in apps)
        check("three people signed in", ok)

        a.run("collab.create", name="Line List 100")
        ok = wait(lambda: any(d["book"] == "Line List 100" and d["active"] for d in a.status()["docs"]))
        check("vikram created a shared workbook", ok)
        for app in (b, c):
            wait(lambda app=app: any(x["name"] == "Line List 100" for x in app.status()["books"]))
            app.run("collab.open", book="Line List 100")
        ok = all(wait(lambda app=app: any(d["book"] == "Line List 100" and d["active"] for d in app.status()["docs"])) for app in (b, c))
        check("anita and rahul opened it", ok)

        # Live propagation and latency.
        a.run("cell.set", cell="A1", input="Line No")
        a.run("cell.set", cell="B1", input="Size (in)")
        a.run("cell.set", cell="C1", input="Length (m)")
        a.run("cell.set", cell="D1", input="Weight (kg)")
        t0 = time.time()
        a.run("cell.set", cell="A2", input="100-P-2001-A1A")
        got = wait(lambda: b.value("A2") == "100-P-2001-A1A", timeout=10, step=0.01)
        lat = (time.time() - t0) * 1000
        check("edit reaches another window", got, f"{lat:.0f} ms including polling")
        a.run("cell.set", cell="C2", input="12.5")
        a.run("cell.set", cell="D2", input="=C2*28.26")
        ok = wait(lambda: abs(float(c.value("D2") or 0) - 353.25) < 1e-9)
        check("formula result arrives (=C2*28.26 -> 353.25)", ok, str(c.value("D2")))
        # Someone else changes the precedent; dependents recalculate everywhere.
        b.run("cell.set", cell="C2", input="10")
        ok = wait(lambda: abs(float(a.value("D2") or 0) - 282.6) < 1e-9) and wait(lambda: abs(float(c.value("D2") or 0) - 282.6) < 1e-9)
        check("remote precedent change recalculates dependents", ok, f"A={a.value('D2')} C={c.value('D2')}")

        # Who edited what.
        le = wait(lambda: (lambda r: r if r and r.get("user") == "anita" else None)(c.run("collab.lastEdit", cell="C2")))
        check("C2 attributed to anita", le, json.dumps(le))
        le = c.run("collab.lastEdit", cell="A2")
        check("A2 attributed to vikram", le and le.get("user") == "vikram", json.dumps(le))

        # Simultaneous edits to the same cell converge.
        b.run("cell.set", cell="B2", input="6")
        c.run("cell.set", cell="B2", input="8")
        time.sleep(1.0)
        vals = [x.value("B2") for x in apps]
        check("conflicting edits converge on one value", len(set(map(str, vals))) == 1, str(vals))

        # Formatting and structure.
        c.run("home.bold", range="A1:D1", on=True)
        ok = wait(lambda: (b.cell("A1").get("style") or {}).get("font", {}).get("bold") or "bold" in json.dumps(b.cell("A1")).lower())
        check("bold header reaches others", ok)
        c.run("home.insertRows", rows="2:2")
        ok = wait(lambda: a.value("A3") == "100-P-2001-A1A" and b.value("A3") == "100-P-2001-A1A")
        check("row insert moves data for everyone", ok, f"A3={a.value('A3')}")
        ok = wait(lambda: abs(float(a.value("D3") or 0) - 282.6) < 1e-9)
        check("formula follows the inserted row", ok, str(a.cell("D3")))

        # Undo only takes back your own edit.
        a.run("cell.set", cell="A10", input="vikram's note")
        b.run("cell.set", cell="A11", input="anita's note")
        wait(lambda: a.value("A11") == "anita's note")
        a.run("edit.undo")
        ok = wait(lambda: b.value("A10") in (None, "") and b.value("A11") == "anita's note")
        check("undo reverts only vikram's own edit", ok, f"A10={b.value('A10')!r} A11={b.value('A11')!r}")

        # Presence: anita sees where rahul is.
        c.run("selection.set", range="C5:D7")
        ok = wait(lambda: any(p["user"] == "rahul" and p["range"] == "C5:D7" for d in b.status()["docs"] for p in d["peers"]))
        check("anita sees rahul's selection", ok)
        b.shot(os.path.join(OUT, "anita-sees-rahul.png"))

        # Server crash: edits made offline are kept and sent after it comes back.
        srv.kill()
        srv.wait()
        wait(lambda: a.status()["status"].startswith("Reconnecting"), timeout=30)
        a.run("cell.set", cell="A20", input="typed while the server was down")
        pend = a.status()["docs"][0]["pending"]
        check("offline edit is held", pend >= 1, f"pending={pend}")
        srv = start_server(data)
        ok = wait(lambda: b.value("A20") == "typed while the server was down", timeout=60, step=0.25)
        check("offline edit delivered after the server restarts", ok)

        # Persistence: a fresh sign-in after another restart sees everything.
        srv.kill()
        srv.wait()
        srv = start_server(data)
        wait(lambda: all(x.status()["status"] == "Online" for x in apps), timeout=60, step=0.25)
        time.sleep(1.0)
        reads = [x.call("sheet.read", {"range": "A1:D20"}) for x in apps]
        check("all three windows identical after two restarts", reads[0] == reads[1] == reads[2])
        hist_path = None
        for root, _, files in os.walk(os.path.join(data, "books")):
            if "audit.jsonl" in files:
                hist_path = os.path.join(root, "audit.jsonl")
        lines = open(hist_path, encoding="utf-8").read().splitlines() if hist_path else []
        users = {json.loads(l)["user"] for l in lines}
        check("server audit trail names every editor", {"vikram", "anita", "rahul"} <= users, f"{len(lines)} audit lines, users={sorted(users)}")
        a.shot(os.path.join(OUT, "vikram-window.png"))
        time.sleep(1.5)
    finally:
        for x in apps:
            x.close()
        try:
            srv.kill()
        except Exception:
            pass
        shutil.copytree(data, os.path.join(OUT, "server-data"), dirs_exist_ok=True)
        shutil.rmtree(data, ignore_errors=True)
    passed = sum(1 for _, ok, _ in results if ok)
    print(f"\n{passed}/{len(results)} checks passed")
    with open(os.path.join(OUT, "results.json"), "w", encoding="utf-8") as f:
        json.dump([{"check": n, "ok": o, "detail": d} for n, o, d in results], f, indent=2)
    sys.exit(0 if passed == len(results) else 1)


if __name__ == "__main__":
    main()
