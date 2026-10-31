# Dexterity Sheets roadmap: requests of 28 October 2026

> Kept current: each item's row changes in the same commit as the work. The persistence and
> collaboration P0 came first (no new features until it was fixed): see
> [`dexterity-p0-persistence.md`](dexterity-p0-persistence.md).

## Three separate systems

They work together but are never one mechanism:

| System | Responsibility | Where |
|---|---|---|
| Application updates | Installs, verifies and rolls back **the app** (1.5.0 → 1.4.0) | `updates_ui.rs`, the server's `updates/` folder, Server Manager › App updates |
| Document versions and recovery | Keeps and restores **a workbook's** content (version 12 → 11; unsaved work after a crash) | server `books/<id>/versions`, local recovery folder |
| Sharing and synchronisation | Who can open a shared workbook and how edits reach everyone | `crates/collab`, `collab_ui.rs` |

Rolling the app back never touches a workbook or its versions; restoring a version never changes
the app; stopping sharing never deletes anyone's own files.

## Items

| # | Request | Status | Notes |
|---|---|---|---|
| 13 | P0: blank shared workbook, persistence, real-time collaboration | **done** | root causes, fix, tests A–H: `dexterity-p0-persistence.md` |
| 5 | Autosave, crash recovery, unsaved-work protection | **done** | AutoSave and atomic saves (upstream); recovery copies of unsaved work, the Recover Unsaved Work dialog (recover, open saved, compare, discard): `crates/ui-egui/src/dex_ui/recovery.rs`. Not done: telling that another program changed an open file before Save overwrites it (shared files already are protected) |
| 8 | Compare two spreadsheet versions | **done** | Review › Compare Files (`review.compareFiles`): values, formulas, results, formatting, sheets, structure, names; filter; .xlsx/.csv report. Comparing with the server's copy: open the shared workbook and compare it with the file |
| 11 | Spreadsheet quality checks | **done** | Review › Quality Check (`data.qualityCheck`), automatic checks or explicit rules per register; saved rule profiles are passed as `rules` (no profile library yet) |
| 7 | Formula diagnostics and compatibility warnings | **done** (diagnostics) | Formulas › Diagnostics and Explain Error (`formulas.diagnostics`, `formulas.explainError`). Preserving features on open/save is the file-format work tracked in `dexterity-parity-audit.md`; opening a file already warns in an info bar about what it cannot keep (upstream's open warnings) |
| 1 | App icon and branding everywhere | **done** | "linked cells" icon from one source (`make_icon.py`): window, taskbar, exe, installer, file types, Server Manager, notifications (AppUserModelID); start-up splash; About with build, channel, repository. Light and dark themes are upstream's |
| 3 | Shareable links that open a workbook in the app | **done** | `dexterity://` links (installer registers them), the browser fallback page on the office server, per-workbook access (office / people, editor / viewer, end date, downloads). HTTPS: not on the office LAN (plain HTTP page, no workbook named) |
| 2 | Rollback, release notes, per-feature problem reports, release status | **done** | Automate › What's New, Report a Problem, Roll Back Version, Recover Unsaved Work, next to Check for Updates; the updates dialog (channel, GitHub switch, what each source answered); server release status (ok / known issue / deprecated / blocked) and channels; feedback to the server; GitHub issues from the Server Manager, labelled by area. Updates come from the office server first and from the GitHub releases page otherwise (`crates/ui-egui/src/updates_ui.rs`); an update or roll-back installs only after every unsaved workbook was asked about |
| 6 | Admin dashboard in the Server Manager | **done** | Installed versions, Releases, Feedback, System health, Feature adoption read the files of [`dexterity-admin-data.md`](dexterity-admin-data.md), which the server writes, and write `updates/releases.json`, which it obeys (including policy `manual`); the server fetches releases from the official repository from the start |
| 4 | Native notifications for shared files | **done** | Windows notifications, history panel, settings (kinds, mute, hourly summary). Not done: @mention notices |
| 9 | Template library and controlled masters | **done** | office templates on the server (`templates/`), published by administrators, versioned, "required for" label; personal templates; File › New and the Templates window; new-version notices; creating copies, never links. Enforcing a required template in a workflow is not done (it is shown, not enforced) |
| 10 | Scheduled reports and automation rules | **done** (first set of steps) | rules per shared workbook (`books/<id>/automation.json`), run by the server: schedule (daily / weekly) or a value change; steps: notify, Quality Check, overdue items, a report workbook, a completion time. Not done: e-mail or chat integrations, duplicate detection as its own step (the Quality Check covers it), formula-based triggers |
| 12 | Keep updates, document versions and sharing separate | **adopted** | the table above; applied to every item |
