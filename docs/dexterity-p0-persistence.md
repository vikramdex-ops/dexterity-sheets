# P0: blank shared workbook — investigation and fix

> **Date:** 2026-10-28 · **Status:** reproduced, root causes fixed, regression tests pass
> (`crates/collab` 18/18, `lan_e2e.py` 38/38 with real windows and a real server).

**Report.** User A created a workbook, typed text, saved it, shared it with User B. B saw the
workbook in the shared list, but it opened blank.

## How it was traced

The whole lifecycle was driven with real app windows and a real server (control channel, the same
harness as `packaging/dexterity/tests/lan_e2e.py`), in variants: save then share, share an unsaved
workbook, share then type, share while signed out, type during the share, create from the Shared
page then Save As, Save As on a shared workbook, and real keystrokes (not just `cell.set`). At each
hop the content was checked: A's window, the server's own files on disk (`snapshot-*.xlsx` and
`log.jsonl`), and B's window.

Lifecycle, as the code does it:

| Step | Code |
|---|---|
| Share asked (title bar, Share menu, Ctrl+Shift+S…) | `collab_ui::share_active` → `share_doc` |
| Serialise the window's workbook to .xlsx, upload | `share_doc`: `io::save_bytes` → `ClientMsg::CreateBook` |
| Server stores it as the first snapshot, makes the id | `server::State::create_book` (`snapshot-0.xlsx`, `book.json`) |
| Server sends its copy back | `OpenBook` → `ServerMsg::Opened` (snapshot + edits since) |
| Window replaced by the shared workbook | `collab_ui::message(Opened)` → `SharedDoc::open` → `arrived` |
| Every later edit | `send_local` → `SharedDoc::local_changes` (diff) → `Patch` → `take_patch` (fsync to `log.jsonl`, then broadcast) |
| B opens it | `collab.open` → `OpenBook` → `Opened` → `SharedDoc::open` |
| Reconnect | `Connected` → `OpenBook` again → `SharedDoc::resync` (resends what the server never ordered) |

## Root causes (confirmed by reproduction)

1. **Text still being typed in a cell was not part of the upload.** Share does not start with
   `file.`, so unlike Save it never entered the cell being edited. The upload was the workbook
   without that text, the server stored a blank workbook, and `arrived` closed A's window because
   its workbook had not changed (the draft lives in the cell editor, not the workbook). B opened a
   blank workbook and A's text was gone on both sides. Reproduced: A types in A1, clicks Share
   without Enter: the server's snapshot has no text, B's A1 is empty, A's A1 is empty.
2. **Edits typed while the workbook was on its way stayed behind.** Between the upload and the
   server's reply, anything typed went into the old window. `arrived` then left that window open
   (unshared) beside the shared one, which lacked those edits; nothing sent them. Reproduced:
   `A2` typed right after Share is in an unshared "S4.xlsx" window, not on the server, not at B.

Not a cause (verified working): saving and then sharing with Enter pressed, sharing while signed
out, typing after sharing, Save / Save As on a shared workbook, reconnecting, a server restart.

The server side was sound: every edit is on its disk (`fsync`) before anyone is told; ids are
stable (`book.json` id, independent of the name and the file path); a reconnecting client resends
only what the server never ordered (`origin`). A gap was found there too and closed:
same-cell edits made at the same moment were resolved last-writer-wins **silently**.

## Changes

