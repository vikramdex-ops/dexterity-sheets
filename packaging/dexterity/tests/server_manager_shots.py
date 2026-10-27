"""Screenshots of the Server Manager for the README and the landing page, from the released build.

Makes a throw-away server data folder with a few accounts and an update installer, then opens the
Server Manager on it (no administrator rights needed for a folder of its own) once per page:
  server-people.png      People: the accounts, roles, Reset password / Disable / Remove
  server-add-many.png    Add many people: a filled-in template imported and checked row by row
  server-updates.png     App updates: the installers offered to the office PCs
The overview of a really installed, running server comes from the server installer smoke test
(tests/out/server-installer/server-manager.png) and is copied as server-manager.png.

Run:  python packaging/dexterity/tests/server_manager_shots.py <target/release> <out folder>
"""
import os
import shutil
import subprocess
import sys
import tempfile

REL = os.path.abspath(sys.argv[1] if len(sys.argv) > 1 else os.path.join("target", "release"))
OUT = os.path.abspath(sys.argv[2] if len(sys.argv) > 2 else "media")
SERVER = os.path.join(REL, "DexSheetsServer.exe")
MANAGER = os.path.join(REL, "DexSheetsServerManager.exe")
PEOPLE = [
    ("vikram", "Vikram Kumar", "admin"),
    ("anita.rao", "Anita Rao", "editor"),
    ("rahul.mehta", "Rahul Mehta", "editor"),
    ("priya.nair", "Priya Nair", "editor"),
    ("site.office", "Site Office", "viewer"),
]
IMPORT = [
    ("User name", "Full name", "Role", "Password"),
    ("", "Suresh Iyer", "Editor", ""),
    ("meera.k", "Meera Krishnan", "Viewer", ""),
    ("", "Arjun Das", "", ""),
    ("anita.rao", "Anita Rao", "Editor", ""),
    ("deepak s", "Deepak Sharma", "Editor", "123"),
]


def shot(data, page, name, extra=()):
    path = os.path.join(OUT, name)
    subprocess.run([MANAGER, "--no-elevate", "--data", data, *extra, "--shot", page, path], timeout=120, check=False)
    print(("saved " if os.path.isfile(path) else "MISSING ") + path, flush=True)


def main():
    if not (os.path.isfile(SERVER) and os.path.isfile(MANAGER)):
        print("build the server and the Server Manager first:", REL)
        sys.exit(2)
    os.makedirs(OUT, exist_ok=True)
    data = tempfile.mkdtemp(prefix="dexsheets-manager-shots-")
    try:
        for user, display, role in PEOPLE:
            subprocess.run([SERVER, "--data", data, "user", "add", user, "--display", display, "--password", "shots-pass-1", "--role", role], check=True, capture_output=True)
        subprocess.run([SERVER, "--data", data, "user", "disable", "priya.nair"], check=True, capture_output=True)
        os.makedirs(os.path.join(data, "updates"), exist_ok=True)
        with open(os.path.join(data, "updates", "dexterity-sheets-1.2.0-windows-x64.msi"), "wb") as f:
            f.write(b"\0" * 16_240_640)
        with open(os.path.join(data, "updates", "dexterity-sheets-1.2.0-windows-x64.md"), "w", encoding="utf-8") as f:
            f.write("- Share any file from where it is saved\n- Server Manager for IT")
        with open(os.path.join(data, "server.json"), "w", encoding="utf-8") as f:
            f.write('{"name": "Dexterity Sheets on DDS-SERVER"}')
        people = os.path.join(data, "new-people.csv")
        with open(people, "w", encoding="utf-8", newline="") as f:
            f.write("\n".join(",".join(r) for r in IMPORT) + "\n")
        shot(data, "people", "server-people.png")
        shot(data, "add-many", "server-add-many.png", ["--import", people])
        shot(data, "updates", "server-updates.png")
        installed = os.path.join(os.getcwd(), "tests", "out", "server-installer", "server-manager.png")
        if os.path.isfile(installed):
            shutil.copyfile(installed, os.path.join(OUT, "server-manager.png"))
            print("copied", installed)
        else:
            shot(data, "overview", "server-manager.png")
    finally:
        shutil.rmtree(data, ignore_errors=True)


if __name__ == "__main__":
    main()
