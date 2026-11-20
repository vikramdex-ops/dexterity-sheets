# Shared workbooks on the office network

Dexterity Sheets lets several people edit the same workbook at the same time over the office
LAN. Any workbook can be shared from wherever it is saved (a folder on your PC, a project folder,
the network drive), and that file keeps everyone's edits. One PC runs **Dexterity Sheets
Server**, which passes edits between PCs and keeps the history. Each person's app connects to it
on its own, as their Windows account: nobody creates an account or types a password.

## What you get

- **Live editing.** Everyone sees everyone else's edits as they happen (typically well under a
  tenth of a second on a LAN). Other people's selected cells are outlined in their colour with
  their name; "(typing)" shows while they are editing a cell.
- **Named people only.** You are who Windows says you are (`DOMAIN\user`), on a PC the server
  knows. There is no anonymous or guest access. Every edit is stored with the person who made it.
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
- **Dexterity Connect.** Share › Share to Dexterity Connect (Windows, shared spreadsheets) opens the
  office messenger's share dialog (`dexconnect://share?…`) with the spreadsheet's name and its
  `dexterity://` link filled in; the person picks who it is for and sends it there, where the link
  shows as an "Open in Dexterity Sheets" chip. Only that scheme is handed to Windows
  (`collab.shareToConnect`, `{open?: false}` returns the link without opening anything); if the
  messenger is not installed, the message says so.
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
  summary an hour.
