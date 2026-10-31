# Shared workbooks on the office network

Dexterity Sheets lets several people edit the same workbook at the same time over the office
LAN. Any workbook can be shared from wherever it is saved (a folder on your PC, a project folder,
the network drive), and that file keeps everyone's edits. One PC runs **Dexterity Sheets
Server**, which passes edits between PCs and keeps the history; everyone signs in to it from the
app.

## What you get

- **Live editing.** Everyone sees everyone else's edits as they happen (typically well under a
  tenth of a second on a LAN). Other people's selected cells are outlined in their colour with
  their name; "(typing)" shows while they are editing a cell.
- **Named accounts only.** There is no anonymous or guest access. Every edit is stored with the
  account that made it.
- **Who changed what.** The status bar shows who last changed the active cell and when; click it
  (or Review › Edit History, also reached as Excel's Review › Show Changes) for the full history of that cell, or of the whole workbook,
  filterable by person: old value → new value, time, user.
- **Nothing to save, nothing to lose.** Every edit is written to the server's disk (and flushed)
  before anyone else sees it. If the network drops, edits are kept and sent when it comes back
  (the status bar says "Offline · N edit(s) waiting"). Ctrl+S on a shared workbook keeps a named
  version on the server.
- **Versions.** The server keeps a version every hour while a workbook is being edited, whenever
  someone presses Save, on request (Review › Versions › Keep version), and before any restore.
  Any version can be opened as a separate copy or restored for everyone.
- **Undo is yours.** Undo takes back your own edits, never someone else's.
- **The status bar says where your edits are.** *Shared · saved on the server* only once the server has
  written every edit of yours to its disk; *saving N edit(s)* while they are on their way; *Offline · N
  edit(s) not on the server yet* while the connection is down (they are sent when it is back); *out of
  step, reloading* when the server refused an edit or this copy missed one and it is fetched again;
  *Sharing · uploading* for a workbook being shared, until the server's copy is back.
- **Two people, one cell, same moment.** The edit the server receives later is kept, on every PC alike,
  and both people are told who changed the cell and which value was kept. The replaced value stays in
  the cell's Edit History. Edits to different cells never conflict.
- **Roles.** `admin` (everything, including deleting any shared workbook), `editor` (edit, create,
  share; delete only their own workbooks), `viewer` (read only).
- **Who may open it.** Share › Share… (or Who has access…) chooses: **everyone at the office**
  or **specific people**, each as an **Editor** or a **Viewer**; an optional end date after which
  only the creator and administrators can open it; and whether viewers may save a copy or export
  it. The server checks access on every open and every edit; a person removed while it is open
  sees it close. A view-only account never edits, whatever a workbook allows. "Allow downloading"
  is enforced by the app (Save As and Export are refused for a viewer); nothing stops someone
  copying cells by hand or taking a screenshot.
- **Links.** Share › Copy link gives `dexterity://open/<server>/<id>`: paste it in a chat or an
  e-mail; clicking it opens the workbook in Dexterity Sheets (installed with the app), which
  checks the reader's own access, so the link itself grants nothing. A second copy of the app hands
  the link to the one already running. In a browser, `http://<server>:47810/open/<id>` opens the
  app or offers the installer from the office server (plain HTTP on the office network; it names
  no workbook). Links work whether the workbook was shared from a PC's own disk or from a network
  drive: the content comes from the office server, so the reader needs no access to the folder.
- **Notifications.** Windows tells you, under the Dexterity Sheets name and icon, when a
  spreadsheet is shared with you (with your permission), when your access changes or is removed,
  when others changed a shared spreadsheet while you were away (one summary per workbook at
  sign-in, never one per edit), when a new version is ready, and when unsaved work can be
  recovered. Clicking one opens the spreadsheet. Notices for someone who is not signed in wait on
  the server (`notices/<user>.jsonl`). The status bar's **Notifications** lists them; its
  settings turn notifications off, pick the kinds, mute single workbooks, or collect them into one
  summary an hour. Not done yet: @mentions in comments.
- **Templates.** File › New lists the office's approved templates (from the server's
  `templates/` folder) and your own (`%APPDATA%\Dexterity Sheets\Templates`). Creating from a
  template makes a new, unsaved copy; a template's next version never changes it. Administrators
  publish (Templates window › Save as template › Office library), give a category, a description
  and optionally what it is required for, and remove templates (kept in `trash/templates`).