| Where | What |
|---|---|
| `collab_ui::share_active` | Enters the cell being edited (as Enter would) before anything is uploaded; a draft the workbook refuses stops the share with the reason. |
| `collab_ui::arrived` | For an upload: compares the server's copy with what was uploaded, sheet by sheet and cell by cell (constants and formulas, `sync::content_differences`). Anything missing: the window is **not** replaced; it stays open with its content and the person is told what is missing. Then carries whatever was typed meanwhile into the shared workbook (as an edit, so it is sent and stored), and closes the old window. |
| `proto` | `CreateBook.sha256` (the server refuses a damaged upload), `Opened.sha256` (the client refuses a damaged download: an error, never a blank window), `Patch.base` (the last edit the client had seen), `Patched.conflicts`. All fields are optional, so older clients and servers still work together. |
| `server::take_patch` | Tracks who last changed each cell (`writes`, rebuilt from the log on load). A cell changed by another client after the patch's `base` is a conflict: the later edit still wins (the defined policy, unchanged), and the conflict travels in the broadcast edit. |
| `collab_ui::message(Patched)` | Tells the person whose value was replaced, and the person whose edit replaced it, which cell and which value; the replaced value is in Edit History. |
| `SharedDoc::open` | Refuses bytes that are not an .xlsx (a zip) rather than letting the reader treat them as CSV and show a near-empty sheet. |
| Status bar, `collab.status` | `Shared · saved on the server` only when the server has ordered every edit made here; `saving N edit(s)`, `Offline · N edit(s) not on the server yet`, `out of step, reloading`, `Not signed in · N edit(s) not on the server`; `Sharing · uploading…` while a workbook is on its way. `collab.status` reports the same as `state` per workbook and `sharing`. |

No database or on-disk format changes. `server.log` lines and the files under `books/<id>/` are as
before; the protocol version stays 1 (new fields are optional).

## Synchronisation model (kept, verified)

Server-ordered patches: each client sends absolute per-cell assignments found by diffing its
workbook; the server numbers them, writes them to `log.jsonl` (fsync), then broadcasts; each client
shows "server state + its own unacknowledged patches", so every copy converges on the server's
order. This already handles concurrent edits to different cells, formulas, formatting, sheet
structure, reconnection and restarts; a CRDT or OT framework was not needed for the required
behaviour. Same-cell conflicts: the edit the server receives later wins, everywhere alike, and both
people are now told.

## Tests

| Required | Test | Result |
|---|---|---|
| A: the reported flow | `lan_e2e.py` `blank_share_regression` (real windows: typing not entered, Share, server's files read from disk, B opens) and `a_shared_workbook_reaches_the_server_and_the_recipient_with_its_content` | pass |
| B: text, numbers, formulas, two sheets, formatting | same Rust test (`rich_workbook`, compared cell by cell and style by style) | pass |
| C / D: owner → collaborator, collaborator → owner, without reopening | `owner_and_collaborator_edits_propagate_converge_and_survive_a_restart`; `lan_e2e.py` "B's edit reaches A", "edit reaches another window" | pass |
| E: different cells at once; same cell at once | same Rust test (both kept; conflict reported to both, later edit wins, replaced value in history); `lan_e2e.py` "conflicting edits converge" | pass |
| F: close, reopen in new sessions | same Rust test (fresh clients after restart get every edit) | pass |
| G: server crash and restart, offline edits | same Rust test (a second server on the same folder, no clean shutdown); `lan_e2e.py` offline edit held and delivered, identical after two restarts | pass |
| G: damaged transfer | `a_damaged_upload_is_refused` (upload and download checksums; text is never opened as a workbook) | pass |
| H: viewer cannot edit; no access without signing in | `viewers_cannot_edit_and_strangers_cannot_read` | pass |
| 3: share after editing an existing workbook; 9: sheet added; 13: large workbook (10 000 cells); 14: rapid edits (70 in a burst, same cell rewritten) | Rust tests above and `a_large_workbook_and_a_burst_of_edits_arrive_complete_and_in_order` | pass |

Run them:

```
cargo test -p gridcraft-collab
python -I packaging/dexterity/tests/lan_e2e.py target/release
```

CI runs both (`.github/workflows/dexterity-ci.yml`: the workspace tests and the LAN end-to-end job).

## Limits and what was not verified

- **Sharing is with the whole office.** Every signed-in account sees every shared workbook; roles
  (admin, editor, viewer) are per account, not per workbook. "Share with specific people" and
  links are a separate feature (see `docs/dexterity-roadmap.md`).
- **Same-cell conflicts are resolved, not merged:** the later edit wins and both people are told.
- **A row inserted while someone else edits below it** (in the same split second, or offline):
  the edit lands at the cell address it was typed at and is reported as a conflict if it hits a
  moved cell; it is not shifted with the rows.
- Cross-PC runs over a real office LAN were not repeated for this fix; the tests use one PC with
  separate app instances and a server on `127.0.0.1`.