- **People.** Review › People lists everyone on the office server: the name they chose, their
  Windows account, their colour, online or offline, and the shared spreadsheets they have open
  (only those you may open yourself; nobody sees anyone's devices or keys). The server sends the
  list at sign-in and again whenever someone comes, goes, opens or closes a spreadsheet. It is
  also the list @mentions and Assign To pick from.
- **@mentions.** Typing `@` in a comment (Review › New Comment), a reply in the Comments pane, a
  note, or the text of a cell (in the cell or the formula bar; never in a formula) lists the people
  on the office server; picking one writes `@Display Name` (↑/↓, Enter or Tab, or a click; Esc
  closes the list). Mentions are plain text, so they survive saving as .xlsx. The
  server finds mentions that are new in an edit (a new comment, reply or note, a cell whose text
  names someone, or a name added to an edited one; moving, copying, resolving or deleting tells
  nobody again) and sends each person a
  notice with the sheet, cell and text (`crates/collab/src/mention.rs`). Clicking it (or the
  notification's **Go to cell**) opens
  the workbook at that cell with the Comments pane, through a link that carries the cell:
  `dexterity://open/<server>/<id>?at=<sheet>!<cell>`. Someone signed out is told at sign-in.
  Nobody hears about their own comment, and a person who can't open the workbook is not told:
  the author gets a notice naming them instead, so they can share it. Mentions only reach people
  from a shared workbook; in one that isn't shared, the suggestion list says nobody will be told.
  Each mention is also kept for the person in `mentions/<user>.jsonl` (the newest 500 to 1,000),
  which the read API below answers from.
- **Assign To.** Review › Assign To gives the selected cell or range of a shared workbook to
  someone (anyone who may edit it may assign; the person must be able to open it), with an
  optional due date (YYYY-MM-DD), a note of what to do and a status (open or done). The server
  keeps them in `books/<id>/assignments.json`. The person gets a notice and a Windows notification
  with **Go to cell**; **Review › Assigned to Me** lists their items in every shared workbook
  (open first, overdue in red) with Go to cell and Mark done (or Reopen). Whoever assigned it is
  told when it is done. Only the assignee, who assigned it, the workbook's creator or an
  administrator changes its status.
- **Connect quick panel.** The **Connect** button at the left of the status bar (with the number
  of open assignments) shows your recent mentions and assignments with Go to cell, opens People
  and Assigned to Me, and has **Discuss in Connect**: it opens Dexterity Connect's share dialog
  (`dexconnect://share?from=Dexterity%20Sheets&title=<book>&text=<Sheet>!<Cell> in <book>&link=<link>`)
  with a `dexterity://open/<server>/<id>?at=<Sheet>!<Cell>` link to the current cell. Nothing is
  sent until you press Share in Connect.
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
   - Windows Firewall rules for TCP 47810 (workbooks), TCP 47812 (the read API for Dexterity
     Connect, below) and UDP 47811 (finding the server), for the Domain and Private profiles only,
     never Public;
   - the **Dexterity Sheets Server Manager** (Start menu), which opens when setup finishes;
   - the data folder `C:\ProgramData\Dexterity Sheets\Server`, readable only by SYSTEM and
     Administrators because it holds the people and their PCs' device keys.
   An older script-based install (`Install-Server.ps1`) is replaced cleanly, data included.
2. There are no accounts to create. Open Dexterity Sheets on your own PC first: the first person
   to connect to a new server becomes its administrator. Everyone after that is an editor until
   an administrator changes their role.
3. When someone uses a second PC (a laptop, a new desktop), that PC waits for approval. Their app
   shows "Waiting for an administrator to approve this PC — device code ABCD-EFGH"; the Server
   Manager's **People and devices** page lists it, highlighted. Approve it when the person tells
   you the same code. Revoke a PC that is lost, sold or no longer theirs: it is disconnected within
   half a minute and can't connect again.
4. Keep backups: **Backup and restore › Back up** writes one `.zip` with everything. Restoring
   stops the server for a moment and keeps the data it replaces beside it.

The Server Manager also starts, stops and restarts the server, shows its log, the addresses
teammates can type, the firewall state (with a button to open it), lists everyone who has
connected with their PCs (device code, PC name, last seen; approved, waiting or revoked), changes
roles, approves and revokes PCs, removes people (never the last administrator), and manages app
updates (below). Unattended installs: `DexSheetsServer-Setup.exe /quiet` or
`msiexec /i DexSheetsServer-<version>-x64.msi /qn`; setup asks for no account. The same people
commands are available for scripts: `DexSheetsServer.exe profile list`,
`profile role DOMAIN\user admin|editor|viewer` (also before the person has connected),
`profile approve CODE`, `profile revoke CODE` and `profile remove DOMAIN\user` (run `--help`).
A running server applies them within half a minute.

**Moving from user names and passwords.** A server that had accounts (`users.json`) carries them
over at its first start with this version and keeps the old file as `users.json.bak`. Each old
account passes, with its name, role, colour, workbooks, access lists and waiting notices, to the
first Windows account with the same user name that connects (any domain, any case: `vikram`
becomes `DDS\Vikram`). A disabled account stays out. **An administrator's account is not taken
over by name alone:** any domain's account with that user name would match it, so that person's
PC waits for approval (the app shows its device code) and signs in as the administrator only
once it is approved. Approve it in the Server Manager's **People and devices** on the server PC,
which edits `profiles.json` directly and so works even when no administrator is signed in (the
Server page says an administrator is waiting), or from another administrator's app. Refuse it if
the code does not match, and remove that person. Carried-over editors and viewers are let in at
once, as before. Until then the Server Manager lists them
under *Carried over from the old accounts list*. Apps older than this version can't sign in any
more: they are told "This server needs Dexterity Sheets X.Y.Z or newer — update from Help ›
Updates", and Help › Updates still brings them the new version from the server.

### What is on the server's disk

```
profiles.json                 people (Windows account, name shown, role, colour) and their PCs'
                              public device keys, codes and approval; no passwords
users.json.bak                an older server's accounts list, kept after it was carried over
server.log                    sign-ins, workbooks opened/created/deleted, errors
books/<id>/book.json          name, creator, dates
books/<id>/snapshot-<n>.xlsx  the workbook as of edit n (ordinary .xlsx files Excel opens)
books/<id>/log.jsonl          every edit, in order (replayed after the newest snapshot)
books/<id>/audit.jsonl        one line per changed cell: who, when, old → new
books/<id>/versions/*.xlsx    kept versions (ordinary .xlsx files)
books/<id>/automation.json    the workbook's automation rules
books/<id>/assignments.json   cells and ranges assigned to people (due date, open or done)
mentions/<user>.jsonl         where each person was mentioned (the read API's answers)
api-tokens.json               read-API tokens: name, dates and SHA-256 only, never the token
connect.json                  the Dexterity Connect hub, its integration token and certificate
connect-outbox.json           notices waiting to be posted to Connect
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
  checksums and puts the installer in `updates/`. Every connected app hears of it at once (and asks
  every four hours); the status bar shows **Update to X.Y.Z**. The Server Manager's **App updates**
  page changes the repository (a private one needs a token), turns fetching off, or takes an
  `.msi` added by hand for a server without internet.
- **Releases** (Server Manager › Dashboard) gives each version a channel (stable or testing) and a
  status (OK, known issue, deprecated, blocked) with a note, and sets the policy: **Offer** (the
  default), **Install on their own** when nothing is unsaved, or **Never offer** (people use
  Automate › Check for Updates). `docs/dexterity-admin-data.md` has the file formats.
- A PC that is not connected to any server reads the GitHub releases page itself: a few seconds
  after it starts, once a day, and on Check for Updates. Only that repository is read and every
  download is checksummed; the switch is in the updates dialog.
- **Roll back**: Automate › Roll Back Version lists the earlier versions the server holds and those
  on the releases page; installing one never touches workbooks, shared workbooks or their versions.

### Dexterity Connect

Server Manager › **Dexterity Connect** has two optional parts.

**Read access for Connect (Nib).** *Make a token* shows a new token once (copy it into Connect:
Control room › Integrations › Dexterity Sheets, then Test); the server keeps only its SHA-256 in
`api-tokens.json`. *Revoke* stops a token at once. The read-only API answers plain HTTP with JSON
on TCP **47812** (all interfaces; the installer's firewall rule opens it for the office network;
`DEXSHEETS_API_PORT` moves it), with `Authorization: Bearer <token>` on every request:

| Request | Answer |
| --- | --- |
| `GET /api/v1/health` | `{name, version}` |
| `GET /api/v1/people/{user}/assigned` | `[{book, sheet, cell, title, status, due, link}]` (`due` is `YYYY-MM-DD` or null; `status` open or done) |
| `GET /api/v1/people/{user}/mentions?since=<ISO 8601 or ms>` | `[{book, sheet, cell, from, text, at, link}]` (`at` in ISO 8601, oldest first) |
| `GET /api/v1/books` | `[{id, name, sharer, online: [user], link}]` |

`{user}` is `DOMAIN\user` URL-encoded (`dex%5Cpriya`), matched without regard to case; someone the
server doesn't know gets `[]`. `book` is the workbook's name, `from` the name the mentioning person
shows, `sharer` and `online` Windows accounts. `link` is
`dexterity://open/<server>:<port>/<id>?at=<Sheet>!<Cell>` (the server's computer name and
workbook port). A missing or refused token gets `401`, any other path `404`, a `since` that is not
a time `400`. Requests are read with hard limits (16 KB of headers, no body, 5 seconds), on one
thread per connection, at most 32 at once (`503` past that); at most 500 rows per answer.

**Send notices to Connect.** With the hub's address (`https://<hub>:47900`), an integration token
from Connect (Control room › Integrations) and, for a hub with its own certificate, that
certificate's SHA-256 fingerprint, the server also sends each mention and assignment as a Direct
message to the person in Connect (`POST /api/bot/message`, matched by Windows account), with the
`dexterity://…?at=` link in the text, and keeps a **Sheets pulse** card on everyone's Pulse up to
date (`POST /api/bot/widget`, key `sheets-pulse`: workbooks, people connected, open and overdue
assignments, the busiest workbooks; sent when it changes and at least every five minutes). The
settings are in `connect.json` (readable only by SYSTEM and Administrators, like the rest of the
data folder; the token is never logged). With a fingerprint set, the server checks the hub's
certificate before the token is sent and sends nothing to a hub showing another one. Notices wait in
`connect-outbox.json` while the hub is away or refuses the token, and are tried again (from 30
seconds up to an hour apart; dropped after three days, or at once when Connect knows nobody with
that account). Posting runs on its own thread and never holds up the server. *Turn off* stops it
and forgets the token.

## Using it (everyone)

1. Open Dexterity Sheets. A moment after the window is up it connects by itself, as your Windows
   account, to the server it used last time, or to the only one on the network. Nothing to type.
2. If there are several servers, **File › Shared** (or Review › Shared Workbooks) lists them:
   click **Connect** beside yours (or type its computer name). The app remembers it.
3. File › Shared says "Signed in as *name* (*DOMAIN\user*) on *server*" and shows this PC's
   **device code**. Change the name others see there; **Change server** picks another one.
4. On a PC you haven't used with the server before, it shows "Waiting for an administrator to
   approve this PC — device code ABCD-EFGH". Tell your administrator the code; the app connects
   by itself once they approve it.
5. Open a workbook from the list, create a new one, or share the workbook you have open.

## Security notes

- There are no passwords to steal or reuse. Each PC makes its own Ed25519 key the first time
  (`%LOCALAPPDATA%\Dexterity Sheets\device-key`, protected with Windows data protection for the
  signed-in Windows user; if that is not available, a plain file in the same per-user folder, and
  the app's log says so). At sign-in the server sends a fresh challenge and the app signs it with
  that key; the private key never leaves the PC.
- A person's first PC is trusted the first time it connects; every further PC waits for an
  administrator, who compares its device code with the one the person reads out. The name comes
  from Windows (`USERDOMAIN\USERNAME`), so approve PCs, and make administrators, only for people
  you recognise.
- Workbook contents travel unencrypted on the LAN (like an SMB share without encryption). Do not
  expose port 47810 to the internet. Encrypting the link (TLS) is a possible follow-up.
- The read API (TCP 47812) is plain HTTP too: its token and its answers (names, workbook names,
  assignment and mention texts) cross the office network unencrypted. Keep it off the internet,
  give each consumer its own token and revoke one that may have leaked.
- An administrator account carried over from `users.json` is never adopted by user name alone:
  the PC waits for approval (see *Moving from user names and passwords*).
- The server delays its answer to every refused sign-in and logs it in `server.log`, as it does
  each PC waiting for approval.

## How it works (for maintainers)

Code: `crates/collab` (protocol, patches, client, server; `api.rs` the read API, `tasks.rs`
assignments and the mention log, `connect.rs` the Connect settings and outbox),
`crates/ui-egui/src/collab_ui.rs` (window integration; `dex_ui/people.rs` People, Assign To,
Assigned to Me and the Connect panel), `apps/gridcraft-server` (the server executable;
`connect.rs` posts to the hub).

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
