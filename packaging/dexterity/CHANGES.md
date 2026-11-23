# What changed in Dexterity Sheets

Every change a user could notice gets one line under **Unreleased**, in plain words, in the same
pull request as the change (RULES.md §3; `docs_check.py` fails the build otherwise). Releasing
moves them under the new version; the release notes, the README's "What's new" and the landing
page are made from this file. What each upstream sync brings from official GridCraft is added
here by the sync itself (`sync_upstream.py`: the GridCraft release and upstream's own titles of
its newest changes); the release notes on GitHub list every one of them with its link.

## Unreleased

## 1.7.5 · 23 November 2026

- More fixes and improvements from official GridCraft (still GridCraft 0.19.1): 3 changes, the newest below. The release notes on GitHub list them all.
- From GridCraft: xlsx: drop unresolved/non-workbook XLSB external formulas before remapping
- From GridCraft: xlsx: emit schema-valid externalLink child for missing/other XLSB slots
- From GridCraft: xlsx: emit placeholder externalLink part and r:id for missing XLSB link slots

## 1.7.4 · 21 November 2026

- **Save works in shared spreadsheets again.** Since 1.7.0 (people are Windows accounts), Save in a shared spreadsheet failed with "network or disk error: The system cannot find the path specified. (os error 3)", and the server's hourly versions were not kept either. The office server needs this version for the fix.
- **You are told when a spreadsheet is shared with you.** Sharing with everyone at the office (the default) now sends each person a Windows notification with **Open Spreadsheet**, as sharing with named people does; someone not connected is told at their next sign-in. Choices made in the Share dialog are sent with the workbook, so each person is told once.
- **Watch teammates type, live.** In a shared spreadsheet, what a teammate types in a cell now shows in that cell as they type, letter by letter, with a caret and their name tag in their colour, before they press Enter, as co-editing works in Slate. It disappears when they press Enter (their edit arrives) or Esc. Python cell code stays private until it is entered.

## 1.7.3 · 20 November 2026

