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
  (or Review › Edit History) for the full history of that cell, or of the whole workbook,
  filterable by person: old value → new value, time, user.
- **Nothing to save, nothing to lose.** Every edit is written to the server's disk (and flushed)
  before anyone else sees it. If the network drops, edits are kept and sent when it comes back
  (the status bar says "Offline · N edit(s) waiting"). Ctrl+S on a shared workbook keeps a named
  version on the server.
- **Versions.** The server keeps a version every hour while a workbook is being edited, whenever
  someone presses Save, on request (Review › Versions › Keep version), and before any restore.
  Any version can be opened as a separate copy or restored for everyone.
- **Undo is yours.** Undo takes back your own edits, never someone else's.
- **Roles.** `admin` (everything, including deleting any shared workbook), `editor` (edit, create,
  share; delete only their own workbooks), `viewer` (read only).
- **Share a file from where it is.** Open it and click Share › Share with my team. The file is
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

1. Pick an always-on PC on the office network. Copy `DexSheetsServer.exe` and
   `Install-Server.ps1` to it.
2. In an elevated PowerShell in that folder:
   `powershell -ExecutionPolicy Bypass -File .\Install-Server.ps1`
   It creates the first administrator, opens TCP 47810 / UDP 47811 for the Domain and Private
   firewall profiles, and registers a startup task so the server runs after every reboot.
3. Add teammates:

   ```
   & "C:\Program Files\Dexterity Sheets Server\DexSheetsServer.exe" --data "C:\ProgramData\Dexterity Sheets\Server" user add anita --display "Anita Rao" --role editor
   ```

   Other commands: `user list`, `user passwd NAME`, `user role NAME admin|editor|viewer`,
   `user disable NAME`, `user enable NAME`. Changes apply at the person's next sign-in.
4. **Back up** `C:\ProgramData\Dexterity Sheets\Server` with the office's normal backups.

### What is on the server's disk

```
users.json                    accounts (salted PBKDF2 keys; passwords are never stored)
server.log                    sign-ins, workbooks opened/created/deleted, errors
books/<id>/book.json          name, creator, dates
books/<id>/snapshot-<n>.xlsx  the workbook as of edit n (ordinary .xlsx files Excel opens)
books/<id>/log.jsonl          every edit, in order (replayed after the newest snapshot)
books/<id>/audit.jsonl        one line per changed cell: who, when, old → new
books/<id>/versions/*.xlsx    kept versions (ordinary .xlsx files)
trash/                        deleted workbooks
```

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

- Each client compares its workbook with the last state it synchronised and sends the difference
  as a **patch** of absolute assignments: cells (value, formula, the style itself), top-level
  sheet and workbook fields (object fields such as column widths key by key), and the sheet list.
  Because patches are found by comparing workbooks, every way of changing a workbook (commands,
  paste, undo, fill, sort, the control channel) is shared without per-command code.
- The server gives every patch a sequence number, appends it to `log.jsonl` (fsync), records the
  audit lines, then broadcasts it. Each client shows "server state + its own unacknowledged
  patches", so all replicas converge on the server's order; a conflicting edit to the same cell
  is resolved last-writer-wins in that order.
- Remote patches are also applied to each client's undo/redo snapshots, so Undo never reverts
  someone else's edit.
- Values that are only recalculated are not attributed to anyone in the audit trail.
- A snapshot (.xlsx) is written every 300 edits and on version saves; the server's own copy is
  re-read from the snapshot so that "snapshot + later edits" is exactly what every client has.
