<p align="center">
  <img src="assets/app-icon/dexterity-sheets-1024.png" width="112" height="112" alt="Dexterity Sheets icon: a blue tile with a white grid and a bold D">
</p>

<h1 align="center">Dexterity Sheets</h1>

<p align="center">
  The spreadsheet of <b>Dexterity Design Services Pvt. Ltd.</b>: Excel-compatible workbooks with
  live, named, audited co-editing over the office LAN.
</p>

---

## What it is

- **A desktop spreadsheet for Windows** that opens, edits and saves `.xlsx`/`.xlsm`, CSV and TSV:
  ribbon, formula bar, 500+ functions with dynamic arrays, tables, pivot tables, charts,
  conditional formatting, validation, sort/filter, print and PDF. Files go to and from clients
  who use Microsoft Excel (verified against Excel itself: see *Quality* below).
- **Live co-editing on the office network.** One PC runs *Dexterity Sheets Server*; everyone signs
  in with their own account and edits the same workbook at the same time. Other people's
  selections show in their colour with their name.
- **Who changed what, always.** No anonymous access. Every change is stored with the person and
  the time; the status bar shows who last changed the active cell, and *Review › Edit History*
  lists every change to a cell or the whole workbook (old → new).
- **Nothing to save, nothing lost.** Every edit is written to the server's disk before anyone else
  sees it; edits made while the network is down are sent when it returns. The server keeps
  versions (hourly, on Save, on request) that can be opened or restored.
- **Updates itself.** When a new version is released, every running copy shows *Update to X.Y.Z*
  in the status bar; one click downloads it from the office server, checks it and installs it
  (no administrator rights needed).

## Install

From the [latest release](../../releases/latest):

| File | For |
|---|---|
| `dexterity-sheets-<version>-windows-x64.msi` | every PC: per-user install, no admin needed |
| `DexSheetsServer-<version>-x64.zip` | the one always-on office PC that hosts shared workbooks |
| `dexterity-sheets-<version>-windows-x64-portable.zip` | running without installing |
| `SHA256SUMS` | checksums |

Server setup (once): unzip on the server PC and run, in an elevated PowerShell,
`powershell -ExecutionPolicy Bypass -File .\Install-Server.ps1`. It creates the first
administrator, opens the firewall for the LAN, and starts the server at every boot. Then add
teammates with `DexSheetsServer user add <name> --role editor`. Details:
[docs/collaboration.md](docs/collaboration.md).

To let the server fetch new versions from this repository's releases, put
`update-source.json` in its data folder (`C:\ProgramData\Dexterity Sheets\Server`):

```json
{ "repo": "<owner>/<this repository>", "token": "<fine-grained read-only token, only if the repository is private>" }
```

## Build from source

Rust 1.95+ (MSVC). Everything for this project lives on `D:\GridCraft` on the build PC:
`. D:\GridCraft\env.ps1` keeps caches and temp files there.

```powershell
cargo build --release -p gridcraft -p gridcraft-server -p gridcraft-cli
# target\release\DexteritySheets.exe, DexSheetsServer.exe, gridcraft-cli.exe
pwsh packaging/windows/package.ps1 -Arch x64      # MSI + portable zip (needs WiX 5)
```

## How this repository relates to GridCraft

Dexterity Sheets is [GridCraft](https://github.com/storytold/gridcraft) (MIT / Apache-2.0) plus
Dexterity's layer: LAN co-authoring (`crates/collab`), the server (`apps/gridcraft-server`), the
sharing UI (`crates/ui-egui/src/collab_ui.rs`), branding, the per-user installer and the updater.

The **Upstream sync** workflow merges official GridCraft every 6 hours. A clean merge is tested
(format, clippy, engine tests, collaboration tests, build, end-to-end) and released
automatically; anything that needs judgement becomes a pull request. The `upstream` branch always
mirrors official GridCraft. See [packaging/dexterity/UPSTREAM.md](packaging/dexterity/UPSTREAM.md).

Versions: `packaging/dexterity/VERSION` is the Dexterity Sheets version (what users see and the
updater compares); `Cargo.toml` keeps upstream's GridCraft version.

## Quality

- `cargo test -p gridcraft-collab`: patch replay of every kind of edit, convergence of
  simultaneous edits in server order, undo that only takes back your own edits, and two people
  editing through a real TCP server (including a refused password and a view-only account).
- `packaging/dexterity/tests/lan_e2e.py`: three real app windows and a server; live edits,
  formulas recalculating across windows, attribution, conflicts, row inserts, undo, live cursors,
  a server crash with an offline edit delivered afterwards, and persistence across restarts.
- `packaging/dexterity/tests/update_e2e.py`: installs an old version, gets offered a new one by
  the server, installs it from inside the app and reopens.
- Excel parity: a harness drives Microsoft Excel through COM as a black-box oracle (formula
  results, number formats, files written by each app opened in the other).

## License

Built from GridCraft, dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE),
Copyright (c) 2026 ArtCraft Team and the GridCraft contributors; notices in [NOTICE](NOTICE),
assets in [ATTRIBUTION.md](ATTRIBUTION.md). Dexterity Sheets is not affiliated with or endorsed
by the ArtCraft team and ships none of its trademarks. Excel is a trademark of Microsoft; no
Microsoft code or assets are used.
