# Dexterity Sheets with the office's other apps

Dexterity Sheets works with the other Dexterity office apps: Dexterity Connect (the office
messenger), Slate (the text editor), Sleuth (file search) and Project Opener. Each link stays out of
the way until it is wanted. A menu item shows only when its app is installed on the PC.

The code is in `crates/ui-egui/src/dex_ui/suite.rs`. Its tests are in
`crates/ui-egui/tests/it/dex_suite.rs`.

## What a person sees

| Where | Item | What it does | Shown when |
|---|---|---|---|
| Title bar › Share | **Share to Dexterity Connect** | The Share menu's own item (since 1.6.4, shared spreadsheets): opens Connect's share window with the spreadsheet's name and its link. The command `suite.shareToConnect` builds the same kind of link with the selected cell or range, for scripts and the command search. | A shared spreadsheet is open |
| Title bar › Share, cell menu | **Discuss This Row in Connect…** | The same, but the message also carries the active row as `header: value` lines. The headers come from the first used row of the sheet, or from the table's header row when the cell is in a table. At most 12 pairs are sent, each value cut to 80 characters. | Connect is installed |
| Cell menu | **Copy Link to This Cell** | For a shared workbook, copies `dexterity://open/<server>/<book>?at=<Sheet>!<Cell or range>`. For a file that is not shared, copies the file's path and the cell, and says that only sharing gives a link that opens at the cell. | The workbook is saved or shared |
| Status bar, bottom right | **Notes** | Opens the workbook's notes in Slate. Without Slate, it opens them in a small notes pane in the app. | A workbook is open. A new workbook nobody has saved says on hover why it has no notes yet |
| Cell menu | **Add Note in Slate for This Row** (**Add Note for This Row** without Slate) | Adds a heading to the notes with the place (`Sheet1!B4`) and the time, then the row's values and a link back to it. | The workbook is saved or shared |
| Cell menu | **Find "…" in Sleuth** | Searches the office's files for the cell's text: `sleuth://search?q=<text>`. The text is cut to 200 characters, the most Sleuth reads. | Sleuth is installed and the cell has text |
| Cell menu | **Open in Project Opener** | Opens the project the cell names. See *Project Opener* below. | Project Opener is installed and the text names exactly one project |
| Cell menu, or a click on a link | **Open Link in …** | Opens a link in the cell whose address starts with `dexterity:`, `dexconnect:`, `sleuth:`, `datum:` or `projectopener:`. The link can be an Insert › Link hyperlink or the first argument of a `HYPERLINK` formula. | The cell holds such a link |

## Links to a cell (`?at=`)

`dexterity://open/<server>/<book>?at=<Sheet>!<place>` opens the shared workbook, goes to its
sheet and selects the place, which can be a cell (`B4`, `$B$4`) or a range (`A1:C9`).

- **Sheet names.** The sheet name is percent-encoded, so `Line List` becomes `Line%20List` and
  `O'Brien` becomes `O%27Brien`. A sheet written the Excel way is read as well, for example
  `'Line List'!B4` or `'O''Brien'!A1`.
- **Damaged places.** If the place is damaged (bad escapes, characters a sheet name cannot have,
  or something that is not a cell or range), the workbook still opens, just without going to a
  cell.
- **Comments.** The Comments pane opens only when the cell has a comment, which is the case for
  an @mention's link. A plain link to a cell just goes there.

## Connect

`dexconnect://share?from=Dexterity%20Sheets&title=<book — sheet>&text=<row>&link=<link>`. Every
value is percent-encoded, and empty parts are left out.

- **Nothing is sent by itself.** Connect shows the message and sends it only when the person
  clicks Share.
- **Workbooks that are not shared.** For a saved workbook that is not shared, the link is the
  file's path. The message then also says *Local file (not shared), Sheet1!B4. Only people who
  can reach this folder can open it.*

## Notes

| Workbook | Notes file |
|---|---|
| Saved anywhere (a PC, a project folder, the network drive), shared or not | `<workbook folder>\<workbook name>.notes.md`, beside the file, so the notes go wherever the file is kept |
| Shared on the office server with no file on this PC (someone else shared it) | `%APPDATA%\Dexterity Sheets\Notes\<server>\<book id>.notes.md`, which is personal to this Windows account on this PC |
| New, never saved | none yet. The Notes button says on hover to save the workbook first |

How the notes are opened and kept safe:

- **Starting the file.** Slate only opens a file that already exists, so Dexterity Sheets writes
  the file first, starting with `# Notes: <workbook>`. It is written through a temporary file, so
  a failed write never leaves half a file.
