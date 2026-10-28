//! Review ▸ Compare Files: a deterministic list of what changed between two versions of a
//! workbook (values, formulas, calculated results, formatting, sheets, structure and defined
//! names), optionally written out as a CSV or XLSX change report.
//!
//! The comparison only reads: neither workbook is changed, and nothing is journaled.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, HashMap};
use std::sync::Arc;

use gridcraft_core::{CellRef, RangeRef, Value, col_to_letters, number_to_text};
use gridcraft_model::{Cell, Color, LineInfo, PatternType, Sheet, Style, StyleId, Theme, Underline, Visibility, Workbook};
use serde_json::{Value as Json, json};

use super::*;

const ID: &str = "review.compareFiles";
/// Changes listed when the call gives no `limit`.
const DEFAULT_LIMIT: usize = 5_000;
/// The most changes one call lists, whatever `limit` asks for.
const MAX_LIMIT: usize = 50_000;
/// Distinct (old style, new style) pairs whose difference is remembered while comparing.
const STYLE_CACHE: usize = 100_000;
/// Two numbers closer than this, relative to the larger, are the same value.
const REL_TOLERANCE: f64 = 1e-12;
/// Row heights and column widths closer than this many pixels are the same size.
const SIZE_TOLERANCE: f32 = 0.5;
/// Sheets matched against each other for rename detection, at most.
const MAX_RENAME_CHECKS: usize = 10_000;

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        query "review.compareFiles",
        "Compare Files",
        ["Review", "Compare"],
        None,
        "{old: path | \"active\", new: path | \"active\", oldLabel?, newLabel?, kinds?: [\"value\",\"formula\",\"result\",\"format\",\"sheet\",\"structure\",\"name\"], limit?: 5000 (at most 50000), report?: path.xlsx | path.csv} → {old, new, counts: {value, formula, result, format, sheet, structure, name}, changes: [{kind, sheet, cell, old, new}], truncated, report?}",
        always,
        compare_files
    )]
}

// ---------------------------------------------------------------- change kinds

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Value,
    Formula,
    Result,
    Format,
    Sheet,
    Structure,
    Name,
}

const KINDS: [Kind; 7] = [Kind::Value, Kind::Formula, Kind::Result, Kind::Format, Kind::Sheet, Kind::Structure, Kind::Name];

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Value => "value",
            Kind::Formula => "formula",
            Kind::Result => "result",
            Kind::Format => "format",
            Kind::Sheet => "sheet",
            Kind::Structure => "structure",
            Kind::Name => "name",
        }
    }
    fn parse(t: &str) -> Option<Kind> {
        KINDS.iter().copied().find(|k| k.name().eq_ignore_ascii_case(t.trim()))
    }
    fn index(self) -> usize {
        self as usize
    }
}

#[derive(Clone, Debug)]
struct Change {
    kind: Kind,
    sheet: String,
    cell: String,
    old: String,
    new: String,
}

/// Where a change sorts: sheet order, then sheet-level changes before positioned ones, then row,
/// then column, then the order it was found in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    order: u32,
    tier: u8,
    row: u32,
    col: u32,
    seq: u64,
}

#[derive(Clone, Copy, Debug)]
struct Pos {
    order: u32,
    tier: u8,
    row: u32,
    col: u32,
}

impl Pos {
    /// A change about the sheet as a whole (added, renamed, used range).
    fn sheet(order: u32) -> Pos {
        Pos { order, tier: 0, row: 0, col: 0 }
    }
    fn at(order: u32, row: u32, col: u32) -> Pos {
        Pos { order, tier: 1, row, col }
    }
}

struct Entry {
    key: Key,
    change: Change,
}

impl PartialEq for Entry {
    fn eq(&self, o: &Self) -> bool {
        self.key == o.key
    }
}
impl Eq for Entry {}
impl PartialOrd for Entry {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Entry {
    fn cmp(&self, o: &Self) -> Ordering {
        self.key.cmp(&o.key)
    }
}

/// Counts every change found and keeps the first `limit` in report order (a max-heap of the
/// smallest keys, so memory never exceeds `limit` entries however many changes there are).
struct Collector {
    want: [bool; 7],
    counts: [u64; 7],
    limit: usize,
    kept: BinaryHeap<Entry>,
    seq: u64,
}

impl Collector {
    fn wants(&self, k: Kind) -> bool {
        self.want.get(k.index()).copied().unwrap_or(false)
    }

    /// Records a change; `build` gives its (cell, old, new) and runs only when it is listed.
    fn add(&mut self, kind: Kind, pos: Pos, sheet: &str, build: impl FnOnce() -> (String, String, String)) {
        if !self.wants(kind) {
            return;
        }
        if let Some(c) = self.counts.get_mut(kind.index()) {
            *c = c.saturating_add(1);
        }
        if self.limit == 0 {
            return;
        }
        self.seq = self.seq.saturating_add(1);
        let key = Key { order: pos.order, tier: pos.tier, row: pos.row, col: pos.col, seq: self.seq };
        if self.kept.len() >= self.limit && self.kept.peek().is_some_and(|top| key >= top.key) {
            return;
        }
        let (cell, old, new) = build();
        self.kept.push(Entry { key, change: Change { kind, sheet: sheet.to_string(), cell, old, new } });
        if self.kept.len() > self.limit {
            self.kept.pop();
        }
    }

