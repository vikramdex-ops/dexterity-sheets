# Dexterity Sheets: standing rules

These rules bind every change to this repository: people, automatic workflows and AI agents
alike. Where a rule can be checked by a machine it is, and the check is named; a red check blocks
the release. Agents read this file before any work (the `AGENTS.md` / `CLAUDE.md` pointers).

## 1. What the product must always do

1. **Share any file from wherever it is saved.** A workbook on a PC, a project folder or the
   network drive can be shared live with the team at any moment; the file stays where it is and
   keeps everyone's edits. The office server only passes edits along and keeps history; files
   never have to live on it. (`lan_e2e.py`: share-from-a-folder checks.)
2. **Never lose work.** No panics in non-test code; an edit the server acknowledged is on disk; a
   file changed by another program is never overwritten. (`cargo xtask ci`, `lan_e2e.py`.)
3. **Every screen is real.** No dummy handlers, placeholders, "coming soon", dead buttons or
   controls disabled without a reason shown, and nothing a user needs for a task is missing.
   (`docs/dexterity-ui-audit.md`, kept current; `docs_check.py` rejects placeholder words in
   shipped UI text.)
4. **Feels light and snappy, always.** Startup, typing, scrolling, opening big files, memory
   and disk use stay inside the budgets in `packaging/dexterity/perf-budget.json`; a change that
   breaks a budget does not ship. (`perf_gate.py`, every CI run and every release.)
5. **IT never needs a command line.** The office server installs from `DexSheetsServer-Setup.exe`
   / `.msi` and is run from the Server Manager (people one by one, in bulk, or from a template;
   service, firewall, updates, backups). (`server_installer_smoke.py`.)

## 2. Staying current with GridCraft and Microsoft Excel

1. **GridCraft.** Every new upstream commit is merged automatically (`upstream-sync.yml`, every 6
   hours): safety checks, a candidate branch, the full test suite, both installers built and
   tried, then released and delivered to every PC. A merge that needs judgement becomes a pull
   request (and, when the `ANTHROPIC_API_KEY` secret is set, an AI-proposed resolution PR); it is
   never released untested. A failed run opens one `sync-failure` issue that the next good run
   closes.
2. **Microsoft Excel.** `excel-watch.yml` reads Microsoft's public Current Channel release notes
   every day. Each new Excel version becomes an `excel-release` issue listing its Excel items, each
   checked against `docs/dexterity-parity-audit.md`. When the `ANTHROPIC_API_KEY` secret is set, an
   agent is asked to implement each missing item as a pull request. Nothing reaches users until
   that PR passes every check below and is merged. Clean-room: Excel is observed black-box and
   from public documentation only; its code, assets, icons and wording are never copied.
3. **Nothing is silently skipped.** Each item in an `excel-release` issue ends as *implemented*
   (with the PR), *already there* (with the evidence), or *not applicable* (with the reason). The
   parity audit document says which.

## 3. Docs move with the product

Every change a user could notice (a feature added, changed, removed or renamed; a new file in a
release; a changed screen) updates, in the same pull request:

- `packaging/dexterity/CHANGES.md` (the *Unreleased* list): the release notes, the README's
  "What's new" and the landing page are generated from it, so writing it once updates all three;
- `README.md`, `site/index.html` and the relevant `docs/*.md` wherever the change is described;
- screenshots: the release pipeline retakes every picture from the released build and commits
  the ones that changed.

Machine checks (`docs_check.py`, in CI): a user-visible change with no `CHANGES.md` entry fails
(a commit can say `[no-user-change]` with a reason); every release file the README or landing page
names is one the release really produces; every image and internal link resolves; nothing names
a removed file, command or page. Upstream merges are described automatically in the release
notes.

## 4. Releasing

A release happens only through `dexterity-release.yml`: full CI, the performance gate, the app
and server installers built and smoke-tested on a clean Windows machine, then draft → upload →
verify → publish, then README, screenshots, demo and landing page refreshed from that build. A
version that fails any step is never published.

## 5. How work is done here

- All builds, caches and temporary files on `D:\GridCraft` (see `D:\GridCraft\env.ps1`).
- Rust only; never break wasm; every behaviour is an engine command with tests (`CLAUDE.md`).
- Keep edits to GridCraft-owned files small and register Dexterity hooks in
  `packaging/dexterity/reapply_overlay.py` / `check_overlay.py`, so upstream merges stay clean.
- Report what was verified and what was not. "Not checked" is an acceptable answer; a false
  "done" is not.