- More fixes and improvements from official GridCraft (still GridCraft 0.19.1): 10 changes, the newest below. The release notes on GitHub list them all.
- From GridCraft: xlsx: preserve external workbook target relationships on XLSB -> XLSX save
- From GridCraft: xlsx: adopt craft-libs#1347 for XLSB external-book formula decoding (craft-libs#778)
- From GridCraft: xlsx: add full workbook import/save/reopen round-trip parity test (craft-libs#778)
- From GridCraft: mcp: add protocol transcript tests for initialize, cancel, and batch
- From GridCraft: i18n(ru): translate 547 more labels, dialog strings and tooltips
- From GridCraft: xlsx: partial sheet retention, trendline metadata pairing, and XLSB formulas/names
- From GridCraft: #2126: use shared XLSB text budget on desktop
- From GridCraft: xlsx: delegate worksheet, drawing, and chart XML to craft-xlsx (storytold/craft-libs#778)

## 1.7.2 · 20 November 2026

- **Sheets notices arrive through Nib:** with a Connect hub set in Server Manager › Dexterity Connect, mentions and assignments now arrive in each person's conversation with Nib in Dexterity Connect, marked **Dexterity Sheets** · Mention or Task, instead of as Direct messages from a separate Sheets contact. A notice is never shown twice, even when the server had to send it again. Nothing to set up again: the same hub and token are used, and a Connect hub that has not been updated yet keeps receiving them as Direct messages. In Connect, an administrator can switch this off per integration (Control room › Integrations › Notices through Nib).

## 1.7.1 · 20 November 2026

- More fixes and improvements from official GridCraft (still GridCraft 0.19.1): 2 changes, the newest below. The release notes on GitHub list them all.
- From GridCraft: xlsx: delegate XLSB record and package parsing to craft-xlsx
- From GridCraft: mcp: delegate server framing to craft-mcp-server

## 1.7.0 · 20 November 2026

- **No more user names or passwords for the office server.** Dexterity Sheets connects by itself as your Windows account, to the server it used last or the only one on the network (File › Shared lists them when there are several, one click each, and has **Change server**). Your first PC is trusted at once; another PC shows "Waiting for an administrator to approve this PC" with a short device code. File › Shared shows who you are, this PC's device code and the name others see, which you can change.
- **Server Manager › People and devices** replaces People and Add many people: everyone who has connected, with their PCs, device codes and last-seen times; approve or revoke a PC, change a role, remove a person. Nobody is created by hand and there are no passwords, sign-in sheets or templates. The first person on a new server becomes its administrator, and the server installer no longer asks for one. For scripts: `DexSheetsServer profile list|role|approve|revoke|remove`.
- **Upgrading an office server:** its accounts (`users.json`) are carried over at the first start and the file is kept as `users.json.bak`; each account, with its role, colour and workbooks, passes to the first Windows account with the same user name. **Every PC must update to this version**: older apps can't sign in any more and are told "This server needs Dexterity Sheets … or newer — update from Help › Updates", and Help › Updates still brings them the new version from the server.
- **Highlight Row/Column** replaces Focus Cell and the Options row highlight: one button at the bottom right of the status bar (by the zoom), View › Highlight on the ribbon or Ctrl+Alt+H turns it on; its popover picks None, Blue, Green, Yellow, Orange, Pink, Purple or Grey and Row, Column or Both. A selection highlights all its rows and columns, the headers show it more strongly, cell fills stay visible through it, it works with frozen panes and while typing, and it is remembered for you. When co-editing, a tick shows each teammate's row and column faintly in their colour.
- **Works with the office's other apps.** Each item shows only when its app is installed on your PC:
- **An old administrator account is no longer taken over by name alone.** When a carried-over account was an administrator, the first Windows account with that user name now waits for approval (device code) before it signs in with the administrator role; approve it in Server Manager › People and devices, which works even when no administrator is signed in, or from another administrator's app. Carried-over editors and viewers are let in as before.
- **People:** Review › People lists everyone on the office server with the name they chose, their Windows account, their colour, whether they are online and which shared spreadsheet they are in (only spreadsheets you may open yourself). It keeps itself up to date.
- **@mentions everywhere:** besides comments and replies, typing `@` in a note or in a cell's text (in the cell or the formula bar) suggests people from the People list. The person mentioned gets a Windows notification with **Go to cell**, which opens the spreadsheet at that cell.
- **Assign To:** Review › Assign To gives the selected cells of a shared spreadsheet to a colleague, with an optional due date (YYYY-MM-DD) and a status (open or done). They are notified with Go to cell; **Review › Assigned to Me** lists their open items in every shared spreadsheet, with Go to cell and Mark done, and whoever assigned it is told when it is done.
- **Connect quick panel:** the **Connect** button at the left of the status bar shows your recent mentions and assignments and has **Discuss in Connect**, which opens Dexterity Connect's share dialog with a link to the current cell.
- **Dexterity Connect can read the office server:** Server Manager › Dexterity Connect makes and revokes tokens for a read-only API on TCP 47812 (people's assignments and mentions, the shared spreadsheets), so Nib can answer "what's assigned to me?". The server installer opens TCP 47812 for the office network; every request needs a token, which is shown once and only its fingerprint is kept.
- **Notices in Dexterity Connect:** with a Connect hub and an integration token set on the same page, mentions and assignments also arrive as Direct messages in Connect, and everyone's Pulse shows a "Sheets pulse" card (spreadsheets, people working, open and overdue assignments). A hub that is away never holds anything up: notices wait and are sent later.

## 1.6.9 · 19 November 2026

- More fixes and improvements from official GridCraft (still GridCraft 0.19.1): 1 change, the newest below. The release notes on GitHub list them all.
- From GridCraft: Preserve OMML import fallback after shared math cutover

## 1.6.8 · 19 November 2026

- More fixes and improvements from official GridCraft (still GridCraft 0.19.1): 6 changes, the newest below. The release notes on GitHub list them all.
- From GridCraft: Shapes: select, resize from every handle and rotate
- From GridCraft: Wire craft-xls and craft-drawingml from craft-libs main
- From GridCraft: Cut over xlsx omml to craft-math
- From GridCraft: xlsx: fix main after the ODS cutover (clippy, empty first sheet, values, package checks)
- From GridCraft: Find and Replace: Undo and Redo reach the sheet while the dialog is open
- From GridCraft: Cut over gridcraft-xlsx ODS reader to craft-ods

## 1.6.7 · 19 November 2026

- More fixes and improvements from official GridCraft (still GridCraft 0.19.1): 23 changes, the newest below. The release notes on GitHub list them all.
- From GridCraft: Drag a selected chart's data outline to change its range (#4389, #4390)
- From GridCraft: XLSX: read, evaluate and write Excel 2010 conditional formats
- From GridCraft: RATE follows Excel's iteration
- From GridCraft: Fix main's build and tests after the recent merges
- From GridCraft: xtask: `parity --check` accepts a CRLF checkout of the checklist
- From GridCraft: xlsx/csv: cap secondary reader allocations
- From GridCraft: Delete separate rows or columns in one pass
- From GridCraft: Create new saved files without group/other write permission

## 1.6.6 · 18 November 2026

- More fixes and improvements from official GridCraft (still GridCraft 0.19.1): 3 changes, the newest below. The release notes on GitHub list them all.
- From GridCraft: Fuzzer: undo checks follow the bounded undo history
- From GridCraft: Flush a saved workbook to disk before replacing the old one
- From GridCraft: Sort: Custom Sort dialog redesign

## 1.6.5 · 17 November 2026

- More fixes and improvements from official GridCraft (still GridCraft 0.19.1): 22 changes, the newest below. The release notes on GitHub list them all.
- From GridCraft: pdf: follow up shared writer cutover with merged-library pin
- From GridCraft: pdf: move object and xref envelope writing to craft-pdf
- From GridCraft: Equation parse errors follow the interface language
- From GridCraft: Status bar statistics, resize tips, sheet notes and macro toasts follow the interface language
- From GridCraft: Equation fallback text in xlsx is drawn at a normal font size
- From GridCraft: xlsx: share SpreadsheetML string reader/writer codec (cutover stage 1)
- From GridCraft: XLSX: a hidden Total Row keeps its calculations
- From GridCraft: XLSX: keep Page Setup's first page number

## 1.6.4 · 17 November 2026

- **Share › Share to Dexterity Connect**: opens Dexterity Connect, the office messenger, with the shared spreadsheet's name and its link filled in; choose who it is for and send. In the message the link opens the spreadsheet in Dexterity Sheets. If the messenger is not installed, a message says so.
- More fixes and improvements from official GridCraft (still GridCraft 0.19.1): 3 changes, the newest below. The release notes on GitHub list them all.
- From GridCraft: Sandbox: test the socket owner ioctls with a real pointer

## 1.6.3 · 14 November 2026

- More fixes and improvements from official GridCraft (still GridCraft 0.19.1): 8 changes, the newest below. The release notes on GitHub list them all.
- From GridCraft: perf: win back formula-entry and XLSX-save time lost since 23829d8c
- From GridCraft: XLSX: warn when a pivot cache can't be read or is too large
- From GridCraft: Keep mid-field quotes when splitting with Text to Columns
- From GridCraft: calc: linear dependency extraction for range-operator chains
- From GridCraft: fix(functions): reject negative fractions in GCD and LCM
- From GridCraft: functions: ACCRINTM accrues zero when issue equals settlement

## 1.6.2 · 13 November 2026

- More fixes and improvements from official GridCraft (still GridCraft 0.19.1): 6 changes, the newest below. The release notes on GitHub list them all.
- From GridCraft: Charts: resolved data and rendering are cached between frames
- From GridCraft: PDF: writer cleanup, q nesting limit, image dedupe, style only on change
- From GridCraft: LINEST, LOGEST, TREND, GROWTH: solve by Householder QR
- From GridCraft: Grid: resizing several selected columns applies the width on release
- From GridCraft: Shift+arrows, Enter and Tab no longer get stuck on a merged cell
- From GridCraft: Insert: WordArt (insert.wordArt)

## 1.6.1 · 13 November 2026

- More fixes and improvements from official GridCraft (still GridCraft 0.19.1): 5 changes, the newest below. The release notes on GitHub list them all.
- From GridCraft: xlsx: .xls data validation from craft-xls
- From GridCraft: xlsx: OPC packages through craft-opc; craft-libs at aa84415b
- From GridCraft: xlsx: encrypted workbooks through craft-offcrypto and craft-cfb
- From GridCraft: xlsx, engine: CSV and SYLK through craft-csv
- From GridCraft: xlsx: read .xls through craft-xls

## 1.6.0 · 12 November 2026

- **Now built on GridCraft 0.19.1** (was 0.18.0): 16 changes from official GridCraft are in this version. The newest are below; the release notes on GitHub list them all.
- From GridCraft: engine: work limits for Advanced Filter, Remove Duplicates, filters, AutoFit, macro footprints; range sizes saturate
- From GridCraft: engine: index merges and bound the page in HTML export and clipboard HTML
- From GridCraft: check, xtask stats: visited directories keyed by (dev, inode); stats reads only regular files
- From GridCraft: Security: one work budget across every call of a lifted function
- From GridCraft: print: bound the print area to the grid; render charts once and budget picture and chart output
- From GridCraft: fix(formula): bound the wrappers stacked on one path of a parsed formula
- From GridCraft: fix(mcp): send the control token only to a loopback address
- From GridCraft: fix(engine): calculated pivot fields may not build arrays from scalars

## 1.5.1 · 11 November 2026

- More fixes and improvements from official GridCraft (still GridCraft 0.18.0): 22 changes, the newest below. The release notes on GitHub list them all.
- From GridCraft: xlsx: share inline dxf number formats within a styles part
- From GridCraft: Save as Web Page: a byte budget for the charts' SVG (an error past it)
- From GridCraft: xls: charge expanded shared-formula text to the import's text allowance
- From GridCraft: pivot: Rank and Running Total computed once per sibling set; accumulator, caption and chart-series bounds
- From GridCraft: xlsx: resolve cell pictures through a per-structure layout and a relationship index
- From GridCraft: engine: Scenario Summary size limit, bounded solver report names, undo history counts comment and link text
- From GridCraft: Row auto-fit: sweep merges once per sheet instead of per row
- From GridCraft: calc: one cumulative work budget per formula, and INDEX decides multi-cell picks from their shape

## 1.5.0 · 11 November 2026

- **Now built on GridCraft 0.18.0** (was 0.14.2): everything official GridCraft released in 0.15, 0.16, 0.17 and 0.18 is in this version, 150 merged changes in all. The main ones are below, in GridCraft's own words; the release notes on GitHub list every change.
- From GridCraft: **Formulas › Function Library** as labelled buttons with category menus, Recently Used, and categories in Insert Function. **Trace Error** draws arrows back to the cell an error starts in.
- From GridCraft: **Solver** gets Options, Load/Save Model, Reset All and the Sensitivity report; the **Analysis ToolPak** gets two-factor Anova, z-Test, Fourier Analysis, Random Number Generation and Sampling; Goal Seek, Solver and Data Analysis have reference boxes you can point into the sheet with.
- From GridCraft: **Insert › Equation** with a LaTeX editor (kept in .xlsx files, printed in PDF); **form controls** (buttons, sliders, spin buttons) with Assign Action and Format Control; data tables beneath charts; every floating object is selected the same way, with eight resize handles.
- From GridCraft: **Python cells** are kept as data: stored, edited and saved as Excel's PY formula (no code runs). A shared spreadsheet carries them to everyone as well.
- From GridCraft: opens **SYLK (.slk)** files and a web page's table saved as .xls; reads cell formatting from .xls files; a workbook keeps its sheet-tab and scroll-bar settings, its R1C1 reference style and "remove personal information on save" through open and save; checkboxes no longer make Excel repair the file.
- From GridCraft: Find and Replace can search only the selected range; **Ctrl+.** moves to the next corner of the selection; the Excel 2003 menu keys work (Alt+I, R; Alt+D, S; …) and every Paste Special option has its Excel access key; a picture can be pasted from the clipboard; drag and drop asks before replacing data; Options can highlight the active cell's row; "Set precision as displayed" is applied in calculation.
- From GridCraft: sheets with a million hidden rows or whole-column conditional formats scroll smoothly again; MEDIAN, PERCENTILE, QUARTILE, LARGE and SMALL, SUM/COUNT/AVERAGE over big ranges, inserting and deleting rows and columns, and charts with many points are faster; many sheet-protection and file-safety fixes.
- **Formulas › Explain Error** explains the new `#PYTHON!` and `#TIMEOUT!` errors, and what `#BUSY!` and `#BLOCKED!` mean for a Python cell.
- **File › Account** names the GridCraft release this version contains (it showed an older number).

## 1.4.1 · 1 November 2026

- Brings in the latest fixes and features of official GridCraft (listed in the release notes on GitHub). Nothing else changed in Dexterity Sheets.

## 1.4.0 · 1 November 2026

- Fixed: opening a `dexterity:` link in a running window through automation (`collab.openLink` over the control channel or MCP) could close that window when a second Dexterity Sheets was open under the same Windows account, losing anything unsaved in it. Only a copy just started with a link now hands it to the one already running.
- **@mentions in comments.** Type **@** in a comment or a reply in the Comments pane and pick a colleague from the list (↑/↓ and Enter or Tab, or click; Esc closes it): their name goes in as **@Name**. When the comment reaches the office server, each person mentioned gets a Windows notification saying who mentioned them, in which spreadsheet and cell, and what the comment says; clicking it opens the spreadsheet at that cell with the Comments pane. Someone signed out is told at their next sign-in. If a person you mention can't open the spreadsheet, you are told so you can share it with them. Mentions only count in shared spreadsheets, and the list says so in one that isn't shared. Turn them off in Notifications › Settings ("I am mentioned in a comment").

## 1.3.3 · 1 November 2026

- Brings in the latest fixes and features of official GridCraft (listed in the release notes on GitHub). Nothing else changed in Dexterity Sheets.

## 1.3.2 · 1 November 2026

- **Updates without an office server**: Dexterity Sheets now also reads its own GitHub releases page, a few seconds after it starts, once a day while it stays open, and whenever you choose **Automate › Check for Updates**. A newer version shows as **Update to X.Y.Z** in the status bar exactly as one from the office server does; the office server's installer is preferred when it has one. Every download is checked against the release's checksum. Turn the GitHub check off in the updates dialog if a PC must not reach the internet.
- Fixed: **Check for Updates** said "You have the latest version" before any answer had come back (and when the PC was not signed in to a server). The dialog now says what the office server and the GitHub releases page each answered, and when.
- Fixed: **Update now** and **Roll Back** started the installer while workbooks still had unsaved changes. Both now ask about each unsaved workbook first, as closing the app does; cancelling keeps the current version.
- **Roll Back** lists earlier versions from the office server and from the GitHub releases page, with the source beside each.
- The updates dialog now has the **Stable / Testing** channel switch, the GitHub check switch, **Check again**, and links to What's New, Report a Problem and Roll Back.
- **Automate** tab: **What's New**, **Report a Problem**, **Roll Back Version** and **Recover Unsaved Work** sit next to Check for Updates, so they no longer need the command search.
- The administrator's update policy **Never offer** now works as described: a newer version is shown only when a person checks for updates.
- The office server fetches new versions from the official GitHub releases from the start; the Server Manager's App updates page shows that, and **Turn off** really turns it off. (Before, nothing was fetched until IT typed the repository.)
- The release notes and What's New are never empty after an automatic upstream sync: a release with no change of Dexterity's own says so and points at GridCraft's list.
- The office server keeps the release before the newest one too, so **Roll Back** always has a version to go to; a version's status (known issue, deprecated, blocked) reaches the people running it even when its installer is no longer on the server.
- Fixed: an edit the server refused was sent again after every reload, so the shared workbook reloaded for ever; it is now taken back once, and the reload also picks up your current access (view-only or edit). A reload that failed asked the server again every frame; it is now said once and you can reopen the workbook from File › Shared.
- Fixed: "Others changed a shared spreadsheet while you were away" now also appears when you quit the app with the workbook still open, not only after closing its tab first.
- Fixed: a refused open left "Opening…" for ever, and a refused share left "Sharing · uploading…" in the status bar.
- Fixed: Keep version, Restore, sharing changes, Delete and Change password sent while reconnecting were silently lost. They are now refused with a message; Ctrl+S says honestly that no version was kept while offline, and the Versions buttons wait until you are back online.
- **Change my password** now asks for your current password. An account the administrator disables is disconnected within half a minute. Signing out no longer leaves you listed as "open now by" on the server. Links copied from the Share dialog name the server you are actually signed in to.
- **Recover Unsaved Work**: **Compare** now opens the Compare window; a recovered copy is kept until this session has made its own copy (or the workbook is saved), and a recovery folder whose lock cannot be read is left alone instead of deleted.
- **Compare Files** has **Browse…** buttons for the earlier and later files.
- **Templates**: saving a personal template over an existing name asks for a new name (or to replace it); creating from an office template shows progress and says when the server does not answer within 20 seconds.
- **Automation rules**: a rule cannot be added with a missing column, an invalid time (HH:MM) or no step; the window says what to fix.
- For scripts and the control channel: `file.newFromTemplate`, `automate.rules`, `review.compareFiles`, `file.recoverUnsaved` and the Help commands return their data when called with no parameters and open their window only with `show: true`.

## 1.3.0 · 29 October 2026

- The Server Manager has a **Dashboard**: **Installed versions** (which version each PC runs), **Releases** (mark a version stable or testing, OK, known issue, deprecated or blocked, with a note for the people running it; see what stable and testing PCs are offered and where a blocked version rolls back to; choose whether updates are offered, installed on their own or never offered), **Feedback** (problem reports by version and feature, each readable in full, with **Open GitHub issue** that leaves out the names of people and PCs), **System health** (failed updates, recoveries after a crash, errors) and **Feature adoption** (the most and least used commands).
- **F4** repeats your last action (formatting, inserting rows, typing an entry…) on the cells now selected, as in Excel. **Ctrl+Y** does the same when there is nothing to redo.
- Fixed: **Data › Data Validation › Circle Invalid Data** found the cells but drew nothing. It now circles them in red; a circle goes away when the value is corrected, and the new **Clear Validation Circles** removes them all.
- **Formulas › Remove Arrows** now also offers **Remove Precedent Arrows** and **Remove Dependent Arrows**.
- **Data › Auto Outline** groups the rows (and columns) that your SUM and subtotal formulas add up, with a grand total one level up; **Clear Outline** removes the outline.
- **View › Focus Cell** highlights the active cell's row and column so it is easy to follow across a wide sheet; pick the colour from its arrow.
- **Review › Show Changes** (the name Excel uses) opens Edit History.
- **Alt, Q** opens the command search (as Excel's Search box does), the same as the search icon in the title bar.
- Fixed: **Workbook Statistics**, **Check Accessibility**, **Error Checking** and **Evaluate Formula** showed raw program output. They now show readable tables; in Error Checking each cell (and each circular reference) is a link that takes you there.
- Fixed: choosing **Conditional Formatting** from the command search or a key tip did nothing; it now opens New Formatting Rule. The **Rules Manager** has a **New Rule…** button.
- Fixed: **Watch Window** and **Show Comments** run from the command search or key tips now open their panes, as the ribbon buttons do.
- Fixed: sharing a workbook while still typing in a cell sent the team a blank workbook and lost what was typed. Share now enters the cell first, and the server's copy is checked against yours before your window is replaced; if anything is missing your window stays open and you are told.
- Fixed: anything typed while a workbook was on its way to the server stayed behind in an unshared window. It now goes into the shared workbook.
- Shared workbooks say where your edits are: **saved on the server**, **saving N edit(s)**, **offline**, or **out of step, reloading**; a workbook being shared shows **Sharing · uploading** until the server has it.
- When two people change the same cell at the same moment, both are now told who changed it and which value was kept (the later edit wins, as before); the replaced value stays in Edit History.
- Workbooks sent to and from the server are checked against a checksum, so a damaged transfer is refused instead of opening as a blank or partial workbook.
- **New app icon**: "linked cells", a grid with three cells joined by a rising amber tick. It is the same everywhere: the window and taskbar, the Start menu, the installer, spreadsheet files opened with Dexterity Sheets, the Server Manager and Windows notifications. The logo shows while the app starts, and **Help › About** now gives the build, the update channel and the GitHub repository.
- **Recover unsaved work**: while a workbook has unsaved changes, a copy is kept every few seconds in your own folder, never over the file itself. If the app or Windows stops unexpectedly, the next start offers to recover each workbook, open its last saved version, compare the two, or discard the copy. Saving or closing normally removes the copy. Search for **Recover Unsaved Work** to see the copies again later.
- **Review › Compare Files** compares two versions of a spreadsheet: changed values, formulas and calculated results, formatting, added, removed or renamed sheets, rows, columns and merged cells, and named ranges. Filter by kind of change, click a cell to go there, and save the list as an .xlsx or .csv report.
- **Review › Quality Check** finds duplicate document numbers, blank mandatory fields, invalid dates, inconsistent status spellings, broken formulas, constants where formulas are expected, unusual numbers and missing worksheets. Each issue lists its cell and a suggested action, and you can set up rules for a particular kind of register.
- **Formulas › Diagnostics** lists formulas showing errors, broken references, formulas that differ from their neighbours, values typed over formulas, circular references and links to other workbooks. **Explain Error** says what the error in the active cell means, why it happened in this formula (for example, which divisor is empty or which cell blocks a spill) and how to fix it.
- **Help › What's New** shows what changed in the version you have. After an update it opens by itself, with **Report a problem with this version** below it. A report goes to the office server's administrator: pick the areas affected, describe what happened, what you expected and how serious it is, and choose whether to include the app's log (never workbook contents).
- **Help › Roll Back to an Earlier Version** installs an earlier version from the office server if a new one causes trouble. Only the app changes: your workbooks, shared workbooks and their versions stay as they are. If the administrator marks a version as having a known issue, or blocks it, the people running it are told and offered the rollback.
- **Share…** now shares with everyone at the office or with specific people, each as an **Editor** or a **Viewer**. You can set an end date for the access, and choose whether viewers may save a copy. **Copy link** gives a link to paste into a chat or e-mail: it opens the spreadsheet in Dexterity Sheets, which checks the reader's access, and the web version of the link opens the app or offers the installer.
- **Templates** on **File › New**: the office's approved templates (project trackers, registers, minutes…) with search, categories and a preview, and your own templates. A new spreadsheet made from a template is a separate copy: publishing a new version of the template never changes spreadsheets already made from it (people who used it are told a new version exists). Administrators publish office templates and can mark one as required for a workflow.
- **Automation rules** (Share › Automation rules…) for shared spreadsheets, run by the office server so nobody needs to have the file open. For example, every Monday at 9:00 find overdue items and run the Quality Check, write a report and notify the team. Or, when a status becomes Completed, write the completion time in the row and tell the person responsible. Only these fixed steps exist: no macros or scripts.
- **Notifications**: Windows tells you when someone shares a spreadsheet with you, when your access changes or is removed, when others changed a shared spreadsheet while you were away (one summary, never one per edit), when a new version is ready and when unsaved work can be recovered. Clicking a notification opens the spreadsheet. The status bar's **Notifications** shows the history; choose which kinds you get, mute single spreadsheets, or get one summary an hour.

## 1.2.2 · 27 October 2026

- Fixed: the server's log file was missing its own lines (when it started, its port, where app updates come from, and why a start failed). They are written again, so the Server Manager's log shows what the server is doing.

## 1.2.1 · 27 October 2026

- The server log no longer fills with "login refused" lines every few seconds while the Server Manager is open: its "is the server running" check is no longer counted as a failed sign-in. Real failed sign-ins are still logged and slowed down.

## 1.2.0 · 27 October 2026

- The office server installs from **DexSheetsServer-Setup.exe** (or the .msi): a Windows service that starts with Windows and restarts itself, the firewall opened for the office network only, the data folder locked to administrators.
- New **Server Manager** for IT: start and stop the server, see how teammates reach it, add people one by one or many at once (type them, paste from Excel, or import a filled-in template), reset passwords, change roles, disable or remove people, manage app updates, back up and restore. No commands to type.
- The office server never offers its own installer to office PCs as an app update.
- Deleting a shared workbook on the server now works on Windows even after someone has opened it.
- Fixed: if the server PC lost power in the middle of saving an edit, the next edit could be lost at the following restart. It is now kept.
- A damaged snapshot on the server is now recognised and the previous good one is used, instead of showing a near-empty workbook.
- The server stays light over months of use: workbooks nobody has open leave memory, its edit log and its own log file no longer grow without end, and hourly automatic versions thin out with age (all from the last two weeks, one a day for three months, one a week after that). Versions someone kept by name are never removed.

## 1.1.0 · 27 October 2026

- Share any workbook from wherever it is saved (your PC, a project folder, the network drive): the file stays where it is and keeps everyone's edits.
- Opening a file that is being shared joins the live workbook, so it never splits into two versions; reopening your own shared file catches it up.
- Share › Stop sharing (keep my file) hands the file back with every edit in it.
- A file changed by another program while shared is never overwritten.

## 1.0.2 · 27 October 2026

- DexteritySheets-Setup.exe: a double-click installer next to the .msi.
- A restyled README and a new landing page.
