# Dexterity Sheets app icon

Original artwork by Dexterity Design Services Pvt. Ltd., drawn entirely in code by
[`packaging/dexterity/make_icon.py`](../../packaging/dexterity/make_icon.py), the single source
every size and format is generated from. The "linked cells" mark: a rounded tile shading from
deep indigo to teal, a 3 × 3 grid of spreadsheet cells, and three cells lit and joined by an amber
line with a node in each, rising like a tick: a spreadsheet, connected data, work done. Plain
geometry only (no font, no third-party art). It replaced the earlier blue "D" tile on 28 October
2026, and the upstream GridCraft icon before that.

## Palette

| Colour | Hex | Used for |
|---|---|---|
| Indigo → teal | `#1e2a78` → `#0e8c8c` | The tile (diagonal gradient, top left to bottom right) |
| White | `#ffffff` | The three lit cells and the node centres; the other cells at 22 % opacity |
| Amber | `#ffb020` | The line and its nodes |

## Geometry

A 512 × 512 design space, a rounded square with `rx=112`. Cells are 96 px with 24 px gaps from 88
to 424 (`rx=20`); the lit cells are column 1 row 2, column 2 row 3 and column 3 row 1, joined in
that order by a 30 px line with 25 px nodes. Sizes below 32 px drop the faint cells and the node
centres so the tick stays legible. The macOS icons (`.icns`, `dexterity-sheets-macos-512.png`) sit
on Apple's grid: an 824/1024 body with a transparent margin.

## Files

| File | Use |
|---|---|
| `dexterity-sheets.svg` | Canonical vector artwork; also `hicolor/scalable` |
| `dexterity-sheets-1024.png` | 1024 px render of the full tile |
| `dexterity-sheets-macos-512.png` | Runtime Dock icon on macOS (embedded by `apps/gridcraft`) |
| `dexterity-sheets.icns` | macOS bundle icon |
| `dexterity-sheets.ico` | Windows exe and installer icon, 16–256 px (embedded by `apps/gridcraft/build.rs`, used by `packaging/windows/package.ps1`) |
| `hicolor/<size>/apps/ai.storyteller.gridcraft.png` | Linux icon theme, 16–512 px; the 256 px one is also the runtime window/taskbar icon on Windows and Linux |
| `hicolor/scalable/apps/ai.storyteller.gridcraft.svg` | Linux scalable icon |

The hicolor names keep the internal app id `ai.storyteller.gridcraft` (Wayland app id, `.desktop`
file); only the artwork changed.

## Regenerate

`python packaging/dexterity/make_icon.py` (or `packaging/icons.sh`). Needs Pillow.
