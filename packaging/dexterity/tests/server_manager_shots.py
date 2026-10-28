"""Screenshots of the Server Manager for the README and the landing page, from the released build.

Makes a throw-away server data folder with a few accounts and an update installer, then opens the
Server Manager on it (no administrator rights needed for a folder of its own) once per page:
  server-people.png      People: the accounts, roles, Reset password / Disable / Remove
  server-add-many.png    Add many people: a filled-in template imported and checked row by row
  server-updates.png     App updates: the installers offered to the office PCs
  server-versions.png    Installed versions: which version each PC runs (clients.json)
  server-releases.png    Releases: channel, status and note per version, the update policy
  server-feedback.png    Feedback: problem reports by version and feature (feedback/feedback.jsonl)
  server-health.png      System health: failed updates, recoveries and errors (health.jsonl)
  server-adoption.png    Feature adoption: the most and least used commands (adoption.json)
The dashboard files are made-up examples in the format of docs/dexterity-admin-data.md.
The overview of a really installed, running server comes from the server installer smoke test
(tests/out/server-installer/server-manager.png) and is copied as server-manager.png.

Run:  python packaging/dexterity/tests/server_manager_shots.py <target/release> <out folder>
"""
import datetime
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time

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


def write_json(path, value):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        json.dump(value, f, ensure_ascii=False, indent=1)


def dashboard_data(data):
    """Example files for the dashboard pages, as the server writes them (times relative to now)."""
    now = int(time.time() * 1000)
    minute, hour, day = 60_000, 3_600_000, 86_400_000
    updates = os.path.join(data, "updates")
    for v in ("1.2.1", "1.2.2", "1.3.0"):
        with open(os.path.join(updates, f"dexterity-sheets-{v}-windows-x64.msi"), "wb") as f:
            f.write(b"\0" * 16_400_000)
    write_json(os.path.join(updates, "releases.json"), {
        "policy": {"mode": "prompt", "defaultChannel": "stable"},
        "releases": {
            "1.3.0": {"channel": "testing", "status": "ok", "note": ""},
            "1.2.2": {"channel": "stable", "status": "knownIssue", "note": "Sharing while typing in a cell could send a blank workbook; fixed in 1.3.0."},
            "1.2.1": {"channel": "stable", "status": "blocked", "note": "Do not install: does not start on some PCs."},
        },
    })
    clients = [
        ("vikram", "DDS-SERVER", "1.3.0", "testing", 4 * minute),
        ("anita.rao", "PC-ENG-07", "1.3.0", "testing", 26 * minute),
        ("rahul.mehta", "PC-ENG-02", "1.2.2", "stable", 2 * hour),
        ("priya.nair", "PC-QA-01", "1.2.2", "stable", 5 * hour),
        ("site.office", "SITE-LAPTOP", "1.2.2", "", 3 * day),
        ("rahul.mehta", "PC-DRAFT-04", "1.2.1", "stable", 6 * day),
    ]
    write_json(os.path.join(data, "clients.json"), {"clients": {
        f"{u}|{m}": {"user": u, "machine": m, "version": v, "channel": c, "lastSeen": now - ago} for u, m, v, c, ago in clients
    }})
    reports = [
        ("anita.rao", "PC-ENG-07", "1.2.2", ["sharing"], "high", True, False, 2 * day,
         "I shared the line list while still typing in a cell and the team saw an empty sheet.",
         "The team sees the workbook as it is on my screen.", "Everyone got a blank sheet; my last entry was gone.", True),
        ("rahul.mehta", "PC-ENG-02", "1.2.2", ["sharing", "saving"], "blocking", True, True, 1 * day,
         "After the update, sharing the support schedule sent a blank copy. I rolled back to 1.2.0.",
         "The shared copy has all 1,240 rows.", "The shared copy was empty.", False),
        ("priya.nair", "PC-QA-01", "1.2.2", ["formulas"], "medium", False, False, 3 * hour,
         "VLOOKUP over a closed workbook shows #REF! until I open the other file.",
         "The last calculated value stays.", "#REF! in every lookup cell.", False),
        ("vikram", "DDS-SERVER", "1.3.0", ["performance"], "low", True, False, 40 * minute,
         "Opening a 60 MB MTO workbook takes a few seconds longer than before.",
         "Opens as fast as 1.2.2.", "About 4 s instead of 3 s.", True),
    ]
    lines = []
    for i, (u, m, v, feats, sev, after, rolled, ago, desc, exp, act, logs) in enumerate(reports):
        t = now - ago
        lines.append(json.dumps({
            "id": f"fb-{t}-{i}", "time": t, "user": u, "machine": m, "version": v, "os": "Windows 10 Pro 10.0.19045",
            "features": feats, "description": desc, "expected": exp, "actual": act, "severity": sev,
            "afterUpdate": after, "rolledBack": rolled,
            "logs": "12:01:07 open D:\\Projects\\line-list.xlsx\n12:01:44 collab.share sent 412 KB\n12:01:45 server: shared, 0 rows" if logs else None,
        }, ensure_ascii=False))
    os.makedirs(os.path.join(data, "feedback"), exist_ok=True)
    with open(os.path.join(data, "feedback", "feedback.jsonl"), "w", encoding="utf-8") as f:
        f.write("\n".join(lines) + "\n")
    events = [
        ("rahul.mehta", "PC-DRAFT-04", "1.2.1", "updateFailed", 6 * day, "The installer ended with code 1603; the app kept 1.2.0."),
        ("priya.nair", "PC-QA-01", "1.2.2", "recovered", 2 * day, "Started after a forced shutdown; 1 unsaved workbook offered back."),
        ("anita.rao", "PC-ENG-07", "1.2.2", "error", 2 * day + hour, "Sharing: the server's copy did not match; the window stayed open."),
        ("site.office", "SITE-LAPTOP", "1.2.2", "recovered", 12 * day, "Started after the app ended unexpectedly; nothing was unsaved."),
        ("anita.rao", "PC-ENG-07", "1.3.0", "error", 3 * hour, "Add-in folder could not be read: access denied."),
    ]
    with open(os.path.join(data, "health.jsonl"), "w", encoding="utf-8") as f:
        for u, m, v, k, ago, d in events:
            f.write(json.dumps({"time": now - ago, "user": u, "machine": m, "version": v, "kind": k, "detail": d}) + "\n")
    commands = {
        "cell.set": 4210, "edit.copy": 1630, "edit.paste": 1544, "file.save": 902, "home.bold": 412, "edit.undo": 388,
        "home.autoSum": 240, "data.sort": 188, "home.fillColor": 171, "data.filter": 160, "insert.rows": 133,
        "view.freezePanes": 96, "home.mergeCenter": 81, "formulas.traceDependents": 40, "collab.share": 22,
        "data.removeDuplicates": 18, "insert.pivotTable": 9, "data.qualityCheck": 6, "review.showChanges": 3,
        "insert.chart": 2, "data.textToColumns": 1,
    }
    days = {}
    today = datetime.date.today()
    for back in range(0, 45, 3):
        d = (today - datetime.timedelta(days=back)).isoformat()
        share = 1.0 / (1 + back / 10)
        days[d] = {
            "1.2.2": {c: max(1, int(n * share / 8)) for c, n in commands.items() if n * share >= 8 or back == 0},
            "1.3.0": {c: max(1, int(n * share / 20)) for c, n in list(commands.items())[:12]} if back < 9 else {},
        }
    write_json(os.path.join(data, "adoption.json"), {"days": days})


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
        dashboard_data(data)
        for page in ("versions", "releases", "feedback", "health", "adoption"):
            shot(data, page, f"server-{page}.png")
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
