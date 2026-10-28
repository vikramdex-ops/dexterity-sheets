# What changed in Dexterity Sheets

Every change a user could notice gets one line under **Unreleased**, in plain words, in the same
pull request as the change (RULES.md §3; `docs_check.py` fails the build otherwise). Releasing
moves them under the new version; the release notes, the README's "What's new" and the landing
page are made from this file. Changes from official GridCraft are listed by the release notes
automatically and don't go here.

## Unreleased

- **F4** repeats your last action (formatting, inserting rows, typing an entry…) on the cells now selected, as in Excel. **Ctrl+Y** does the same when there is nothing to redo.
- Fixed: **Data › Data Validation › Circle Invalid Data** found the cells but drew nothing. It now circles them in red; a circle goes away when the value is corrected, and the new **Clear Validation Circles** removes them all.
- **Formulas › Remove Arrows** now also offers **Remove Precedent Arrows** and **Remove Dependent Arrows**.
- **Data › Auto Outline** groups the rows (and columns) that your SUM and subtotal formulas add up, with a grand total one level up; **Clear Outline** removes the outline.
- **View › Focus Cell** highlights the active cell's row and column so it is easy to follow across a wide sheet; pick the colour from its arrow.
- **Review › Show Changes** (the name Excel uses) opens Edit History.
- **Alt, Q** opens the command search (as Excel's Search box does), the same as the search icon in the title bar.

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