- **Finding Slate.** The app looks for `slate.exe` in the same places Dexterity Connect does:
  `%LOCALAPPDATA%\Programs\Slate\` (Slate's per-user installer), then `%ProgramFiles%\Slate\`.
- **The notes pane.** Without Slate, the notes open in a pane in the app. The pane saves with
  **Save**, and saves on close.
- **Changes from other programs.** If another program changed the file since the pane read it,
  saving is refused. The pane keeps your text and offers **Reload**, so the other program's change
  is never overwritten.
- **Size limit.** A notes file over 8 MB is not opened.

## Project Opener

Project Opener's only link is `projectopener://open/<project id or exact name>`. To turn a cell's
text (`P12513`, a job number) into a project, Dexterity Sheets reads Project Opener's own
`%APPDATA%\DexterityDesign\ProjectOpener\workspace.json`. It checks the file at most every 3
seconds, and only when the file has changed.

The text names a project when it matches one of these:

- the project's job number, ignoring case;
- the project's whole name, ignoring case;
- the start of exactly one project's name, followed by a space (`P12513` for *P12513 As-built
  isometrics*).

When nothing matches, or when more than one project matches, the item is not shown. Nothing is
guessed.

## What is not offered, and why

- **Look up in Datum.** Datum's links name an internal component id or sector code, and the
  path segments may only be letters, digits, `-` and `_`. Datum has no search link, so a cell's
  text cannot be looked up. `datum:` links in cells still open, see the table above.
- **Send via Hopper.** Hopper's `hopper://` scheme is internal to Hopper. Its only public entry is
  `Hopper.exe --open <folder>`, which opens a folder and does not send anything. There is no
  public way to start a transfer.

## How the apps are detected

The apps are detected once per run, on a background thread, so the window never waits:

- **Protocol apps.** `reg query HKCR\<scheme> /v "URL Protocol"` is run for `dexconnect`,
  `sleuth`, `datum` and `projectopener`. HKCR covers both per-user and per-machine
  registrations.
- **Slate.** Slate has no protocol, so it is found by its install folder.
- **Until the check finishes,** no suite items are shown.
- **Tests and other hosts** set the installed apps with `suite::set_installed`. Only the desktop
  app (`dex_ui::enable_desktop`) looks at the registry, starts apps or hands links to Windows.
  Elsewhere the addresses are only recorded, in `collab.suite.opened`.

## Safety

- **Only the office apps' links.** Only `dexterity`, `dexconnect`, `sleuth`, `datum` and
  `projectopener` addresses are handed to Windows. Explorer opens them, so no command shell parses
  them. Any other scheme in a cell is refused, as before; http, https and mailto keep upstream's
  Open Hyperlink prompt.
- **Inputs are checked and limited.** Addresses are checked for control characters, the Sleuth
  text and the row summary are cut to size, and files over 8 MB are not read.

## Commands

These are UI-only commands, handled in `ui-egui`. They are available from the control channel,
the CLI and MCP like any other command.

| Command | Parameters | Result |
|---|---|---|
| `suite.apps` | none | `{connect, sleuth, datum, projectOpener, slate}` |
| `suite.copyCellLink` | none (the selection) | `{link, cellLink}` |
| `suite.shareToConnect` | `{row?: bool, text?}` | `{url}` |
| `suite.discussRow` | `{text?}` | `{url}` |
| `suite.findInSleuth` | `{text?}` (default: the active cell's text) | `{url}` |
| `suite.openInProjectOpener` | `{text?}` | `{url, project}` |
| `suite.openLink` | `{url?}` (default: the active cell's link) | `{url}` |
| `suite.notes` | none | `{path, in: "slate" \| "pane"}` |
| `suite.noteForRow` | none (the active row) | `{path, in}` |
| `suite.notesPane` | `{text?, save?, close?}` | `{path, text, saved}` |

## Hooks into upstream files

These are registered in `packaging/dexterity/check_overlay.py` and
`packaging/dexterity/reapply_overlay.py`:

- `tabs.rs`: `crate::dex_ui::suite::notes_button(app, ui, &t);`, the Notes button at the far
  right of the status bar.
- `grid/menus.rs`: `close |= crate::dex_ui::suite::cell_menu(app, ui);`, after upstream's cell
  menu items.
- `grid/interact.rs`: the first line of `follow_link` hands suite links to `suite::follow_link`.
- `tests/it/main.rs`: `mod dex_suite;`.

The Share menu items, the state, the notes pane and the command dispatch are in Dexterity's own
files (`collab_ui.rs`, `dex_ui.rs`, `dex_ui/share.rs`).