    fn total(&self) -> u64 {
        self.counts.iter().fold(0u64, |a, b| a.saturating_add(*b))
    }
}

// ---------------------------------------------------------------- the command

fn compare_files(s: &mut Session, p: &Json) -> Result<Json> {
    let has = |k: &str| p.get(k).is_some_and(|v| !v.is_null());
    if !has("old") && !has("new") {
        return Err(bad(
            ID,
            "Compare Files needs two workbooks: pass {old, new}, each a file path or \"active\" for the open workbook (e.g. {\"old\": \"Budget v1.xlsx\", \"new\": \"active\"})",
        ));
    }
    let mut want = [true; 7];
    if let Some(k) = p.get("kinds").filter(|v| !v.is_null()) {
        want = parse_kinds(k)?;
    }
    let limit = match p.get("limit") {
        None | Some(Json::Null) => DEFAULT_LIMIT,
        Some(v) => {
            let n = v.as_u64().ok_or_else(|| bad(ID, "limit must be a whole number of changes (0 to 50000)"))?;
            usize::try_from(n).unwrap_or(MAX_LIMIT).min(MAX_LIMIT)
        }
    };
    let report = match p.get("report") {
        None | Some(Json::Null) => None,
        Some(Json::String(t)) if !t.trim().is_empty() => Some(t.trim().to_string()),
        Some(_) => return Err(bad(ID, "report must be a file path ending in .xlsx or .csv")),
    };
    let report_kind = match &report {
        Some(path) => Some(report_kind(path)?),
        None => None,
    };
    let (old_wb, old_label, old_path) = load(s, p, "old", "oldLabel")?;
    let (new_wb, new_label, new_path) = load(s, p, "new", "newLabel")?;
    if let Some(r) = &report {
        let active_path = s.active().and_then(|d| d.path.clone());
        for input in [&old_path, &new_path, &active_path].into_iter().flatten() {
            if same_path(r, input) {
                return Err(bad(ID, format!("the report would overwrite {input}: choose another report path")));
            }
        }
    }

    let mut col = Collector { want, counts: [0; 7], limit, kept: BinaryHeap::new(), seq: 0 };
    compare_workbooks(&mut col, &old_wb, &new_wb);

    let truncated = col.total() > limit as u64;
    let total = col.total();
    let counts = col.counts;
    let changes: Vec<Change> = col.kept.into_sorted_vec().into_iter().map(|e| e.change).collect();
    let mut counts_json = serde_json::Map::new();
    for k in KINDS {
        counts_json.insert(k.name().to_string(), json!(counts.get(k.index()).copied().unwrap_or(0)));
    }
    let list: Vec<Json> =
        changes.iter().map(|c| json!({"kind": c.kind.name(), "sheet": c.sheet, "cell": c.cell, "old": c.old, "new": c.new})).collect();
    let mut out = json!({
        "old": old_label,
        "new": new_label,
        "counts": Json::Object(counts_json),
        "total": total,
        "changes": list,
        "truncated": truncated,
    });
    if let (Some(path), Some(kind)) = (report, report_kind) {
        write_report(&path, kind, &changes)?;
        if let Some(o) = out.as_object_mut() {
            o.insert("report".into(), json!(path));
        }
    }
    Ok(out)
}

fn parse_kinds(v: &Json) -> Result<[bool; 7]> {
    let names: Vec<&str> = match v {
        Json::Array(a) => a.iter().map(|x| x.as_str().ok_or_else(|| bad(ID, "kinds must be a list of kind names"))).collect::<Result<_>>()?,
        Json::String(t) => t.split(',').filter(|x| !x.trim().is_empty()).collect(),
        _ => return Err(bad(ID, "kinds must be a list such as [\"value\", \"formula\"]")),
    };
    if names.is_empty() {
        return Err(bad(ID, "kinds is empty: name at least one of value, formula, result, format, sheet, structure, name"));
    }
    let mut want = [false; 7];
    for n in names {
        let k = Kind::parse(n).ok_or_else(|| bad(ID, format!("unknown kind `{n}`: use value, formula, result, format, sheet, structure or name")))?;
        if let Some(w) = want.get_mut(k.index()) {
            *w = true;
        }
    }
    Ok(want)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReportKind {
    Csv,
    Xlsx,
}

fn report_kind(path: &str) -> Result<ReportKind> {
    let ext = std::path::Path::new(path).extension().and_then(std::ffi::OsStr::to_str).map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("csv") => Ok(ReportKind::Csv),
        Some("xlsx") => Ok(ReportKind::Xlsx),
        _ => Err(bad(ID, format!("report must end in .xlsx or .csv, not `{path}`"))),
    }
}

fn same_path(a: &str, b: &str) -> bool {
    let norm = |p: &str| std::path::absolute(p).map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_else(|_| p.to_lowercase());
    norm(a) == norm(b)
}

/// The workbook a `old`/`new` param names, its label, and the file it came from.
fn load(s: &Session, p: &Json, key: &str, label_key: &str) -> Result<(Arc<Workbook>, String, Option<String>)> {
    let spec = match p.get(key) {
        Some(Json::String(t)) if !t.trim().is_empty() => t.trim(),
        None | Some(Json::Null) => {
            return Err(bad(ID, format!("`{key}` is missing: pass a file path or \"active\" for the open workbook")));
        }
        Some(_) => return Err(bad(ID, format!("`{key}` must be a file path or \"active\""))),
    };
    let label = str_param(p, label_key).map(str::trim).filter(|t| !t.is_empty()).map(str::to_string);
    if spec.eq_ignore_ascii_case("active") {
        let d = s.doc()?;
        return Ok((d.wb.clone(), label.unwrap_or_else(|| d.display_title()), d.path.clone()));
    }
    let bytes = crate::io::read_file(spec)?;
    let (wb, _warnings) = crate::io::open_bytes(spec, &bytes)?;
    let name = std::path::Path::new(spec).file_name().and_then(std::ffi::OsStr::to_str).unwrap_or(spec).to_string();
    Ok((Arc::new(wb), label.unwrap_or(name), Some(spec.to_string())))
}

// ---------------------------------------------------------------- workbooks and sheets

fn compare_workbooks(col: &mut Collector, old: &Workbook, new: &Workbook) {
    // Sheets with the same name (case-insensitive, as sheet names are) are the same sheet.
    let old_names: Vec<String> = old.sheets.iter().map(|s| s.name.to_lowercase()).collect();
    let new_names: Vec<String> = new.sheets.iter().map(|s| s.name.to_lowercase()).collect();
    let mut old_for_new: Vec<Option<usize>> = new_names.iter().map(|n| old_names.iter().position(|o| o == n)).collect();
    let mut renamed: Vec<bool> = vec![false; new_names.len()];
    // A removed sheet and an added one holding the same cells are one sheet renamed.
    let mut checks = 0usize;
    for (oi, osh) in old.sheets.iter().enumerate() {
        if old_for_new.contains(&Some(oi)) {
            continue;
        }
        for (ni, nsh) in new.sheets.iter().enumerate() {
            if old_for_new.get(ni).copied().flatten().is_some() || checks >= MAX_RENAME_CHECKS {
                continue;
            }
            checks += 1;
            if same_cells(osh, nsh) {
                if let Some(slot) = old_for_new.get_mut(ni) {
                    *slot = Some(oi);
                }
                if let Some(r) = renamed.get_mut(ni) {
                    *r = true;
                }
                break;
            }
        }
    }
    let mut styles = StyleDiffs { cache: HashMap::new() };
    let new_len = new.sheets.len();
    for (ni, nsh) in new.sheets.iter().enumerate() {
        let order = u32::try_from(ni).unwrap_or(u32::MAX - 1);
        match old_for_new.get(ni).copied().flatten().and_then(|oi| old.sheets.get(oi)) {
            Some(osh) => {
                if renamed.get(ni).copied().unwrap_or(false) {
                    col.add(Kind::Sheet, Pos::sheet(order), &nsh.name, || (String::new(), osh.name.clone(), nsh.name.clone()));
                }
                if osh.visibility != nsh.visibility {
                    col.add(Kind::Sheet, Pos::sheet(order), &nsh.name, || {
                        ("visibility".into(), visibility(osh.visibility).into(), visibility(nsh.visibility).into())
                    });
                }
                let ctx = SheetPair { order, name: &nsh.name, old_wb: old, old: osh, new_wb: new, new: nsh };
                compare_sheet(col, &ctx, &mut styles);
            }
            None => col.add(Kind::Sheet, Pos::sheet(order), &nsh.name, || (String::new(), String::new(), nsh.name.clone())),
        }
    }
    for (oi, osh) in old.sheets.iter().enumerate() {
        if !old_for_new.contains(&Some(oi)) {
            let order = u32::try_from(new_len.saturating_add(oi)).unwrap_or(u32::MAX - 1);
            col.add(Kind::Sheet, Pos::sheet(order), &osh.name, || (String::new(), osh.name.clone(), String::new()));
        }
    }
    compare_names(col, old, new);
}

fn visibility(v: Visibility) -> &'static str {
    match v {
        Visibility::Visible => "visible",
        Visibility::Hidden => "hidden",
        Visibility::VeryHidden => "very hidden",
    }
}

