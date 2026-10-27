"""Takes the README screenshots of Dexterity Sheets from real app windows.

Starts a DexSheetsServer with three accounts, opens app windows through the control channel,
builds a small line list together and captures: the samples, the Formulas and Insert tabs (light
and dark), live co-editing with Edit History, Versions, File > Shared, sign-in, and the update
prompt. Output: docs/images/*.png (or $DEXSHEETS_SHOTS_OUT), plus the frames of the README demo
animation in tests/out/demo-frames (or $DEXSHEETS_FRAMES_OUT): Vikram's window while the three of
them type the line list. packaging/dexterity/make_demo.py turns the frames into the animation.

Run from the repository root after a release build:
    py -3 -I packaging/dexterity/tests/screenshots.py target/release
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
APP = os.path.join(REL, "DexteritySheets.exe")
SERVER = os.path.join(REL, "DexSheetsServer.exe")
OUT = os.path.abspath(os.environ.get("DEXSHEETS_SHOTS_OUT") or os.path.join("docs", "images"))
FRAMES = os.path.abspath(os.environ.get("DEXSHEETS_FRAMES_OUT") or os.path.join("tests", "out", "demo-frames"))
PORT = 47920
W, H = 1440, 860
procs = []


class App:
    def __init__(self, port, args=()):
        token = f"shots-token-{port}-abcdef"
        self.proc = subprocess.Popen([APP, "--control", str(port), *args], env=dict(os.environ, GRIDCRAFT_CONTROL_TOKEN=token))
        procs.append(self.proc)
        end = time.time() + 60
        while True:
            try:
                self.sock = socket.create_connection(("127.0.0.1", port), timeout=30)
                break
            except OSError:
                if time.time() > end:
                    raise
                time.sleep(0.5)
        self.f = self.sock.makefile("rw", encoding="utf-8", newline="\n")
        self.n = 0
        self.call("auth", {"token": token})
        self.call("ui.resize", {"width": W, "height": H})
        time.sleep(1.5)

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

    def shot(self, name, settle=1.2, folder=None):
        time.sleep(settle)
        path = os.path.join(folder or OUT, name)
        for _ in range(12):
            try:
                self.call("ui.screenshot", {"path": path})
                print("saved", path, flush=True)
                return
            except RuntimeError:
                time.sleep(0.5)
        print("FAILED", name, flush=True)


def wait(pred, timeout=20):
    end = time.time() + timeout
    while time.time() < end:
        try:
            if pred():
                return True
        except Exception:
            pass
        time.sleep(0.2)
    return False


def main():
    os.makedirs(OUT, exist_ok=True)
    shutil.rmtree(FRAMES, ignore_errors=True)
    os.makedirs(FRAMES)
    data = tempfile.mkdtemp(prefix="dexsheets-shots-")
    try:
        for u, d, p, r in [("vikram", "Vikram", "shots-pass-1", "admin"), ("anita", "Anita Rao", "shots-pass-2", "editor"), ("rahul", "Rahul Mehta", "shots-pass-3", "editor")]:
            subprocess.run([SERVER, "--data", data, "user", "add", u, "--display", d, "--password", p, "--role", r], check=True, capture_output=True)
        # A newer version waiting on the server, for the update prompt.
        os.makedirs(os.path.join(data, "updates"), exist_ok=True)
        with open(os.path.join(data, "updates", "dexterity-sheets-9.9.9-windows-x64.msi"), "wb") as f:
            f.write(b"\0" * 16_146_432)  # installer-sized placeholder
        with open(os.path.join(data, "updates", "dexterity-sheets-9.9.9-windows-x64.md"), "w", encoding="utf-8") as f:
            f.write("- Faster opening of large line lists\n- Merged the latest GridCraft fixes\n- Edit History: filter by date")
        srv = subprocess.Popen([SERVER, "--data", data, "--port", str(PORT), "--name", "Dexterity Sheets on DDS-SERVER"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        procs.append(srv)
        time.sleep(1.5)

        # Samples.
        a = App(7931, ["--sample", "sales"])
        a.shot("hero-sales.png", 2.5)
        a.run("ui.ribbonTab", tab="Insert")
        a.run("view.darkMode", on=True)
        a.shot("dark-insert.png", 2.0)
        a.run("view.darkMode", on=False)
        a.proc.kill()
        b = App(7932, ["--sample", "budget"])
        b.shot("budget.png", 2.5)
        b.proc.kill()
        g = App(7933, ["--sample", "grades"])
        g.run("ui.ribbonTab", tab="Formulas")
        g.shot("grades-formulas.png", 2.5)
        g.proc.kill()

        # Sign-in page with the server found on the network.
        s = App(7934)
        s.run("ui.backstage", page="shared")
        s.shot("sign-in.png", 3.0)
        s.proc.kill()

        # Three people build a line list together.
        v, an, ra = App(7935), App(7936), App(7937)
        server = f"127.0.0.1:{PORT}"
        for app, user, pw in [(v, "vikram", "shots-pass-1"), (an, "anita", "shots-pass-2"), (ra, "rahul", "shots-pass-3")]:
            app.run("collab.signIn", server=server, user=user, password=pw)
            wait(lambda app=app: app.run("collab.status")["status"] == "Online")
        v.run("collab.create", name="Line List - Unit 100")
        wait(lambda: any(d["active"] for d in v.run("collab.status")["docs"]))
        for app in (an, ra):
            wait(lambda app=app: any(b["name"].startswith("Line List") for b in app.run("collab.status")["books"]))
            app.run("collab.open", book="Line List - Unit 100")
            wait(lambda app=app: any(d["active"] for d in app.run("collab.status")["docs"]))
        head = ["Line No", "Size (in)", "Spec", "From", "To", "Length (m)", "kg/m", "Weight (kg)", "Status"]
        rows = [
            ["100-P-2001-A1A", "6", "A1A", "P-101A", "E-102", "48.5", "28.26"],
            ["100-P-2002-A1A", "4", "A1A", "E-102", "V-103", "36.2", "16.07"],
            ["100-P-2003-B2C", "8", "B2C", "V-103", "P-104", "62.0", "42.55"],
            ["100-CW-2004-C1", "3", "C1", "CWS", "E-102", "21.7", "11.29"],
            ["100-CW-2005-C1", "3", "C1", "E-102", "CWR", "23.1", "11.29"],
            ["100-ST-2006-D3", "2", "D3", "ST-HDR", "E-105", "17.4", "5.44"],
            ["100-P-2007-A1A", "10", "A1A", "P-104", "T-106", "84.9", "60.29"],
        ]
        cols = "ABCDEFGHI"
        frame = [0]

        def capture():
            v.shot(f"frame-{frame[0]:03d}.png", 0.7, FRAMES)
            frame[0] += 1

        capture()
        for c, h in zip(cols, head):
            v.run("cell.set", cell=f"{c}1", input=h)
        v.run("home.bold", range="A1:I1", on=True)
        for col, width in (("A", 130), ("B", 70), ("F", 80), ("H", 90), ("I", 120)):
            v.run("home.columnWidth", range=f"{col}1", width=width)
        capture()
        for i, row in enumerate(rows, start=2):
            who = [v, an, ra][i % 3]
            who.run("selection.set", range=f"A{i}:I{i}")
            for c, val in zip(cols, row):
                who.run("cell.set", cell=f"{c}{i}", input=val)
            who.run("cell.set", cell=f"H{i}", input=f"=ROUND(F{i}*G{i},1)")
            who.run("cell.set", cell=f"I{i}", input=["IFC", "Issued for review", "Hold"][i % 3])
            capture()
        v.run("cell.set", cell="G10", input="Total")
        v.run("cell.set", cell="H10", input="=SUM(H2:H8)")
        v.run("home.bold", range="A1:I1", on=True)
        v.run("home.bold", range="G10:H10", on=True)
        capture()
        an.run("selection.set", range="F3")
        an.run("cell.set", cell="F3", input="38.0")
        ra.run("selection.set", range="I4")
        ra.run("cell.set", cell="I4", input="IFC")
        capture()
        time.sleep(1.0)
        an.run("selection.set", range="F3:F5")
        ra.run("selection.set", range="I2:I4")
        v.run("selection.set", range="F3")
        v.run("collab.history", cell=True)
        v.shot("collab-history.png", 2.5)
        capture()
        v.run("file.save")
        time.sleep(1.0)
        v.run("collab.versions")
        v.shot("versions.png", 2.5)
        v.run("ui.backstage", page="shared")
        v.shot("shared-workbooks.png", 2.5)
        an.run("collab.checkUpdate")
        an.shot("update.png", 3.0)
    finally:
        for p in procs:
            try:
                p.kill()
            except Exception:
                pass
        shutil.rmtree(data, ignore_errors=True)


if __name__ == "__main__":
    main()
