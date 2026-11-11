//! Dexterity Sheets: one addition to the writer's choice between a legacy and a dynamic-array
//! formula (`super::export`). Kept in its own file so official GridCraft's `array_formula.rs` is
//! taken as it comes; the hook is one `if` in `export` (packaging/dexterity/reapply_overlay.py).
//!
//! Excel shows a formula saved without dynamic-array metadata with an `@` in front of IFS,
//! SWITCH, INDIRECT and CELL wherever they give a single value: `=@IFS(…)`, `=1+@INDIRECT(…)`,
//! `=UPPER(@SWITCH(…))` (observed with Excel 16; the values are the same). Excel saves its own
//! such formulas with array evaluation, and so does Dexterity Sheets, so a workbook made here
//! opens in Excel with the formulas as they were typed.

use gridcraft_formula::{BinOp, Expr};

/// True when a call to IFS, SWITCH, INDIRECT or CELL sits where a single value is used. A
/// reference-taking argument (`SUM(INDIRECT(…))`) gets no `@` in Excel and is not counted.
pub(super) fn reference_function_used_as_value(e: &Expr) -> bool {
    let mut stack = vec![(e, true)];
    // The node budget of a formula as long as Excel allows (8,192 characters make at most about
    // 8,200 nodes); past it the formula keeps the spelling `export` chooses.
    let mut budget = 16_384usize;
    while let Some((e, value)) = stack.pop() {
        if budget == 0 {
            return false;
        }
        budget -= 1;
        match e {
            Expr::Call(name, args) => {
                if value && matches!(name.as_str(), "IFS" | "SWITCH" | "INDIRECT" | "CELL") {
                    return true;
                }
                stack.extend(args.iter().enumerate().map(|(i, a)| (a, super::scalar_arg(name, i))));
            }
            Expr::Paren(x) | Expr::Unary(_, x) => stack.push((x, value)),
            Expr::Binary(op, a, b) => {
                let value = !matches!(op, BinOp::Range | BinOp::Union | BinOp::Intersect);
                stack.push((a, value));
                stack.push((b, value));
            }
            Expr::Invoke(c, args) => {
                stack.push((c, true));
                stack.extend(args.iter().map(|a| (a, true)));
            }
            _ => {}
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use gridcraft_core::{CellRef, Value};
    use gridcraft_model::{Cell, Formula, Workbook};

    use super::reference_function_used_as_value;
    use crate::package::Package;
    use crate::xml::El;
    use crate::{read_xlsx, write_xlsx};

    fn at(s: &str) -> CellRef {
        CellRef::parse(s).unwrap()
    }

    fn formula(text: &str, value: f64) -> Cell {
        let mut cell = Cell::formula(Formula::new(text));
        cell.value = Value::Number(value);
        cell
    }

    fn cell<'a>(sheet: &'a El, address: &str) -> &'a El {
        sheet.child("sheetData").unwrap().kids("row").flat_map(|r| r.kids("c")).find(|c| c.attr("r") == Some(address)).unwrap()
    }

    const AS_VALUE: [(&str, &str); 7] = [
        ("C1", "IFS(A1>0,1,TRUE,2)"),
        ("C2", "SWITCH(A1,1,\"a\",\"b\")"),
        ("C3", "INDIRECT(\"A1\")"),
        ("C4", "CELL(\"address\",A1)"),
        ("C5", "1+IFS(A1>0,1,TRUE,2)"),
        ("C6", "UPPER(SWITCH(A1,1,\"a\",\"b\"))"),
        ("C7", "INDIRECT(\"A1\")+1"),
    ];
    const PLAIN: [(&str, &str); 4] =
        [("D1", "SUM(INDIRECT(\"A1:A2\"))"), ("D2", "IF(A1>0,1,2)"), ("D3", "ROWS(INDIRECT(\"A1:A2\"))"), ("D4", "OFFSET(A1,0,0)")];

    #[test]
    fn finds_the_four_functions_only_where_a_value_is_used() {
        for (_, text) in AS_VALUE {
            assert!(reference_function_used_as_value(&gridcraft_formula::parse(text).unwrap()), "{text}");
        }
        for (_, text) in PLAIN {
            assert!(!reference_function_used_as_value(&gridcraft_formula::parse(text).unwrap()), "{text}");
        }
    }

    /// Excel shows a legacy (non-array) IFS, SWITCH, INDIRECT or CELL used as a value with a
    /// spurious `@`; written with array evaluation, as Excel writes them, they read back as
    /// typed. A reference-taking argument (`SUM(INDIRECT(…))`) and other functions stay plain.
    #[test]
    fn reference_functions_used_as_values_keep_array_evaluation() {
        let mut wb = Workbook::new();
        let sh = wb.sheet_mut(0).unwrap();
        sh.cells.set(at("A1"), Cell::value(Value::Number(5.0)));
        for (address, text) in AS_VALUE.iter().chain(&PLAIN) {
            sh.cells.set(at(address), formula(text, 1.0));
        }
        let bytes = write_xlsx(&wb).unwrap();
        let mut pkg = Package::open(&bytes).unwrap();
        let sheet = pkg.read_xml("xl/worksheets/sheet1.xml").unwrap().unwrap();
        for (address, _) in AS_VALUE {
            let c = cell(&sheet, address);
            assert_eq!(c.attr("cm"), Some("1"), "array evaluation metadata at {address}");
            let f = c.child("f").unwrap();
            assert_eq!(f.attr("t"), Some("array"));
            assert_eq!(f.attr("ref"), Some(address));
        }
        for (address, _) in PLAIN {
            let c = cell(&sheet, address);
            assert_eq!(c.attr("cm"), None, "no dynamic metadata at {address}");
            assert_eq!(c.child("f").unwrap().attr("t"), None);
        }
        let (back, _) = read_xlsx(&bytes).unwrap();
        for (address, text) in AS_VALUE.iter().chain(&PLAIN) {
            let f = back.sheet(0).unwrap().cell(at(address)).unwrap().formula.clone().unwrap();
            assert_eq!(f.text, *text);
            assert_eq!(f.array, None);
        }
    }
}