/// The same cells with the same contents (formulas and values; formatting aside).
fn same_cells(a: &Sheet, b: &Sheet) -> bool {
    a.cells.len() == b.cells.len()
        && a.cells
            .iter()
            .zip(b.cells.iter())
            .all(|((ca, a), (cb, b))| ca == cb && formula_text(Some(a)) == formula_text(Some(b)) && same_value(&a.value, &b.value))
}

struct SheetPair<'a> {
    order: u32,
    name: &'a str,
    old_wb: &'a Workbook,
    old: &'a Sheet,
    new_wb: &'a Workbook,
    new: &'a Sheet,
}

fn compare_sheet(col: &mut Collector, p: &SheetPair, styles: &mut StyleDiffs) {
    // Used range.
    if col.wants(Kind::Structure) {
        let (ou, nu) = (p.old.used_range(), p.new.used_range());
        if ou != nu {
            col.add(Kind::Structure, Pos::sheet(p.order), p.name, || ("used range".into(), range_text(ou), range_text(nu)));
        }
    }
    // Cells: both stores iterate row by row, so walking them side by side visits every cell
    // either holds once, in order.
    let want_cells = col.wants(Kind::Value) || col.wants(Kind::Formula) || col.wants(Kind::Result) || col.wants(Kind::Format);
    if want_cells {
        let mut oi = p.old.cells.iter().peekable();
        let mut ni = p.new.cells.iter().peekable();
        loop {
            let step = match (oi.peek(), ni.peek()) {
                (None, None) => break,
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (Some((a, _)), Some((b, _))) => a.cmp(b),
            };
            let (at, o, n) = match step {
                Ordering::Less => match oi.next() {
                    Some((at, c)) => (at, Some(c), None),
                    None => break,
                },
                Ordering::Greater => match ni.next() {
                    Some((at, c)) => (at, None, Some(c)),
                    None => break,
                },
                Ordering::Equal => match (oi.next(), ni.next()) {
                    (Some((at, a)), Some((_, b))) => (at, Some(a), Some(b)),
                    _ => break,
                },
            };
            compare_cell(col, p, styles, at, o, n);
        }
    }
    if col.wants(Kind::Structure) {
        compare_structure(col, p);
    }
}

fn compare_cell(col: &mut Collector, p: &SheetPair, styles: &mut StyleDiffs, at: CellRef, o: Option<&Cell>, n: Option<&Cell>) {
    let pos = Pos::at(p.order, at.row, at.col);
    let cell = || at.a1();
    let empty = Value::Empty;
    let ov = o.map_or(&empty, |c| &c.value);
    let nv = n.map_or(&empty, |c| &c.value);
    let changed = match (formula_text(o), formula_text(n)) {
        (Some(a), Some(b)) if a != b => {
            col.add(Kind::Formula, pos, p.name, || (cell(), format!("={a}"), format!("={b}")));
            true
        }
        (Some(_), Some(_)) => {
            let differs = !same_value(ov, nv);
            if differs {
                col.add(Kind::Result, pos, p.name, || (cell(), value_text(ov), value_text(nv)));
            }
            differs
        }
        (Some(a), None) => {
            col.add(Kind::Formula, pos, p.name, || (cell(), format!("={a}"), value_text(nv)));
            true
        }
        (None, Some(b)) => {
            col.add(Kind::Formula, pos, p.name, || (cell(), value_text(ov), format!("={b}")));
            true
        }
        (None, None) => {
            let differs = !same_value(ov, nv);
            if differs {
                col.add(Kind::Value, pos, p.name, || (cell(), value_text(ov), value_text(nv)));
            }
            differs
        }
    };
    if !changed && col.wants(Kind::Format) {
        let (os, ns) = (p.old.style_id(at), p.new.style_id(at));
        if let Some((a, b)) = styles.diff(p.old_wb, os, p.new_wb, ns) {
            col.add(Kind::Format, pos, p.name, || (cell(), a, b));
        }
    }
}

fn formula_text(c: Option<&Cell>) -> Option<&str> {
    c.and_then(|c| c.formula.as_deref()).map(|f| f.text.as_str())
}

/// Values compare by type; numbers within a tiny relative tolerance; empty text is empty.
fn same_value(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x == y || (x - y).abs() <= REL_TOLERANCE * x.abs().max(y.abs()),
        (Value::Empty, Value::Text(t)) | (Value::Text(t), Value::Empty) => t.is_empty(),
        _ => a == b,
    }
}

