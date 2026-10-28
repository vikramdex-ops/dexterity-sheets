# Dexterity Sheets parity audit: official GridCraft and Microsoft Excel

> **Audited:** 2026-10-28 · **Dexterity Sheets:** 1.2.2 (`main` at `b4ad76be`) · **GridCraft
> upstream:** `storytold/gridcraft` main at `11bb6738` (latest release v0.12.1, 2026-10-25) ·
> **Excel:** Microsoft 365 Current Channel Version 2610 (Build 20527.20088, 2026-10-21)

This is the audit `RULES.md` §2 refers to. Every `excel-release` issue item is checked against it,
and it records what Dexterity Sheets does, does partly, or does not do compared with official
GridCraft and with Microsoft Excel, so the office knows what to expect before moving a job across.

**Status:** *present* (works as in Excel for everyday use), *partial* (works, with a stated
limit), *missing*, *broken* (exists but does not work), *n/a* (does not apply to an offline LAN
office app; the reason is given). **Priority** is for an engineering office that works in Excel
all day: **H** gets in the way of daily work, **M** is noticed weekly, **L** is rare or niche.
The *Done* column is filled in as the gaps are closed (see the commits on `main`).

Clean-room: Excel was studied only from Microsoft's public release notes, support articles and
function reference; nothing from Microsoft's code, files, icons or wording beyond feature names
is used.

## How this was measured

| What | How (re-runnable) | Result |
|---|---|---|
| Upstream position | `git fetch upstream; git rev-list --count main..upstream/main` | 15 upstream commits not in `main` |
| Why the automatic sync stopped | `gh run view 40593366984 --log` (Upstream sync, 2026-10-27) | `status: conflict`: 4 overlay hooks could not be re-placed in `apps/gridcraft/Cargo.toml` and `packaging/windows/gridcraft.wxs` after upstream's per-user installer change (#2704) |
| Ribbon and menu commands | `cargo xtask parity` (catalog `crates/engine/src/catalog.rs`) | 275 / 291 (94.5%) |
| Registered commands | `gridcraft-cli commands --json` | 355 engine commands |
| Worksheet functions | `gridcraft-cli functions --json` diffed against Microsoft's "Excel functions (alphabetical)" page, fetched 2026-10-28 | 514 ours; 518 on Microsoft's page; 8 missing (below) |
| Reserved but unbuilt controls | `Action::Pending` entries in `crates/ui-egui/src/keytips/keytip_table/generated.rs` | 38 key tips reserved for commands that do not exist yet |
| Excel's recent changes | Current Channel release notes 2501–2610 and the monthly "What's New in Excel" posts, 2024–2026 | listed in each area below |

## 1. Official GridCraft (upstream)

