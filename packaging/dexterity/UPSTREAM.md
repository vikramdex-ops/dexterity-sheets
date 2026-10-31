# Staying in step with official GridCraft

Dexterity Sheets is official GridCraft (`storytold/gridcraft`) plus Dexterity's own layer. The
goal is that every upstream fix and feature reaches us without anyone redoing work, and that our
layer never has to be re-written.

## How the code is split

| Ours (upstream never touches) | Hooks in upstream files (small, checked) |
|---|---|
| `crates/collab` (LAN co-authoring, server, updates protocol) | `crates/ui-egui/src/lib.rs`: module, app field, per-frame tick, command hook |
| `apps/gridcraft-server` (DexSheetsServer) | `backstage.rs`: File › Shared page |
| `crates/ui-egui/src/collab_ui.rs` (all sharing UI) | `panes.rs`, `grid.rs`, `tabs.rs`, `ribbon.rs`: one call each |
| `packaging/dexterity` (version, scripts, icon, tests), `packaging/windows/setup` | `crates/engine/src/lib.rs`: `Session::document_mut` |
| `.github/workflows/dexterity-*`, `upstream-sync.yml` | `Cargo.toml`, `xtask/src/layers.rs`, crate manifests |
| `docs/collaboration.md`, `README.md`, `docs/readme`, `site/`, icons | branding text in translations and dialogs |

`packaging/dexterity/check_overlay.py` lists every hook (a hook may live in any file of a module,
so upstream splitting a file is fine). CI fails if one disappears. `reapply_overlay.py` re-adds
them all, idempotently; it was first used on upstream PR #2690, which split `grid.rs`,
`ribbon.rs`, `lib.rs` and `dialogs.rs` into modules.

## Versions

- `Cargo.toml` `[workspace.package] version` is upstream's GridCraft version. Never edit it.
- `packaging/dexterity/VERSION` is the Dexterity Sheets version that users, the installer and the
  updater see. Each automatic upstream sync bumps its patch number; bump minor/major by hand for
  Dexterity features.

## The automatic sync (`.github/workflows/upstream-sync.yml`)

Every 6 hours (or Actions › Upstream sync › Run workflow; tick *dry run* to stop before main):

1. **Safety checks** (`sync_upstream.py`). It refuses to start, and opens a `sync-failure` issue,
   if upstream rewrote its history (the commit we mirrored last time is no longer in upstream
   main), if `main` itself fails `check_overlay.py`, or if the checkout is dirty.
2. **Merge** into a candidate commit, never into `main` directly:
   - our files: ours win; one of ours that we removed or renamed (the old icon names) stays
     removed even when upstream keeps editing its copy;
   - files we deleted (upstream CI, ArtCraft brand files) and files upstream deleted: stay
     deleted; new upstream workflows are dropped;
   - files where we only changed branding text: upstream's file + the branding rule (also on
     clean merges, so new upstream text gets rebranded too);
   - other files both sides changed, **when `check_overlay.py` has a hook for the file** (or the
     overlay restores it: test files, docs): upstream's file, then `reapply_overlay.py` puts our
     hooks, branding and test updates back in the new layout, `cargo fmt` formats the re-added
     lines, and `check_overlay.py` proves every hook is there;
   - a both-changed file nothing can verify (numfmt's regional dates live in several files with
     no marker), a file we deleted that is on no list, or a hook the overlay can't place: stop →
     pull request. Taking upstream's copy blindly once dropped half of a feature and broke the
     build; a person (or the `ai-assist` job, with an `ANTHROPIC_API_KEY` secret) merges those.
   - `Cargo.lock`: upstream's, then `cargo update --workspace` adds the entries our crates need
     (after the overlay, which adds dependencies to upstream manifests); CI builds `--locked`.
   - new user-visible "GridCraft" text outside the auto-rebranded files → a `branding` issue to review.
   The version becomes the next patch number that has no tag yet. The `upstream` branch is
   fast-forwarded to official GridCraft (never forced).
