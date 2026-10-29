//! Data ▸ Quality Check: a deterministic read of a structured register (document register,
//! tracker, material take-off) for duplicate keys, blanks in mandatory-looking columns, bad dates,
//! inconsistent spellings, broken or overwritten formulas and unusual numbers. It reports what it
//! sees, cell by cell, and changes nothing: whether a finding matters is for the reader to judge.
//!
//! Without `rules` every column gets the automatic checks, each gated by what the column looks
//! like (a key-like header for duplicates, a mostly filled column for blanks, and so on). Explicit
//! `rules` replace them and drop the gates, which is how a reusable register profile is written.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, HashMap};
use std::sync::Arc;

use gridcraft_core::date::datetime_from_serial;
use gridcraft_core::parse::parse_input;
use gridcraft_core::{CellRef, DateSystem, MAX_ROWS, RangeRef, Value, letters_to_col, number_to_text, quote_sheet};
use gridcraft_model::style::StyleId;
use gridcraft_model::{Formula, Sheet};
use gridcraft_numfmt::{FormatKind, NumberFormat};
use serde_json::{Value as Json, json};

use super::{CommandSpec, Result, Session, bad, cmd, has_doc, split_sheet, str_param, target_sheet};

const ID: &str = "data.qualityCheck";
/// The most cells one check reads; a larger range is checked from the top down to this many.
const MAX_CELLS: u64 = 1_000_000;
const DEFAULT_LIMIT: usize = 2_000;
const MAX_LIMIT: usize = 20_000;
/// A column with at most this many distinct texts reads as a status column.
const STATUS_MAX_DISTINCT: usize = 20;
/// Numbers further than this many median absolute deviations from the median are unusual.
const MAD_LIMIT: f64 = 6.0;
/// Fewer numbers than this say nothing about what is usual.
const MIN_NUMBERS: usize = 10;
/// Whole words in a header that mark an identifier column (checked for duplicates).
const KEY_WORDS: &[&str] = &["no", "number", "id", "tag", "ref", "code"];
const YEARS: std::ops::RangeInclusive<i32> = 1900..=2200;

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        query "data.qualityCheck",
        "Quality Check",
        ["Data", "Data Tools"],
        None,
        "{sheet?, range?: \"A1:F200\" (default: the used range), header?: row number (default: the first non-empty row), rules?: [{check: duplicate|blank|invalidDate|inconsistentValue|brokenFormula|missingFormula|unusualNumber, column?: \"B\" or header text (default: every column), allowed?: [\"Open\", \"Closed\"]}] (replace the automatic checks), requiredSheets?: [\"Register\"], limit?: n (default 2000, at most 20000)} → {sheet, range, header, issues: [{check, cell, message, suggestion}], counts, total, truncated, scanned, capped, note}",
        has_doc,
        quality_check
    )]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Check {
    RequiredSheet,
    Duplicate,
    Blank,
    InvalidDate,
    InconsistentValue,
    BrokenFormula,
    MissingFormula,
    UnusualNumber,
}

impl Check {
    const COLUMN: [Check; 7] = [
        Check::Duplicate,
        Check::Blank,
        Check::InvalidDate,
        Check::InconsistentValue,
        Check::BrokenFormula,
        Check::MissingFormula,
        Check::UnusualNumber,
    ];

    fn name(self) -> &'static str {
        match self {
            Check::RequiredSheet => "requiredSheet",
            Check::Duplicate => "duplicate",
            Check::Blank => "blank",
            Check::InvalidDate => "invalidDate",
            Check::InconsistentValue => "inconsistentValue",
            Check::BrokenFormula => "brokenFormula",
            Check::MissingFormula => "missingFormula",
            Check::UnusualNumber => "unusualNumber",
        }
    }

    fn parse(s: &str) -> Option<Check> {
        Check::COLUMN.into_iter().find(|c| c.name().eq_ignore_ascii_case(s.trim()))
    }

    fn rank(self) -> u8 {
        self as u8
    }
}

/// One finding. Ordered by where it is: workbook-level findings first, then by row, column and
/// check, then in the order found.
struct Issue {
    key: (u8, u32, u32, u8, u64),
    check: &'static str,
    cell: Option<String>,
    message: String,
    suggestion: String,
}

impl PartialEq for Issue {
    fn eq(&self, o: &Self) -> bool {
        self.key == o.key
    }
}
impl Eq for Issue {}
impl PartialOrd for Issue {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Issue {
    fn cmp(&self, o: &Self) -> Ordering {
        self.key.cmp(&o.key)
    }
}

/// Counts every finding and keeps the first `limit` in sheet order (a bounded max-heap, so a sheet
/// full of findings never holds more than `limit` messages).
struct Sink {
    limit: usize,
    kept: BinaryHeap<Issue>,
    counts: BTreeMap<&'static str, usize>,
    total: usize,
    seq: u64,
}

impl Sink {
    fn new(limit: usize) -> Sink {
        Sink { limit, kept: BinaryHeap::new(), counts: BTreeMap::new(), total: 0, seq: 0 }
    }