| Feature | Status | Evidence | Priority | Done |
|---|---|---|---|---|
| Upstream merges reach Dexterity automatically | **broken** since 2026-10-27 | sync run 40593366984: overlay hooks `name = "DexteritySheets"`, `UpgradeCode`, `Scope="perUser"` and "no craft-update" could not be re-placed; issue #1 open | H | |
| Format Chart Area ▸ Chart Type offers stacked and 100% stacked types (#2605) | missing (upstream only) | `git log main..upstream/main` `0b6d2439` | M | |
| Faster XLSX worksheet writing (#2749) | missing (upstream only) | `393a524a` | M | |
| Drop-down menus grow back after a search with no matches (#2743) | missing (upstream only) | `63038650` | M | |
| Per-user default install folder and install-scope policy (#2704) | n/a: Dexterity has its own installer identity (per-user, its own UpgradeCode) | `packaging/windows/gridcraft.wxs`; `packaging/dexterity/check_overlay.py` | L | |
| Internet update checker (`craft-update`) | n/a: removed on purpose, the office server delivers updates and the app never contacts the internet | `apps/gridcraft/Cargo.toml` comment; `docs/collaboration.md` | — | |
| Upstream CI workflows, ArtCraft brand files and icons | n/a: deleted on purpose (Dexterity CI and branding replace them) | `git diff --name-status $(git merge-base main upstream/main) main` (22 deletions, all of these) | — | |
| Everything else in GridCraft 0.12.1 | present: the fork carries all of it and passes the same test suite | Dexterity CI run 40593990312 green on `b4ad76be` | — | |

## 2. Functions

| Feature | Status | Evidence | Priority | Done |
|---|---|---|---|---|
| 514 worksheet functions incl. dynamic arrays, LAMBDA family, GROUPBY, PIVOTBY, PERCENTOF, REGEX*, TRIMRANGE | present | `gridcraft-cli functions --json`; `crates/functions/src/dynamic.rs`, `text.rs` | — | |
| Trim-reference operators `.:` `:.` `.:.` | present | `crates/formula/src/lexer.rs:200` | — | |
| GETPIVOTDATA | missing | not in `gridcraft-cli functions`; only in the `.xls` token table `crates/xlsx/src/xls/ftab.rs:354` | H (engineering summaries pull figures out of PivotTables) | |
| XLOOKUP / XMATCH regex match mode (`match_mode` 3) | n/a for now: announced for Insiders in Aug 2024, but Microsoft's XLOOKUP reference (checked 2026-10-28) still lists only modes -1, 0, 1 and 2, so there is no public behaviour to match | `gridcraft-cli eval '=XMATCH("^ban",{"apple";"banana"},3)'` → `#VALUE!` (`crates/functions/src/lookup.rs:234`) | L (re-check when the reference documents it) | |
| IMAGE | n/a: fetches pictures from web addresses and the app never contacts the internet; pictures in cells (Insert ▸ Pictures ▸ Place in Cell) cover local images | `crates/ui-egui/src/cell_pictures.rs` | L | |
| TRANSLATE, DETECTLANGUAGE, STOCKHISTORY | n/a: Microsoft online services | Microsoft function pages ("requires internet") | L | |
| RTD, CALL, REGISTER.ID | n/a: COM servers and DLL calls; macros and add-ins never run here | `docs/function-parity.md` | L | |
| CUBE functions (7), WEBSERVICE, FILTERXML | partial: return an error by design (no OLAP or web access) | `docs/function-parity.md` | L | |
| COPILOT, PY | n/a: COPILOT was retired by Microsoft on 2026-09-14 and never left preview; PY runs Python in Microsoft's cloud | Microsoft COPILOT function page; Python in Excel availability page | — | |
| IMPORTTEXT, IMPORTCSV, FLATTEN, HAS/HASANY/HASALL, arrays in cells | n/a for now: Excel Insider (beta) builds only, not on Current Channel | Microsoft 365 Insider blog, Jan and Sep 2026 | L (re-check when they reach Current Channel) | |
| Compatibility Versions (calculation version 1/2) | partial: text functions already count Unicode characters (version 2 behaviour); no per-workbook switch | `crates/functions/src/text.rs` | L | |

## 3. Formatting

| Feature | Status | Evidence | Priority | Done |
|---|---|---|---|---|
| Fonts, fills, borders, alignment (incl. Center Across Selection), number formats with Ctrl+Shift+1…6 | present | `crates/model/src/style.rs:339`; `crates/ui-egui/src/keymap.rs` | — | |
| Conditional formatting (highlight rules, top/bottom, data bars, colour scales, icon sets, formulas) | present | `crates/model/src/features.rs` (CF rules); `docs/ui-parity.md` | — | |
| Cell styles, Format as Table, themes, Format Painter | present | `gridcraft-cli commands --search style` | — | |
| Checkboxes in cells (Excel 2024) | present | `crates/engine/src/cmd/extra/cells.rs`; Space toggles (`crates/ui-egui/src/grid/keyboard.rs:139`) | — | |
| Border pen (Draw Border, Erase Border, Line Color, Line Style) | missing | `home.eraseBorder`, `home.borderLineColor`, `home.borderLineStyle` reserved in `keytip_table/generated.rs:45` | L (the Borders gallery and Format Cells cover it) | |
| Dark grid in dark mode (Excel 2503) | partial: dark ribbon and panes, the cell grid stays white | `crates/ui-egui/src/theme.rs:178` (`grid_bg` is white in both palettes) | L | |

## 4. Charts

| Feature | Status | Evidence | Priority | Done |
|---|---|---|---|---|
| Column, bar, line, pie, doughnut, area, XY scatter, bubble, radar, stock, combo, histogram, box & whisker, waterfall, funnel, treemap, sunburst (26 subtypes) | present | `crates/model/src/features.rs:404` (`ChartKind`) | — | |
| Trendlines with equation and R², smoothed lines, secondary axis, data labels, titles, legend | present | `crates/model/src/features.rs:476` (`Trendline`), `Series::smooth` | — | |
| Axis bounds, major unit and logarithmic scale (Format Axis) | missing | `Chart` (`crates/model/src/features.rs:544`) has no axis scaling fields | **H** (XY plots of pump, pressure and temperature data need fixed bounds and log axes) | |
| Error bars, per-point formatting | missing | same struct | M | |
| 3-D charts, surface, pie of pie / bar of pie, Pareto | missing | `docs/chart-parity.md` | L | |
| Filled map (Insert ▸ Maps) | n/a: needs online map data | `catalog.rs:126` | L | |

## 5. Data tools

| Feature | Status | Evidence | Priority | Done |
|---|---|---|---|---|
| Sort, filter, advanced filter, slicers, remove duplicates, text to columns, Flash Fill, consolidate, subtotal, group and outline | present | `gridcraft-cli commands` (`data.*`, 38 commands) | — | |
| PivotTables, Recommended PivotTables, PivotCharts | present | `crates/engine/src/cmd/pivot.rs` | — | |
| Goal Seek, Scenario Manager, Data Table, Solver, Analysis ToolPak, Forecast Sheet | present | `crates/engine/src/cmd/whatif.rs`, `solver.rs`, `analysis.rs`, `forecast.rs` | — | |
| Data validation and Circle Invalid Data | **broken**: Circle Invalid Data finds the cells but nothing is drawn on the sheet, so the menu item looks dead | `crates/engine/src/cmd/data/validation.rs:272` returns a list; no painter uses it (`grep -ri circle crates/ui-egui/src/grid`) | H | |
| Clear Validation Circles | missing | `keytip_table/generated.rs:311` reserved | H (goes with the fix above) | |
| Auto Outline, Clear Outline | missing | `keytip_table/generated.rs:276`, `:307` reserved | M | |
| Get Data: From Text/CSV, From Table/Range, Queries & Connections, Refresh All | present | `data.fromTextCsv`, `data.fromTableRange`, `data.queriesConnections` | — | |
| Get Data from Web, databases, other sources; Power Query editor | missing (web: n/a, no internet) | `catalog.rs:196`; `keytip_table/generated.rs` (`gap #12` entries) | M (from folder / workbook would help; a full Power Query is out of scope for now) | |
| Stocks and Geography data types, Analyze Data, Clean Data (Copilot) | n/a: Microsoft online services | `catalog.rs:200`, `:201`, `:225` | — | |
| Relationships / data model (Power Pivot) | missing | `catalog.rs:215` | L | |
| PivotTable auto refresh | n/a for now: Excel Insider only | Microsoft "Refresh PivotTable data" page | L | |

## 6. File formats

| Feature | Status | Evidence | Priority | Done |
|---|---|---|---|---|
| `.xlsx` / `.xlsm` / `.xltx` read and write, password-protected `.xlsx` open, CSV/TSV, PDF and HTML export | present | `docs/file-format-parity.md`; `crates/xlsx/src/tests/` | — | |
| Unmodelled `.xlsx` parts (custom XML, external links, connections, form controls, OLE objects) kept on save | present | `crates/xlsx/src/passthrough.rs` | — | |
| Slicer caches, timelines, data model, chart details | partial: dropped on save | `docs/file-format-parity.md` (Not preserved) | M | |
| `.xls` (Excel 97-2003) | partial: values, formulas, number formats, merges and sizes; no fonts, fills, borders or alignment | `docs/file-format-parity.md`; `crates/xlsx/src/xls` | M (old client files open without their formatting) | |
| `.xlsb`, `.ods` | partial: values only | `docs/file-format-parity.md` | L | |
| Save with a password | missing | `docs/file-format-parity.md` ("Encrypted workbooks: save with password") | M | |

## 7. Collaboration

| Feature | Status | Evidence | Priority | Done |
|---|---|---|---|---|
| Live co-authoring on the office LAN, from any folder, with presence and named accounts | present (Dexterity) | `docs/collaboration.md`; `packaging/dexterity/tests/lan_e2e.py` | — | |
| Edit history (who changed which cell, when, old → new) and versions | present (Dexterity) | `crates/ui-egui/src/collab_ui.rs:1493` (`history_pane`) | — | |
| Review ▸ Show Changes | missing as a command (the same history is under Review ▸ Edit History) | `catalog.rs:248`; `keytip_table/generated.rs:323` reserved | H (the name Excel users look for) | |
| Threaded comments with replies and resolve, notes | present | `review.*` commands; `docs/file-format-parity.md` (#300) | — | |
| @mentions in comments, comment filtering | missing | no `mention` in `crates/` | L | |
| Sheet View (personal sort/filter while co-authoring) | missing | no sheet-view feature in `crates/ui-egui/src` | L | |
| Track Changes (legacy shared workbook) | n/a: replaced by live co-authoring and edit history, as in Excel | — | — | |

## 8. UI and ribbon

| Feature | Status | Evidence | Priority | Done |
|---|---|---|---|---|
| Ribbon (11 tabs plus contextual tabs), backstage, task panes, key tips | present | `cargo xtask parity`; `crates/ui-egui/src/ribbon/` | — | |
| Search / Tell Me box (Alt+Q) | partial: a command search opens from the title bar's search icon and Automate tab, but not from Alt+Q, and it matches only exact substrings | `crates/ui-egui/src/dialogs/command_search.rs`; `keytip_table/generated.rs:373` (`ui.search` reserved) | **H** (finding a command by name is how people cope with an unfamiliar ribbon) | |
| Focus Cell (View ▸ Focus Cell, Excel Dec 2024) | missing | `keytip_table/generated.rs:354` reserved | M | |
| Formulas ▸ Remove Arrows ▸ Remove Precedent / Dependent Arrows | partial: only Remove Arrows (all) | `formulas.removeArrows` present; `keytip_table/generated.rs:226` reserved | M | |
| Fill ▸ Across Worksheets, grouped sheets | missing | `keytip_table/generated.rs:82` | L | |
| Select Objects pointer, Navigation pane, Reset Window Position | missing | `keytip_table/generated.rs:75`, `:354`… | L | |
| Insert Screenshot, Equation, WordArt, Object, SmartArt, Signature Line, Timeline | missing | `catalog.rs:121`–`137` | L | |
| Quick Analysis (Ctrl+Q) | missing | `docs/ui-parity.md:72` | M | |
| Customize Ribbon / Quick Access Toolbar | missing | `docs/ui-parity.md:20` | L | |
| Smart Lookup, Translate (Review) | n/a: Microsoft online services | `catalog.rs:233`, `:234` | — | |
| Copilot, Agent Mode, Python in Excel, Formula by Example | n/a: Microsoft cloud AI services | Excel release notes 2408–2603 | — | |
| Name the table in Create Table (Excel Oct 2026) | present in the command (`insert.table {name}`), to be checked in the dialog in the UI audit | `crates/engine/src/cmd/insert.rs:34` | L | |

## 9. Keyboard shortcuts

| Feature | Status | Evidence | Priority | Done |
|---|---|---|---|---|
| Navigation, selection, editing, Ctrl+letter commands, number-format and border combinations, F2/F3/F5/F7/F9, Shift+F2/F3/F10, Ctrl+[ ], Alt+= | present | `crates/ui-egui/src/keymap.rs`; `docs/ui-parity.md` §shortcuts | — | |
| F4 / Ctrl+Y outside the formula bar: Repeat last action | missing: F4 on the grid does nothing | `crates/ui-egui/src/grid/keyboard.rs:145` (`Key::F4 if !cmd => {}`) | **H** (used constantly to repeat formatting, inserts and deletes) | |
| Alt+Q (Search) | missing | see §8 | H | |
| Alt+W E F (Focus Cell) | missing | see §8 | M | |
| F8 extend mode, Shift+F8 add mode, F6 pane cycling, Ctrl+Q | missing | `docs/ui-parity.md:139`–`142` | L | |

## Work order (phase 2)

Highest priority first, in batches of related features. Before building anything, upstream
GridCraft's open pull requests were searched (`gh pr list -R storytold/gridcraft --search …`):
where upstream already has the feature in review, taking that pull request keeps the automatic
sync clean (upstream merges with merge commits, so the same commits arrive again without a
conflict), and building a second version would make every later sync conflict.

1. Bring upstream GridCraft in (fix the blocked sync: re-place the four overlay hooks).
2. Repeat last action (F4 / Ctrl+Y); Circle Invalid Data drawn on the sheet plus Clear Validation
   Circles; Remove Precedent / Dependent Arrows; Auto Outline and Clear Outline; Focus Cell;
   Review ▸ Show Changes. Built in Dexterity's own files (`crates/engine/src/cmd/dex.rs`,
   `crates/ui-egui/src/dex_ui.rs`). Upstream's F4 (#2087) and Focus Cell (#1376, #1884) pull
   requests are hundreds of commits behind and conflict, so they could not be taken; a Dexterity
   command whose id upstream later registers gives way to upstream's automatically.
3. GETPIVOTDATA from upstream pull request #290 and the Alt+Q search box from #1339: both merge
   cleanly onto current upstream.
4. Chart axis bounds, major unit and logarithmic scale: **not built here**. Upstream pull request
   #1290 (fixed axis bounds and units) is in review and issue #937 (log scale) is open; both
   change the shared `Chart` model, which 15 upstream files construct, so a fork-only version
   would conflict on every upstream chart change. Taken when upstream merges it.

Medium and low items stay listed above with their reason until they are taken up.
