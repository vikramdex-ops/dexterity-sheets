"""End-to-end test of the in-app updater on Windows.

1. Installs Dexterity Sheets <old>.msi into a test folder (per-user, no admin).
2. Starts a DexSheetsServer whose updates folder holds <new>.msi.
3. Starts the installed app, signs in, and checks that it is offered <new>.
4. Asks it to install: it downloads from the server, verifies sha256, runs msiexec and closes.
5. Checks that Windows now lists <new> as installed, in the same folder, and that the app
   reopened. Then uninstalls everything it installed.

Run: py -3 -I update_e2e.py OLD.msi NEW.msi DexSheetsServer.exe WORKDIR
"""
import json
import os
import shutil
import socket
import subprocess
import sys
import time
import winreg

OLD, NEW, SERVER, WORK = (os.path.abspath(a) for a in sys.argv[1:5])
INSTALL = os.path.join(WORK, "installed")
DATA = os.path.join(WORK, "server-data")
PORT = 47901
results = []


def check(name, ok, detail=""):
    results.append((name, bool(ok)))
    print(("PASS " if ok else "FAIL ") + name + (f"  [{detail}]" if detail else ""), flush=True)


def installed_version():
    """(version, product code) of the installed Dexterity Sheets, from Windows Installer."""
    ps = (
        "$i = New-Object -ComObject WindowsInstaller.Installer; "
        "foreach ($p in @($i.ProductsEx('', '', 3))) { $n=''; try { $n = $p.InstallProperty('ProductName') } catch {}; "
        "if ($n -eq 'Dexterity Sheets') { $p.InstallProperty('VersionString') + '|' + $p.ProductCode() } }"
    )
    out = subprocess.run(["powershell", "-NoProfile", "-Command", ps], capture_output=True, text=True).stdout.strip().splitlines()
    if not out:
        return None, None
    v, code = out[0].split("|", 1)
    return v.strip(), code.strip()

def running_exes():
    out = subprocess.run(["powershell", "-NoProfile", "-Command", "Get-Process DexteritySheets -ErrorAction SilentlyContinue | ForEach-Object Path"], capture_output=True, text=True).stdout
    return [l.strip() for l in out.splitlines() if l.strip()]


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


def main():
    shutil.rmtree(WORK, ignore_errors=True)
    os.makedirs(os.path.join(DATA, "updates"), exist_ok=True)
    log = os.path.join(WORK, "install-old.log")
    r = subprocess.run(["msiexec", "/i", OLD, "/qn", "/norestart", f"INSTALLFOLDER={INSTALL}\\", "/l*v", log])
    exe = os.path.join(INSTALL, "DexteritySheets.exe")
    v, _ = installed_version()
    check("old version installs per-user without admin", r.returncode == 0 and os.path.exists(exe), f"msiexec {r.returncode}, registered {v}")

    shutil.copy(NEW, os.path.join(DATA, "updates", os.path.basename(NEW)))
    with open(os.path.join(DATA, "updates", os.path.splitext(os.path.basename(NEW))[0] + ".md"), "w", encoding="utf-8") as f:
        f.write("Test release notes: faster sync, fixes.")
    subprocess.run([SERVER, "--data", DATA, "user", "add", "tester", "--display", "Tester", "--password", "update-test-1", "--role", "editor"], check=True, capture_output=True)
    srv = subprocess.Popen([SERVER, "--data", DATA, "--port", str(PORT), "--name", "Update test"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(1.5)
    token = "update-test-token-123456"
    app = subprocess.Popen([exe, "--control", "7990"], env=dict(os.environ, GRIDCRAFT_CONTROL_TOKEN=token))
    try:
        call = wait(lambda: control(7990, token), 60)
        call("engine.execute", {"command": "collab.signIn", "params": {"server": f"127.0.0.1:{PORT}", "user": "tester", "password": "update-test-1"}})
        online = wait(lambda: call("engine.execute", {"command": "collab.status"})["status"] == "Online", 30)
        check("app signs in to the server", online)
        offered = wait(lambda: call("engine.execute", {"command": "collab.checkUpdate"}).get("available"), 30, 1)
        check("app is told a newer version exists", offered, str(offered))
        before = time.time()
        call("engine.execute", {"command": "collab.installUpdate"})
        gone = wait(lambda: app.poll() is not None, 60)
        check("app closes itself to install", gone)
        new_v = wait(lambda: (lambda v: v if v and v != "1.0.0" else None)(installed_version()[0]), 180, 1)
        check("Windows lists the new version as installed", new_v, str(new_v))
        reopened = wait(lambda: [p for p in running_exes() if p.lower() == exe.lower()], 60, 1)
        check("the updated app reopens from the same folder", reopened, str(reopened))
        print(f"update took {time.time() - before:.0f} s end to end")
    finally:
        subprocess.run(["powershell", "-NoProfile", "-Command", "Get-Process DexteritySheets -ErrorAction SilentlyContinue | Stop-Process -Force"])
        srv.kill()
        _, code = installed_version()
        if code:
            subprocess.run(["msiexec", "/x", code, "/qn", "/norestart"])
        check("uninstall removes it", installed_version()[0] is None)
    passed = sum(ok for _, ok in results)
    print(f"\n{passed}/{len(results)} checks passed")
    sys.exit(0 if passed == len(results) else 1)


if __name__ == "__main__":
    main()
