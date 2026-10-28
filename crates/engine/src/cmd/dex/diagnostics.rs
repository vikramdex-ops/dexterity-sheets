//! Formula diagnostics: `formulas.explainError` says in plain words why one cell shows an error
//! value, with the evidence found in its formula and the cells it reads; `formulas.diagnostics`
//! sweeps a sheet (or the workbook) for errors, broken references, formulas that break the pattern
//! of their neighbours, constants typed over a run of formulas, circular references and links to
//! other workbooks.
//!
//! Every explanation here is written for Dexterity Sheets. The checks report what the formulas
//! and values show; they never decide that a sheet is correct.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use gridcraft_core::{CellError, CellRef, MAX_COLS, MAX_ROWS, RangeRef, Value};
use gridcraft_formula::{BinOp, Expr, RefKind, Reference, SheetSel};
use gridcraft_model::{Formula, Sheet, Workbook};
use serde_json::{Value as Json, json};

use super::*;

/// `formulas.diagnostics` returns this many issues unless `limit` says otherwise.
const DEFAULT_LIMIT: u64 = 2_000;
/// The most issues one call returns.
const MAX_LIMIT: u64 = 20_000;
/// The most formula cells one diagnostics call reads; the rest are reported as not checked.
const MAX_FORMULAS: usize = 1_000_000;
/// The most cells `formulas.explainError` reads in the ranges a formula refers to.
const MAX_PRECEDENT_CELLS: usize = 10_000;
/// The most evidence lines one explanation carries.
const MAX_EVIDENCE: usize = 20;

/// The diagnostics checks, in the order they are reported for one cell.
const CHECKS: [&str; 6] = ["error", "brokenReference", "inconsistentFormula", "hardcodedValue", "circular", "externalReference"];
const ERROR: usize = 0;
const BROKEN: usize = 1;
const INCONSISTENT: usize = 2;
const HARDCODED: usize = 3;
const CIRCULAR: usize = 4;
const EXTERNAL: usize = 5;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "formulas.explainError",
            "Explain Error",
            ["Formulas", "Formula Auditing"],
            None,
            "{cell?, sheet?} (default the active cell) → {cell, sheet, formula, error, title, explanation, evidence: [..], likelyCauses: [..], fixes: [..], precedents: [{ref, value?, errors}]}; error is null when the cell shows no error",
            has_doc,
            explain_error
        ),
        cmd!(
            query "formulas.diagnostics",
            "Formula Diagnostics",
            ["Formulas", "Formula Auditing"],
            None,
            "{sheet?: name | \"all\" (default all), checks?: [error, brokenReference, inconsistentFormula, hardcodedValue, circular, externalReference] (default all), limit?: 2000 (max 20000)} → {issues: [{check, cell: \"Sheet1!D5\", message, detail}], counts, truncated, scanned, capped}",
            has_doc,
            diagnostics
        ),
    ]
}

// ---------------------------------------------------------------- what each error means

/// What one error value means, in Dexterity Sheets' own words.
pub struct ErrorInfo {
    pub code: &'static str,
    pub title: &'static str,
    pub explanation: &'static str,
    pub causes: &'static [&'static str],
    pub fixes: &'static [&'static str],
}