- **Automation rules.** Share › Automation rules… on a shared workbook (its creator or an
  administrator). The server keeps them in `books/<id>/automation.json` and runs them itself:
  on a schedule (every day or one weekday, at a time on the server's clock, checked every half
  minute) or when a cell in a column of the first sheet becomes a value. Steps: Quality Check,
  overdue items (a date column past today while a status column is not the "done" value), a
  report (a new shared workbook only the notified people and the creator can open), a completion
  time written in the changed row (an edit by "automation", which never triggers rules), and
  notifying people. Run now tries a rule at once.
- **Share a file from where it is.** Open it and click Share › Share…. The file is
  not moved: the sharer's app saves everyone's edits into it about a second after each change
  (Ctrl+S saves at once and keeps a version), so the file in the folder is always the team's
  latest. `.xlsx`, `.xlsm`, `.csv` and `.tsv` are written back; an `.xls` or `.ods` can be shared
  but is left untouched (Save As .xlsx keeps the edits as a file).
- **Opening a shared file joins it.** Someone who opens the same file (the same path on the same
  PC, or the same file on a network share, whichever drive letter it is mapped to) joins the
  live workbook instead of editing a copy. The sharer reopening their file rejoins it and the
  file catches up with what the team did meanwhile.
- **Stop sharing.** Share › Stop sharing (keep my file) writes every edit to the file and ends
  the sharing; the server's copy goes to its trash folder, and everyone else is told and keeps a
  copy in their window.
- **Your file is never overwritten by surprise.** If another program saves over a shared file,
  the app stops writing to it and says so; everyone's edits stay in the shared workbook. The
  app remembers what it last wrote (`%APPDATA%\Dexterity Sheets\linked-files.json`), so a file
  changed while it was closed is not replaced on rejoining either: both are opened side by side.
- **Excel files in and out.** Use File › Save As to send an .xlsx copy to a client.

## Setting up the server (once, by IT)

No commands are needed; anyone comfortable installing a program can do it.

1. Pick an always-on PC on the office network. Download **`DexSheetsServer-Setup.exe`** from the
   latest release and run it (Windows asks for administrator rights once). It installs:
   - the server as the Windows service **Dexterity Sheets Server**: starts with Windows, runs in
     the background without anyone signed in, and restarts itself if it ever stops;
   - Windows Firewall rules for TCP 47810 (workbooks) and UDP 47811 (finding the server), for the
     Domain and Private profiles only, never Public;
   - the **Dexterity Sheets Server Manager** (Start menu), which opens when setup finishes;
   - the data folder `C:\ProgramData\Dexterity Sheets\Server`, readable only by SYSTEM and
     Administrators because it holds the accounts.
   An older script-based install (`Install-Server.ps1`) is replaced cleanly, data included.
2. In the Server Manager, create the first administrator.
3. Add the team on **People** (one at a time) or **Add many people**:
   - type them into the list, or paste rows copied from Excel (User name, Full name, Role,
     Password);
   - or click **Download template**, fill in the *People* sheet (one person per row; the *How to
     fill this in* sheet explains each column), and **Import from file** (`.xlsx`, `.xls`, `.ods`,
     `.csv` or `.tsv`). Columns can be in any order; "Name", "Login", "Access" and similar
     headings are recognised;
   - every row is checked first: user name rules, duplicates in the list or on the server, the
     role, password length. Nothing is created until every row is right. An empty user name is
     made from the full name (`anita.rao`); an empty password becomes a readable one;
   - after **Create**, save the **sign-in sheet** (an `.xlsx` with each person's user name,
     password and the server name) to hand out, and delete it once everyone has signed in.
4. Keep backups: **Backup and restore › Back up** writes one `.zip` with everything. Restoring
   stops the server for a moment and keeps the data it replaces beside it.

The Server Manager also starts, stops and restarts the server, shows its log, the addresses
teammates can type, the firewall state (with a button to open it), resets passwords (shown once),
changes roles, disables or removes people (never the last administrator), and manages app updates
(below). Unattended installs: `DexSheetsServer-Setup.exe /quiet` or
`msiexec /i DexSheetsServer-<version>-x64.msi /qn`. The same account commands are available
for scripts: `DexSheetsServer.exe user add|passwd|role|disable|enable|list` (run `--help`).

### What is on the server's disk

