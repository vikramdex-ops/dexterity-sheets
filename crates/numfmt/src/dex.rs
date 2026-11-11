//! Dexterity Sheets: regional settings in what a number format shows.
//!
//! Excel shows a cell with the date separator, decimal point and digit grouping of the computer's
//! regional settings (format codes in files always use `/`, `.` and `,`); `TEXT()` takes its
//! format text as typed. Official GridCraft's renderer (`render.rs`) stays as upstream writes it:
//! it asks this module for those characters at the few places that print one, so a rewrite of
//! the renderer upstream never has to be merged with Dexterity's code by hand
//! (packaging/dexterity/reapply_overlay.py places the calls, check_overlay.py checks them).

use std::cell::Cell;

use gridcraft_core::locale::{Grouping, locale};

/// The locale-dependent characters a value is rendered with.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Ctx {
    /// What `/` in a date section shows.
    pub date_sep: char,
    pub decimal: char,
    pub thousands: char,
    pub grouping: Grouping,
}

thread_local! {
    /// Set while `TEXT()` formats: see [`as_typed`].
    static AS_TYPED: Cell<bool> = const { Cell::new(false) };
}

/// The characters to render with now: the current locale's for a cell's display, or, inside
/// [`as_typed`], the ones of the format text itself.
pub(crate) fn ctx() -> Ctx {
    let l = locale();
    if AS_TYPED.try_with(Cell::get).unwrap_or(false) {
        // `TEXT()`: `/` stays `/` (Excel under en-IN gives `15/03/2026` for
        // `TEXT(x,"dd/mm/yyyy")`); digits still group the locale's way.
        Ctx { date_sep: '/', decimal: '.', thousands: ',', grouping: l.grouping }
    } else {
        Ctx { date_sep: l.date_sep, decimal: l.decimal, thousands: l.thousands, grouping: l.grouping }
    }
}

/// Runs `f` with format text taken as typed: what the `TEXT()` worksheet function does.
pub(crate) fn as_typed<T>(f: impl FnOnce() -> T) -> T {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            let _ = AS_TYPED.try_with(|c| c.set(self.0));
        }
    }
    let _restore = Restore(AS_TYPED.try_with(|c| c.replace(true)).unwrap_or(false));
    f()
}

/// General-format text with the decimal point to show.
pub(crate) fn general(s: String) -> String {
    let decimal = ctx().decimal;
    if decimal == '.' { s } else { s.replace('.', decimal.encode_utf8(&mut [0; 4])) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn as_typed_is_scoped_and_nests() {
        assert_eq!(ctx().date_sep, locale().date_sep);
        let inner = as_typed(|| {
            let nested = as_typed(|| ctx().decimal);
            (nested, ctx().decimal, ctx().thousands, ctx().date_sep)
        });
        assert_eq!(inner, ('.', '.', ',', '/'));
        assert_eq!(ctx().decimal, locale().decimal);
    }
}
