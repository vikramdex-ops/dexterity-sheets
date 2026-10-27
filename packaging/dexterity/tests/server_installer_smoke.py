"""Installs, checks and uninstalls the office server, as IT would: DexSheetsServer-Setup.exe and the
.msi, both silently. Needs administrator rights (GitHub's Windows runners have them).

Checks that the service is installed, running, starts with Windows and restarts on failure; that it
answers on its port; the firewall rules; the data folder locked to SYSTEM and Administrators; the
Server Manager and its Start menu shortcut (and that the Manager opens and shows the server running,
saving a screenshot); that an account made with the server's own command is kept; and that
uninstalling stops and removes the service but keeps the data.

Run:  python packaging/dexterity/tests/server_installer_smoke.py <dist folder> <version>
"""
import os
import socket
import subprocess
import sys
import time

DIST = os.path.abspath(sys.argv[1] if len(sys.argv) > 1 else "dist")
VERSION = sys.argv[2] if len(sys.argv) > 2 else ""
OUT = os.path.join(os.getcwd(), "tests", "out", "server-installer")
SERVICE = "DexSheetsServer"
PROGRAM = os.path.join(os.environ.get("ProgramFiles", r"C:\Program Files"), "Dexterity Sheets Server")
DATA = os.path.join(os.environ.get("ProgramData", r"C:\ProgramData"), "Dexterity Sheets", "Server")
SHORTCUT = os.path.join(os.environ.get("ProgramData", r"C:\ProgramData"), r"Microsoft\Windows\Start Menu\Programs\Dexterity Sheets Server Manager.lnk")
results = []


def check(name, ok, detail=""):
    results.append((name, bool(ok)))
    print(("PASS " if ok else "FAIL ") + name + (f"  [{detail}]" if detail else ""), flush=True)


def run(args, **kw):
    return subprocess.run(args, capture_output=True, text=True, encoding="utf-8", errors="replace", **kw)


def service_state():
    r = run(["sc", "query", SERVICE])
    if r.returncode != 0:
        return "MISSING"
    for state in ("RUNNING", "STOPPED", "START_PENDING", "STOP_PENDING"):
        if state in r.stdout:
            return state
    return "UNKNOWN"


def wait(pred, secs=30):
    end = time.time() + secs
    while time.time() < end:
        if pred():
            return True
        time.sleep(0.5)
    return False


def answers(port=47810):
    try:
        with socket.create_connection(("127.0.0.1", port), timeout=1):
            return True
    except OSError:
        return False


def installed_checks(how):
    check(f"{how}: service running", wait(lambda: service_state() == "RUNNING"), service_state())
    qc = run(["sc", "qc", SERVICE]).stdout
    check(f"{how}: starts with Windows, as LocalSystem", "AUTO_START" in qc and "LocalSystem" in qc)
    check(f"{how}: restarts if it stops", "RESTART" in run(["sc", "qfailure", SERVICE]).stdout)
    check(f"{how}: answers on TCP 47810", wait(answers, 20))
    for name in ("Dexterity Sheets Server (TCP)", "Dexterity Sheets Discovery (UDP)"):
        r = run(["netsh", "advfirewall", "firewall", "show", "rule", f"name={name}"])
        profiles = [l.split(":", 1)[1].strip() for l in r.stdout.splitlines() if l.strip().startswith("Profiles:")]
        check(f"{how}: firewall rule {name}, office networks only", r.returncode == 0 and profiles and not any("Public" in p for p in profiles), ", ".join(profiles))
    for exe in ("DexSheetsServer.exe", "DexSheetsServerManager.exe"):
        check(f"{how}: {exe} installed", os.path.isfile(os.path.join(PROGRAM, exe)))
    check(f"{how}: Start menu shortcut for the Server Manager", os.path.isfile(SHORTCUT))
    acl = run(["icacls", DATA]).stdout
    check(f"{how}: data folder only for SYSTEM and Administrators", "SYSTEM" in acl and "Administrators" in acl and "\\Users:" not in acl and "Everyone" not in acl, acl.strip().replace("\n", " | "))


def main():
    if run(["net", "session"]).returncode != 0:
        print("Run as administrator.")
        sys.exit(2)
    if service_state() != "MISSING":
        print("A Dexterity Sheets Server is already installed here; this test only runs on a clean machine.")
        sys.exit(2)
    os.makedirs(OUT, exist_ok=True)
    setup = os.path.join(DIST, "DexSheetsServer-Setup.exe")
    msi = os.path.join(DIST, f"DexSheetsServer-{VERSION}-x64.msi")
    check("both server installers exist", os.path.isfile(setup) and os.path.isfile(msi))

    # 1. Setup.exe, silently.
    r = run([setup, "/quiet", "/log", os.path.join(OUT, "setup-install.log")], timeout=600)
    check("Setup.exe installs", r.returncode in (0, 3010), f"exit {r.returncode}")
    installed_checks("Setup.exe")

    # An account made with the server's own command, kept across reinstall and uninstall.
    exe = os.path.join(PROGRAM, "DexSheetsServer.exe")
    r = run([exe, "user", "add", "tester", "--display", "Test Person", "--password", "smoke-test-123", "--role", "admin"])
    check("an administrator can be added", r.returncode == 0, (r.stdout + r.stderr).strip())

    # The Server Manager opens and sees the running server.
    shot = os.path.join(OUT, "server-manager.png")
    mgr = os.path.join(PROGRAM, "DexSheetsServerManager.exe")
    try:
        r = run([mgr, "--shot", "overview", shot], timeout=120)
        check("the Server Manager opens", os.path.isfile(shot), f"exit {r.returncode}")
    except subprocess.TimeoutExpired:
        check("the Server Manager opens", False, "timed out")

    r = run(["msiexec", "/x", msi, "/qn", "/l*v", os.path.join(OUT, "setup-uninstall.log")], timeout=600)
    check("uninstall (after Setup.exe)", r.returncode in (0, 3010), f"exit {r.returncode}")
    check("uninstall removes the service", wait(lambda: service_state() == "MISSING"), service_state())
    check("uninstall stops the server", wait(lambda: not answers(), 20))
    check("uninstall keeps the data (accounts)", os.path.isfile(os.path.join(DATA, "users.json")))

    # 2. The .msi, silently, over the kept data.
    r = run(["msiexec", "/i", msi, "/qn", "/l*v", os.path.join(OUT, "msi-install.log")], timeout=600)
    check(".msi installs", r.returncode in (0, 3010), f"exit {r.returncode}")
    installed_checks(".msi")
    r = run([exe, "user", "list"])
    check("the kept account is still there", "tester" in r.stdout, r.stdout.strip())
    r = run(["msiexec", "/x", msi, "/qn", "/l*v", os.path.join(OUT, "msi-uninstall.log")], timeout=600)
    check("uninstall (after .msi)", r.returncode in (0, 3010) and wait(lambda: service_state() == "MISSING"), f"exit {r.returncode}")

    passed = sum(1 for _, ok in results if ok)
    print(f"\n{passed}/{len(results)} server installer checks passed")
    sys.exit(0 if passed == len(results) else 1)


if __name__ == "__main__":
    main()
