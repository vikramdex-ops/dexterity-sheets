"""Proves a release's two Windows installers work before anyone downloads them.

For DexteritySheets-Setup.exe and then the .msi, on this machine and without administrator rights:
1. installs silently;
2. Windows Installer lists exactly one "Dexterity Sheets" with the release's version;
3. the Start Menu shortcut and the app are in place;
4. the installed app starts, answers on its control channel, opens a sample workbook and
   calculates a formula;
5. uninstalls cleanly (app files and the product registration gone).

Usage (the release workflow runs it on the files it is about to publish):
    py -3 -I packaging/dexterity/tests/installer_smoke.py <dist folder> <version> [--install-dir D:\\somewhere]
Without --install-dir the default per-user folder (%LOCALAPPDATA%\\Programs) is used.
Refuses to run when Dexterity Sheets is already installed, so it never removes a real install.
"""
import glob
import json
import os
import socket
import subprocess
import sys
import time

results = []


def check(name, ok, detail=""):
    results.append((name, bool(ok)))
    print(("PASS " if ok else "FAIL ") + name + (f"  [{detail}]" if detail else ""), flush=True)


def installed():
    """[(version, product code)] of every installed Dexterity Sheets, from Windows Installer."""
    ps = (
        "$i = New-Object -ComObject WindowsInstaller.Installer; "
        "foreach ($p in @($i.ProductsEx('', '', 3))) { $n=''; try { $n = $p.InstallProperty('ProductName') } catch {}; "
        "if ($n -eq 'Dexterity Sheets') { $p.InstallProperty('VersionString') + '|' + $p.ProductCode() } }"
    )
    out = subprocess.run(["powershell", "-NoProfile", "-Command", ps], capture_output=True, text=True, encoding="utf-8").stdout
    return [tuple(x.strip() for x in line.split("|", 1)) for line in out.splitlines() if "|" in line]


def wait(pred, timeout, step=0.5):
    end = time.time() + timeout
    while time.time() < end:
        try:
            r = pred()
            if r:
                return r
        except Exception:
            pass
        time.sleep(step)
    return None


def control(port, token):
    s = socket.create_connection(("127.0.0.1", port), timeout=30)
    f = s.makefile("rw", encoding="utf-8", newline="\n")
    n = [0]

    def call(method, params=None):
        n[0] += 1
        f.write(json.dumps({"id": n[0], "method": method, "params": params or {}}) + "\n")
        f.flush()
        r = json.loads(f.readline())
        if not r.get("ok"):
            raise RuntimeError(r.get("error"))
        return r.get("result")

    call("auth", {"token": token})
    return call


def exercise(label, command, version, folder, logs):
    log = os.path.join(logs, f"{label}-install.log")
    r = subprocess.run([*command, "/l*v" if command[0] == "msiexec" else "/log", log], timeout=600)
    check(f"{label}: installs silently without administrator rights", r.returncode == 0, f"exit {r.returncode}, log {log}")
    found = installed()
    check(f"{label}: Windows Installer lists one Dexterity Sheets {version}", [v for v, _ in found] == [version], f"{found}")
    exe = os.path.join(folder, "DexteritySheets.exe")
    check(f"{label}: app installed", os.path.isfile(exe), exe)
    menu = os.path.join(os.environ["APPDATA"], "Microsoft", "Windows", "Start Menu", "Programs")
    check(f"{label}: Start Menu shortcut", bool(glob.glob(os.path.join(menu, "**", "Dexterity Sheets*.lnk"), recursive=True)))
    if os.path.isfile(exe):
        port, token = 7961, "installer-smoke-token-0123456789"
        app = subprocess.Popen([exe, "--sample", "sales", "--control", str(port)], env=dict(os.environ, GRIDCRAFT_CONTROL_TOKEN=token))
        try:
            call = wait(lambda: control(port, token), 90)
            check(f"{label}: installed app starts and answers", call is not None)
            if call:
                call("engine.execute", {"command": "cell.set", "params": {"cell": "Z1", "input": "=SUM(1,2,3)*7"}})
                got = call("engine.execute", {"command": "cell.get", "params": {"cell": "Z1"}})
                value = (got or {}).get("value")
                check(f"{label}: installed app calculates =SUM(1,2,3)*7", value in (42, 42.0, "42"), f"value {value!r}")
        finally:
            app.kill()
            app.wait(30)
            time.sleep(1.0)
    for _, code in installed():
        subprocess.run(["msiexec", "/x", code, "/qn", "/norestart", "/l*v", os.path.join(logs, f"{label}-uninstall.log")], timeout=600)
    check(f"{label}: uninstalls cleanly", not installed() and not os.path.exists(exe))


def main():
    dist, version = os.path.abspath(sys.argv[1]), sys.argv[2]
    folder = os.path.join(os.environ["LOCALAPPDATA"], "Programs", "Dexterity Sheets")
    extra = []
    if "--install-dir" in sys.argv:
        folder = os.path.abspath(sys.argv[sys.argv.index("--install-dir") + 1])
        extra = [f"INSTALLFOLDER={folder}\\"]
    if installed():
        print("Dexterity Sheets is installed on this machine; not touching it. Run this on a clean machine.")
        return 2
    logs = os.path.abspath(os.path.join("tests", "out", "installer"))
    os.makedirs(logs, exist_ok=True)
    setup = os.path.join(dist, "DexteritySheets-Setup.exe")
    msis = glob.glob(os.path.join(dist, f"dexterity-sheets-{version}-windows-x64.msi"))
    check("release has DexteritySheets-Setup.exe", os.path.isfile(setup))
    check(f"release has the {version} .msi", len(msis) == 1, f"{msis}")
    if os.path.isfile(setup):
        exercise("setup.exe", [setup, "/quiet", *extra], version, folder, logs)
    if msis:
        exercise("msi", ["msiexec", "/i", msis[0], "/qn", "/norestart", *extra], version, folder, logs)
    failed = [n for n, ok in results if not ok]
    print(f"\n{len(results) - len(failed)}/{len(results)} passed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