const ERRORS: &[ErrorInfo] = &[
    ErrorInfo {
        code: "#REF!",
        title: "Reference no longer points anywhere",
        explanation: "The formula asks for cells that are not there any more. This usually happens after the rows, columns or sheet a reference pointed at were deleted, or when a reference built while calculating lands outside the sheet.",
        causes: &[
            "Rows, columns or cells the formula read were deleted, and the reference was replaced by #REF!.",
            "A formula was copied to a place where its relative references fall off the top or left edge of the sheet.",
            "The sheet a reference named was deleted.",
            "OFFSET, INDEX or INDIRECT built a reference outside the sheet, or INDIRECT was given text that is not a valid address.",
            "A cell the formula reads already shows #REF!, and the error is passed on.",
        ],
        fixes: &[
            "Undo the deletion if it just happened, so the reference comes back.",
            "Otherwise retype the reference where #REF! now stands in the formula.",
            "Check the arguments given to OFFSET, INDEX or INDIRECT so the reference they build stays on the sheet.",
        ],
    },
    ErrorInfo {
        code: "#NAME?",
        title: "Name not recognised",
        explanation: "The formula uses a word the workbook does not know: a function name, a defined name or a table name that does not exist, or text that was meant to be in quotes.",
        causes: &[
            "A function name is misspelt, or the function does not exist in this program.",
            "A defined name or table name is used before it was created, was deleted, or is misspelt.",
            "Text inside the formula is missing its double quotes, so it is read as a name.",
            "A range is missing its colon (A1B5 instead of A1:B5).",
            "The formula text could not be read as a formula at all.",
        ],
        fixes: &[
            "Correct the spelling of the function or name (Insert Function lists the functions available).",
            "Create the missing name in the Name Manager, or point the formula at an existing one.",
            "Put double quotes around any text the formula uses as a value.",
        ],
    },
    ErrorInfo {
        code: "#VALUE!",
        title: "Wrong kind of value",
        explanation: "An operator or function was given a value of a kind it cannot work with, most often text where a number was needed, or a range where a single value was expected.",
        causes: &[
            "A cell used in arithmetic holds text that does not read as a number (a word, or a number with stray characters).",
            "A cell that looks empty holds one or more spaces.",
            "A function argument expects a single value but was given a range, or the other way round.",
            "A date or time is stored as text the program cannot read as a date.",
            "A function was called with too many or too few arguments.",
        ],
        fixes: &[
            "Replace the text in the cells the formula reads with numbers, or clear cells that hold only spaces.",
            "Use a function that skips text (SUM instead of A1+A2), or convert text with VALUE.",
            "Check each argument against what the function expects.",
        ],
    },
    ErrorInfo {
        code: "#DIV/0!",
        title: "Division by zero",
        explanation: "The formula divides by zero or by an empty cell, which counts as zero. Averages over cells that hold no numbers end the same way.",
        causes: &[
            "The divisor is a cell that is empty or holds 0.",
            "The divisor is a formula whose result is 0.",
            "AVERAGE, AVERAGEIF or AVERAGEIFS found no numbers to average.",
            "MOD or QUOTIENT was given 0 as the divisor.",
        ],
        fixes: &[
            "Fill in the divisor cell, or point the formula at the right one.",
            "Guard the division: =IF(B1=0, \"\", A1/B1), or wrap the formula in IFERROR to show something else.",
        ],
    },
    ErrorInfo {
        code: "#N/A",
        title: "Value not available",
        explanation: "A lookup did not find what it was looking for, or a value the formula needs is marked as not available.",
        causes: &[
            "VLOOKUP, HLOOKUP, XLOOKUP, MATCH, XMATCH or LOOKUP did not find the value it looks for.",
            "The value looked for and the values in the table differ in a way that is hard to see: extra spaces, or a number stored as text on one side.",
            "An exact-match lookup was intended but approximate matching was used on unsorted data, or the other way round.",
            "A cell the formula reads holds #N/A, typed in or returned by NA().",
            "An array formula refers to ranges of different sizes.",
        ],
        fixes: &[
            "Check that the value looked for really is in the lookup range, spelt the same way and of the same type.",
            "Remove stray spaces with TRIM, or convert text numbers with VALUE.",
            "Use XLOOKUP's if-not-found argument, or IFNA, to show something else when nothing matches.",
        ],
    },
    ErrorInfo {
        code: "#NUM!",
        title: "Number out of range",
        explanation: "A calculation produced a number that is not valid: an argument outside what a function accepts, a result too large or too small to store, or a step-by-step search that did not settle.",
        causes: &[
            "SQRT was given a negative number, or LN or LOG a number that is zero or negative.",
            "The result is larger than the largest number the program stores.",
            "IRR, XIRR, RATE or a similar function did not find a result after many tries.",
            "A function was given an argument outside its allowed range (a negative count, a day number before the first date).",
        ],
        fixes: &[
            "Check the arguments against the ranges the function allows.",
            "For IRR, XIRR and RATE, give a guess closer to the answer, or check that the cash flows change sign.",
        ],
    },
    ErrorInfo {
        code: "#NULL!",
        title: "Ranges do not meet",
        explanation: "A space between two references asks for the cells they have in common, and they have none. Often a comma or colon was meant instead of the space.",
        causes: &[
            "Two ranges are separated by a space but do not overlap.",
            "A comma (several ranges) or a colon (one range from end to end) was left out.",
        ],
        fixes: &[
            "Put a colon between the two ends of a range, or a comma between separate ranges.",
            "If the overlap was meant, check that the ranges really cross.",
        ],
    },
    ErrorInfo {
        code: "#SPILL!",
        title: "Result cannot spill",
        explanation: "The formula returns several values that should fill the cells below and to the right of it, but something is in the way, so none of them are shown.",
        causes: &[
            "One of the cells the result needs already holds a value or a formula.",
            "Merged cells sit where the result would go.",
            "The formula is inside a table, where results cannot spill.",
            "The result would run past the last row or column of the sheet.",
            "The result of another formula already spills into those cells.",
        ],
        fixes: &[
            "Clear or move the cells in the way; the result appears as soon as the area is free.",
            "Unmerge the merged cells, or move the formula out of the table.",
            "Make the result smaller (refer to the rows used instead of whole columns).",
        ],
    },
    ErrorInfo {
        code: "#CALC!",
        title: "Calculation could not be done",
        explanation: "The formula asks for something the calculation engine cannot produce, most often an empty array or a function that was defined but never called.",
        causes: &[
            "FILTER found no rows that match and was given nothing to show instead.",
            "A function such as UNIQUE or SORT received an empty range or array.",
            "A LAMBDA was written but never called with values.",
            "A nested array is too large or not supported where it is used.",
        ],
        fixes: &[
            "Give FILTER its third argument (for example \"\") to show when nothing matches.",
            "Call the LAMBDA with values, as in =LAMBDA(x, x+1)(2), or save it as a defined name.",
        ],
    },
    ErrorInfo {
        code: "#FIELD!",
        title: "Field not found",
        explanation: "The formula asks a linked data type (a stock, a place, and the like) for a field it does not have.",
        causes: &[
            "The field name after the dot is misspelt.",
            "The cell referred to does not hold a linked data type, or holds a different one.",
            "The data source no longer offers that field.",
        ],
        fixes: &["Pick the field from the list the data type offers.", "Check that the referenced cell holds the data type you expect."],
    },
    ErrorInfo {
        code: "#BLOCKED!",
        title: "Access blocked",
        explanation: "The formula needs a resource, a service or a feature that settings or policy do not allow here.",
        causes: &[
            "External content, a data connection or a service is turned off by a setting or by your organisation.",
            "The workbook is open in protected or restricted mode.",
        ],
        fixes: &["Allow the content or connection if you trust the workbook.", "Ask your administrator whether the service is allowed."],
    },
    ErrorInfo {
        code: "#CONNECT!",
        title: "Could not connect",
        explanation: "The formula needs data from a service that could not be reached.",
        causes: &["There is no network connection, or the service is down.", "Your account is not signed in to the service, or cannot use it."],
        fixes: &["Check the network connection and try again later.", "Check that you are signed in and allowed to use the service."],
    },
    ErrorInfo {
        code: "#BUSY!",
        title: "Still calculating",
        explanation: "The result is still being fetched or worked out. The cell normally updates on its own once it is ready.",
        causes: &["Data from an outside source has not arrived yet.", "A long calculation is still running."],
        fixes: &["Wait a moment for the result.", "If it stays, recalculate, or check the data source."],
    },
    ErrorInfo {
        code: "#UNKNOWN!",
        title: "Data type not supported",
        explanation: "The cell holds a kind of data this version of the program does not understand, usually saved by a newer program.",
        causes: &["The workbook was saved by a program that supports a data type this one does not."],
        fixes: &["Open the workbook in the program that created the data, or convert the data to plain values there."],
    },
    ErrorInfo {
        code: "#GETTING_DATA",
        title: "Waiting for data",
        explanation: "The formula is waiting for data from a cube or outside source. It is a temporary state, not a mistake in the formula.",
        causes: &["A cube function or data connection is still fetching its result."],
        fixes: &["Wait for the data to arrive; recalculate if it does not."],
    },
    ErrorInfo {
        code: "#CIRC!",
        title: "Circular reference",
        explanation: "The formula ends up reading its own result, directly or through other cells, so it has no value to settle on.",
        causes: &["The formula refers to its own cell (a total that includes itself).", "Two or more formulas refer to one another in a loop."],
        fixes: &[
            "Change one formula in the loop so it no longer reads the others.",
            "If the loop is intended, turn on iterative calculation (File ▸ Options ▸ Formulas).",
        ],
    },
];

/// What an error value (`"#DIV/0!"`, any case) means; `None` for text that is not an error code.
pub fn error_info(code: &str) -> Option<&'static ErrorInfo> {
    ERRORS.iter().find(|e| e.code.eq_ignore_ascii_case(code.trim()))
}

fn error_title(e: CellError) -> &'static str {
    error_info(e.as_str()).map_or("Error", |i| i.title)
}

// ---------------------------------------------------------------- formula helpers

/// The formula's syntax tree: the one stored with it, or parsed again from its text (a formula
/// read from a file keeps only its text). `None` when the text is not a formula.
fn parsed(f: &Formula) -> Option<Arc<Expr>> {
    f.expr.clone().or_else(|| gridcraft_formula::parse(&f.text).ok().map(Arc::new))
}

/// `Sheet1!A1`, with the sheet name quoted when it needs it.
fn place(sh: &Sheet, c: CellRef) -> String {
    format!("{}!{}", gridcraft_formula::quote_sheet(&sh.name), c.a1())
}

/// A value as a message shows it.
fn show(v: &Value) -> String {
    match v {
        Value::Empty => "nothing (empty)".into(),
        Value::Number(n) => format!("{n}"),
        Value::Text(t) => {
            let short: String = t.chars().take(60).collect();
            if short.len() < t.len() { format!("\"{short}…\"") } else { format!("\"{short}\"") }
        }
        Value::Bool(b) => (if *b { "TRUE" } else { "FALSE" }).into(),
        Value::Error(e) => e.as_str().into(),
        Value::Array(a) => format!("an array of {} x {}", a.rows, a.cols),
    }
}

fn bare(mut e: &Expr) -> &Expr {
    while let Expr::Paren(x) = e {
        e = x;
    }
    e
}

/// The sheet a reference reads, seen from a formula on `host`.
fn ref_sheet(wb: &Workbook, host: usize, r: &Reference) -> Option<usize> {
    match &r.sheet {
        SheetSel::Current => Some(host),
        SheetSel::Named(n) => wb.sheet_index(n),
        SheetSel::Span(..) => None,
    }
}

/// The single cell a reference names (`B1`, not `B1:B3`) and the sheet it is on.
fn single_cell(wb: &Workbook, host: usize, e: &Expr) -> Option<(usize, CellRef, String)> {
    let Expr::Ref(r) = bare(e) else { return None };
    let RefKind::Cell(a) = &r.kind else { return None };
    Some((ref_sheet(wb, host, r)?, a.cell(), gridcraft_formula::printer::reference_a1(r)))
}

fn cell_value(wb: &Workbook, sheet: usize, c: CellRef) -> Value {
    wb.sheet(sheet).map(|sh| sh.value(c)).unwrap_or_default()
}

