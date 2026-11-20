//! Highlight Row/Column: a translucent band across the active cell's row and/or column (the
//! selection's rows and columns, for a range), so the eye keeps its place in a wide register.
//!
//! One feature with three ways in: the status bar's highlight button (bottom right, by the zoom
//! controls), View ▸ Highlight on the ribbon and Ctrl+Alt+H. All of them run the engine command
//! `view.focusCell` (Excel's Focus Cell), whose state ([`gridcraft_engine::cmd::dex::DexState`])
//! this module paints and remembers per user in `%APPDATA%\Dexterity Sheets\highlight.json`.
//! GridCraft's own "Highlight the active cell's row" option (File ▸ Options ▸ General) is the same
//! setting seen from there: ticking it turns the row band on here, and it shows ticked whenever the
//! row band is on. Its own painter is switched off by a hook in `grid/show.rs`, so nothing is
//! painted twice.
//!
//! The band is painted over the cells (fills, text, borders) and under the selection, so a
//! register's pink or yellow fills stay visible through it; a thin edge in the band's colour keeps
//! the band readable where it crosses a fill of the same hue.

use egui::{Color32, Key, Modifiers, Painter, Rect, Sense, Stroke, Ui, pos2, vec2};
use gridcraft_engine::cmd::dex::FocusMode;
use gridcraft_engine::core::{CellRef, RangeRef};
use gridcraft_engine::model::Sheet;
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};

use crate::SheetApp;
use crate::grid::Geo;
use crate::icons::Icon;
use crate::theme::Tokens;

/// The keyboard shortcut (free in Excel and everywhere in this app; checked against `keymap.rs`,
/// `ribbon/shortcuts.rs`, `grid/keyboard.rs` and the formula bar).
pub const SHORTCUT: &str = "Ctrl+Alt+H";

/// The colours offered (name, `#RRGGBB`), after "None". Green is GridCraft's own row-highlight
/// colour and the default.
pub const SWATCHES: &[(&str, &str)] = &[
    ("Blue", "#2F80ED"),
    ("Green", "#107C41"),
    ("Yellow", "#F2C200"),
    ("Orange", "#F2780C"),
    ("Pink", "#E0529C"),
    ("Purple", "#8E5CD9"),
    ("Grey", "#808080"),
];
pub const DEFAULT_COLOR: &str = "#107C41";

