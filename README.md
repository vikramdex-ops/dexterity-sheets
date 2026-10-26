# Dexterity Sheets

**Dexterity Sheets** (DexSheets) is the internal spreadsheet application of
**Dexterity Design Services Pvt. Ltd.** It is a native Windows desktop app (it also builds on
macOS and Linux) that opens, edits and saves `.xlsx`/`.xlsm` workbooks and CSV/TSV files.

It is an internal LAN tool: the application makes **no internet calls** (no update checker, no
web links, no telemetry). New versions are distributed by IT.

Dexterity Sheets is based on [GridCraft](https://github.com/storytold/gridcraft) by the ArtCraft
team, an open-source Rust spreadsheet, used under its MIT / Apache-2.0 licence (see
[License](#license)).

## Build

Needs Rust 1.95 or newer (MSVC toolchain on Windows).

```sh
cargo build --release -p gridcraft
```

The desktop executable is `target/release/DexteritySheets.exe` (`DexteritySheets` on macOS and
Linux). The Cargo package keeps its upstream name, `gridcraft`, as do the other crates and the
`gridcraft-cli` command-line tool.

## Run

```sh
cargo run --release -p gridcraft                          # desktop app
cargo run --release -p gridcraft -- book.xlsx             # open a workbook
cargo run --release -p gridcraft -- --sample sales        # open a sample workbook
```

Settings and logs live in `%APPDATA%\Dexterity Sheets\` on Windows (`logs\` holds the log
files), `~/Library/Application Support/Dexterity Sheets/` on macOS and
`~/.config/dexterity-sheets/` on Linux, so they never collide with a stock GridCraft install.

## Windows installer

`packaging/windows/package.ps1` builds the signed MSI and a portable zip from
`packaging/windows/gridcraft.wxs` (product "Dexterity Sheets", manufacturer Dexterity Design
Services Pvt. Ltd.). It needs the WiX Toolset v5.

## LAN server

Shared editing over the office network uses the `gridcraft-server` LAN server. It is described in
[docs/collaboration.md](docs/collaboration.md).

## App icon

The icon (a blue tile with a white grid and a bold "D") is original artwork generated in code by
`packaging/dexterity/make_icon.py`; see [assets/app-icon/README.md](assets/app-icon/README.md).

## License

The GridCraft source this application is built from is dual-licensed under [MIT](LICENSE-MIT) or
[Apache-2.0](LICENSE-APACHE). Copyright (c) 2026 ArtCraft Team and the GridCraft contributors.
Required notices are in [NOTICE](NOTICE); third-party assets are listed in
[ATTRIBUTION.md](ATTRIBUTION.md). The About dialog credits the upstream project.

Dexterity Sheets is not affiliated with or endorsed by the ArtCraft team, and ships none of the
ArtCraft trademarks.
