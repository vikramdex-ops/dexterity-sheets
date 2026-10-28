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
| 5 | Autosave, crash recovery, unsaved-work protection | planned | AutoSave and atomic saves exist; recovery of unsaved work is next |
| 8 | Compare two spreadsheet versions | planned | |
| 11 | Spreadsheet quality checks | planned | |
| 7 | Formula diagnostics and compatibility warnings | planned | Error Checking, Trace and Evaluate Formula exist |
| 1 | App icon and branding everywhere | planned | the icon is generated from one source (`packaging/dexterity/make_icon.py`) |
| 3 | Shareable links that open a workbook in the app | planned | |
| 2 | Rollback, release notes, per-feature problem reports, release status | planned | |
| 6 | Admin dashboard in the Server Manager | planned | |
| 4 | Native notifications for shared files | planned | |
| 9 | Template library and controlled masters | planned | |
| 10 | Scheduled reports and automation rules | planned | |
| 12 | Keep updates, document versions and sharing separate | **adopted** | the table above; applied to every item |
