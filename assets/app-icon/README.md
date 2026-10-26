# Dexterity Sheets app icon

Original artwork by Dexterity Design Services Pvt. Ltd., drawn entirely in code by
[`packaging/dexterity/make_icon.py`](../../packaging/dexterity/make_icon.py): a deep-blue rounded
tile, a faint white spreadsheet grid and a bold white "D" made of plain geometry (a stem and a
half-ellipse bowl; no font, no third-party art). It replaces the upstream GridCraft placeholder
icon.

## Palette

| Colour | Hex | Used for |
|---|---|---|
| Deep blue | `#0d3b8c` → `#082866` | The tile (vertical gradient) |
| White | `#ffffff` | The "D"; the grid at 36 % opacity |

## Geometry

A 512 × 512 design space, clipped to a rounded square with `rx=112`. Windows and Linux use the
full tile; sizes below 32 px drop the grid so the "D" stays legible. The macOS icons (`.icns`,
`dexterity-sheets-macos-512.png`) sit on Apple's grid: an 824/1024 body with a transparent margin.

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