```
users.json                    accounts (salted PBKDF2 keys; passwords are never stored)
server.log                    sign-ins, workbooks opened/created/deleted, errors
books/<id>/book.json          name, creator, dates
books/<id>/snapshot-<n>.xlsx  the workbook as of edit n (ordinary .xlsx files Excel opens)
books/<id>/log.jsonl          every edit, in order (replayed after the newest snapshot)
books/<id>/audit.jsonl        one line per changed cell: who, when, old → new
books/<id>/versions/*.xlsx    kept versions (ordinary .xlsx files)
books/<id>/automation.json    the workbook's automation rules
trash/                        deleted workbooks
updates/                      app installers offered to the PCs, their notes, releases.json
update-source.json            where new versions are fetched from (absent = the official releases)
templates/                    the office's approved templates
notices/<user>.jsonl          notifications waiting for people who were offline
clients.json  feedback/  health.jsonl  adoption.json   the Server Manager's dashboard data
```

### App updates and releases

- From the moment it runs, the server fetches each new release of Dexterity Sheets from the
  official GitHub releases (`vikramdex-ops/dexterity-sheets`), checks it against the release's
  checksums and puts the installer in `updates/`. Every signed-in app hears of it at once (and asks
  every four hours); the status bar shows **Update to X.Y.Z**. The Server Manager's **App updates**
  page changes the repository (a private one needs a token), turns fetching off, or takes an
  `.msi` added by hand for a server without internet.
- **Releases** (Server Manager › Dashboard) gives each version a channel (stable or testing) and a
  status (OK, known issue, deprecated, blocked) with a note, and sets the policy: **Offer** (the
  default), **Install on their own** when nothing is unsaved, or **Never offer** (people use
  Automate › Check for Updates). `docs/dexterity-admin-data.md` has the file formats.
- A PC that is not signed in to any server reads the GitHub releases page itself: a few seconds
  after it starts, once a day, and on Check for Updates. Only that repository is read and every
  download is checksummed; the switch is in the updates dialog.
- **Roll back**: Automate › Roll Back Version lists the earlier versions the server holds and those
  on the releases page; installing one never touches workbooks, shared workbooks or their versions.

## Using it (everyone)

1. Open Dexterity Sheets → **File › Shared** (or Review › Shared Workbooks).
2. The server appears under "Servers on this network" (or type its computer name). Enter your
   user name and password and **Sign in**. The app remembers the server and user name.
3. Open a workbook from the list, create a new one, or share the workbook you have open.

## Security notes

- Passwords never cross the network: sign-in is a challenge-response over a salted PBKDF2 key.
- Workbook contents travel unencrypted on the LAN (like an SMB share without encryption). Do not
  expose port 47810 to the internet. Encrypting the link (TLS) is a possible follow-up.
- The server delays its answer to every failed sign-in and logs it in `server.log`.

## How it works (for maintainers)

Code: `crates/collab` (protocol, patches, client, server), `crates/ui-egui/src/collab_ui.rs`
(window integration), `apps/gridcraft-server` (the server executable).

- **Sharing a workbook.** Share first enters whatever is being typed in a cell, then uploads the
  workbook (.xlsx, with its SHA-256; the server refuses a damaged upload). The server writes it as
  `snapshot-0.xlsx` and sends its own copy back (with the snapshot's SHA-256, checked on arrival). The
  sharer's app compares that copy with what it uploaded, sheet by sheet and cell by cell (constants and
  formulas); only if nothing is missing does the shared workbook take the window's place, with anything
  typed meanwhile carried over as an edit. A download that is not a workbook is reported, never opened
  as a blank one. Regression tests: `crates/collab/src/tests.rs` (server files read back from disk, a
  second person, both-way edits, concurrent edits, a crash and restart, viewers, no sign-in) and the
  blank-share flow with real windows in `packaging/dexterity/tests/lan_e2e.py`.
- Each client compares its workbook with the last state it synchronised and sends the difference
  as a **patch** of absolute assignments: cells (value, formula, the style itself), top-level
  sheet and workbook fields (object fields such as column widths key by key), and the sheet list.
  Because patches are found by comparing workbooks, every way of changing a workbook (commands,
  paste, undo, fill, sort, the control channel) is shared without per-command code.
- The server gives every patch a sequence number, appends it to `log.jsonl` (fsync), records the
  audit lines, then broadcasts it. Each client shows "server state + its own unacknowledged
  patches", so all replicas converge on the server's order; a conflicting edit to the same cell
  is resolved last-writer-wins in that order. Each patch carries the sequence number the client had
  seen (`base`); a cell another client changed after it is reported as a conflict to both, in the
  broadcast edit itself.
- Remote patches are also applied to each client's undo/redo snapshots, so Undo never reverts
  someone else's edit.
- Values that are only recalculated are not attributed to anyone in the audit trail.
- A snapshot (.xlsx) is written every 300 edits and on version saves; the server's own copy is
  re-read from the snapshot so that "snapshot + later edits" is exactly what every client has.
