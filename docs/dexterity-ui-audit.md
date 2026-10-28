# Dexterity Sheets UI audit

> **Audited:** 2026-10-28 · **Build:** `main` after `40e3b3d9` (dev profile) · **Rule:** `RULES.md`
> §1.3, "every screen is real"

Every screen of the app and of the Server Manager was opened and looked at, to find controls that
do nothing, placeholder or "coming soon" text, dummy data, dead ends, and UI that is needed but
missing. Kept current: a change that adds or alters a screen updates its row.

## How it was checked

| Method | What it covers | Re-run |
|---|---|---|
| Control channel driver | Opens each ribbon tab, Backstage page, dialog and task pane in a running app (`--sample sales --control`), reads `ui.inspect`, saves a screenshot of each; the screenshots were then looked at one by one | `python -I D:\GridCraft\ui-audit\drive.py target\debug <out>` (kept outside the repository) |
| Offscreen snapshots | The new ribbon controls, Focus Cell and validation circles with real data | `cargo run -p gridcraft-ui-egui --example snapshot -- --sample sales --tab Data --cmd 'data.circleInvalid={}' out.png` |
| Server Manager `--shot` | Server, People, Add many people, App updates, Backup and restore, on a throw-away data folder | `python packaging/dexterity/tests/server_manager_shots.py target/release <out>` plus `--shot backup` |
| Static scans | Placeholder wording in shipped UI strings; ribbon buttons whose click is never handled; engine dialog requests the UI does not know; commands fired from controls whose result is thrown away | `docs_check.py` (placeholder words); the scans are described in the findings below |
| UI tests | Each fix below has a test in `crates/ui-egui/tests/it/dex_parity.rs`; key tips are regenerated and tested (`cargo xtask keytips`) | `cargo test -p gridcraft-ui-egui --test it dex_parity` |

Not driven by hand this round: right-click context menus, the status bar's own menus and the
sheet-tab menu (covered by upstream's UI tests, `context_menu_stays_open.rs`, `tabs` tests), the
Draw tab's pen strokes, and printing to a real printer.

## Findings and fixes

| Screen | Checked | Found | Fixed |
|---|---|---|---|
| Ribbon: Home, Insert, Draw, Page Layout, Formulas, Data, Review, View, Automate | each tab opened and photographed; every button's click handled (scan of `ribbon/*.rs`) | no dead buttons; no placeholder text | — |
| Data ▸ Data Validation ▸ Circle Invalid Data | run on a sheet with invalid values | **dead**: the command found the cells but nothing was drawn | `07a0f893`: red circles drawn, cleared when the value is corrected; Clear Validation Circles added |
| Formulas ▸ Formula Auditing ▸ Remove Arrows | menu | only "remove all"; Excel's precedent / dependent variants missing | `07a0f893`: split button with all three |
| Data ▸ Outline | buttons | Auto Outline and Clear Outline reserved but missing | `07a0f893` |
| View ▸ Show | buttons | Focus Cell missing | `07a0f893` |
| Command search (title bar, Automate) and Alt, Q | opened from the key tip | the Q key tip was reserved, silent | `40e3b3d9`: opens the search |
| Conditional Formatting from the command search or a key tip | run with no rule | **dead**: the engine asked for a "newFormattingRule" dialog the UI did not have | this change: opens New Formatting Rule |
| Conditional Formatting Rules Manager | dialog | could only reorder and delete; no way to add a rule from it | this change: New Rule… button (Edit Rule: see "Left") |
| Workbook Statistics, Check Accessibility, Error Checking, Evaluate Formula | dialogs | showed the command's raw JSON (`[]`, `{"errors": []…}`, `null`) | this change: readable tables; Error Checking's cells and circular references are links to the cell; a clear line when there is nothing to show |
| Formulas ▸ Watch Window, Review ▸ Show Comments from the command search or key tips | run as commands | **dead from those entry points**: the pane opens only from the ribbon button | this change: the panes open from every entry point |
| Review ▸ Show Changes | key tip R, G and the command | reserved, silent | `07a0f893`: opens Edit History |
| Other dialogs (59 in all: Format Cells, Find and Replace, Go To / Go To Special, Sort, Advanced Filter, Data Validation, Text to Columns, Remove Duplicates, Subtotal, Goal Seek, Solver, Forecast Sheet, Data Analysis, Name Manager, New Name, Paste Name, Paste Special, Insert / Delete cells, Row Height, Column Width, Standard Width, Rename / Move or Copy / Unhide sheet, Protect Sheet / Workbook, Allow Edit Ranges, Insert Hyperlink, Place Picture, Insert Chart, Sparklines, Series, Header & Footer, Page Setup, Zoom, Arrange Windows, Macros, Action Journal, Agent Control, About, Spelling, Insert Function, Calculated Field, Chart Title, Resize Table, Outline Settings) | each opened, photographed, read | real fields and buttons; disabled controls say why ("No names defined" in Paste Name) | — |
| Backstage: Home, New, Open, Shared, Info, Save As, Print, Export, Options, Account | each page opened and photographed | real; "No recent workbooks yet" is an honest empty state | — |
| Task panes: Edit History, Versions, Comments, Watch Window | opened | real; Edit History and Versions explain that they need a shared workbook | — |
| Server Manager: Server, People, Add many people, App updates, Backup and restore | `--shot` of each page, with and without an installed server and accounts | real; Start / Stop / Restart are disabled with the reason shown ("The server is not installed") | — |

## Left, with the reason

| Screen | What | Why not now |
|---|---|---|
| Conditional Formatting Rules Manager | Edit Rule (change a rule in place) | the rule editor upstream is building (`cond_format_rules.rs`) would conflict; New Rule plus Delete covers it meanwhile |
| Page Setup dialog | margins only; page size, orientation, print titles and scaling are on the Page Layout tab rather than in the dialog | works through the ribbon; a full dialog is an upstream GridCraft item (`docs/ui-parity.md`) |
| Move or Copy | the position is typed as a number, not picked from a list of sheets | works; a list is an upstream item |
| Unhide (sheet) | opens even when no sheet is hidden (Excel greys it out) | harmless: OK with no sheet does nothing |
| Action Journal | lists commands as their ids and JSON | it is an automation tool for the Automate tab, meant to be copied into scripts |
| Insert ▸ Screenshot, Equation, WordArt, Object, SmartArt, Signature Line, Timeline | not offered | listed in `docs/dexterity-parity-audit.md` (low priority) |