    fn ran(&mut self, check: Check) {
        self.counts.entry(check.name()).or_insert(0);
    }

    fn push(&mut self, check: Check, at: Option<(CellRef, String)>, message: String, suggestion: String) {
        self.total = self.total.saturating_add(1);
        let n = self.counts.entry(check.name()).or_insert(0);
        *n = n.saturating_add(1);
        self.seq = self.seq.saturating_add(1);
        if self.limit == 0 {
            return;
        }
        let (key, cell) = match at {
            None => ((0, 0, 0, check.rank(), self.seq), None),
            Some((c, label)) => ((1, c.row, c.col, check.rank(), self.seq), Some(label)),
        };
        self.kept.push(Issue { key, check: check.name(), cell, message, suggestion });
        if self.kept.len() > self.limit {
            self.kept.pop();
        }
    }

    fn issues(self) -> (Vec<Json>, BTreeMap<&'static str, usize>, usize) {
        let issues = self
            .kept
            .into_sorted_vec()
            .into_iter()
            .map(|i| json!({"check": i.check, "cell": i.cell, "message": i.message, "suggestion": i.suggestion}))
            .collect();
        (issues, self.counts, self.total)
    }
}

/// A stored cell of a data row.
struct Item {
    row: u32,
    value: Value,
    formula: Option<Arc<Formula>>,
    /// Shown in a calendar-date number format.
    date_fmt: bool,
}

impl Item {
    fn blank(&self) -> bool {
        self.formula.is_none() && is_blank_value(&self.value)
    }
    /// The value as text, trimmed (General formatting).
    fn shown(&self) -> String {
        self.value.scalar().display().trim().to_string()
    }
}

fn is_blank_value(v: &Value) -> bool {
    match v {
        Value::Empty => true,
        Value::Text(t) => t.trim().is_empty(),
        _ => false,
    }
}

struct Column {
    col: u32,
    header: String,
    /// Stored cells, in row order.
    items: Vec<Item>,
}

impl Column {
    fn at(&self, row: u32) -> Option<&Item> {
        self.items.binary_search_by_key(&row, |i| i.row).ok().and_then(|i| self.items.get(i))
    }
    fn filled(&self, row: u32) -> bool {
        self.at(row).is_some_and(|i| !i.blank())
    }
    /// How messages name the column: its header, else its letter.
    fn name(&self) -> String {
        if self.header.is_empty() { format!("column {}", gridcraft_core::col_to_letters(self.col)) } else { self.header.clone() }
    }
}

/// A rule as given, before its column is found.
struct Rule {
    check: Check,
    column: Option<String>,
    allowed: Option<Vec<String>>,
}

struct Scan<'a> {
    sheet: String,
    sys: DateSystem,
    /// Data rows: rows after the header with at least one filled cell in the range.
    rows: &'a BTreeSet<u32>,
    sink: Sink,
}

impl Scan<'_> {
    fn issue(&mut self, check: Check, at: CellRef, message: String, suggestion: impl Into<String>) {
        let label = format!("{}!{}", self.sheet, at.a1());
        self.sink.push(check, Some((at, label)), message, suggestion.into());
    }
}

fn is_date_format(code: &str) -> bool {
    NumberFormat::parse(code).kind() == FormatKind::Date
}

fn is_key_header(header: &str) -> bool {
    let lower = header.to_lowercase();
    lower.split(|c: char| !c.is_alphanumeric()).any(|w| KEY_WORDS.contains(&w))
}

