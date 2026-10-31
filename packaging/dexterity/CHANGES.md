# What changed in Dexterity Sheets

Every change a user could notice gets one line under **Unreleased**, in plain words, in the same
pull request as the change (RULES.md §3; `docs_check.py` fails the build otherwise). Releasing
moves them under the new version; the release notes, the README's "What's new" and the landing
page are made from this file. Changes from official GridCraft are listed by the release notes
automatically and don't go here.

## Unreleased

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