fn parse_hex(s: &str) -> Option<Color32> {
    let h = s.trim_start_matches('#');
    let v = u32::from_str_radix(h, 16).ok().filter(|_| h.len() == 6)?;
    Some(Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

/// The highlight colour picked (`#RRGGBB`), or the default.
pub fn color_hex(app: &SheetApp) -> &str {
    app.session.dex.focus_color.as_deref().unwrap_or(DEFAULT_COLOR)
}

/// The band's fill for colour `hex` in the light or dark theme: translucent, so cell fills show
/// through. Green is exactly GridCraft's own row-highlight token.
pub fn tint(hex: &str, dark: bool) -> Color32 {
    if hex.eq_ignore_ascii_case(DEFAULT_COLOR) {
        return if dark { Tokens::dark().active_row } else { Tokens::light().active_row };
    }
    let c = parse_hex(hex).unwrap_or(Color32::from_rgb(0x10, 0x7C, 0x41));
    let [r, g, b, _] = c.to_array();
    if dark {
        // Lifted toward white so a dark hue still reads on a dark grid.
        let lift = |v: u8| v.saturating_add(((255 - u16::from(v)) * 2 / 5) as u8);
        Color32::from_rgba_unmultiplied(lift(r), lift(g), lift(b), 0x34)
    } else {
        // Light hues (yellow, orange) need more weight to show on white.
        let luma = (u32::from(r) * 299 + u32::from(g) * 587 + u32::from(b) * 114) / 1000;
        Color32::from_rgba_unmultiplied(r, g, b, if luma > 150 { 0x48 } else { 0x2C })
    }
}

/// The stronger tint of the highlighted rows' and columns' headers, and the band's edge.
fn strong(hex: &str, dark: bool) -> Color32 {
    let [r, g, b, a] = tint(hex, dark).to_srgba_unmultiplied();
    Color32::from_rgba_unmultiplied(r, g, b, a.saturating_mul(3).min(0xB0))
}

fn dark(p: &Painter) -> bool {
    p.ctx().global_style().visuals.dark_mode
}

// ---------------------------------------------------------------------------------------------
// Geometry (pure, tested in `tests/it/dex_parity.rs`)

/// Inclusive (first, last) row or column spans, sorted and merged.
pub type Spans = Vec<(u32, u32)>;

/// Sorted, merged inclusive spans.
fn merge(mut v: Vec<(u32, u32)>) -> Vec<(u32, u32)> {
    v.sort_unstable();
    let mut out: Vec<(u32, u32)> = Vec::with_capacity(v.len());
    for (a, b) in v {
        match out.last_mut() {
            Some(last) if a <= last.1.saturating_add(1) => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// The rows and the columns a selection highlights: each area's rows (unless the area is whole
/// columns, which highlights every row anyway) and columns (unless it is whole rows). Selecting the
/// whole sheet highlights nothing.
pub fn spans(ranges: &[RangeRef], mode: FocusMode) -> (Spans, Spans) {
    let rows = ranges.iter().filter(|r| mode.rows() && !r.is_full_cols()).map(|r| (r.start.row, r.end.row)).collect();
    let cols = ranges.iter().filter(|r| mode.cols() && !r.is_full_rows()).map(|r| (r.start.col, r.end.col)).collect();
    (merge(rows), merge(cols))
}

/// `span` minus every span of `cut` (both sorted and merged).
fn subtract(span: (u32, u32), cut: &[(u32, u32)]) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    let mut from = span.0;
    for &(a, b) in cut {
        if b < from || a > span.1 {
            continue;
        }
        if a > from {
            out.push((from, a - 1));
        }
        from = b.saturating_add(1);
        if from > span.1 || b == u32::MAX {
            return out;
        }
    }
    out.push((from, span.1));
    out
}

/// One painted band: its rectangle and whether it runs across (a row band) or down.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Band {
    pub rect: Rect,
    pub across: bool,
}

/// The bands of the selection `ranges` in one pane of the grid (`rows` x `cols`): a row band
/// across the pane's columns for each highlighted row span, and a column band down the pane for
/// each highlighted column span, minus where it crosses a row band (so the crossing is not tinted
/// twice). Hidden rows and columns give no band.
pub fn bands(geo: &Geo, sh: &Sheet, ranges: &[RangeRef], mode: FocusMode, rows: (u32, u32), cols: (u32, u32)) -> Vec<Band> {
    let mut out = Vec::new();
    if rows.0 > rows.1 || cols.0 > cols.1 {
        return out;
    }
    let (row_spans, col_spans) = spans(ranges, mode);
    let mut push = |r: RangeRef, across: bool| {
        let rect = geo.range_rect(sh, r);
        if rect.width() > 0.0 && rect.height() > 0.0 {
            out.push(Band { rect, across });
        }
    };
    let in_rows: Vec<(u32, u32)> = row_spans.iter().filter(|s| s.0 <= rows.1 && s.1 >= rows.0).map(|s| (s.0.max(rows.0), s.1.min(rows.1))).collect();
    for &(a, b) in &in_rows {
        push(RangeRef::new(CellRef::new(a, cols.0), CellRef::new(b, cols.1)), true);
    }
    for &(a, b) in col_spans.iter().filter(|s| s.0 <= cols.1 && s.1 >= cols.0) {
        let (a, b) = (a.max(cols.0), b.min(cols.1));
        for (r0, r1) in subtract(rows, &in_rows) {
            push(RangeRef::new(CellRef::new(r0, a), CellRef::new(r1, b)), false);
        }
    }
    out
}

fn paint_bands(p: &Painter, list: &[Band], fill: Color32, edge: Option<Color32>) {
    for b in list {
        p.rect_filled(b.rect, 0.0, fill);
        if let Some(e) = edge {
            let s = Stroke::new(1.0, e);
            let r = b.rect;
            if b.across {
                p.line_segment([pos2(r.left(), r.top() + 0.5), pos2(r.right(), r.top() + 0.5)], s);
                p.line_segment([pos2(r.left(), r.bottom() - 0.5), pos2(r.right(), r.bottom() - 0.5)], s);
            } else {
                p.line_segment([pos2(r.left() + 0.5, r.top()), pos2(r.left() + 0.5, r.bottom())], s);
                p.line_segment([pos2(r.right() - 0.5, r.top()), pos2(r.right() - 0.5, r.bottom())], s);
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Painting (called from `dex_ui::paint`, the header hook and `collab_ui::paint_peers`)

/// The band of the person's own selection in one pane. The caller has checked `sh` is the active
/// sheet. It is the same while a cell is being edited (the selection stays where it was).
pub fn paint(p: &Painter, geo: &Geo, sh: &Sheet, app: &SheetApp, rows: (u32, u32), cols: (u32, u32)) {
    let dex = &app.session.dex;
    let Some(d) = app.session.active() else { return };
    if !dex.focus_cell {
        return;
    }
    let hex = color_hex(app);
    let dark = dark(p);
    let list = bands(geo, sh, &d.selection.ranges, dex.focus_mode, rows, cols);
    paint_bands(p, &list, tint(hex, dark), Some(strong(hex, dark).gamma_multiply(0.6)));
}

/// A teammate's band, faint, in their presence colour (`collab_ui::paint_peers`).
#[allow(clippy::too_many_arguments)]
pub fn paint_peer(p: &Painter, geo: &Geo, sh: &Sheet, app: &SheetApp, ranges: &[RangeRef], color: Color32, rows: (u32, u32), cols: (u32, u32)) {
    let dex = &app.session.dex;
    if !dex.focus_peers {
        return;
    }
    let list = bands(geo, sh, ranges, dex.focus_mode, rows, cols);
    paint_bands(p, &list, color.gamma_multiply(if dark(p) { 0.16 } else { 0.10 }), None);
}

/// The highlighted rows' and columns' headers, tinted stronger than the band. `quads` are the
/// panes as painted: (rows, cols, screen rectangle). Normal and Page Break Preview views only.
pub fn paint_headers(p: &Painter, geo: &Geo, sh: &Sheet, app: &SheetApp, quads: &[crate::grid::PaneQuad]) {
    let dex = &app.session.dex;
    if !dex.focus_cell || geo.paper.is_some() {
        return;
    }
    let Some(d) = app.session.active() else { return };
    if d.wb.active().is_none_or(|a| a.name != sh.name) {
        return;
    }
    let fill = strong(color_hex(app), dark(p));
    let (row_spans, col_spans) = spans(&d.selection.ranges, dex.focus_mode);
    let top = Rect::from_min_max(pos2(geo.cells.left(), geo.head_top()), pos2(geo.rect.right(), geo.cells.top()));
    let left = Rect::from_min_max(pos2(geo.head_left(), geo.cells.top()), pos2(geo.cells.left(), geo.rect.bottom()));
    // Panes share header strips (the frozen rows' pane and the one below it share the column
    // headers): each strip is tinted once.
    let mut done: Vec<(bool, u32, u32, i32)> = Vec::new();
    for (rows, cols, clip) in quads {
        let strip = Rect::from_min_max(pos2(clip.left(), top.top()), pos2(clip.right(), top.bottom()));
        for &(a, b) in col_spans.iter().filter(|s| s.0 <= cols.1 && s.1 >= cols.0) {
            let (a, b) = (a.max(cols.0), b.min(cols.1));
            let key = (false, a, b, clip.left() as i32);
            if top.height() <= 0.0 || done.contains(&key) {
                continue;
            }
            done.push(key);
            let r = Rect::from_min_max(pos2(geo.x(sh, a), top.top()), pos2(geo.x_end(sh, b), top.bottom()));
            p.with_clip_rect(strip.intersect(p.clip_rect())).rect_filled(r, 0.0, fill);
        }
        let strip = Rect::from_min_max(pos2(left.left(), clip.top()), pos2(left.right(), clip.bottom()));
        for &(a, b) in row_spans.iter().filter(|s| s.0 <= rows.1 && s.1 >= rows.0) {
            let (a, b) = (a.max(rows.0), b.min(rows.1));
            let key = (true, a, b, clip.top() as i32);
            if left.width() <= 0.0 || done.contains(&key) {
                continue;
            }
            done.push(key);
            let r = Rect::from_min_max(pos2(left.left(), geo.y(sh, a)), pos2(left.right(), geo.y_end(sh, b)));
            p.with_clip_rect(strip.intersect(p.clip_rect())).rect_filled(r, 0.0, fill);
        }
    }
}

/// GridCraft's own row band (`QuadrantPaint::active_row`) in Normal view: none, since this
/// module paints the row (with the colour and mode picked here). Page Layout still uses
/// GridCraft's band, which [`tick`] keeps in step with this setting.
pub fn upstream_row(_app: &SheetApp, _row: u32) -> Option<u32> {
    None
}

// ---------------------------------------------------------------------------------------------
// Controls

/// The status bar's highlight button (bottom right, beside the zoom controls): a click toggles the
/// highlight, the arrow (or a right-click) opens the colour and mode popover. Drawn in the status
/// bar's right-to-left layout.
pub fn status_button(app: &mut SheetApp, ui: &mut Ui, t: &Tokens) {
    let on = app.session.dex.focus_cell;
    let lang = app.ui.language;
    let (arrow_rect, arrow) = ui.allocate_exact_size(vec2(12.0, 20.0), Sense::click());
    let (main_rect, main) = ui.allocate_exact_size(vec2(24.0, 20.0), Sense::click());
    let hovered = main.hovered() || arrow.hovered();
    if on {
        ui.painter().rect_filled(main_rect.union(arrow_rect), 3.0, t.pressed);
    } else if hovered {
        ui.painter().rect_filled(main_rect.union(arrow_rect), 3.0, t.hover);
    }
    let dark = ui.visuals().dark_mode;
    paint_icon(ui.painter(), Rect::from_center_size(main_rect.center(), vec2(14.0, 14.0)), on.then(|| strong(color_hex(app), dark)), t.text_dim);
    crate::icons::paint(ui.painter(), Rect::from_center_size(arrow_rect.center(), vec2(8.0, 8.0)), Icon::Chevron, t.text_dim);
    let tip = format!("{} ({SHORTCUT})", lang.tr(if on { "Highlight Row/Column: on" } else { "Highlight Row/Column: off" }));
    let main = main.on_hover_text(tip);
    ui.ctx().accesskit_node_builder(main.id, |node| node.set_label("Highlight Row/Column"));
    let arrow = arrow.on_hover_text(lang.tr("Highlight colour and mode"));
    if main.clicked() {
        app.run_or_alert("view.focusCell", json!({}));
    }
    popover_on(app, &arrow);
    egui::Popup::context_menu(&main).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| popover(app, ui));
    ui.add_space(10.0);
}

fn popover_on(app: &mut SheetApp, r: &egui::Response) {
    egui::Popup::menu(r).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| popover(app, ui));
}

/// A small grid with its middle row and column tinted: the button's icon.
fn paint_icon(p: &Painter, r: Rect, band: Option<Color32>, line: Color32) {
    let third = r.width() / 3.0;
    if let Some(c) = band {
        p.rect_filled(Rect::from_min_size(pos2(r.left(), r.top() + third), vec2(r.width(), third)), 0.0, c);
        p.rect_filled(Rect::from_min_size(pos2(r.left() + third, r.top()), vec2(third, r.height())), 0.0, c);
    }
    let s = Stroke::new(1.0, line);
    p.rect_stroke(r, 1.0, s, egui::StrokeKind::Inside);
    for i in 1..3 {
        let d = third * i as f32;
        p.line_segment([pos2(r.left() + d, r.top()), pos2(r.left() + d, r.bottom())], s);
        p.line_segment([pos2(r.left(), r.top() + d), pos2(r.right(), r.top() + d)], s);
    }
}

/// The popover: None and the colours (the current one ticked), Row / Column / Both, and the
/// teammates option.
pub fn popover(app: &mut SheetApp, ui: &mut Ui) {
    let lang = app.ui.language;
    let dex = &app.session.dex;
    let (on, mode, peers) = (dex.focus_cell, dex.focus_mode, dex.focus_peers);
    let current = color_hex(app).to_string();
    let t = Tokens::get(ui.ctx());
    ui.set_min_width(230.0);
    ui.strong(lang.tr("Highlight Row/Column"));
    ui.add_space(4.0);
    let mut run: Option<Json> = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let none = std::iter::once(("None", None));
        for (name, hex) in none.chain(SWATCHES.iter().map(|(n, h)| (*n, Some(*h)))) {
            let (r, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
            let ticked = match hex {
                None => !on,
                Some(h) => on && h.eq_ignore_ascii_case(&current),
            };
            let sw = r.shrink(2.0);
            match hex.and_then(parse_hex) {
                Some(c) => {
                    ui.painter().rect_filled(sw, 3.0, c);
                }
                None => {
                    ui.painter().rect_filled(sw, 3.0, t.grid_bg);
                    ui.painter().line_segment([sw.left_bottom(), sw.right_top()], Stroke::new(1.5, t.danger));
                }
            }
            let ring = if ticked {
                Stroke::new(2.0, t.accent)
            } else if resp.hovered() {
                Stroke::new(1.0, t.text)
            } else {
                Stroke::new(1.0, t.text_disabled)
            };
            ui.painter().rect_stroke(sw, 3.0, ring, egui::StrokeKind::Outside);
            if ticked {
                let mark = if hex.is_some_and(|h| h.eq_ignore_ascii_case("#F2C200")) {
                    Color32::BLACK
                } else if hex.is_some() {
                    Color32::WHITE
                } else {
                    t.text
                };
                crate::icons::paint(ui.painter(), sw.shrink(3.0), Icon::Check, mark);
            }
            let resp = resp.on_hover_text(lang.tr(name));
            ui.ctx().accesskit_node_builder(resp.id, |node| node.set_label(name));
            if resp.clicked() {
                run = Some(json!({"color": hex.unwrap_or("none")}));
            }
        }
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        for (m, label) in [(FocusMode::Row, "Row"), (FocusMode::Column, "Column"), (FocusMode::Both, "Both")] {
            if ui.selectable_label(mode == m, lang.tr(label)).clicked() {
                run = Some(json!({"mode": m.as_str()}));
            }
        }
    });
    ui.add_space(4.0);
    let mut p = peers;
    if ui.checkbox(&mut p, lang.tr("Show teammates' rows and columns when co-editing")).changed() {
        run = Some(json!({"peers": p}));
    }
    ui.add_space(4.0);
    ui.label(egui::RichText::new(format!("{} {SHORTCUT}", lang.tr("Turn on or off:"))).small().color(t.text_dim));
    if let Some(params) = run {
        app.run_or_alert("view.focusCell", params);
    }
}

/// View ▸ Show ▸ Highlight on the ribbon: the button toggles it, the arrow opens the popover.
pub fn ribbon_button(app: &mut SheetApp, ui: &mut Ui) {
    let on = app.session.dex.focus_cell;
    let (main, arrow) = crate::widgets::small_split_button(
        ui,
        // The check mark shows it is on (an icon: not every interface font has a ✓ glyph).
        if on { Icon::Check } else { Icon::Gridlines },
        "Highlight",
        "Highlight Row/Column (Ctrl+Alt+H): tint the active cell's row and column",
    );
    if main.clicked() {
        app.run_or_alert("view.focusCell", json!({}));
    }
    popover_on(app, &arrow);
}

// ---------------------------------------------------------------------------------------------
// Commands, the shortcut and the remembered setting

/// What is remembered per user.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Prefs {
    pub on: bool,
    pub color: Option<String>,
    pub mode: String,
    pub peers: bool,
}

impl Default for Prefs {
    fn default() -> Prefs {
        Prefs { on: false, color: None, mode: "both".into(), peers: false }
    }
}

/// Front-end state: whether the remembered setting was read, and what was last saved.
#[derive(Default)]
pub struct Highlight {
    loaded: bool,
    saved: Option<Prefs>,
}

fn prefs_path() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("APPDATA")?;
    Some(std::path::PathBuf::from(base).join("Dexterity Sheets").join("highlight.json"))
}

fn snapshot(app: &SheetApp) -> Prefs {
    let d = &app.session.dex;
    Prefs { on: d.focus_cell, color: d.focus_color.clone(), mode: d.focus_mode.as_str().into(), peers: d.focus_peers }
}

fn apply(app: &mut SheetApp, p: &Prefs) {
    let d = &mut app.session.dex;
    d.focus_cell = p.on;
    d.focus_color = p.color.as_deref().filter(|c| c.len() == 7 && parse_hex(c).is_some()).map(str::to_ascii_uppercase);
    d.focus_mode = FocusMode::parse(&p.mode).unwrap_or_default();
    d.focus_peers = p.peers;
}

fn load() -> Option<Prefs> {
    let b = std::fs::read(prefs_path()?).ok()?;
    serde_json::from_slice(&b).ok()
}

fn save(p: &Prefs) {
    let Some(path) = prefs_path() else { return };
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if let Ok(b) = serde_json::to_vec_pretty(p) {
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, b).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

/// Once a frame (`collab_ui::tick`): reads the remembered setting at start, takes the shortcut
/// (before the grid or a cell being edited sees it), saves a changed setting and keeps GridCraft's
/// row-highlight option in step.
pub fn tick(app: &mut SheetApp, ctx: &egui::Context) {
    if !app.collab.highlight.loaded {
        app.collab.highlight.loaded = true;
        let remembered = if crate::dex_ui::desktop() { load() } else { None };
        match remembered {
            Some(p) => apply(app, &p),
            // Someone who had GridCraft's row highlight on keeps it.
            None if app.ui.highlight_active_row => {
                app.session.dex.focus_cell = true;
                app.session.dex.focus_mode = FocusMode::Row;
            }
            None => {}
        }
        app.collab.highlight.saved = Some(snapshot(app));
    }
    // AltGr is Ctrl+Alt on Windows: a key that typed a character is text, not this shortcut.
    let typed = ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Text(_))));
    if !typed && ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND | Modifiers::ALT, Key::H)) {
        app.run_or_alert("view.focusCell", json!({}));
    }
    let now = snapshot(app);
    if app.collab.highlight.saved.as_ref() != Some(&now) {
        if crate::dex_ui::desktop() {
            save(&now);
        }
        app.collab.highlight.saved = Some(now);
    }
    app.ui.highlight_active_row = app.session.dex.focus_cell && app.session.dex.focus_mode.rows();
}

/// GridCraft's `options.highlightActiveRow` (File ▸ Options ▸ "Highlight the active cell's row"),
/// answered here so there is one setting: on adds the row to what is highlighted, off takes it
/// away. Returns `{on}` as GridCraft's command does.
pub fn upstream_option(app: &mut SheetApp, p: &Json) -> Result<Json, String> {
    let d = &mut app.session.dex;
    let row_on = d.focus_cell && d.focus_mode.rows();
    let want = p.get("on").and_then(Json::as_bool).unwrap_or(!row_on);
    if want && !row_on {
        d.focus_mode = if d.focus_cell { FocusMode::Both } else { FocusMode::Row };
        d.focus_cell = true;
    } else if !want && row_on {
        match d.focus_mode {
            FocusMode::Both => d.focus_mode = FocusMode::Column,
            _ => d.focus_cell = false,
        }
    }
    app.ui.highlight_active_row = want;
    Ok(json!({"on": want}))
}