/// A value as typed: text as it is, numbers without formatting, TRUE/FALSE, error codes.
fn value_text(v: &Value) -> String {
    match v {
        Value::Empty => String::new(),
        Value::Text(t) => t.to_string(),
        other => other.display(),
    }
}

fn range_text(r: Option<RangeRef>) -> String {
    r.map(|r| r.a1()).unwrap_or_else(|| "empty".into())
}

fn compare_structure(col: &mut Collector, p: &SheetPair) {
    // Merged ranges.
    let merges =
        |sh: &Sheet| -> BTreeSet<(u32, u32, u32, u32)> { sh.merges.iter().map(|r| (r.start.row, r.start.col, r.end.row, r.end.col)).collect() };
    let (om, nm) = (merges(p.old), merges(p.new));
    for &(r0, c0, r1, c1) in om.symmetric_difference(&nm).collect::<BTreeSet<_>>() {
        let added = nm.contains(&(r0, c0, r1, c1));
        let text = RangeRef::new(CellRef::new(r0, c0), CellRef::new(r1, c1)).a1();
        let (a, b) = if added { ("not merged", "merged") } else { ("merged", "not merged") };
        col.add(Kind::Structure, Pos::at(p.order, r0, c0), p.name, || (text, a.into(), b.into()));
    }
    // Rows: hidden, and heights someone set (automatic heights follow the content).
    for row in keys(&p.old.rows, &p.new.rows) {
        let (o, n) = (p.old.rows.get(&row), p.new.rows.get(&row));
        let label = || format!("row {}", row as u64 + 1);
        let pos = Pos::at(p.order, row, 0);
        let (oh, nh) = (o.is_some_and(|i| i.hidden), n.is_some_and(|i| i.hidden));
        if oh != nh {
            col.add(Kind::Structure, pos, p.name, || (label(), shown(oh).into(), shown(nh).into()));
        }
        if o.is_some_and(|i| i.custom) || n.is_some_and(|i| i.custom) {
            let (a, b) = (line_size(o, p.old.default_row_height), line_size(n, p.new.default_row_height));
            if (a - b).abs() > SIZE_TOLERANCE {
                col.add(Kind::Structure, pos, p.name, || {
                    (label(), format!("height {} pt", short(a as f64 * 0.75)), format!("height {} pt", short(b as f64 * 0.75)))
                });
            }
        }
    }
    // Columns: hidden and width.
    for c in keys(&p.old.cols, &p.new.cols) {
        let (o, n) = (p.old.cols.get(&c), p.new.cols.get(&c));
        let label = || format!("column {}", col_to_letters(c));
        let pos = Pos::at(p.order, 0, c);
        let (oh, nh) = (o.is_some_and(|i| i.hidden), n.is_some_and(|i| i.hidden));
        if oh != nh {
            col.add(Kind::Structure, pos, p.name, || (label(), shown(oh).into(), shown(nh).into()));
        }
        if o.is_some_and(|i| i.size.is_some()) || n.is_some_and(|i| i.size.is_some()) {
            let (a, b) = (line_size(o, p.old.default_col_width), line_size(n, p.new.default_col_width));
            if (a - b).abs() > SIZE_TOLERANCE {
                col.add(Kind::Structure, pos, p.name, || {
                    (
                        label(),
                        format!("width {}", short(gridcraft_model::points_to_chars(a as f64))),
                        format!("width {}", short(gridcraft_model::points_to_chars(b as f64))),
                    )
                });
            }
        }
    }
}

fn keys(a: &BTreeMap<u32, LineInfo>, b: &BTreeMap<u32, LineInfo>) -> BTreeSet<u32> {
    a.keys().chain(b.keys()).copied().collect()
}

fn line_size(i: Option<&LineInfo>, default: f32) -> f32 {
    i.and_then(|i| i.size).unwrap_or(default)
}

fn shown(hidden: bool) -> &'static str {
    if hidden { "hidden" } else { "visible" }
}

/// A size with at most two decimals and no trailing zeros.
fn short(x: f64) -> String {
    number_to_text((x * 100.0).round() / 100.0)
}

// ---------------------------------------------------------------- defined names

fn compare_names(col: &mut Collector, old: &Workbook, new: &Workbook) {
    if !col.wants(Kind::Name) {
        return;
    }
    // (name, scope sheet) → (shown name, scope sheet's name, formula); names are case-insensitive.
    type Names = BTreeMap<(String, String), (String, String, String)>;
    let names = |wb: &Workbook| -> Names {
        wb.names
            .iter()
            .map(|n| {
                let scope = n.scope.and_then(|i| wb.sheet(i)).map(|s| s.name.clone()).unwrap_or_default();
                ((n.name.to_lowercase(), scope.to_lowercase()), (n.name.clone(), scope, n.formula.trim_start_matches('=').to_string()))
            })
            .collect()
    };
    let (on, nn) = (names(old), names(new));
    let all: BTreeSet<&(String, String)> = on.keys().chain(nn.keys()).collect();
    for k in all {
        let pos = Pos { order: u32::MAX, tier: 0, row: 0, col: 0 };
        match (on.get(k), nn.get(k)) {
            (Some((name, scope, a)), Some((_, _, b))) if a != b => {
                col.add(Kind::Name, pos, scope, || (name.clone(), format!("={a}"), format!("={b}")))
            }
            (Some((name, scope, a)), None) => col.add(Kind::Name, pos, scope, || (name.clone(), format!("={a}"), String::new())),
            (None, Some((name, scope, b))) => col.add(Kind::Name, pos, scope, || (name.clone(), String::new(), format!("={b}"))),
            _ => {}
        }
    }
}

// ---------------------------------------------------------------- formatting

struct StyleDiffs {
    cache: HashMap<(StyleId, StyleId), Option<(String, String)>>,
}

impl StyleDiffs {
    /// What differs between two cell styles, as (old, new) descriptions; `None` when they look
    /// the same. Colours compare by the RGB they show, so a theme colour equals its own RGB.
    fn diff(&mut self, old_wb: &Workbook, a: StyleId, new_wb: &Workbook, b: StyleId) -> Option<(String, String)> {
        if let Some(hit) = self.cache.get(&(a, b)) {
            return hit.clone();
        }
        let (sa, sb) = (style_aspects(old_wb.styles.get(a), &old_wb.theme), style_aspects(new_wb.styles.get(b), &new_wb.theme));
        let (mut olds, mut news) = (Vec::new(), Vec::new());
        for (x, y) in sa.iter().zip(sb.iter()) {
            if x != y {
                olds.push(x.as_str());
                news.push(y.as_str());
            }
        }
        let out = if olds.is_empty() { None } else { Some((olds.join(", "), news.join(", "))) };
        if self.cache.len() < STYLE_CACHE {
            self.cache.insert((a, b), out.clone());
        }
        out
    }
}

