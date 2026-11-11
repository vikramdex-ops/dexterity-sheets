//! Dexterity Sheets: the regional settings decide what a text file leaves open.
//!
//! Official GridCraft reads each column's own evidence for the order of day and month
//! (`31/05/2026` can only be day first), which Dexterity Sheets keeps. Where a column gives no
//! evidence (`03/04/2026` everywhere) upstream assumes month first and stores `m/d/yyyy`; here
//! the computer's regional settings answer, as they do for a date typed into a cell and as Excel
//! does: day first under en-IN, shown in the locale's short date format. Under en-US nothing
//! differs from upstream. The calls into this file are placed by
//! packaging/dexterity/reapply_overlay.py and checked by check_overlay.py.

use gridcraft_core::locale::{DateOrder, locale};

fn day_first() -> bool {
    matches!(locale().date_order, DateOrder::Dmy)
}

/// The serial of a date that reads both ways, given its two readings.
pub(super) fn ambiguous_serial(day_first_serial: Option<f64>, month_first_serial: Option<f64>) -> Option<f64> {
    if day_first() { day_first_serial } else { month_first_serial }
}

/// How the open report words the order an all-ambiguous column was read in.
pub(super) fn ambiguous_order() -> &'static str {
    if day_first() { "day first" } else { "month first" }
}

/// The format a date read from a text file gets: the locale's short date (built-in 14), with
/// the time when the field has one (built-in 22), as typed input gives.
pub(super) fn date_format(with_time: bool) -> &'static str {
    let (id, en_us) = if with_time { (22, "m/d/yyyy h:mm") } else { (14, "m/d/yyyy") };
    locale().builtin(id).unwrap_or(en_us)
}

#[cfg(test)]
mod tests {
    use gridcraft_core::locale::{Locale, scoped};
    use gridcraft_core::{CellRef, Value};

    use crate::{CsvOptions, read_csv, read_csv_with_report};

    #[test]
    fn en_us_reads_as_upstream_does() {
        assert_eq!(super::ambiguous_serial(Some(1.0), Some(2.0)), Some(2.0));
        assert_eq!(super::ambiguous_order(), "month first");
        assert_eq!((super::date_format(false), super::date_format(true)), ("m/d/yyyy", "m/d/yyyy h:mm"));
    }

    /// Under en-IN a column of dates that read both ways is day first, is shown in the locale's
    /// short date format, and the report says how it was read; a column whose own dates settle
    /// the order follows them, whatever the locale.
    #[test]
    fn en_in_reads_an_ambiguous_column_day_first() {
        let _l = scoped(Locale::en_in());
        assert_eq!(super::ambiguous_order(), "day first");
        let (wb, report) = read_csv_with_report(b"03/04/2026,12/31/2026\r\n05/06/2026,01/02/2026\r\n", &CsvOptions::default()).unwrap();
        let sheet = wb.sheet(0).unwrap();
        // 3 April and 5 June 2026.
        assert_eq!(sheet.value(CellRef::new(0, 0)), Value::Number(46115.0));
        assert_eq!(sheet.value(CellRef::new(1, 0)), Value::Number(46178.0));
        assert_eq!(wb.styles.get(sheet.style_id(CellRef::new(0, 0))).num_fmt.as_str(), "dd/mm/yyyy");
        assert!(report.warnings.iter().any(|w| w.contains("column A") && w.contains("read day first")), "{:?}", report.warnings);
        // Column B can only be month first (31 December), so 2 January follows it.
        assert_eq!(sheet.value(CellRef::new(0, 1)), Value::Number(46387.0));
        assert_eq!(sheet.value(CellRef::new(1, 1)), Value::Number(46024.0));
        let plain = read_csv(b"03/04/2026\r\n", &CsvOptions::default()).unwrap();
        assert_eq!(plain.sheet(0).unwrap().value(CellRef::new(0, 0)), Value::Number(46115.0));
    }
}