/// Case and spacing folded away: `" Open  item"` and `"open item"` read the same.
fn fold(t: &str) -> String {
    t.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

fn cell_is_blank(c: &gridcraft_model::Cell) -> bool {
    c.formula.is_none() && is_blank_value(&c.value)
}

fn header_text(sh: &Sheet, row: Option<u32>, col: u32) -> String {
    row.map(|r| sh.value(CellRef::new(r, col)).scalar().display().trim().to_string()).unwrap_or_default()
}

fn string_list(p: &Json, key: &str, what: &str) -> Result<Vec<String>> {
    match p.get(key) {
        None | Some(Json::Null) => Ok(Vec::new()),
        Some(Json::Array(a)) => a
            .iter()
            .map(|v| match v {
                Json::String(t) => Ok(t.clone()),
                Json::Number(n) => Ok(n.to_string()),
                Json::Bool(b) => Ok(if *b { "TRUE".into() } else { "FALSE".into() }),
                other => Err(bad(ID, format!("{what}: {other} is not a name"))),
            })
            .collect(),
        Some(other) => Err(bad(ID, format!("{what} must be a list, not {other}"))),
    }
}

fn parse_rules(p: &Json) -> Result<Option<Vec<Rule>>> {
    let list = match p.get("rules") {
        None | Some(Json::Null) => return Ok(None),
        Some(Json::Array(a)) => a,
        Some(other) => return Err(bad(ID, format!("rules must be a list, not {other}"))),
    };
    let mut rules = Vec::with_capacity(list.len().min(1_000));
    for r in list {
        let Some(name) = str_param(r, "check") else { return Err(bad(ID, "each rule needs a `check`")) };
        let check = Check::parse(name).ok_or_else(|| {
            bad(
                ID,
                format!("unknown check `{name}`: duplicate, blank, invalidDate, inconsistentValue, brokenFormula, missingFormula or unusualNumber"),
            )
        })?;
        let column = match r.get("column") {
            None | Some(Json::Null) => None,
            Some(Json::String(t)) => Some(t.clone()),
            Some(other) => return Err(bad(ID, format!("column must be a letter or header text, not {other}"))),
        };
        let allowed = match r.get("allowed") {
            None | Some(Json::Null) => None,
            Some(_) => Some(string_list(r, "allowed", "allowed")?),
        };
        rules.push(Rule { check, column, allowed });
    }
    Ok(Some(rules))
}

/// The column a rule names: header text first (case-insensitive), then a column letter.
fn find_column(columns: &[Column], name: &str) -> Option<usize> {
    let t = name.trim();
    if let Some(i) = columns.iter().position(|c| !c.header.is_empty() && c.header.eq_ignore_ascii_case(t)) {
        return Some(i);
    }
    let col = letters_to_col(&t.to_ascii_uppercase())?;
    columns.iter().position(|c| c.col == col)
}

fn quality_check(s: &mut Session, p: &Json) -> Result<Json> {
    let si = target_sheet(s, p)?;
    let rules = parse_rules(p)?;
    let required = string_list(p, "requiredSheets", "requiredSheets")?;
    let limit = match p.get("limit") {
        None | Some(Json::Null) => DEFAULT_LIMIT,
        Some(v) => v.as_u64().ok_or_else(|| bad(ID, "limit must be a whole number"))?.min(MAX_LIMIT as u64) as usize,
    };
    let header_param = match p.get("header") {
        None | Some(Json::Null) => None,
        Some(v) => {
            let n = v.as_u64().filter(|n| (1..=u64::from(MAX_ROWS)).contains(n)).ok_or_else(|| bad(ID, "header must be a row number from 1"))?;
            Some((n - 1) as u32)
        }
    };
    let asked = match str_param(p, "range") {
        Some(r) => {
            let (_, body) = split_sheet(r);
            Some(RangeRef::parse(body).ok_or_else(|| bad(ID, format!("not a range: `{r}`")))?)
        }
        None => None,
    };

    let d = s.doc()?;
    let wb = &d.wb;
    let Some(sh) = wb.sheet(si) else { return Err(bad(ID, "no such sheet")) };
    let mut sink = Sink::new(limit);

    if !required.is_empty() {
        sink.ran(Check::RequiredSheet);
    }
    for name in &required {
        if wb.sheet_index(name.trim()).is_none() {
            sink.push(
                Check::RequiredSheet,
                None,
                format!("Required sheet \"{name}\" is missing from the workbook"),
                format!("Add a sheet named \"{name}\", or check the spelling of the sheet names"),
            );
        }
    }
    let checks_to_run: Vec<Check> = match &rules {
        None => Check::COLUMN.to_vec(),
        Some(r) => r.iter().map(|r| r.check).collect(),
    };
    for c in &checks_to_run {
        sink.ran(*c);
    }

    // The block to read: the range asked for, inside the sheet's used range.
    let used = sh.cells.used_range();
    let area = match (asked, used) {
        (Some(a), Some(u)) => a.intersection(&u),
        (None, u) => u,
        (Some(_), None) => None,
    };
    let header_row = match header_param {
        Some(h) => Some(h),
        None => area.and_then(|a| sh.cells.iter_range(a).find(|(_, c)| !cell_is_blank(c)).map(|(at, _)| at.row)),
    };
    // Data rows: below the header, inside the area, at most MAX_CELLS cells in all.
    let span = area.and_then(|a| {
        let first = header_row.map_or(a.start.row, |h| h.saturating_add(1).max(a.start.row));
        if first > a.end.row {
            return None;
        }
        let max_rows = (MAX_CELLS / u64::from(a.width().max(1))).max(1);
        let last_allowed = u64::from(first).saturating_add(max_rows - 1);
        let last = u64::from(a.end.row).min(last_allowed) as u32;
        Some((a, first, last))
    });

    let mut columns: Vec<Column> = Vec::new();
    let mut rows: BTreeSet<u32> = BTreeSet::new();
    let mut scanned: u64 = 0;
    let mut capped = false;
    let mut checked: Option<RangeRef> = area;
    if let Some((a, first, last)) = span {
        capped = last < a.end.row;
        checked = Some(RangeRef::new(a.start, CellRef::new(last, a.end.col)));
        scanned = u64::from(a.width()).saturating_mul(u64::from(last - first) + 1);
        columns = (a.start.col..=a.end.col).map(|c| Column { col: c, header: header_text(sh, header_row, c), items: Vec::new() }).collect();
        let mut date_styles: HashMap<StyleId, bool> = HashMap::new();
        let block = RangeRef::new(CellRef::new(first, a.start.col), CellRef::new(last, a.end.col));
        for (at, cell) in sh.cells.iter_range(block) {
            let Some(column) = at.col.checked_sub(a.start.col).and_then(|i| columns.get_mut(i as usize)) else { continue };
            let date_fmt = *date_styles.entry(cell.style).or_insert_with(|| is_date_format(wb.styles.get(cell.style).num_fmt.as_str()));
            let item = Item { row: at.row, value: cell.value.clone(), formula: cell.formula.clone(), date_fmt };
            if !item.blank() {
                rows.insert(at.row);
            }
            column.items.push(item);
        }
    }

    let mut scan = Scan { sheet: quote_sheet(&sh.name), sys: wb.date_system, rows: &rows, sink };
    if !columns.is_empty() {
        match &rules {
            None => {
                for column in &columns {
                    for check in Check::COLUMN {
                        run_check(&mut scan, column, check, false, None);
                    }
                }
            }
            Some(rules) => {
                for rule in rules {
                    let targets: Vec<usize> = match &rule.column {
                        None => (0..columns.len()).collect(),
                        Some(name) => match find_column(&columns, name) {
                            Some(i) => vec![i],
                            None => return Err(bad(ID, format!("no column `{name}` in the checked range (give its letter or its header text)"))),
                        },
                    };
                    for i in targets {
                        if let Some(column) = columns.get(i) {
                            run_check(&mut scan, column, rule.check, true, rule.allowed.as_deref());
                        }
                    }
                }
            }
        }
    }

    let (issues, counts, total) = scan.sink.issues();
    let note = if capped {
        span.map(|(_, first, last)| {
            format!("Only rows {} to {} were checked: a quality check reads at most 1,000,000 cells", first.saturating_add(1), last.saturating_add(1))
        })
    } else {
        None
    };
    Ok(json!({
        "sheet": sh.name,
        "range": checked.map(|r| r.a1()),
        "header": header_row.map(|r| r.saturating_add(1)),
        "issues": issues,
        "counts": counts,
        "total": total,
        "truncated": total > limit,
        "scanned": scanned,
        "capped": capped,
        "note": note,
    }))
}

fn run_check(sc: &mut Scan, column: &Column, check: Check, explicit: bool, allowed: Option<&[String]>) {
    match check {
        Check::Duplicate => duplicates(sc, column, explicit),
        Check::Blank => blanks(sc, column, explicit),
        Check::InvalidDate => invalid_dates(sc, column, explicit),
        Check::InconsistentValue => inconsistent(sc, column, explicit, allowed),
        Check::BrokenFormula => broken_formulas(sc, column),
        Check::MissingFormula => missing_formulas(sc, column),
        Check::UnusualNumber => unusual_numbers(sc, column),
        Check::RequiredSheet => {}
    }
}

fn duplicates(sc: &mut Scan, column: &Column, explicit: bool) {
    if !explicit && !is_key_header(&column.header) {
        return;
    }
    let name = column.name();
    let mut first: HashMap<String, u32> = HashMap::new();
    for item in &column.items {
        if item.blank() || item.value.scalar().is_error() {
            continue;
        }
        let shown = item.shown();
        match first.get(&shown.to_lowercase()) {
            Some(&row) => {
                let at = CellRef::new(row, column.col).a1();
                sc.issue(
                    Check::Duplicate,
                    CellRef::new(item.row, column.col),
                    format!("Duplicate {name} \"{shown}\" (first at {at})"),
                    "Review the duplicate",
                );
            }
            None => {
                first.insert(shown.to_lowercase(), item.row);
            }
        }
    }
}

fn blanks(sc: &mut Scan, column: &Column, explicit: bool) {
    let rows = sc.rows;
    let total = rows.len();
    if total == 0 {
        return;
    }
    let filled = rows.iter().filter(|r| column.filled(**r)).count();
    if filled == total || (!explicit && filled.saturating_mul(10) < total.saturating_mul(9)) {
        return;
    }
    let name = column.name();
    let pct = filled.saturating_mul(100) / total;
    for &row in rows {
        if column.filled(row) {
            continue;
        }
        let message = if explicit { format!("Blank {name}") } else { format!("Blank {name} ({pct}% of the rows fill this column)") };
        sc.issue(Check::Blank, CellRef::new(row, column.col), message, "Fill in the missing value");
    }
}

/// How a cell reads as a date.
enum DateRead {
    /// A serial: a number in a date format, or text that typed in would be a date.
    Date(f64),
    /// Text that is not a date.
    NotDate(String),
    /// Anything else (a plain number, TRUE, an error).
    Other,
}

fn read_date(item: &Item, sys: DateSystem) -> DateRead {
    match item.value.scalar() {
        Value::Number(n) if item.date_fmt => DateRead::Date(n),
        Value::Text(t) => {
            let parsed = parse_input(t.trim(), sys);
            match (parsed.value, parsed.format) {
                (Value::Number(n), Some(f)) if is_date_format(f) => DateRead::Date(n),
                _ => DateRead::NotDate(t.trim().to_string()),
            }
        }
        _ => DateRead::Other,
    }
}

fn invalid_dates(sc: &mut Scan, column: &Column, explicit: bool) {
    let reads: Vec<(u32, DateRead)> = column.items.iter().filter(|i| !i.blank()).map(|i| (i.row, read_date(i, sc.sys))).collect();
    if reads.is_empty() {
        return;
    }
    let dates = reads.iter().filter(|(_, r)| matches!(r, DateRead::Date(_))).count();
    // Mostly dates: more than half of the filled cells.
    if !explicit && dates.saturating_mul(2) <= reads.len() {
        return;
    }
    let name = column.name();
    for (row, read) in reads {
        let at = CellRef::new(row, column.col);
        match read {
            DateRead::NotDate(t) => sc.issue(
                Check::InvalidDate,
                at,
                format!("\"{t}\" in {name} is not a date"),
                "Enter a date, or leave the cell blank if the date is not known",
            ),
            DateRead::Date(n) => match datetime_from_serial(sc.sys, n) {
                Some(dt) if YEARS.contains(&dt.year) => {}
                Some(dt) => sc.issue(
                    Check::InvalidDate,
                    at,
                    format!("{name} date {:04}-{:02}-{:02} is outside the years 1900 to 2200", dt.year, dt.month, dt.day),
                    "Check the year",
                ),
                None => sc.issue(
                    Check::InvalidDate,
                    at,
                    format!("{name} holds {} in a date format, which is not a date", number_to_text(n)),
                    "Enter a date",
                ),
            },
            DateRead::Other => {}
        }
    }
}

fn inconsistent(sc: &mut Scan, column: &Column, explicit: bool, allowed: Option<&[String]>) {
    let name = column.name();
    if let Some(allowed) = allowed {
        let list = allowed.join(", ");
        for item in column.items.iter().filter(|i| !i.blank()) {
            let raw = match item.value.scalar() {
                Value::Text(t) => t.to_string(),
                v => v.display(),
            };
            if allowed.contains(&raw) {
                continue;
            }
            let at = CellRef::new(item.row, column.col);
            match allowed.iter().find(|a| fold(a) == fold(&raw)) {
                Some(a) => sc.issue(
                    Check::InconsistentValue,
                    at,
                    format!("\"{raw}\" in {name} differs from the allowed \"{a}\" only in case or spacing"),
                    format!("Use \"{a}\""),
                ),
                None => sc.issue(
                    Check::InconsistentValue,
                    at,
                    format!("\"{raw}\" is not an allowed {name} (allowed: {list})"),
                    format!("Use one of: {list}"),
                ),
            }
        }
        return;
    }
    // Identifier columns are checked for duplicates, not spellings.
    if !explicit && is_key_header(&column.header) {
        return;
    }
    let texts: Vec<(u32, String)> =
        column.items.iter().filter(|i| !i.blank()).filter_map(|i| i.value.scalar().as_text().map(|t| (i.row, t.to_string()))).collect();
    // Folded value → its spellings with how often each appears, in the order first seen.
    let mut groups: BTreeMap<String, Vec<(String, usize)>> = BTreeMap::new();
    for (_, t) in &texts {
        let g = groups.entry(fold(t)).or_default();
        match g.iter_mut().find(|(v, _)| v == t) {
            Some((_, n)) => *n = n.saturating_add(1),
            None => g.push((t.clone(), 1)),
        }
    }
    if !explicit && groups.len() > STATUS_MAX_DISTINCT {
        return;
    }
    // The majority spelling of each group with more than one; a tie goes to the first seen.
    let mut majority: HashMap<String, String> = HashMap::new();
    for (key, spellings) in &groups {
        if spellings.len() < 2 {
            continue;
        }
        let mut best: Option<&(String, usize)> = None;
        for s in spellings {
            if best.is_none_or(|b| s.1 > b.1) {
                best = Some(s);
            }
        }
        if let Some((v, _)) = best {
            majority.insert(key.clone(), v.clone());
        }
    }
    for (row, t) in &texts {
        let Some(m) = majority.get(&fold(t)) else { continue };
        if m != t {
            sc.issue(
                Check::InconsistentValue,
                CellRef::new(*row, column.col),
                format!("\"{t}\" in {name} differs from \"{m}\" only in case or spacing"),
                format!("Use \"{m}\""),
            );
        }
    }
}

fn broken_formulas(sc: &mut Scan, column: &Column) {
    for item in &column.items {
        let Some(f) = &item.formula else { continue };
        let ref_text = f.text.to_ascii_uppercase().contains("#REF!");
        let at = CellRef::new(item.row, column.col);
        match item.value.scalar() {
            Value::Error(e) if ref_text || e == gridcraft_core::CellError::Ref => sc.issue(
                Check::BrokenFormula,
                at,
                format!("Formula ={} refers to a cell that no longer exists ({})", f.text, e.as_str()),
                "Point the reference at the right cell again",
            ),
            Value::Error(e) => {
                sc.issue(Check::BrokenFormula, at, format!("Formula ={} returns {}", f.text, e.as_str()), "Check the formula and the cells it reads")
            }
            _ if ref_text => sc.issue(
                Check::BrokenFormula,
                at,
                format!("Formula ={} contains #REF! (a reference to a deleted cell)", f.text),
                "Point the reference at the right cell again",
            ),
            _ => {}
        }
    }
}

fn missing_formulas(sc: &mut Scan, column: &Column) {
    for item in &column.items {
        if item.formula.is_some() || item.blank() {
            continue;
        }
        let above = item.row.checked_sub(1).and_then(|r| column.at(r)).and_then(|i| i.formula.clone());
        let below = item.row.checked_add(1).and_then(|r| column.at(r)).and_then(|i| i.formula.as_ref());
        let (Some(above), Some(_)) = (above, below) else { continue };
        sc.issue(
            Check::MissingFormula,
            CellRef::new(item.row, column.col),
            format!("Constant {} where the cells above and below hold formulas", item.shown()),
            format!("Check whether a formula was overwritten (the cell above holds ={})", above.text),
        );
    }
}

fn median(sorted: &[f64]) -> Option<f64> {
    let n = sorted.len();
    if n == 0 {
        return None;
    }
    let hi = sorted.get(n / 2).copied()?;
    if n % 2 == 1 { Some(hi) } else { sorted.get(n / 2 - 1).map(|lo| (lo + hi) / 2.0) }
}

fn unusual_numbers(sc: &mut Scan, column: &Column) {
    let nums: Vec<(u32, f64)> = column.items.iter().filter(|i| !i.date_fmt).filter_map(|i| i.value.scalar().as_f64().map(|n| (i.row, n))).collect();
    if nums.len() < MIN_NUMBERS {
        return;
    }
    let mut sorted: Vec<f64> = nums.iter().map(|(_, n)| *n).collect();
    sorted.sort_by(f64::total_cmp);
    let Some(med) = median(&sorted) else { return };
    let mut dev: Vec<f64> = sorted.iter().map(|n| (n - med).abs()).collect();
    dev.sort_by(f64::total_cmp);
    let Some(mad) = median(&dev) else { return };
    // No spread (most values equal): nothing to measure "unusual" against.
    if !mad.is_finite() || mad <= 0.0 {
        return;
    }
    let name = column.name();
    for (row, n) in nums {
        if (n - med).abs() > MAD_LIMIT * mad {
            sc.issue(
                Check::UnusualNumber,
                CellRef::new(row, column.col),
                format!(
                    "Unusual {name} {}: more than 6 median absolute deviations from the column median {}",
                    number_to_text(n),
                    number_to_text(med)
                ),
                "Check the value for a typing or unit error",
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{Value as Json, json};

    use crate::Session;

    fn book(rows: Json) -> Session {
        let mut s = Session::new();
        s.new_workbook();
        s.execute("range.setValues", json!({"range": "A1", "values": rows})).unwrap();
        s
    }

    fn check(s: &mut Session, p: Json) -> Json {
        s.execute("data.qualityCheck", p).unwrap()
    }

    fn cells(r: &Json, check: &str) -> Vec<String> {
        r["issues"].as_array().unwrap().iter().filter(|i| i["check"] == check).map(|i| i["cell"].as_str().unwrap_or("").to_string()).collect()
    }

    fn issue<'a>(r: &'a Json, check: &str, cell: &str) -> &'a Json {
        r["issues"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["check"] == check && i["cell"] == cell)
            .unwrap_or_else(|| panic!("no {check} at {cell}: {r:#}"))
    }

    /// A clean twelve-row register: unique numbers, two statuses, dates, quantities.
    fn clean_rows() -> Vec<Json> {
        let mut rows = vec![json!(["Doc No", "Rev", "Status", "Date", "Qty"])];
        for i in 0..12 {
            let status = if i % 3 == 0 { "Closed" } else { "Open" };
            rows.push(json!([format!("D-{:03}", i + 1), "A", status, format!("2026-01-{:02}", i + 1), 10 + i]));
        }
        rows
    }

    #[test]
    fn clean_register_has_no_issues() {
        let mut s = book(Json::Array(clean_rows()));
        let r = check(&mut s, json!({}));
        assert_eq!(r["issues"], json!([]), "{r:#}");
        assert_eq!(r["total"], 0);
        assert_eq!(r["header"], 1);
        assert_eq!(r["range"], "A1:E13");
        assert_eq!(r["truncated"], false);
        assert_eq!(r["capped"], false);
        for c in ["duplicate", "blank", "invalidDate", "inconsistentValue", "brokenFormula", "missingFormula", "unusualNumber"] {
            assert_eq!(r["counts"][c], 0, "{c}");
        }
    }

    #[test]
    fn duplicate_document_numbers_name_the_first_occurrence() {
        let mut rows = clean_rows();
        rows[5] = json!(["d-002 ", "A", "Open", "2026-01-05", 14]);
        let mut s = book(Json::Array(rows));
        let r = check(&mut s, json!({}));
        assert_eq!(cells(&r, "duplicate"), ["Sheet1!A6"]);
        let i = issue(&r, "duplicate", "Sheet1!A6");
        assert_eq!(i["message"], "Duplicate Doc No \"d-002\" (first at A3)");
        assert_eq!(i["suggestion"], "Review the duplicate");
        // "Rev" holds "A" on every row but is not an identifier column.
        assert_eq!(r["counts"]["duplicate"], 1);
    }

    #[test]
    fn blanks_are_found_in_mostly_filled_columns_only() {
        let mut rows = vec![json!(["Doc No", "Status", "Remarks"])];
        for i in 0..10 {
            let status = if i == 4 { "" } else { "Open" };
            let remark = if i % 2 == 0 { "see note" } else { "" };
            rows.push(json!([format!("D-{i}"), status, remark]));
        }
        let mut s = book(Json::Array(rows));
        let r = check(&mut s, json!({}));
        // Status is 90% filled: its blank is flagged. Remarks is half filled: nothing.
        assert_eq!(cells(&r, "blank"), ["Sheet1!B6"]);
        assert!(issue(&r, "blank", "Sheet1!B6")["message"].as_str().unwrap().contains("90%"));
    }

    #[test]
    fn invalid_dates_and_years_out_of_range() {
        let mut rows = clean_rows();
        rows[3][3] = json!("TBA");
        rows[7][3] = json!("2999-01-01");
        let mut s = book(Json::Array(rows));
        let r = check(&mut s, json!({}));
        assert_eq!(cells(&r, "invalidDate"), ["Sheet1!D4", "Sheet1!D8"]);
        assert_eq!(issue(&r, "invalidDate", "Sheet1!D4")["message"], "\"TBA\" in Date is not a date");
        assert!(issue(&r, "invalidDate", "Sheet1!D8")["message"].as_str().unwrap().contains("2999-01-01"));
        // Date text that was never converted counts as a date too.
        let mut s = book(json!([["Date"]]));
        s.execute("range.setValues", json!({"range": "A2", "values": [["2026-01-01"], ["2026-02-01"], ["31/02/x"]], "parse": false})).unwrap();
        let r = check(&mut s, json!({}));
        assert_eq!(cells(&r, "invalidDate"), ["Sheet1!A4"]);
    }

    #[test]
    fn inconsistent_spellings_suggest_the_majority() {
        let mut rows = clean_rows();
        rows[2][2] = json!("open ");
        rows[6][2] = json!("CLOSED");
        let mut s = book(Json::Array(rows));
        let r = check(&mut s, json!({}));
        assert_eq!(cells(&r, "inconsistentValue"), ["Sheet1!C3", "Sheet1!C7"]);
        assert_eq!(issue(&r, "inconsistentValue", "Sheet1!C3")["suggestion"], "Use \"Open\"");
        assert_eq!(issue(&r, "inconsistentValue", "Sheet1!C7")["suggestion"], "Use \"Closed\"");
    }

    #[test]
    fn broken_and_missing_formulas() {
        let mut s = book(json!([["Item", "Qty", "Total"], ["a", 1], ["b", 2], ["c", 3], ["d", 4], ["e", 5]]));
        for (cell, input) in [("C2", "=B2*2"), ("C3", "=B3*2"), ("C4", "10"), ("C5", "=B5*2"), ("C6", "=1/0"), ("B8", "=#REF!+1")] {
            s.execute("cell.set", json!({"cell": cell, "input": input})).unwrap();
        }
        let r = check(&mut s, json!({}));
        assert_eq!(cells(&r, "missingFormula"), ["Sheet1!C4"]);
        assert!(issue(&r, "missingFormula", "Sheet1!C4")["suggestion"].as_str().unwrap().contains("=B3*2"));
        assert_eq!(cells(&r, "brokenFormula"), ["Sheet1!C6", "Sheet1!B8"]);
        assert!(issue(&r, "brokenFormula", "Sheet1!C6")["message"].as_str().unwrap().contains("#DIV/0!"));
        assert!(issue(&r, "brokenFormula", "Sheet1!B8")["message"].as_str().unwrap().contains("#REF!"));
    }

    #[test]
    fn unusual_numbers_against_the_median() {
        let mut rows = clean_rows();
        rows[9][4] = json!(10_000);
        let mut s = book(Json::Array(rows));
        let r = check(&mut s, json!({}));
        assert_eq!(cells(&r, "unusualNumber"), ["Sheet1!E10"]);
        // Fewer than ten numbers: nothing is judged unusual.
        let mut s = book(json!([["Qty"], [1], [2], [3], [1000]]));
        assert_eq!(check(&mut s, json!({}))["counts"]["unusualNumber"], 0);
    }

    #[test]
    fn explicit_rules_replace_the_automatic_checks() {
        let mut rows = clean_rows();
        rows[2][2] = json!("open");
        rows[4][2] = json!("Pending");
        rows[5][0] = json!("D-001");
        for row in [3, 4, 5] {
            rows[row][1] = json!("");
        }
        let mut s = book(Json::Array(rows));
        let r = check(&mut s, json!({"rules": [{"check": "inconsistentValue", "column": "Status", "allowed": ["Open", "Closed"]}]}));
        assert_eq!(cells(&r, "inconsistentValue"), ["Sheet1!C3", "Sheet1!C5"]);
        assert_eq!(issue(&r, "inconsistentValue", "Sheet1!C3")["suggestion"], "Use \"Open\"");
        assert_eq!(issue(&r, "inconsistentValue", "Sheet1!C5")["suggestion"], "Use one of: Open, Closed");
        assert_eq!(r["counts"], json!({"inconsistentValue": 2}), "the automatic duplicate check did not run");
        // A column by letter; an explicit blank rule flags every blank, however empty the column.
        let r = check(&mut s, json!({"rules": [{"check": "duplicate", "column": "a"}, {"check": "blank", "column": "B"}]}));
        assert_eq!(cells(&r, "duplicate"), ["Sheet1!A6"]);
        assert_eq!(cells(&r, "blank"), ["Sheet1!B4", "Sheet1!B5", "Sheet1!B6"]);
        // Automatically, a column 75% filled is not a mandatory-looking one.
        assert_eq!(check(&mut s, json!({}))["counts"]["blank"], 0);
        assert!(s.execute("data.qualityCheck", json!({"rules": [{"check": "blank", "column": "Nope"}]})).is_err());
        assert!(s.execute("data.qualityCheck", json!({"rules": [{"check": "blank", "column": "F"}]})).is_err(), "F is outside the checked range");
        assert!(s.execute("data.qualityCheck", json!({"rules": [{"check": "spelling", "column": "A"}]})).is_err());
        assert!(s.execute("data.qualityCheck", json!({"rules": "duplicate"})).is_err());
    }

    #[test]
    fn required_sheets_are_reported_first() {
        let mut rows = clean_rows();
        rows[5][0] = json!("D-001");
        let mut s = book(Json::Array(rows));
        let r = check(&mut s, json!({"requiredSheets": ["Register", "sheet1"]}));
        let first = &r["issues"][0];
        assert_eq!(first["check"], "requiredSheet");
        assert_eq!(first["cell"], Json::Null);
        assert!(first["message"].as_str().unwrap().contains("\"Register\""));
        assert_eq!(r["counts"]["requiredSheet"], 1);
    }

    #[test]
    fn limit_truncates_in_sheet_order() {
        let mut rows = vec![json!(["Tag", "Status"])];
        for _ in 0..30 {
            rows.push(json!(["P-1", "Open"]));
        }
        let mut s = book(Json::Array(rows));
        let r = check(&mut s, json!({"limit": 5}));
        assert_eq!(r["total"], 29);
        assert_eq!(r["counts"]["duplicate"], 29);
        assert_eq!(r["truncated"], true);
        assert_eq!(cells(&r, "duplicate"), ["Sheet1!A3", "Sheet1!A4", "Sheet1!A5", "Sheet1!A6", "Sheet1!A7"]);
        let r = check(&mut s, json!({"limit": 0}));
        assert_eq!(r["issues"], json!([]));
        assert_eq!(r["truncated"], true);
        assert_eq!(check(&mut s, json!({"limit": 1_000_000}))["issues"].as_array().unwrap().len(), 29);
    }

    #[test]
    fn header_row_and_range_are_honoured() {
        let mut s = book(json!([["Document register"], [""], ["Doc No", "Status"], ["D-1", "Open"], ["D-1", "Open"]]));
        let r = check(&mut s, json!({"header": 3}));
        assert_eq!(r["header"], 3);
        assert_eq!(cells(&r, "duplicate"), ["Sheet1!A5"]);
        // Without `header` the title row is the header, and "Doc No" is just data.
        let r = check(&mut s, json!({}));
        assert_eq!(r["header"], 1);
        assert_eq!(r["counts"]["duplicate"], 0);
        let r = check(&mut s, json!({"range": "Sheet1!A3:B5"}));
        assert_eq!(r["range"], "A3:B5");
        assert_eq!(cells(&r, "duplicate"), ["Sheet1!A5"]);
        assert!(s.execute("data.qualityCheck", json!({"header": 0})).is_err());
        assert!(s.execute("data.qualityCheck", json!({"range": "nonsense"})).is_err());
    }

    #[test]
    fn empty_sheet_has_no_issues() {
        let mut s = Session::new();
        s.new_workbook();
        let r = check(&mut s, json!({}));
        assert_eq!(r["issues"], json!([]));
        assert_eq!(r["range"], Json::Null);
        assert_eq!(r["header"], Json::Null);
        let r = check(&mut s, json!({"range": "C3:F9", "header": 2, "rules": [{"check": "blank", "column": "Status"}]}));
        assert_eq!(r["total"], 0);
        // With no workbook open the command is disabled, not a crash.
        assert!(Session::new().execute("data.qualityCheck", json!({})).is_err());
    }
}