/// Character positions (0-based, in `text`) where `#REF!` stands outside string literals.
fn ref_error_positions(text: &str) -> Vec<usize> {
    let ch: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let mut in_string = false;
    while let Some(&c) = ch.get(i) {
        if c == '"' {
            in_string = !in_string;
        } else if !in_string && c == '#' {
            let word: String = ch.iter().skip(i).take(5).collect();
            if word.eq_ignore_ascii_case("#REF!") {
                out.push(i);
                i += 5;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Whether `e` holds the `#REF!` literal outside any function call it contains.
fn has_ref_literal_here(e: &Expr) -> bool {
    let mut pending = vec![e];
    while let Some(x) = pending.pop() {
        match x {
            Expr::Error(CellError::Ref) => return true,
            Expr::Unary(_, a) | Expr::Paren(a) => pending.push(a),
            Expr::Binary(_, a, b) => {
                pending.push(a);
                pending.push(b);
            }
            Expr::Array(rows) => pending.extend(rows.iter().flatten()),
            _ => {}
        }
    }
    false
}

/// The workbooks a formula's text refers to: `[Book.xlsx]Sheet1!A1`, `'C:\dir\[Book.xlsx]Sheet 1'!A1`,
/// and the numbered links (`[1]Sheet1!A1`) a file stores. Text in double quotes is skipped, and a
/// structured reference (`Table1[Col]`, `[@Col]`) is not a workbook.
pub fn external_books(text: &str) -> Vec<String> {
    let ch: Vec<char> = text.chars().collect();
    let mut out: Vec<String> = Vec::new();
    let mut add = |b: String| {
        if !b.is_empty() && !out.contains(&b) {
            out.push(b);
        }
    };
    let ident = |c: char| c.is_alphanumeric() || matches!(c, '_' | '.' | '\\');
    let mut i = 0;
    while let Some(&c) = ch.get(i) {
        match c {
            '"' => {
                i += 1;
                while let Some(&x) = ch.get(i) {
                    i += 1;
                    if x == '"' {
                        if ch.get(i) == Some(&'"') {
                            i += 1;
                        } else {
                            break;
                        }
                    }
                }
            }
            '\'' => {
                // A quoted sheet name, `''` standing for one quote.
                i += 1;
                let mut name = String::new();
                while let Some(&x) = ch.get(i) {
                    i += 1;
                    if x == '\'' {
                        if ch.get(i) == Some(&'\'') {
                            name.push('\'');
                            i += 1;
                        } else {
                            break;
                        }
                    } else {
                        name.push(x);
                    }
                }
                if ch.get(i) == Some(&'!')
                    && let Some(book) = bracketed(&name)
                {
                    add(book);
                }
            }
            '[' => {
                let after_word = i.checked_sub(1).and_then(|p| ch.get(p)).is_some_and(|p| ident(*p) || *p == ']');
                // The balanced bracket group (a structured reference nests brackets; `'` escapes).
                let mut depth = 0usize;
                let mut inner = String::new();
                while let Some(&x) = ch.get(i) {
                    i += 1;
                    match x {
                        '\'' => {
                            if let Some(&n) = ch.get(i) {
                                inner.push(n);
                                i += 1;
                            }
                        }
                        '[' => {
                            depth += 1;
                            if depth > 1 {
                                inner.push(x);
                            }
                        }
                        ']' => {
                            depth = depth.saturating_sub(1);
                            if depth == 0 {
                                break;
                            }
                            inner.push(x);
                        }
                        _ => inner.push(x),
                    }
                }
                if after_word {
                    continue;
                }
                // `[Book.xlsx]Sheet1!` or `[1]Sheet1!`: a sheet name and `!` follow the bracket.
                let mut j = i;
                while ch.get(j).is_some_and(|x| ident(*x)) {
                    j += 1;
                }
                if ch.get(j) == Some(&'!') {
                    add(inner);
                }
                i = j;
            }
            _ => i += 1,
        }
    }
    out
}

/// The workbook name between the brackets of `C:\dir\[Book.xlsx]Sheet1`.
fn bracketed(name: &str) -> Option<String> {
    let open = name.find('[')?;
    let rest = name.get(open + 1..)?;
    let close = rest.find(']')?;
    rest.get(..close).map(str::to_string)
}

/// The workbooks a parsed formula names in a sheet-qualified name (`Book.xlsx!Rate`).
fn external_name_books(e: &Expr) -> Vec<String> {
    let mut out = Vec::new();
    e.walk(&mut |x| {
        if let Expr::Name(n) = x
            && let Some((book, _)) = n.split_once('!')
        {
            let book = gridcraft_core::unquote_sheet(book);
            let lower = book.to_ascii_lowercase();
            if [".xlsx", ".xlsm", ".xlsb", ".xls"].iter().any(|ext| lower.ends_with(ext)) && !out.contains(&book) {
                out.push(book);
            }
        }
    });
    out
}

fn name_defined(wb: &Workbook, sheet: usize, n: &str) -> bool {
    match n.split_once('!') {
        Some((s, b)) => wb.sheet_index(&gridcraft_core::unquote_sheet(s)).is_some_and(|si| wb.name(b, si).is_some()),
        None => wb.name(n, sheet).is_some(),
    }
}

/// Function names and names the workbook doesn't know, in the order the formula uses them.
fn unknown_names(wb: &Workbook, sheet: usize, e: &Expr) -> (Vec<String>, Vec<String>) {
    // Names a LET or LAMBDA in the formula binds.
    let mut bound: Vec<&str> = Vec::new();
    e.walk(&mut |x| {
        if let Expr::Call(n, args) = x {
            let last = args.len().saturating_sub(1);
            for (i, a) in args.iter().enumerate() {
                let binds = match n.as_str() {
                    "LET" => i % 2 == 0 && i < last,
                    "LAMBDA" => i < last,
                    _ => false,
                };
                if binds && let Expr::Name(b) = a {
                    bound.push(b.as_str());
                }
            }
        }
    });
    let is_bound = |n: &str| bound.iter().any(|b| gridcraft_formula::same_name(b, n));
    let (mut funcs, mut names) = (Vec::new(), Vec::new());
    e.walk(&mut |x| match x {
        Expr::Call(n, _) if !gridcraft_calc::is_known_function(n) && !name_defined(wb, sheet, n) && !is_bound(n) => {
            if !funcs.contains(n) {
                funcs.push(n.clone());
            }
        }
        Expr::Name(n) => {
            let table = n.split_once('!').map_or(n.as_str(), |(_, b)| b);
            if !is_bound(n) && !name_defined(wb, sheet, n) && wb.table(table).is_none() && !names.contains(n) {
                names.push(n.clone());
            }
        }
        _ => {}
    });
    (funcs, names)
}

/// A stable fingerprint of a formula's R1C1 shape: two formulas that were filled from one
/// another have the same one.
fn shape(f: &Formula, at: CellRef) -> u64 {
    let mut h = DefaultHasher::new();
    match parsed(f) {
        Some(e) => gridcraft_formula::print_r1c1(&e, at).hash(&mut h),
        None => {
            // Unparseable text has no relative shape; only identical text matches.
            '?'.hash(&mut h);
            f.text.hash(&mut h);
        }
    }
    h.finish()
}

// ---------------------------------------------------------------- formulas.explainError

fn explain_error(s: &mut Session, p: &Json) -> Result<Json> {
    let d = s.doc()?;
    let sheet = target_sheet(s, p)?;
    let at = cell_param_on(s, p, "cell", sheet)?.unwrap_or(d.selection.active);
    let wb = &d.wb;
    let Some(sh) = wb.sheet(sheet) else { return Err(bad("formulas.explainError", "no such sheet")) };
    let shown = sh.value(at);
    let Value::Error(err) = shown else {
        return Ok(json!({"cell": at.a1(), "sheet": sh.name, "error": null, "value": super::super::inspect::value_json(&shown)}));
    };
    let mut evidence: Vec<String> = Vec::new();
    // The formula that produced the value: the cell's own, or the array formula it spills from.
    let mut host = at;
    let mut formula = sh.cell(at).and_then(|c| c.formula.as_ref());
    if formula.is_none()
        && let Some(anchor) = sh.spill_anchor(at)
    {
        evidence.push(format!("This value is part of the array the formula in {} spills; the evidence below is about that formula.", anchor.a1()));
        host = anchor;
        formula = sh.cell(anchor).and_then(|c| c.formula.as_ref());
    }
    let text = formula.map(|f| format!("={}", f.text));
    match formula {
        None => evidence.push(format!("The cell holds {} as a typed value; no formula produced it.", err.as_str())),
        Some(f) => {
            let expr = parsed(f);
            evidence.extend(formula_evidence(wb, &d.calc, sheet, host, f, expr.as_deref(), err));
        }
    }
    // The cells the formula reads, and the errors already showing in them.
    let mut precedents = Vec::new();
    let mut budget = MAX_PRECEDENT_CELLS;
    let mut inherited: Vec<String> = Vec::new();
    for a in d.calc.graph.precedents_of((sheet, host)).iter().take(50) {
        let Some(psh) = wb.sheet(a.sheet) else { continue };
        let name = format!("{}!{}", gridcraft_formula::quote_sheet(&psh.name), a.range.a1());
        let mut errors = Vec::new();
        for (c, cell) in psh.cells.iter_range(a.range).take(budget) {
            budget = budget.saturating_sub(1);
            if (a.sheet, c) == (sheet, host) {
                continue;
            }
            if let Value::Error(e) = &cell.value {
                if errors.len() < 5 {
                    errors.push(json!({"cell": c.a1(), "error": e.as_str()}));
                }
                if inherited.len() < 5 {
                    inherited.push(format!(
                        "{} already shows {}; this formula passes the error on, so fix that cell first.",
                        place(psh, c),
                        e.as_str()
                    ));
                }
            }
        }
        let mut entry = json!({"ref": name, "errors": errors});
        if a.range.start == a.range.end {
            entry["value"] = super::super::inspect::value_json(&psh.value(a.range.start));
        }
        precedents.push(entry);
    }
    evidence.extend(inherited);
    evidence.truncate(MAX_EVIDENCE);
    let info = error_info(err.as_str());
    Ok(json!({
        "cell": at.a1(),
        "sheet": sh.name,
        "formula": text,
        "error": err.as_str(),
        "title": info.map_or("Error", |i| i.title),
        "explanation": info.map_or("", |i| i.explanation),
        "evidence": evidence,
        "likelyCauses": info.map_or(&[][..], |i| i.causes),
        "fixes": info.map_or(&[][..], |i| i.fixes),
        "precedents": precedents,
    }))
}

/// What the formula itself shows about why it gives `err`.
fn formula_evidence(
    wb: &Workbook,
    calc: &gridcraft_calc::Calc,
    sheet: usize,
    host: CellRef,
    f: &Formula,
    expr: Option<&Expr>,
    err: CellError,
) -> Vec<String> {
    let mut out = Vec::new();
    let shown = format!("={}", f.text);
    let Some(expr) = expr else {
        let why =
            gridcraft_formula::parse(&f.text).err().map(|e| format!(" ({} at character {})", e.msg, e.pos.saturating_add(2))).unwrap_or_default();
        out.push(format!("The text {shown} could not be read as a formula{why}, so it gives #NAME?."));
        return out;
    };
    match err {
        CellError::Ref => ref_evidence(wb, &shown, f, expr, &mut out),
        CellError::Name => {
            let (funcs, names) = unknown_names(wb, sheet, expr);
            for n in funcs {
                out.push(format!("{n} is not a function this program knows, and no defined name is called {n}."));
            }
            for n in names {
                out.push(format!("{n} is not a defined name, a table or a LET/LAMBDA name; if it is meant as text, put it in double quotes."));
            }
        }
        CellError::Div0 => div0_evidence(wb, sheet, expr, &mut out),
        CellError::Value => value_evidence(wb, sheet, expr, &mut out),
        CellError::NA => na_evidence(wb, sheet, host, expr, &mut out),
        CellError::Num => num_evidence(wb, sheet, host, expr, &mut out),
        CellError::Null => expr.walk(&mut |e| {
            if let Expr::Binary(BinOp::Intersect, a, b) = e {
                out.push(format!(
                    "The space between {} and {} asks for the cells both ranges share, and they share none.",
                    gridcraft_formula::print(a),
                    gridcraft_formula::print(b)
                ));
            }
        }),
        CellError::Spill => spill_evidence(wb, sheet, host, expr, &mut out),
        CellError::Calc => expr.walk(&mut |e| match e {
            Expr::Call(n, args) if n == "FILTER" && args.len() < 3 => {
                out.push("FILTER found no rows that match and has no third argument saying what to show instead.".into());
            }
            Expr::Call(n, _) if n == "LAMBDA" && std::ptr::eq(e, bare(expr)) => {
                out.push("The whole formula is a LAMBDA that is never called with values.".into());
            }
            _ => {}
        }),
        CellError::Circ => {
            let loop_cells: Vec<String> = calc.circular.iter().filter_map(|(si, c)| wb.sheet(*si).map(|s| place(s, *c))).take(10).collect();
            out.push(format!("This cell is in a circular reference. Cells in circular references: {}.", loop_cells.join(", ")));
        }
        CellError::GettingData => {}
    }
    if calc.circular.contains(&(sheet, host)) && err != CellError::Circ {
        out.push("This cell is also part of a circular reference.".into());
    }
    out
}

fn ref_evidence(wb: &Workbook, shown: &str, f: &Formula, expr: &Expr, out: &mut Vec<String>) {
    let positions: Vec<String> = ref_error_positions(&f.text).iter().map(|p| (p + 2).to_string()).collect();
    if !positions.is_empty() {
        let word = if positions.len() == 1 { "character" } else { "characters" };
        out.push(format!(
            "The formula {shown} contains #REF! where a reference used to be, at {word} {}: the cells it pointed at were deleted or moved off the sheet.",
            positions.join(", ")
        ));
        let mut places = Vec::new();
        expr.walk(&mut |e| {
            if let Expr::Call(n, args) = e {
                for (i, a) in args.iter().enumerate() {
                    if has_ref_literal_here(a) {
                        places.push(format!("argument {} of {n}", i + 1));
                    }
                }
            }
        });
        if places.is_empty() && has_ref_literal_here(expr) {
            places.push("the formula's own expression, outside any function".into());
        }
        if !places.is_empty() {
            out.push(format!("#REF! stands as {}.", places.join(", ")));
        }
    }
    let mut missing: Vec<String> = Vec::new();
    for r in expr.references() {
        if let SheetSel::Named(n) = &r.sheet
            && wb.sheet_index(n).is_none()
            && !missing.contains(n)
        {
            missing.push(n.clone());
        }
    }
    for n in missing {
        match bracketed(&n) {
            Some(book) => out.push(format!("The formula refers to the workbook {book}, which is not open here, so its cells can't be read.")),
            None => out.push(format!("The formula refers to the sheet {}, which this workbook does not have.", gridcraft_formula::quote_sheet(&n))),
        }
    }
    let builders: Vec<&str> = expr.functions().into_iter().filter(|n| matches!(*n, "OFFSET" | "INDIRECT" | "INDEX")).collect();
    if let Some(first) = builders.first() {
        out.push(format!(
            "The formula uses {first}, which gives #REF! when the reference it builds falls outside the sheet or names something that does not exist."
        ));
    }
}

fn div0_evidence(wb: &Workbook, sheet: usize, expr: &Expr, out: &mut Vec<String>) {
    let divisor = |what: String, e: &Expr, out: &mut Vec<String>| {
        if let Expr::Number(n) = bare(e)
            && *n == 0.0
        {
            out.push(format!("{what} is the constant 0."));
            return;
        }
        let Some((si, c, name)) = single_cell(wb, sheet, e) else { return };
        match cell_value(wb, si, c) {
            Value::Empty => out.push(format!("{what} is {name}, which is empty (an empty cell counts as 0).")),
            Value::Number(0.0) => out.push(format!("{what} is {name}, which holds 0.")),
            Value::Bool(false) => out.push(format!("{what} is {name}, which holds FALSE (counted as 0).")),
            _ => {}
        }
    };
    expr.walk(&mut |e| match e {
        Expr::Binary(BinOp::Div, _, b) => divisor(format!("The divisor of {}", gridcraft_formula::print(e)), b, out),
        Expr::Call(n, args) if n == "MOD" || n == "QUOTIENT" => {
            if let Some(b) = args.get(1) {
                divisor(format!("The divisor of {n}"), b, out);
            }
        }
        Expr::Call(n, args) if n == "AVERAGE" && !args.is_empty() => {
            // Plain references with no number in them leave nothing to average.
            let mut any_number = false;
            for a in args {
                let Expr::Ref(r) = bare(a) else { return };
                let Some(si) = ref_sheet(wb, sheet, r) else { return };
                let Some(sh) = wb.sheet(si) else { return };
                if sh.cells.iter_range(r.range()).take(MAX_PRECEDENT_CELLS).any(|(_, c)| matches!(c.value, Value::Number(_))) {
                    any_number = true;
                }
            }
            if !any_number {
                out.push(format!("{} finds no numbers to average.", gridcraft_formula::print(e)));
            }
        }
        _ => {}
    });
}

fn value_evidence(wb: &Workbook, sheet: usize, expr: &Expr, out: &mut Vec<String>) {
    let mut seen: HashSet<String> = HashSet::new();
    expr.walk(&mut |e| {
        let Expr::Binary(op, a, b) = e else { return };
        if !matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Pow) {
            return;
        }
        for side in [a, b] {
            let Some((si, c, name)) = single_cell(wb, sheet, side) else { continue };
            let Value::Text(t) = cell_value(wb, si, c) else { continue };
            if gridcraft_core::parse::number_stored_as_text(&t).is_some() || !seen.insert(name.clone()) {
                continue;
            }
            if t.trim().is_empty() {
                out.push(format!("{name} looks empty but holds {} space character(s); arithmetic on it gives #VALUE!.", t.chars().count()));
            } else {
                out.push(format!(
                    "{name} holds the text {}, which does not read as a number, and the formula does arithmetic with it.",
                    show(&Value::Text(t))
                ));
            }
        }
    });
}

fn na_evidence(wb: &Workbook, sheet: usize, host: CellRef, expr: &Expr, out: &mut Vec<String>) {
    expr.walk(&mut |e| {
        let Expr::Call(n, args) = e else { return };
        match n.as_str() {
            "NA" => out.push("The formula calls NA(), which gives #N/A on purpose.".into()),
            "VLOOKUP" | "HLOOKUP" | "XLOOKUP" | "MATCH" | "XMATCH" | "LOOKUP" => {
                let (Some(key), Some(table)) = (args.first(), args.get(1)) else { return };
                let v = gridcraft_calc::recalc::evaluate_expr(wb, sheet, host, key);
                let what = gridcraft_formula::print(key);
                let looked = match &v {
                    Value::Error(x) => format!("The value {n} looks for ({what}) is itself {}, so nothing can match.", x.as_str()),
                    Value::Array(_) => return,
                    v => format!("{n} looks for {} (from {what}) in {} and does not find it.", show(v), gridcraft_formula::print(table)),
                };
                out.push(looked);
                if let Value::Text(t) = &v
                    && t.trim() != t.as_ref()
                {
                    out.push(format!("The value looked for has spaces at its start or end: {}.", show(&v)));
                }
            }
            _ => {}
        }
    });
}

fn num_evidence(wb: &Workbook, sheet: usize, host: CellRef, expr: &Expr, out: &mut Vec<String>) {
    expr.walk(&mut |e| {
        let Expr::Call(n, args) = e else { return };
        let first = || args.first().map(|a| gridcraft_calc::recalc::evaluate_expr(wb, sheet, host, a));
        match n.as_str() {
            "SQRT" => {
                if let Some(Value::Number(x)) = first()
                    && x < 0.0
                {
                    out.push(format!("SQRT is given {x}, a negative number."));
                }
            }
            "LN" | "LOG" | "LOG10" => {
                if let Some(Value::Number(x)) = first()
                    && x <= 0.0
                {
                    out.push(format!("{n} is given {x}; logarithms need a number above 0."));
                }
            }
            "IRR" | "XIRR" | "RATE" | "MIRR" => {
                out.push(format!("{n} searches for its result step by step; it gives #NUM! when no result is found."));
            }
            _ => {}
        }
    });
}

fn spill_evidence(wb: &Workbook, sheet: usize, host: CellRef, expr: &Expr, out: &mut Vec<String>) {
    let Some(sh) = wb.sheet(sheet) else { return };
    for t in sh.tables.iter().filter(|t| t.range.contains(host)) {
        out.push(format!("The formula is inside the table {}, where results cannot spill.", t.name));
    }
    let Value::Array(a) = gridcraft_calc::recalc::evaluate_expr(wb, sheet, host, expr) else {
        out.push("Evaluated now, the formula returns a single value, so nothing in its way was found.".into());
        return;
    };
    let (rows, cols) = (a.rows, a.cols);
    let end_row = u64::from(host.row) + (rows as u64).saturating_sub(1);
    let end_col = u64::from(host.col) + (cols as u64).saturating_sub(1);
    if end_row >= u64::from(MAX_ROWS) || end_col >= u64::from(MAX_COLS) {
        out.push(format!("The result has {rows} row(s) and {cols} column(s); starting at {} it would run past the edge of the sheet.", host.a1()));
        return;
    }
    let range = RangeRef::new(host, CellRef::new(end_row as u32, end_col as u32));
    out.push(format!("The result needs {} ({rows} row(s) by {cols} column(s)).", range.a1()));
    for (c, cell) in sh.cells.iter_range(range).filter(|(c, cell)| *c != host && (!cell.value.is_empty() || cell.formula.is_some())).take(10) {
        match &cell.formula {
            Some(f) => out.push(format!("{} is in the way: it holds the formula ={}.", c.a1(), f.text)),
            None => out.push(format!("{} is in the way: it holds {}.", c.a1(), show(&cell.value))),
        }
    }
    for (anchor, r) in sh.spill_ranges.iter().filter(|(anchor, r)| **anchor != host && r.intersects(&range)).take(5) {
        out.push(format!("The result of the formula in {} already spills into {}, which overlaps.", anchor.a1(), r.a1()));
    }
    for m in sh.merges.iter().filter(|m| m.intersects(&range)).take(5) {
        out.push(format!("The merged cells {} are in the way.", m.a1()));
    }
}

// ---------------------------------------------------------------- formulas.diagnostics

/// The issues found so far: all are counted, the first `limit` are kept.
struct Found {
    issues: Vec<Json>,
    counts: [usize; 6],
    limit: usize,
}

impl Found {
    fn add(&mut self, check: usize, make: impl FnOnce() -> (String, Json), cell: String) {
        if let Some(n) = self.counts.get_mut(check) {
            *n += 1;
        }
        if self.issues.len() < self.limit {
            let (message, detail) = make();
            self.issues.push(json!({"check": CHECKS.get(check).copied().unwrap_or(""), "cell": cell, "message": message, "detail": detail}));
        }
    }
}

/// The two neighbours of `c` on one axis, when both hold formulas of the same shape.
fn agreeing(shapes: &HashMap<CellRef, u64>, c: CellRef, vertical: bool) -> Option<(CellRef, CellRef, u64)> {
    let (a, b) = if vertical {
        (CellRef::new(c.row.checked_sub(1)?, c.col), CellRef::new(c.row.checked_add(1)?, c.col))
    } else {
        (CellRef::new(c.row, c.col.checked_sub(1)?), CellRef::new(c.row, c.col.checked_add(1)?))
    };
    let (sa, sb) = (shapes.get(&a)?, shapes.get(&b)?);
    (sa == sb).then_some((a, b, *sa))
}

/// The neighbour's formula as it would read filled into `c`.
fn expected_formula(sh: &Sheet, from: CellRef, c: CellRef) -> Option<String> {
    let f = sh.cell(from)?.formula.as_ref()?;
    let e = parsed(f)?;
    let shifted =
        gridcraft_formula::adjust::shift_relative((*e).clone(), i64::from(c.row) - i64::from(from.row), i64::from(c.col) - i64::from(from.col));
    Some(format!("={}", gridcraft_formula::print(&shifted)))
}

fn neighbour_detail(sh: &Sheet, c: CellRef, a: CellRef, b: CellRef) -> Json {
    json!({
        "neighbours": [a.a1(), b.a1()],
        "neighbourFormula": sh.cell(a).and_then(|x| x.formula.as_ref()).map(|f| format!("={}", f.text)),
        "expected": expected_formula(sh, a, c),
    })
}

fn diagnostics(s: &mut Session, p: &Json) -> Result<Json> {
    const CMD: &str = "formulas.diagnostics";
    let d = s.doc()?;
    let wb = &d.wb;
    let sheets: Vec<usize> = match p.get("sheet") {
        None | Some(Json::Null) => (0..wb.sheets.len()).collect(),
        Some(Json::String(n)) if n.eq_ignore_ascii_case("all") => (0..wb.sheets.len()).collect(),
        Some(_) => vec![target_sheet(s, p)?],
    };
    let mut on = [false; 6];
    let names: Vec<&Json> = match p.get("checks") {
        None | Some(Json::Null) => Vec::new(),
        Some(Json::Array(a)) => a.iter().collect(),
        Some(v @ Json::String(_)) => vec![v],
        Some(_) => return Err(bad(CMD, "checks: a list of check names")),
    };
    for v in &names {
        let Some(name) = v.as_str() else { return Err(bad(CMD, "checks: a list of check names")) };
        let Some(i) = CHECKS.iter().position(|c| c.eq_ignore_ascii_case(name)) else {
            return Err(bad(CMD, format!("unknown check `{name}`; the checks are {}", CHECKS.join(", "))));
        };
        if let Some(x) = on.get_mut(i) {
            *x = true;
        }
    }
    if names.is_empty() {
        on = [true; 6];
    }
    let limit = match p.get("limit") {
        None | Some(Json::Null) => DEFAULT_LIMIT,
        Some(v) => match v.as_f64() {
            Some(x) if x.is_finite() && x >= 0.0 => (x as u64).min(MAX_LIMIT),
            _ => return Err(bad(CMD, "limit: a whole number from 0 to 20000")),
        },
    };
    let has = |i: usize| on.get(i).copied().unwrap_or(false);
    let circular: HashSet<(usize, CellRef)> = d.calc.circular.iter().copied().collect();
    let mut found = Found { issues: Vec::new(), counts: [0; 6], limit: limit as usize };
    let mut budget = MAX_FORMULAS;
    let mut scanned = 0usize;
    let mut capped = false;
    let need_shapes = has(INCONSISTENT) || has(HARDCODED);
    for &si in &sheets {
        let Some(sh) = wb.sheet(si) else { continue };
        if capped {
            break;
        }
        // First pass: the formula shapes, the legacy array areas, and where the budget runs out.
        let mut shapes: HashMap<CellRef, u64> = HashMap::new();
        let mut arrays: Vec<RangeRef> = Vec::new();
        let mut stop_after: Option<CellRef> = None;
        let mut last: Option<CellRef> = None;
        for (c, cell) in sh.cells.iter() {
            let Some(f) = &cell.formula else { continue };
            if budget == 0 {
                capped = true;
                stop_after = last;
                break;
            }
            budget -= 1;
            scanned += 1;
            last = Some(c);
            if let Some(r) = f.array {
                arrays.push(r);
            }
            if need_shapes {
                shapes.insert(c, shape(f, c));
            }
        }
        if capped && stop_after.is_none() {
            break;
        }
        // Second pass, row by row: each cell's issues in check order.
        for (c, cell) in sh.cells.iter() {
            if stop_after.is_some_and(|l| (c.row, c.col) > (l.row, l.col)) {
                break;
            }
            let Some(f) = &cell.formula else {
                if has(HARDCODED) && !cell.value.is_empty() && !arrays.iter().any(|r| r.contains(c)) {
                    let hit = agreeing(&shapes, c, true).or_else(|| agreeing(&shapes, c, false));
                    if let Some((a, b, _)) = hit {
                        found.add(
                            HARDCODED,
                            || {
                                let mut detail = neighbour_detail(sh, c, a, b);
                                detail["value"] = super::super::inspect::value_json(&cell.value);
                                (
                                    format!("Holds the constant {} between {} and {}, whose formulas follow one pattern; a formula may have been typed over.", show(&cell.value), a.a1(), b.a1()),
                                    detail,
                                )
                            },
                            place(sh, c),
                        );
                    }
                }
                continue;
            };
            let text = format!("={}", f.text);
            if has(ERROR)
                && let Value::Error(e) = &cell.value
            {
                let e = *e;
                found.add(ERROR, || (format!("{}: {}", e.as_str(), error_title(e)), json!({"error": e.as_str(), "formula": &text})), place(sh, c));
            }
            if has(BROKEN) {
                let at = ref_error_positions(&f.text);
                if !at.is_empty() {
                    found.add(
                        BROKEN,
                        || {
                            let chars: Vec<usize> = at.iter().map(|p| p + 2).collect();
                            ("The formula contains #REF! where a reference used to be.".to_string(), json!({"formula": &text, "characters": chars}))
                        },
                        place(sh, c),
                    );
                }
            }
            if has(INCONSISTENT)
                && let Some(own) = shapes.get(&c)
            {
                let hit = agreeing(&shapes, c, true).filter(|h| h.2 != *own).or_else(|| agreeing(&shapes, c, false).filter(|h| h.2 != *own));
                if let Some((a, b, _)) = hit {
                    found.add(
                        INCONSISTENT,
                        || {
                            let mut detail = neighbour_detail(sh, c, a, b);
                            detail["formula"] = json!(&text);
                            (format!("The formula differs from the pattern {} and {} share.", a.a1(), b.a1()), detail)
                        },
                        place(sh, c),
                    );
                }
            }
            if has(CIRCULAR) && (circular.contains(&(si, c)) || cell.value == Value::Error(CellError::Circ)) {
                found.add(CIRCULAR, || ("The cell is part of a circular reference.".to_string(), json!({"formula": &text})), place(sh, c));
            }
            if has(EXTERNAL) {
                let mut books = external_books(&f.text);
                if let Some(e) = parsed(f) {
                    for b in external_name_books(&e) {
                        if !books.contains(&b) {
                            books.push(b);
                        }
                    }
                }
                if !books.is_empty() {
                    found.add(
                        EXTERNAL,
                        || (format!("The formula refers to another workbook: {}.", books.join(", ")), json!({"formula": &text, "workbooks": books})),
                        place(sh, c),
                    );
                }
            }
        }
    }
    let total: usize = found.counts.iter().sum();
    let counts: serde_json::Map<String, Json> =
        CHECKS.iter().enumerate().filter(|(i, _)| has(*i)).map(|(i, c)| (c.to_string(), json!(found.counts.get(i).copied().unwrap_or(0)))).collect();
    let mut out = json!({
        "issues": found.issues,
        "counts": counts,
        "total": total,
        "truncated": total > found.limit,
        "scanned": {"sheets": sheets.len(), "formulas": scanned},
        "capped": capped,
    });
    if capped {
        out["note"] = json!(format!("Stopped after {MAX_FORMULAS} formula cells; the cells after the last one listed were not checked."));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use serde_json::{Value as Json, json};

    use crate::Session;

    fn book() -> Session {
        let mut s = Session::new();
        s.new_workbook();
        s
    }

    fn set(s: &mut Session, cells: &[(&str, &str)]) {
        for (cell, input) in cells {
            s.execute("cell.set", json!({"cell": cell, "input": input})).unwrap();
        }
    }

    fn explain(s: &mut Session, cell: &str) -> Json {
        s.execute("formulas.explainError", json!({"cell": cell})).unwrap()
    }

    fn evidence(r: &Json) -> String {
        r["evidence"].as_array().unwrap().iter().filter_map(Json::as_str).collect::<Vec<_>>().join(" | ")
    }

    fn issues(s: &mut Session, params: Json) -> Vec<(String, String)> {
        let r = s.execute("formulas.diagnostics", params).unwrap();
        r["issues"].as_array().unwrap().iter().map(|i| (i["check"].as_str().unwrap().to_string(), i["cell"].as_str().unwrap().to_string())).collect()
    }

    fn only(s: &mut Session, check: &str) -> Vec<String> {
        issues(s, json!({"checks": [check]})).into_iter().map(|(_, c)| c).collect()
    }

    #[test]
    fn every_error_code_has_an_explanation() {
        for code in [
            "#REF!",
            "#NAME?",
            "#VALUE!",
            "#DIV/0!",
            "#N/A",
            "#NUM!",
            "#NULL!",
            "#SPILL!",
            "#CALC!",
            "#FIELD!",
            "#BLOCKED!",
            "#CONNECT!",
            "#BUSY!",
            "#UNKNOWN!",
            "#GETTING_DATA",
            "#CIRC!",
        ] {
            let i = super::error_info(code).unwrap_or_else(|| panic!("{code}"));
            assert!(!i.title.is_empty() && !i.explanation.is_empty() && !i.causes.is_empty() && !i.fixes.is_empty(), "{code}");
        }
        assert!(super::error_info("#div/0!").is_some(), "any case");
        assert!(super::error_info("hello").is_none());
        // Every error the engine can produce is covered.
        for e in gridcraft_core::CellError::ALL.iter().chain([gridcraft_core::CellError::Circ].iter()) {
            assert!(super::error_info(e.as_str()).is_some(), "{}", e.as_str());
        }
    }

    #[test]
    fn div0_names_the_empty_or_zero_divisor() {
        let mut s = book();
        set(&mut s, &[("A1", "=1/0"), ("B1", "5"), ("C1", "=B1/D1"), ("E1", "0"), ("F1", "=MOD(B1,E1)"), ("G1", "=AVERAGE(H1:H3)")]);
        let r = explain(&mut s, "A1");
        assert_eq!(r["error"], "#DIV/0!");
        assert_eq!(r["title"], "Division by zero");
        assert!(evidence(&r).contains("the constant 0"), "{r}");
        assert!(!r["likelyCauses"].as_array().unwrap().is_empty() && !r["fixes"].as_array().unwrap().is_empty());
        let r = explain(&mut s, "C1");
        assert!(evidence(&r).contains("D1, which is empty"), "{r}");
        assert!(r["precedents"].as_array().unwrap().iter().any(|p| p["ref"].as_str().unwrap().ends_with("D1")), "{r}");
        let r = explain(&mut s, "F1");
        assert!(evidence(&r).contains("The divisor of MOD is E1, which holds 0"), "{r}");
        let r = explain(&mut s, "G1");
        assert!(evidence(&r).contains("AVERAGE(H1:H3) finds no numbers"), "{r}");
    }

    #[test]
    fn ref_names_where_the_reference_was() {
        let mut s = book();
        set(&mut s, &[("A1", "=SUM(#REF!,2)"), ("B1", "3"), ("C1", "=B1*2")]);
        let r = explain(&mut s, "A1");
        assert_eq!(r["error"], "#REF!");
        let ev = evidence(&r);
        assert!(ev.contains("contains #REF! where a reference used to be, at character 6"), "{ev}");
        assert!(ev.contains("argument 1 of SUM"), "{ev}");
        // Deleting the column a formula reads leaves #REF! in its text.
        s.execute("home.deleteColumns", json!({"cols": "B"})).unwrap();
        let r = explain(&mut s, "B1");
        assert_eq!(r["error"], "#REF!", "{r}");
        assert!(evidence(&r).contains("#REF! stands as the formula's own expression"), "{r}");
    }

    #[test]
    fn name_lists_the_unknown_function_and_name() {
        let mut s = book();
        set(&mut s, &[("A1", "=SUMM(1,2)"), ("A2", "=rate*2"), ("A3", "=LET(x,2,x*3)")]);
        let r = explain(&mut s, "A1");
        assert_eq!(r["error"], "#NAME?");
        assert!(evidence(&r).contains("SUMM is not a function"), "{r}");
        let r = explain(&mut s, "A2");
        assert!(evidence(&r).contains("rate is not a defined name"), "{r}");
        // A LET name is known; the cell has no error at all.
        assert_eq!(explain(&mut s, "A3")["error"], Json::Null);
    }

    #[test]
    fn value_na_num_null_calc_and_circ_carry_their_evidence() {
        let mut s = book();
        set(
            &mut s,
            &[
                ("A1", "abc"),
                ("B1", "=A1+1"),
                ("A2", "apple"),
                ("A3", "pear"),
                ("B2", "=MATCH(\"kiwi\",A2:A3,0)"),
                ("B3", "=SQRT(-4)"),
                ("B4", "=A1:A2 C1:C2"),
                ("B5", "=FILTER(A2:A3,A2:A3=\"plum\")"),
                ("B6", "=NA()"),
            ],
        );
        let r = explain(&mut s, "B1");
        assert_eq!(r["error"], "#VALUE!");
        assert!(evidence(&r).contains("A1 holds the text \"abc\""), "{r}");
        let r = explain(&mut s, "B2");
        assert_eq!(r["error"], "#N/A");
        assert!(evidence(&r).contains("MATCH looks for \"kiwi\""), "{r}");
        let r = explain(&mut s, "B3");
        assert_eq!(r["error"], "#NUM!");
        assert!(evidence(&r).contains("SQRT is given -4"), "{r}");
        let r = explain(&mut s, "B4");
        assert_eq!(r["error"], "#NULL!");
        assert!(evidence(&r).contains("The space between A1:A2 and C1:C2"), "{r}");
        let r = explain(&mut s, "B5");
        assert_eq!(r["error"], "#CALC!");
        assert!(evidence(&r).contains("FILTER found no rows"), "{r}");
        assert!(evidence(&explain(&mut s, "B6")).contains("calls NA()"));
        // A cell reading an error passes it on, and says where it came from.
        set(&mut s, &[("C5", "=B6*2")]);
        let r = explain(&mut s, "C5");
        assert!(evidence(&r).contains("B6 already shows #N/A"), "{r}");
        // A circular reference.
        set(&mut s, &[("D1", "=D2+1"), ("D2", "=D1+1")]);
        let r = explain(&mut s, "D1");
        assert_eq!(r["error"], "#CIRC!", "{r}");
        assert!(evidence(&r).contains("circular reference"), "{r}");
    }

    #[test]
    fn spill_names_the_cell_in_the_way() {
        let mut s = book();
        set(&mut s, &[("A3", "blocker"), ("A1", "=SEQUENCE(4)")]);
        let r = explain(&mut s, "A1");
        assert_eq!(r["error"], "#SPILL!", "{r}");
        let ev = evidence(&r);
        assert!(ev.contains("The result needs A1:A4"), "{ev}");
        assert!(ev.contains("A3 is in the way: it holds \"blocker\""), "{ev}");
        // Clearing the blocker lets it spill; a spilled member explains nothing.
        set(&mut s, &[("A3", "")]);
        assert_eq!(explain(&mut s, "A1")["error"], Json::Null);
    }

    #[test]
    fn non_error_cells_and_empty_params_are_not_errors() {
        let mut s = book();
        let r = s.execute("formulas.explainError", json!({})).unwrap();
        assert_eq!(r["error"], Json::Null);
        assert_eq!(r["cell"], "A1");
        set(&mut s, &[("B2", "=1+1")]);
        let r = explain(&mut s, "B2");
        assert_eq!(r, json!({"cell": "B2", "sheet": "Sheet1", "error": null, "value": 2.0}));
        let r = s.execute("formulas.diagnostics", json!({})).unwrap();
        assert_eq!(r["issues"], json!([]));
        assert_eq!(r["truncated"], false);
        assert_eq!(r["capped"], false);
        assert!(s.execute("formulas.diagnostics", json!({"checks": ["nope"]})).is_err());
        assert!(s.execute("formulas.diagnostics", json!({"sheet": "Missing"})).is_err());
        assert!(s.execute("formulas.diagnostics", json!({"limit": -1})).is_err());
        assert!(s.execute("formulas.explainError", json!({"cell": "not a cell"})).is_err());
    }

    #[test]
    fn error_and_broken_reference_checks_fire_on_the_right_cells() {
        let mut s = book();
        set(&mut s, &[("A1", "1"), ("B1", "=A1/0"), ("C1", "=SUM(#REF!)"), ("D1", "=A1*2")]);
        assert_eq!(only(&mut s, "error"), ["Sheet1!B1", "Sheet1!C1"]);
        assert_eq!(only(&mut s, "brokenReference"), ["Sheet1!C1"]);
        let r = s.execute("formulas.diagnostics", json!({"checks": ["error"]})).unwrap();
        assert_eq!(r["issues"][0]["message"], "#DIV/0!: Division by zero");
        assert_eq!(r["counts"], json!({"error": 2}));
        // A clean sheet.
        let mut clean = book();
        set(&mut clean, &[("A1", "1"), ("A2", "=A1+1"), ("A3", "=A2+1")]);
        assert!(issues(&mut clean, json!({})).is_empty());
    }

    #[test]
    fn inconsistent_formula_and_hardcoded_value_checks() {
        let mut s = book();
        set(
            &mut s,
            &[
                ("A1", "1"),
                ("B1", "2"),
                ("A2", "3"),
                ("B2", "4"),
                ("A3", "5"),
                ("B3", "6"),
                ("A4", "7"),
                ("B4", "8"),
                ("A5", "9"),
                ("B5", "10"),
                ("C1", "=A1+B1"),
                ("C2", "=A2+B2"),
                ("C3", "=A3*B3"),
                ("C4", "=A4+B4"),
                ("C5", "=A5+B5"),
                ("C6", "=A6+B6"),
                ("C7", "=A7+B7"),
            ],
        );
        // C5 is replaced by a typed value.
        set(&mut s, &[("C5", "19")]);
        assert_eq!(only(&mut s, "inconsistentFormula"), ["Sheet1!C3"]);
        let r = s.execute("formulas.diagnostics", json!({"checks": ["inconsistentFormula"]})).unwrap();
        assert_eq!(r["issues"][0]["detail"]["expected"], "=A3+B3", "{r}");
        assert_eq!(only(&mut s, "hardcodedValue"), ["Sheet1!C5"]);
        let r = s.execute("formulas.diagnostics", json!({"checks": "hardcodedValue"})).unwrap();
        assert_eq!(r["issues"][0]["detail"]["expected"], "=A5+B5", "{r}");
        // A filled run with nothing out of place.
        let mut clean = book();
        set(&mut clean, &[("A1", "1"), ("A2", "2"), ("A3", "3"), ("B1", "=A1*2"), ("B2", "=A2*2"), ("B3", "=A3*2")]);
        assert!(only(&mut clean, "inconsistentFormula").is_empty() && only(&mut clean, "hardcodedValue").is_empty());
    }

    #[test]
    fn circular_and_external_reference_checks() {
        let mut s = book();
        set(&mut s, &[("A1", "=A2+1"), ("A2", "=A1+1"), ("B1", "=SUM(C1:C3)")]);
        assert_eq!(only(&mut s, "circular"), ["Sheet1!A1", "Sheet1!A2"]);
        assert!(only(&mut s, "externalReference").is_empty());
        assert_eq!(super::external_books("[Book.xlsx]Sheet1!A1+1"), ["Book.xlsx"]);
        assert_eq!(super::external_books("'C:\\data\\[Prices 2026.xlsx]Rates'!B2*2"), ["Prices 2026.xlsx"]);
        assert_eq!(super::external_books("[1]Sheet1!$A$1"), ["1"]);
        assert!(super::external_books("SUM(Table1[Amount])+[@Qty]").is_empty());
        assert!(super::external_books("\"[Book.xlsx]Sheet1!A1\"").is_empty());
        s.execute("cell.set", json!({"cell": "C2", "input": "='[Other.xlsx]Sheet1'!A1*2"})).unwrap();
        let r = s.execute("formulas.diagnostics", json!({"checks": ["externalReference"]})).unwrap();
        assert_eq!(r["issues"][0]["cell"], "Sheet1!C2", "{r}");
        assert_eq!(r["issues"][0]["detail"]["workbooks"], json!(["Other.xlsx"]));
    }

    #[test]
    fn order_is_by_sheet_row_and_column_and_the_limit_holds() {
        let mut s = book();
        for col in ["C", "A", "B"] {
            for row in 1..=4 {
                set(&mut s, &[(&format!("{col}{row}"), "=1/0")]);
            }
        }
        s.execute("home.insertSheet", json!({})).unwrap();
        let sheets = s.doc().unwrap().wb.sheets.len();
        assert!(sheets >= 2);
        let all = only(&mut s, "error");
        assert_eq!(all.first().map(String::as_str), Some("Sheet1!A1"));
        assert_eq!(all.get(1).map(String::as_str), Some("Sheet1!B1"));
        assert_eq!(all.get(3).map(String::as_str), Some("Sheet1!A2"));
        let r = s.execute("formulas.diagnostics", json!({"checks": ["error"], "limit": 5})).unwrap();
        assert_eq!(r["issues"].as_array().unwrap().len(), 5);
        assert_eq!(r["counts"]["error"], 12);
        assert_eq!(r["truncated"], true);
        let r = s.execute("formulas.diagnostics", json!({"limit": 999_999})).unwrap();
        assert_eq!(r["truncated"], false);
        assert_eq!(r["scanned"]["sheets"], sheets);
        let r = s.execute("formulas.diagnostics", json!({"sheet": "Sheet1", "checks": ["error"]})).unwrap();
        assert_eq!(r["scanned"]["sheets"], 1);
        assert_eq!(r["counts"]["error"], 12);
    }
}