fn color_text(c: Color, theme: &Theme) -> String {
    c.hex(theme).unwrap_or_else(|| "automatic".into())
}

/// A style as a fixed list of described aspects, compared position by position.
fn style_aspects(st: &Style, theme: &Theme) -> [String; 12] {
    let f = &st.font;
    let underline = match f.underline {
        Underline::None => "no underline".to_string(),
        Underline::Single => "underline".to_string(),
        Underline::Double => "double underline".to_string(),
        Underline::SingleAccounting => "accounting underline".to_string(),
        Underline::DoubleAccounting => "double accounting underline".to_string(),
    };
    let fill = match st.fill.pattern {
        PatternType::None => "no fill".to_string(),
        PatternType::Solid => format!("fill {}", color_text(st.fill.fg, theme)),
        other => format!("fill {} {} on {}", format!("{other:?}").to_lowercase(), color_text(st.fill.fg, theme), color_text(st.fill.bg, theme)),
    };
    let b = &st.borders;
    let sides: Vec<String> =
        [("left", b.left), ("right", b.right), ("top", b.top), ("bottom", b.bottom), ("diagonal down", b.diag_down), ("diagonal up", b.diag_up)]
            .iter()
            .filter(|(_, l)| !l.is_none())
            .map(|(side, l)| {
                let style = format!("{:?}", l.style).to_lowercase();
                match l.color {
                    Color::Auto => format!("{side} {style}"),
                    c => format!("{side} {style} {}", color_text(c, theme)),
                }
            })
            .collect();
    let borders = if sides.is_empty() { "no borders".to_string() } else { format!("borders {}", sides.join(" ")) };
    let a = &st.align;
    let mut align = format!("alignment {} {}", format!("{:?}", a.h).to_lowercase(), format!("{:?}", a.v).to_lowercase());
    if a.wrap {
        align.push_str(" wrap");
    }
    if a.shrink {
        align.push_str(" shrink");
    }
    if a.indent > 0 {
        align.push_str(&format!(" indent {}", a.indent));
    }
    if a.rotation != 0 {
        align.push_str(&format!(" rotation {}", a.rotation));
    }
    let num = st.num_fmt.as_str();
    let protection = match (st.protection.locked, st.protection.hidden) {
        (true, false) => "locked",
        (true, true) => "locked, formula hidden",
        (false, false) => "unlocked",
        (false, true) => "unlocked, formula hidden",
    };
    [
        if f.bold { "bold".into() } else { "not bold".into() },
        if f.italic { "italic".into() } else { "not italic".into() },
        underline,
        if f.strike { "strikethrough".into() } else { "no strikethrough".into() },
        format!("font {}", f.name),
        format!("size {}", short(f.size as f64)),
        format!("font color {}", color_text(f.color, theme)),
        fill,
        format!("number format {}", if num.is_empty() { "General" } else { num }),
        borders,
        align,
        protection.into(),
    ]
}

// ---------------------------------------------------------------- report

fn csv_field(out: &mut String, s: &str) {
    if s.contains([',', '"', '\n', '\r']) || s.starts_with(' ') || s.ends_with(' ') {
        out.push('"');
        out.push_str(&s.replace('"', "\"\""));
        out.push('"');
    } else {
        out.push_str(s);
    }
}

fn report_csv(changes: &[Change]) -> String {
    let mut out = String::from("\u{FEFF}Kind,Sheet,Cell,Old,New\r\n");
    for c in changes {
        for (i, field) in [c.kind.name(), c.sheet.as_str(), c.cell.as_str(), c.old.as_str(), c.new.as_str()].iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            csv_field(&mut out, field);
        }
        out.push_str("\r\n");
    }
    out
}

fn write_report(path: &str, kind: ReportKind, changes: &[Change]) -> Result<()> {
    let csv = report_csv(changes);
    let bytes = match kind {
        ReportKind::Csv => csv.into_bytes(),
        ReportKind::Xlsx => {
            // Read back as text, so formula texts and codes like 007 stay exactly as listed.
            let (mut wb, _) = crate::io::open_bytes_with("Comparison.csv", csv.as_bytes(), None, true)?;
            let bold = wb.styles.derive(StyleId::DEFAULT, |st| st.font.bold = true);
            if let Some(sh) = wb.sheet_mut(0) {
                sh.name = "Comparison".into();
                for c in 0..5 {
                    sh.set_style(CellRef::new(0, c), bold);
                }
                sh.freeze = Some((1, 0));
            }
            crate::io::save_bytes(&wb, path)?
        }
    };
    crate::io::write_file(path, &bytes)
}

#[cfg(test)]
mod tests {
    use gridcraft_core::{CellRef, Value};
    use gridcraft_model::{BorderLine, Color, DefinedName, Fill, StyleId, Visibility, Workbook};
    use serde_json::{Value as Json, json};

    use crate::Session;

    fn book(cells: &[(&str, &str)]) -> Session {
        let mut s = Session::new();
        s.new_workbook();
        for (cell, input) in cells {
            s.execute("cell.set", json!({"cell": cell, "input": input})).unwrap();
        }
        s
    }

    fn wb_of(s: &Session) -> Workbook {
        (*s.doc().unwrap().wb).clone()
    }

