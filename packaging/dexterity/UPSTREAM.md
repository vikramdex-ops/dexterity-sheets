# Staying in step with official GridCraft

Dexterity Sheets is official GridCraft (`storytold/gridcraft`) plus Dexterity's own layer. The
goal is that every upstream fix and feature reaches us without anyone redoing work, and that our
layer never has to be re-written.

## How the code is split

| Ours (upstream never touches) | Hooks in upstream files (small, checked) |
|---|---|
| `crates/collab` (LAN co-authoring, server, updates protocol) | `crates/ui-egui/src/lib.rs`: module, app field, per-frame tick, command hook |
| `apps/gridcraft-server` (DexSheetsServer), `apps/gridcraft-server-manager` | `backstage.rs`: File › Shared page |
| `crates/ui-egui/src/collab_ui.rs`, `dex_ui.rs`, `dex_ui/` (all Dexterity UI) | `panes.rs`, `grid.rs`, `tabs.rs`, `ribbon.rs`: one call each |
| `crates/engine/src/cmd/dex.rs`, `cmd/dex/` (Dexterity commands) | `crates/engine/src/lib.rs`: `Session::document_mut`, the command hook |
| `crates/numfmt/src/dex.rs` (regional settings in number display) | `crates/numfmt/src/render.rs`, `lib.rs`, `parse.rs`: one call where a separator is printed |
| `crates/xlsx/src/array_formula/dex.rs` (formulas Excel would show with `@`) | `crates/xlsx/src/array_formula.rs`: one `if` in `export` |
| `packaging/dexterity` (version, scripts, icon, tests), `packaging/windows/setup` | `Cargo.toml`, `xtask/src/layers.rs`, crate manifests |
| `.github/workflows/dexterity-*`, `upstream-sync.yml` | branding text in translations and dialogs |
| `docs/collaboration.md`, `docs/dexterity-*.md`, `README.md`, `docs/readme`, `site/`, icons | the `RULES.md` pointer in `AGENTS.md` and `CLAUDE.md` |

`packaging/dexterity/check_overlay.py` lists every hook (a hook may live in any file of a module,
so upstream splitting a file is fine) and what must never come back from upstream. CI fails if one
disappears. `reapply_overlay.py` re-adds them all, idempotently, in whatever shape upstream's
current code needs.

**The rule that keeps the sync automatic:** a Dexterity change inside a file official GridCraft
owns is a hook, with its code in a Dexterity-owned file, a rule in `reapply_overlay.py` and a
marker in `check_overlay.py`. A change written straight into upstream's code merges only until
upstream rewrites those lines; then the sync has to stop and ask (see *When the sync stops*).
What still sits in upstream files without a rule is listed at the end of this document.

## Versions

- `Cargo.toml` `[workspace.package] version` is upstream's. Never edit it. Upstream bumps it on
  its release branches, so on `main` it lags behind the newest GridCraft release.
- `packaging/dexterity/GRIDCRAFT_COMMIT` is the official GridCraft commit this tree contains and
  `GRIDCRAFT_RELEASE` the newest GridCraft release it contains (a release counts once the point
  of upstream `main` it was cut from is merged). The sync writes both; the release notes, the
  README, the landing page and File › Account read them. No branch or remote has to mirror
  upstream for any of that.
- `packaging/dexterity/VERSION` is the Dexterity Sheets version that users, the installer and the
  updater see. Each automatic sync raises it: the **minor** number when it brings a new GridCraft
  minor release (1.4.1 → 1.5.0 for GridCraft 0.14 → 0.18), the patch number otherwise. Bump
  minor/major by hand for Dexterity features.

## The automatic sync (`.github/workflows/upstream-sync.yml`)

Every 6 hours (or Actions › Upstream sync › Run workflow; tick *dry run* to stop before main).
Nothing in it needs an AI service, an API key or any secret.

1. **Safety checks** (`sync_upstream.py`). It refuses to start, and opens a `sync-failure` issue,
   if upstream rewrote its history (the commit in `GRIDCRAFT_COMMIT` is no longer in upstream
   main), if `main` itself fails `check_overlay.py`, or if the checkout is dirty.