3. **Test** the exact candidate commit with the full CI: format, clippy, every engine test, the
   collaboration tests, the build, and the three-window end-to-end test.
4. **Build** `DexteritySheets-Setup.exe`, the `.msi`, the portable zip and the server bundle from
   the candidate; the **installer smoke test** installs each installer silently, checks Windows
   lists exactly one Dexterity Sheets of the new version, starts it, calculates a formula through
   its control channel and uninstalls it.
5. **Promote**: `main` is fast-forwarded to the tested candidate. If someone pushed to `main`
   meanwhile, nothing is forced; the next run merges again on top of their work.
6. **Release**: checksums verified, a *draft* release with every file, the upload checked, then
   published as "latest". Office servers never see a half-uploaded release.
7. **Docs**: the README's version and "what's new" blocks are refreshed from the release notes;
   the screenshots and the demo animation are re-captured from the released build and replaced
   only where they really changed; the landing page (GitHub Pages) is redeployed.
8. **Report**: any failure opens (or comments on) one `sync-failure` issue with the run link; the
   next successful sync closes it.

Office servers with `update-source.json` download the new `.msi` within 30 minutes and every open
Dexterity Sheets shows "Update to X.Y.Z" in its status bar.

A person releasing by hand (bumping `packaging/dexterity/VERSION` on `main`) gets the same
pipeline: CI, build, smoke test, draft → publish, docs, landing page.

Tested on a real case: replaying upstream PR #2690 (which split `grid.rs`, `ribbon.rs`, `lib.rs`
and `dialogs.rs` into modules) through `sync_upstream.py` produces a tree identical to the
hand-resolved v1.0.1 merge.

## The SYNC_TOKEN secret

GitHub's built-in Actions token may not push a commit that adds or changes a file under
`.github/workflows/`, and official GridCraft's history does (for example `windows-arm64.yml`).
Without a token of its own the sync still merges, tests, builds and releases, but it cannot
update the informational `upstream` mirror branch (a warning in the run), and a push to `main`
or `sync/candidate` that carries such an upstream commit can be refused the same way.

To remove the limit, add a repository secret named **`SYNC_TOKEN`**: a fine-grained personal
access token of the repository owner (the account that may push), limited to this repository,
with **Contents**, **Workflows**, **Pull requests** and **Issues** set to read and write, and an
expiry you note in the calendar. The workflow uses it wherever it exists and the built-in token
otherwise; nothing else changes.

## Resolving a sync by hand

```
git fetch origin
git checkout main
git merge origin/upstream
# resolve: keep upstream's logic; keep our hook lines; keep "Dexterity Sheets" in visible text
python packaging/dexterity/check_overlay.py
cargo test --release -p gridcraft-collab
cargo build --release -p gridcraft -p gridcraft-server
# bump packaging/dexterity/VERSION (patch), commit, push: the release workflow publishes it
```

## Rules that keep merges easy

- New Dexterity behaviour goes in our files; upstream files get at most a one-line call.
- When a hook is unavoidable, add its marker to `check_overlay.py` in the same commit.
- Don't reformat or reorganise upstream files.
- Contribute general fixes (Excel compatibility bugs) back to upstream when possible: once
  upstream has them, our diff shrinks.

## Tests that need the craft-fonts input

`crates/ui-egui/tests/it/interaction.rs` (`column_autocomplete_folds_case_beyond_ascii`,
`language_switch_renders_the_ribbon_and_persists`) draw glyphs (ẞ, ẻ) that egui's bundled faces
lack; without `CRAFT_FONTS_DIR` they panic with "No glyph for …". They are upstream's tests and
fail the same way on an upstream checkout without that input, so Dexterity CI does not run the
`ui-egui` integration tests; everything else in that crate passes here (`cargo test -p
gridcraft-ui-egui`: 913 of 916, those two and nothing else).