    fn dir(test: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("dex-compare-{}-{test}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn save(wb: &Workbook, test: &str, name: &str) -> String {
        let p = dir(test).join(name);
        let path = p.to_str().unwrap().to_string();
        std::fs::write(&p, crate::io::save_bytes(wb, &path).unwrap()).unwrap();
        path
    }

    fn compare_with(s: &mut Session, old: &str, new: &str, extra: Json) -> Json {
        let mut p = json!({"old": old, "new": new});
        if let (Some(o), Some(e)) = (p.as_object_mut(), extra.as_object()) {
            o.extend(e.clone());
        }
        s.execute("review.compareFiles", p).unwrap()
    }

    fn compare(old: &Workbook, new: &Workbook, test: &str, extra: Json) -> Json {
        let (a, b) = (save(old, test, "old.xlsx"), save(new, test, "new.xlsx"));
        compare_with(&mut Session::new(), &a, &b, extra)
    }

    /// (kind, sheet, cell, old, new) of every listed change.
    fn rows(r: &Json) -> Vec<(String, String, String, String, String)> {
        let f = |c: &Json, k: &str| c[k].as_str().unwrap().to_string();
        r["changes"].as_array().unwrap().iter().map(|c| (f(c, "kind"), f(c, "sheet"), f(c, "cell"), f(c, "old"), f(c, "new"))).collect()
    }

    fn row(kind: &str, sheet: &str, cell: &str, old: &str, new: &str) -> (String, String, String, String, String) {
        (kind.into(), sheet.into(), cell.into(), old.into(), new.into())
    }

    fn sheet_mut(wb: &mut Workbook, i: usize) -> &mut gridcraft_model::Sheet {
        wb.sheet_mut(i).unwrap()
    }

    fn base_cells() -> Vec<(String, String)> {
        let mut cells: Vec<(String, String)> = (2..=29).map(|r| (format!("F{r}"), format!("{r}"))).collect();
        cells.push(("B14".into(), "In Progress".into()));
        cells.push(("D22".into(), "120".into()));
        cells.push(("F30".into(), "=SUM(F2:F29)".into()));
        cells
    }

    fn base() -> Session {
        let cells = base_cells();
        let refs: Vec<(&str, &str)> = cells.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        book(&refs)
    }

    #[test]
    fn acceptance_values_and_formula_are_listed_in_order() {
        let old = base();
        let mut new = base();
        for (cell, input) in [("B14", "Completed"), ("D22", "135"), ("F30", "=SUM(F2:F30)")] {
            new.execute("cell.set", json!({"cell": cell, "input": input})).unwrap();
        }
        let r = compare(&wb_of(&old), &wb_of(&new), "acceptance", json!({}));
        assert_eq!(
            rows(&r),
            [
                row("value", "Sheet1", "B14", "In Progress", "Completed"),
                row("value", "Sheet1", "D22", "120", "135"),
                row("formula", "Sheet1", "F30", "=SUM(F2:F29)", "=SUM(F2:F30)"),
            ]
        );
        assert_eq!(r["counts"], json!({"value": 2, "formula": 1, "result": 0, "format": 0, "sheet": 0, "structure": 0, "name": 0}));
        assert_eq!(r["truncated"], false);
        assert_eq!((r["old"].as_str(), r["new"].as_str()), (Some("old.xlsx"), Some("new.xlsx")));
    }

    #[test]
    fn unchanged_copy_has_no_changes() {
        let s = base();
        let wb = wb_of(&s);
        let r = compare(&wb, &wb, "unchanged", json!({}));
        assert_eq!(r["changes"], json!([]));
        assert_eq!(r["total"], 0);
        // A file against itself, too.
        let p = save(&wb, "unchanged", "same.xlsx");
        let r = compare_with(&mut Session::new(), &p, &p, json!({}));
        assert_eq!(r["changes"], json!([]));
    }

    #[test]
    fn result_changes_when_only_the_calculated_value_differs() {
        let old = book(&[("A1", "1"), ("A2", "=A1*2"), ("A3", "=0.1+0.2")]);
        let new = book(&[("A1", "5"), ("A2", "=A1*2"), ("A3", "=0.3")]);
        let r = compare(&wb_of(&old), &wb_of(&new), "result", json!({}));
        assert_eq!(
            rows(&r),
            [row("value", "Sheet1", "A1", "1", "5"), row("result", "Sheet1", "A2", "2", "10"), row("formula", "Sheet1", "A3", "=0.1+0.2", "=0.3"),]
        );
    }

    #[test]
    fn numbers_compare_with_a_tiny_tolerance_and_text_exactly() {
        let mut old = wb_of(&book(&[("A2", "x")]));
        let mut new = old.clone();
        sheet_mut(&mut old, 0).set_value(CellRef::new(0, 0), Value::Number(0.1 + 0.2));
        sheet_mut(&mut new, 0).set_value(CellRef::new(0, 0), Value::Number(0.3));
        sheet_mut(&mut old, 0).set_value(CellRef::new(0, 1), Value::Number(1.0));
        sheet_mut(&mut new, 0).set_value(CellRef::new(0, 1), Value::Number(1.000_000_1));
        sheet_mut(&mut new, 0).set_value(CellRef::new(1, 0), Value::text("X"));
        let r = compare(&old, &new, "tolerance", json!({}));
        assert_eq!(rows(&r), [row("value", "Sheet1", "B1", "1", "1.0000001"), row("value", "Sheet1", "A2", "x", "X")]);
    }

    #[test]
    fn constants_and_formulas_added_and_removed() {
        let old = book(&[("A1", "keep"), ("B1", "gone"), ("C1", "7"), ("D1", "=1+1")]);
        let new = book(&[("A1", "keep"), ("C1", "=3+4"), ("D1", "2"), ("E1", "new"), ("F1", "=A1")]);
        let r = compare(&wb_of(&old), &wb_of(&new), "addremove", json!({}));
        assert_eq!(
            rows(&r),
            [
                row("structure", "Sheet1", "used range", "A1:D1", "A1:F1"),
                row("value", "Sheet1", "B1", "gone", ""),
                row("formula", "Sheet1", "C1", "7", "=3+4"),
                row("formula", "Sheet1", "D1", "=1+1", "2"),
                row("value", "Sheet1", "E1", "", "new"),
                row("formula", "Sheet1", "F1", "", "=A1"),
            ]
        );
    }

    #[test]
    fn format_changes_name_what_differs() {
        let old = wb_of(&book(&[("A1", "1"), ("A2", "2"), ("A3", "3"), ("A4", "4"), ("A5", "5"), ("A6", "6")]));
        let mut new = old.clone();
        let bold = new.styles.derive(StyleId::DEFAULT, |s| s.font.bold = true);
        let fill = new.styles.derive(StyleId::DEFAULT, |s| s.fill = Fill::solid(Color::Rgb(0xFFFF00)));
        let num = new.styles.derive(StyleId::DEFAULT, |s| s.num_fmt = gridcraft_model::NumFmt::new("0.00"));
        let border = new.styles.derive(StyleId::DEFAULT, |s| s.borders.bottom = BorderLine::thin());
        let align = new.styles.derive(StyleId::DEFAULT, |s| s.align.h = gridcraft_model::HAlign::Center);
        let sh = sheet_mut(&mut new, 0);
        for (row, st) in [(0, bold), (1, fill), (2, num), (3, border), (4, align)] {
            sh.set_style(CellRef::new(row, 0), st);
        }
        // A changed value with a changed style is listed as the value change.
        sh.set_value(CellRef::new(5, 0), Value::Number(60.0));
        sh.set_style(CellRef::new(5, 0), bold);
        let r = compare(&old, &new, "format", json!({}));
        assert_eq!(
            rows(&r),
            [
                row("format", "Sheet1", "A1", "not bold", "bold"),
                row("format", "Sheet1", "A2", "no fill", "fill #FFFF00"),
                row("format", "Sheet1", "A3", "number format General", "number format 0.00"),
                row("format", "Sheet1", "A4", "no borders", "borders bottom thin"),
                row("format", "Sheet1", "A5", "alignment general bottom", "alignment center bottom"),
                row("value", "Sheet1", "A6", "6", "60"),
            ]
        );
        assert_eq!(r["counts"]["format"], 5);
    }

    #[test]
    fn sheets_added_removed_renamed_and_hidden() {
        let mut old = wb_of(&book(&[("A1", "1")]));
        for (name, cell) in [("Data", "d"), ("Notes", "n"), ("Old", "o")] {
            let mut sh = gridcraft_model::Sheet::new(name);
            sh.set_value(CellRef::new(0, 0), Value::text(cell));
            old.sheets.push(std::sync::Arc::new(sh));
        }
        let mut new = old.clone();
        // Notes → Remarks (same cells), Old removed, Extra added, Data hidden.
        sheet_mut(&mut new, 2).name = "Remarks".into();
        new.sheets.remove(3);
        let mut extra = gridcraft_model::Sheet::new("Extra");
        extra.set_value(CellRef::new(0, 0), Value::text("e"));
        new.sheets.push(std::sync::Arc::new(extra));
        sheet_mut(&mut new, 1).visibility = Visibility::Hidden;
        let r = compare(&old, &new, "sheets", json!({}));
        assert_eq!(
            rows(&r),
            [
                row("sheet", "Data", "visibility", "visible", "hidden"),
                row("sheet", "Remarks", "", "Notes", "Remarks"),
                row("sheet", "Extra", "", "", "Extra"),
                row("sheet", "Old", "", "Old", ""),
            ]
        );
        assert_eq!(r["counts"]["sheet"], 4);
    }

    #[test]
    fn structure_merges_rows_columns_and_used_range() {
        let mut s = book(&[("A1", "a"), ("B5", "b")]);
        s.execute("home.mergeCells", json!({"range": "C1:D2"})).unwrap();
        let old = wb_of(&s);
        s.execute("home.unmergeCells", json!({"range": "C1:D2"})).unwrap();
        s.execute("home.mergeCells", json!({"range": "A8:B8"})).unwrap();
        s.execute("home.rowHeight", json!({"rows": "5", "points": 30})).unwrap();
        s.execute("home.hideColumns", json!({"cols": "C"})).unwrap();
        s.execute("home.columnWidth", json!({"cols": "E", "width": 120})).unwrap();
        s.execute("cell.set", json!({"cell": "E10", "input": "z"})).unwrap();
        let new = wb_of(&s);
        let r = compare(&old, &new, "structure", json!({"kinds": ["structure"]}));
        let got = rows(&r);
        assert_eq!(got.first(), Some(&row("structure", "Sheet1", "used range", "A1:B5", "A1:E10")), "{got:?}");
        for want in [
            row("structure", "Sheet1", "C1:D2", "merged", "not merged"),
            row("structure", "Sheet1", "column C", "visible", "hidden"),
            row("structure", "Sheet1", "row 5", "height 15 pt", "height 30 pt"),
            row("structure", "Sheet1", "A8:B8", "not merged", "merged"),
        ] {
            assert!(got.contains(&want), "missing {want:?} in {got:?}");
        }
        assert!(got.iter().any(|g| g.2 == "column E" && g.4.starts_with("width ")), "{got:?}");
        // Order: row then column.
        let cells: Vec<&str> = got.iter().map(|g| g.2.as_str()).collect();
        let at = |c: &str| cells.iter().position(|x| *x == c).unwrap();
        assert!(
            at("C1:D2") < at("column C") && at("column C") < at("column E") && at("column E") < at("row 5") && at("row 5") < at("A8:B8"),
            "{cells:?}"
        );
        assert_eq!(r["counts"]["value"], 0, "kinds filter leaves the rest uncounted");
    }

    #[test]
    fn defined_names_added_removed_and_changed() {
        let mut old = wb_of(&book(&[("A1", "1")]));
        let name =
            |n: &str, f: &str, scope: Option<usize>| DefinedName { name: n.into(), scope, formula: f.into(), comment: String::new(), hidden: false };
        old.names = vec![name("Rate", "0.07", None), name("Gone", "Sheet1!$A$1", None), name("Local", "1", Some(0))];
        let mut new = old.clone();
        new.names = vec![name("Rate", "0.08", None), name("Local", "1", Some(0)), name("Added", "Sheet1!$A$1:$A$3", None)];
        let r = compare(&old, &new, "names", json!({}));
        assert_eq!(
            rows(&r),
            [
                row("name", "", "Added", "", "=Sheet1!$A$1:$A$3"),
                row("name", "", "Gone", "=Sheet1!$A$1", ""),
                row("name", "", "Rate", "=0.07", "=0.08"),
            ]
        );
    }

    #[test]
    fn limit_truncates_but_counts_everything() {
        let old = wb_of(&book(&[]));
        let cells: Vec<(String, String)> = (1..=20).map(|r| (format!("A{r}"), format!("{r}"))).collect();
        let refs: Vec<(&str, &str)> = cells.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        let new = wb_of(&book(&refs));
        let r = compare(&old, &new, "limit", json!({"limit": 5, "kinds": ["value"]}));
        assert_eq!(r["truncated"], true);
        assert_eq!(r["counts"]["value"], 20);
        let got: Vec<String> = rows(&r).into_iter().map(|g| g.2).collect();
        assert_eq!(got, ["A1", "A2", "A3", "A4", "A5"]);
        let r = compare(&old, &new, "limit", json!({"limit": 0}));
        assert_eq!((r["changes"].as_array().unwrap().len(), r["truncated"].as_bool()), (0, Some(true)));
        // A huge limit is capped, not allocated.
        let r = compare(&old, &new, "limit", json!({"limit": u64::MAX}));
        assert_eq!(r["truncated"], false);
    }

    #[test]
    fn kinds_filter_lists_only_those_kinds() {
        let old = base();
        let mut new = base();
        new.execute("cell.set", json!({"cell": "B14", "input": "Completed"})).unwrap();
        new.execute("cell.set", json!({"cell": "F30", "input": "=SUM(F2:F30)"})).unwrap();
        let r = compare(&wb_of(&old), &wb_of(&new), "kinds", json!({"kinds": ["formula"]}));
        assert_eq!(rows(&r), [row("formula", "Sheet1", "F30", "=SUM(F2:F29)", "=SUM(F2:F30)")]);
        assert_eq!(r["counts"]["value"], 0);
        let r = compare(&wb_of(&old), &wb_of(&new), "kinds", json!({"kinds": "value"}));
        assert_eq!(rows(&r), [row("value", "Sheet1", "B14", "In Progress", "Completed")]);
    }

    #[test]
    fn csv_and_xlsx_reports_are_written_and_readable() {
        let old = base();
        let mut new = base();
        new.execute("cell.set", json!({"cell": "B14", "input": "Done, \"finally\""})).unwrap();
        new.execute("cell.set", json!({"cell": "F30", "input": "=SUM(F2:F30)"})).unwrap();
        let csv_path = dir("report").join("changes.csv").to_str().unwrap().to_string();
        let r = compare(&wb_of(&old), &wb_of(&new), "report", json!({"report": csv_path}));
        assert_eq!(r["report"], csv_path);
        let text = String::from_utf8(std::fs::read(&csv_path).unwrap()).unwrap();
        assert_eq!(
            text,
            "\u{FEFF}Kind,Sheet,Cell,Old,New\r\nvalue,Sheet1,B14,In Progress,\"Done, \"\"finally\"\"\"\r\nformula,Sheet1,F30,=SUM(F2:F29),=SUM(F2:F30)\r\n"
        );
        let xlsx_path = dir("report").join("changes.xlsx").to_str().unwrap().to_string();
        compare(&wb_of(&old), &wb_of(&new), "report", json!({"report": xlsx_path}));
        let (wb, _) = crate::io::open_bytes(&xlsx_path, &std::fs::read(&xlsx_path).unwrap()).unwrap();
        let sh = wb.sheet(0).unwrap();
        assert_eq!(sh.name, "Comparison");
        let at = |a: &str| sh.cell(CellRef::parse(a).unwrap()).unwrap().clone();
        assert_eq!(at("A1").value, Value::text("Kind"));
        assert_eq!(at("E1").value, Value::text("New"));
        assert!(wb.styles.get(at("A1").style).font.bold);
        assert_eq!(at("D2").value, Value::text("In Progress"));
        assert_eq!(at("E2").value, Value::text("Done, \"finally\""));
        // Formula texts stay text in the report.
        let f = at("E3");
        assert!(f.formula.is_none(), "{f:?}");
        assert_eq!(f.value, Value::text("=SUM(F2:F30)"));
    }

    #[test]
    fn active_document_compares_with_a_file() {
        let mut s = base();
        let saved = save(&wb_of(&s), "active", "saved.xlsx");
        let r = compare_with(&mut s, &saved, "active", json!({}));
        assert_eq!(r["changes"], json!([]), "the open workbook matches the copy it saved");
        assert_eq!(r["new"], "Book1");
        s.execute("cell.set", json!({"cell": "D22", "input": "135"})).unwrap();
        let r = compare_with(&mut s, &saved, "active", json!({"oldLabel": "v1", "newLabel": "v2"}));
        assert_eq!(rows(&r), [row("value", "Sheet1", "D22", "120", "135")]);
        assert_eq!((r["old"].as_str(), r["new"].as_str()), (Some("v1"), Some("v2")));
        // "active" with nothing open is an error.
        assert!(Session::new().execute("review.compareFiles", json!({"old": saved, "new": "active"})).is_err());
    }

    #[test]
    fn bad_params_are_errors_not_dialogs() {
        let mut s = base();
        let p = save(&wb_of(&s), "bad", "x.xlsx");
        let err = s.execute("review.compareFiles", json!({})).unwrap_err().to_string();
        assert!(err.contains("old") && err.contains("new") && err.contains("active"), "{err}");
        assert!(s.ui_requests.is_empty(), "no dialog");
        for params in [
            json!([1, 2]),
            json!({"old": p}),
            json!({"old": 5, "new": p}),
            json!({"old": p, "new": "no such file.xlsx"}),
            json!({"old": p, "new": p, "kinds": ["colour"]}),
            json!({"old": p, "new": p, "kinds": []}),
            json!({"old": p, "new": p, "kinds": 3}),
            json!({"old": p, "new": p, "limit": -1}),
            json!({"old": p, "new": p, "limit": "many"}),
            json!({"old": p, "new": p, "report": "out.pdf"}),
            json!({"old": p, "new": p, "report": 1}),
            json!({"old": p, "new": "active", "report": p}),
        ] {
            assert!(s.execute("review.compareFiles", params.clone()).is_err(), "{params}");
        }
        // The refused report did not overwrite the input.
        assert!(crate::io::open_bytes(&p, &std::fs::read(&p).unwrap()).is_ok());
    }

    #[test]
    fn rename_detection_needs_identical_cells() {
        let mut old = wb_of(&book(&[("A1", "1")]));
        let mut b = gridcraft_model::Sheet::new("B");
        b.set_value(CellRef::new(0, 0), Value::text("b"));
        old.sheets.push(std::sync::Arc::new(b));
        let mut new = old.clone();
        sheet_mut(&mut new, 1).name = "C".into();
        sheet_mut(&mut new, 1).set_value(CellRef::new(0, 0), Value::text("different"));
        let r = compare(&old, &new, "rename", json!({}));
        assert_eq!(rows(&r), [row("sheet", "C", "", "", "C"), row("sheet", "B", "", "B", "")]);
    }
}
