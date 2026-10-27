"""The performance gate (RULES.md §1.4): measures the real app and fails when it is over budget.

Budgets live in packaging/dexterity/perf-budget.json. Measured on the machine that runs it (the CI
runner for releases):
  startup_ms           launch until the window answers
  idle_ram_mb          memory in use right after start (working set: what Task Manager shows)
  idle_commit_mb       memory reserved (private bytes; mostly the graphics driver's, tracked only)
  idle_cpu_ms_per_10s  CPU used while nobody touches it (should be ~0: it only draws on demand)
  edit_ms              the app's own time for one typed cell: applying it plus drawing the
                       frame that shows it (median over 200 edits)
  wake_ms              a do-nothing request's round trip: one screen refresh. Tracked only:
                       Windows slows the refresh of a window nobody can see (16.7 ms visible
                       and 111 ms minimised on a laptop; some CI runners show 300+ ms)
  open_big_ms          opening a 200,000-row CSV (2 million cells)
  big_ram_mb           memory in use with that workbook open
  scroll_frame_ms      slowest frame while paging through it (95th percentile)
  after_close_ram_mb   memory in use once it is closed again
  leak_after_close_mb  how much of it was not given back
  exe_mb               DexteritySheets.exe on disk

Run:  python packaging/dexterity/tests/perf_gate.py <target/release> [--report out.json]
"""
import json
import os
import socket
import subprocess
import sys
import tempfile
import time

REL = os.path.abspath(sys.argv[1] if len(sys.argv) > 1 else os.path.join("target", "release"))
APP = os.path.join(REL, "DexteritySheets.exe")
BUDGET = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "perf-budget.json")
PORT = 7991
TOKEN = "perf-gate-token-0123456789"


def proc_stats(pid):
    """(working set MB, private bytes MB, total CPU ms) of a process, via PowerShell (no extra
    Python packages)."""
    cmd = f"$p = Get-Process -Id {pid}; '{{0}} {{1}} {{2}}' -f $p.WorkingSet64, $p.PrivateMemorySize64, [int64]$p.TotalProcessorTime.TotalMilliseconds"
    out = subprocess.run(["powershell", "-NoProfile", "-Command", cmd], capture_output=True, text=True).stdout.split()
    return float(out[0]) / (1024 * 1024), float(out[1]) / (1024 * 1024), float(out[2])


class App:
    def __init__(self):
        env = dict(os.environ, GRIDCRAFT_CONTROL_TOKEN=TOKEN)
        self.t0 = time.perf_counter()
        self.proc = subprocess.Popen([APP, "--control", str(PORT)], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        self.n = 0
        deadline = time.time() + 90
        while True:
            try:
                self.sock = socket.create_connection(("127.0.0.1", PORT), timeout=60)
                break
            except OSError:
                if time.time() > deadline:
                    raise
                time.sleep(0.05)
        self.f = self.sock.makefile("rw", encoding="utf-8", newline="\n")
        self.call("auth", {"token": TOKEN})
        self.call("ui.inspect")
        self.startup_ms = (time.perf_counter() - self.t0) * 1000

    def call(self, method, params=None):
        self.n += 1
        self.f.write(json.dumps({"id": self.n, "method": method, "params": params or {}}) + "\n")
        self.f.flush()
        r = json.loads(self.f.readline())
        if not r.get("ok"):
            raise RuntimeError(f"{method}: {r.get('error')}")
        return r.get("result")

    def run(self, command, **p):
        return self.call("engine.execute", {"command": command, "params": p})


def main():
    with open(BUDGET, encoding="utf-8") as f:
        budget = json.load(f)["budgets"]
    work = tempfile.mkdtemp(prefix="perf-gate-")
    big = os.path.join(work, "big.csv")
    with open(big, "w", encoding="utf-8", newline="\n") as f:
        f.write("Line,Size,Spec,From,To,Length,kgm,Weight,Status,Note\n")
        for i in range(200_000):
            f.write(f"100-P-{i:06d},{(i % 12) + 1},A1A,P-{i % 97},E-{i % 89},{(i % 500) / 10},{(i % 60) + 0.5},{i * 1.5},IFC,row {i}\n")
    m = {"exe_mb": os.path.getsize(APP) / (1024 * 1024)}
    app = App()
    try:
        m["startup_ms"] = app.startup_ms
        time.sleep(3)
        ram, commit, cpu0 = proc_stats(app.proc.pid)
        m["idle_ram_mb"] = ram
        m["idle_commit_mb"] = commit
        time.sleep(10)
        m["idle_cpu_ms_per_10s"] = proc_stats(app.proc.pid)[2] - cpu0
        # A control request waits for the next frame, so its round trip is mostly the screen's
        # refresh, which Windows throttles when the window can't be seen. Each edit is paired
        # with a do-nothing request: the difference is the time spent applying the edit (the
        # reply leaves straight after), and that request reports how long drawing the edit took.
        edits, wakes = [], []
        for i in range(200):
            t = time.perf_counter()
            app.run("cell.set", cell=f"B{i + 2}", input=str(i * 3))
            edit = (time.perf_counter() - t) * 1000
            t = time.perf_counter()
            info = app.call("ui.inspect") or {}
            wake = (time.perf_counter() - t) * 1000
            paint = float((info.get("perf") or {}).get("frameMs") or 0)
            edits.append(max(0.0, edit - wake) + paint)
            wakes.append(wake)
        edits.sort()
        wakes.sort()
        m["edit_ms"] = edits[len(edits) // 2]
        m["wake_ms"] = wakes[len(wakes) // 2]
        t = time.perf_counter()
        app.call("app.open", {"path": big})
        m["open_big_ms"] = (time.perf_counter() - t) * 1000
        time.sleep(2)
        m["big_ram_mb"] = proc_stats(app.proc.pid)[0]
        frames = []
        for _ in range(60):
            app.call("ui.key", {"key": "PageDown"})
            frames.append(float((app.call("ui.inspect").get("perf") or {}).get("frameMs") or 0))
        frames.sort()
        m["scroll_frame_ms"] = frames[int(len(frames) * 0.95) - 1]
        app.run("file.close", force=True, document=True)
        time.sleep(4)
        m["after_close_ram_mb"] = proc_stats(app.proc.pid)[0]
        m["leak_after_close_mb"] = max(0.0, m["after_close_ram_mb"] - m["idle_ram_mb"])
    finally:
        try:
            app.proc.kill()
        except Exception:
            pass
    over = []
    lines = ["| Measure | Value | Budget | |", "|---|---:|---:|---|"]
    for k, v in m.items():
        b = budget.get(k, {}).get("max")
        ok = b is None or v <= b
        if not ok:
            over.append(k)
        lines.append(f"| {k} | {v:,.1f} | {b if b is not None else '—'} | {'ok' if ok else '**over**'} |")
    report = "\n".join(lines)
    sys.stdout.reconfigure(encoding="utf-8")
    print(report)
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a", encoding="utf-8") as f:
            f.write("### Performance gate\n\n" + report + "\n")
    if "--report" in sys.argv:
        with open(sys.argv[sys.argv.index("--report") + 1], "w", encoding="utf-8") as f:
            json.dump(m, f, indent=1)
    if over:
        print("\nOver budget: " + ", ".join(over) + " (RULES.md §1.4: fix it, or raise the budget with a reason in perf-budget.json)")
        sys.exit(1)


if __name__ == "__main__":
    main()
