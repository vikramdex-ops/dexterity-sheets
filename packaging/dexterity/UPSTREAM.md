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
| `packaging/dexterity` (version, scripts, icon, tests) | `crates/engine/src/lib.rs`: `Session::document_mut` |
| `.github/workflows/dexterity-*`, `upstream-sync.yml` | `Cargo.toml`, `xtask/src/layers.rs`, crate manifests |
| `docs/collaboration.md`, `README.md`, icons | branding text in translations and dialogs |

`packaging/dexterity/check_overlay.py` lists every hook. CI fails if one disappears.

## Versions

- `Cargo.toml` `[workspace.package] version` is upstream's GridCraft version. Never edit it.
- `packaging/dexterity/VERSION` is the Dexterity Sheets version that users, the installer and the
  updater see. Each automatic upstream sync bumps its patch number; bump minor/major by hand for
  Dexterity features.

## The automatic sync (`.github/workflows/upstream-sync.yml`)

Every 6 hours (or Actions › Upstream sync › Run workflow):

1. The `upstream` branch is updated to official GridCraft `main`.
2. `sync_upstream.py` merges it into `main`:
   - our files: ours win;
   - files we deleted (upstream CI, ArtCraft brand files): stay deleted; new upstream workflows are dropped;
   - files where we only changed branding text: upstream's file + the branding rule;
   - anything else: stop.
3. Clean merge → full CI (format, clippy, all engine tests, collaboration tests, build, the
   three-window end-to-end test) → fast-forward `main`, tag `vX.Y.Z`, publish the release.
4. Office servers with `update-source.json` download the new `.msi` within 30 minutes and every
   open Dexterity Sheets shows "Update to X.Y.Z" in its status bar.

When it can't finish on its own you get a pull request (`upstream` → `main`, label
`upstream-sync`) or an issue (tests failed on `sync/candidate`). Nothing is released until a
person merges.

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