2. **Merge** into a candidate commit, never into `main` directly. Where git cannot merge a file
   by itself:
   - our files: ours win; one of ours that we removed or renamed stays removed even when
     upstream keeps editing its copy;
   - files we deleted on purpose (upstream CI, ArtCraft brand files): stay deleted; new upstream
     workflows are dropped;
   - generated files (the key tip table): upstream's copy, then `cargo xtask keytips` regenerates
     them from the merged `keytips.json`;
   - upstream's own documents: upstream's wording where both sides changed the same lines
     (Dexterity's notes live in `docs/dexterity-*.md`);
   - `Cargo.lock`: upstream's, then `cargo update --workspace` adds the entries our crates need;
   - **every other file both sides changed is merged hunk by hunk**: upstream's lines are taken
     only where the two sides touch the same lines, every other Dexterity change in the file is
     kept, and `reapply_overlay.py` puts the hooks back. Then the sync proves that nothing of ours
     was dropped: each Dexterity line that sat in a conflicting hunk must be in the file again
     (or the marker of the hook it carried must be). If one is not, the file gets its conflict
     markers back and the sync stops;
   - a file Dexterity changed that upstream removed or moved, a file we deleted that is on no
     list, a binary both sides changed: the sync stops.
   Then `cargo fmt`, `check_overlay.py` (every hook present, nothing forbidden back), new
   user-visible "GridCraft" text outside the rebranded files → a `branding` issue to review.
   `GRIDCRAFT_COMMIT` / `GRIDCRAFT_RELEASE` are written, `CHANGES.md` gets what came from
   GridCraft (so What's New in the app, the README and the landing page show it), and the
   version goes up.
3. **Test** the exact candidate commit with the full CI: format, clippy, every engine test, the
   collaboration tests, the build, the three-window end-to-end test, crash recovery and the
   performance gate.
4. **Build** `DexteritySheets-Setup.exe`, the `.msi`, the portable zip and the server installers
   from the candidate; the **installer smoke tests** install each one silently, check Windows
   lists exactly one Dexterity Sheets of the new version, start it, calculate a formula through
   its control channel and uninstall it.
5. **Promote**: `main` is fast-forwarded to the tested candidate. If someone pushed to `main`
   meanwhile, nothing is forced; the next run merges again on top of their work.
6. **Release**: checksums verified, a *draft* release with every file, the upload checked, then
   published as "latest". Office servers never see a half-uploaded release.
7. **Docs**: the README's version and "what's new" blocks are refreshed from the release notes;
   the screenshots and the demo animation are re-captured from the released build and replaced
   only where they really changed; the landing page (GitHub Pages) is redeployed.
8. **Report**: any failure opens (or comments on, once a day) one `sync-failure` issue with the
   run link; the next successful sync closes it.

Office servers with `update-source.json` download the new `.msi` within 30 minutes and every open
Dexterity Sheets shows "Update to X.Y.Z" in its status bar.

A person releasing by hand (bumping `packaging/dexterity/VERSION` on `main`) gets the same
pipeline: CI, build, smoke test, draft → publish, docs, landing page.

## When the sync stops

It stops only for what a rule cannot decide, and then it leaves the work ready:

- The half-merged tree is committed to the branch **`sync/pending`** with conflict markers in
  the files to settle, a pull request is opened for it and the `sync-failure` issue says which
  files and which Dexterity lines are at stake (`packaging/dexterity/sync-state.json` on that
  branch has the same list).
- **Settle it on `sync/pending`**: in each conflict the first block is Dexterity's, the second
  the common ancestor, the third official GridCraft's. Keep GridCraft's new code and put our
  change back onto it. Editing in the browser (the pencil icon) is enough. A file listed as
  *removed upstream*: move our changes to where that code lives now, then delete the file.
- **Push, and that is all.** The next run (the push starts one) sees the branch, brings in
  anything `main` got meanwhile, re-applies the hooks, regenerates, formats, completes
  `Cargo.lock`, bumps the version and runs the whole pipeline. Do not merge the pull request and
  do not push to `main`; it closes by itself when `main` has the result.
- A candidate that merged but failed its tests or its build is put on `sync/pending` too. Push
  the fix there; the next run tests it again.
- While nobody has touched a parked merge, each run starts again from the newest upstream (the
  conflict may be gone). Once it has a commit of yours it is yours: the sync only ever finishes
  it.

Locally the same thing is:

```
git fetch origin && git checkout main && git pull
python packaging/dexterity/sync_upstream.py              # stops with markers in the working tree
# settle the marked files
python packaging/dexterity/sync_upstream.py --continue   # hooks, generated files, format, lock, version, commit
. D:\GridCraft\env.ps1; D:\GridCraft\ci-local.ps1 -Locked # the CI gate, on this PC
git push origin HEAD:main                                 # the release workflow publishes it
```

When you settle a conflict, make it the last time: move the Dexterity code into one of our own
files, leave a one-line call in upstream's, and add the call to `reapply_overlay.py` and its
marker to `check_overlay.py` in the same commit.

## Optional secrets (the sync works without any)

| Secret | What it adds | How to make it |
|---|---|---|
| `SYNC_TOKEN` | Pushes made by the sync start the other workflows (CI and the landing page on `main` after a promote), and a push is never refused because an upstream commit touched a workflow file. | GitHub › Settings › Developer settings › Fine-grained tokens: this repository only; **Contents**, **Workflows**, **Pull requests**, **Issues** read and write; note the expiry in the calendar. |
| `CLAUDE_CODE_OAUTH_TOKEN` | The `assist` job: an agent settles a parked merge (or fixes a candidate that failed its tests) on `sync/pending`, at most three times per merge, and the pipeline then tests and releases it like any other. It runs on a Claude subscription; **no API key** is involved and nothing is released untested. | On a PC signed in to Claude Code: `claude setup-token`, then paste the token as the secret. |

Also in the repository's Settings › Actions › General: **Read and write permissions** for
workflows, and **Allow GitHub Actions to create and approve pull requests** (without it the
conflict pull request cannot be opened; the issue still carries the link).

## Rules that keep merges easy

- New Dexterity behaviour goes in our files; upstream files get at most a one-line call.
- When a hook is unavoidable, add its rule to `reapply_overlay.py` and its marker to
  `check_overlay.py` in the same commit.
- Don't reformat or reorganise upstream files.
- Dexterity's notes about a feature go in `docs/dexterity-*.md`, never into upstream's documents.
- Contribute general fixes (Excel compatibility bugs) back to upstream when possible: once
  upstream has them, our diff shrinks.

## Dexterity changes still written straight into upstream files

These merge cleanly until upstream rewrites the same lines; then the sync stops for them. Each is
a candidate for a hook or for a pull request to official GridCraft.
`python packaging/dexterity/check_overlay.py --unruled` prints the current list (every upstream
file that differs here, by how many lines, and how many markers cover it). On 11 November 2026,
with GridCraft 0.18.0 merged, the files with Dexterity changes and no marker were:

| Where | Lines added here | What |
|---|---|---|
| `crates/core/src/parse.rs` (and `pub mod locale;` in `core/src/lib.rs`) | 311 | Typed dates and numbers are read with the regional settings: day/month order, date separator, digit grouping, currency (`core/src/locale.rs` itself is ours) |
| `crates/numfmt/src/lib.rs` (`select`, `Selected`) and `parse.rs` (`Cond::negative_only`) | about 80 | Which section a conditional number format picks, and when it shows the minus sign, as Excel does |
| `crates/xlsx/src/fmla.rs` (`TableNames`) | 133 | Bare table names are written as `Table[]` (the two calls into it have markers) |
| `crates/xlsx/src/tables.rs` | 22 | Built-in format ids ask the locale first; id 47 is `mm:ss.0`; only locale-independent ids are written as ids |
| `crates/xlsx/src/csv.rs` | 11 | CSV is saved as UTF-8 with a byte order mark, as Excel's "CSV UTF-8" |
| `crates/engine/src/cmd/file.rs`, `crates/functions/src/engineering.rs` | 9 and 3 | Small changes (see `git diff $(cat packaging/dexterity/GRIDCRAFT_COMMIT) -- <file>`) |
| `apps/gridcraft-cli/src/main.rs`, `check.rs` | 24 and 2 | `DEXSHEETS_LOCALE` for the command-line tool; a lint allowance for Windows |
| `apps/gridcraft/build.rs`, `packaging/windows/*`, `packaging/{icons.sh,linux,macos,freebsd}`, `xtask/src/ico.rs` | | The executable's, the icon's and the installer's names |
| Tests inside upstream's test files: `crates/numfmt/src/tests.rs` (225), `crates/engine/src/tests.rs` (72), `crates/xlsx/src/tests/{csv_tests,locale_numfmt,array_export}.rs`, `crates/engine/src/{cmd/edit,io}/tests.rs` | | Dexterity's tests of the changes above (new tests belong in a Dexterity-owned test file) |

## Tests that need the craft-fonts input

`crates/ui-egui/tests/it/interaction.rs` (`column_autocomplete_folds_case_beyond_ascii`,
`language_switch_renders_the_ribbon_and_persists`) draw glyphs (ẞ, ẻ) that egui's bundled faces
lack; without `CRAFT_FONTS_DIR` they panic with "No glyph for …". They are upstream's tests and
fail the same way on an upstream checkout without that input, so Dexterity CI does not run the
`ui-egui` integration tests; everything else in that crate passes here.
