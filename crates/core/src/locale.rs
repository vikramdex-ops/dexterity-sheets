//! Regional settings: how typed dates and numbers are read, and how the locale-dependent parts of
//! number formats display.
//!
//! Excel reads `3/4/2026`, `15-03-2026` and `1,23,456` by the computer's regional settings, shows
//! the `/` of a date format as the locale's date separator, groups `#,##0` digits the locale's way
//! and gives the built-in formats 5–8, 14–22 and 37–44 a locale-specific code. GridCraft keeps one
//! process-wide [`Locale`]: en-US unless the desktop app (from the operating system) or the command
//! line (`DEXSHEETS_LOCALE`) sets another at start-up.
//!
//! Files never depend on it. Workbooks store serial numbers and format codes; writers never read
//! the locale, so the same workbook always saves to the same bytes.

use std::cell::Cell;
use std::sync::{LazyLock, RwLock};

/// Order of day, month and year in an all-numeric date (`iDate` on Windows).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DateOrder {
    /// 3/4/2026 is 4 March (en-US).
    Mdy,
    /// 3/4/2026 is 3 April (en-IN, en-GB).
    Dmy,
    /// 2026/3/4 (ja-JP, sv-SE).
    Ymd,
}

/// How the digits left of the decimal point are grouped (`sGrouping` on Windows).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grouping {
    /// Groups of three: 123,456,789.
    Thousands,
    /// Three, then pairs (`3;2`): 12,34,56,789.
    Indian,
}

impl Grouping {
    /// True when a separator goes between two digits with `pos` digits to its right.
    pub fn separator_at(self, pos: usize) -> bool {
        match self {
            Grouping::Thousands => pos > 0 && pos.is_multiple_of(3),
            Grouping::Indian => pos == 3 || (pos > 3 && (pos - 3).is_multiple_of(2)),
        }
    }
}

/// Regional settings that change how text is read and how formats display.
#[derive(Clone, Debug, PartialEq)]
pub struct Locale {
    /// BCP 47 name, e.g. `en-IN`.
    pub name: String,
    pub date_order: DateOrder,
    /// The date separator: what `/` in a date format shows and what typed dates may use besides
    /// `/` and `-`.
    pub date_sep: char,
    pub decimal: char,
    pub thousands: char,
    pub grouping: Grouping,
    /// The currency symbol typed amounts may start with (`$`, `₹`).
    pub currency: String,
    /// A typed `2026-03-04` keeps `yyyy-mm-dd` (upstream en-US behaviour) instead of taking the
    /// short date format (observed in Excel under en-IN).
    pub iso_keeps_format: bool,
    /// Codes of the locale-dependent built-in number formats, in file form (`/` is the date
    /// separator).
    builtins: Vec<(u32, String)>,
}

/// Ids whose code depends on the locale. Excel shows these with the reader's own settings, so a
/// file that names one by id displays differently on a computer with other settings.
pub fn is_locale_dependent_builtin(id: u32) -> bool {
    matches!(id, 5..=8 | 14..=22 | 27..=47)
}

const EN_US_MONEY: [(u32, &str); 12] = [
    (5, "\"$\"#,##0_);\\(\"$\"#,##0\\)"),
    (6, "\"$\"#,##0_);[Red]\\(\"$\"#,##0\\)"),
    (7, "\"$\"#,##0.00_);\\(\"$\"#,##0.00\\)"),
    (8, "\"$\"#,##0.00_);[Red]\\(\"$\"#,##0.00\\)"),
    (37, "#,##0 ;(#,##0)"),
    (38, "#,##0 ;[Red](#,##0)"),
    (39, "#,##0.00;(#,##0.00)"),
    (40, "#,##0.00;[Red](#,##0.00)"),
    (41, "_(* #,##0_);_(* \\(#,##0\\);_(* \"-\"_);_(@_)"),
    (42, "_(\"$\"* #,##0_);_(\"$\"* \\(#,##0\\);_(\"$\"* \"-\"_);_(@_)"),
    (43, "_(* #,##0.00_);_(* \\(#,##0.00\\);_(* \"-\"??_);_(@_)"),
    (44, "_(\"$\"* #,##0.00_);_(\"$\"* \\(#,##0.00\\);_(\"$\"* \"-\"??_);_(@_)"),
];

/// The same ids as Excel shows them under en-IN (observed, Excel 16 on Windows): rupee currency,
/// minus-sign negatives.
const EN_IN_MONEY: [(u32, &str); 12] = [
    (5, "\"₹\"\\ #,##0;\"₹\"\\ \\-#,##0"),
    (6, "\"₹\"\\ #,##0;[Red]\"₹\"\\ \\-#,##0"),
    (7, "\"₹\"\\ #,##0.00;\"₹\"\\ \\-#,##0.00"),
    (8, "\"₹\"\\ #,##0.00;[Red]\"₹\"\\ \\-#,##0.00"),
    (37, "#,##0;\\-#,##0"),
    (38, "#,##0;[Red]\\-#,##0"),
    (39, "#,##0.00;\\-#,##0.00"),
    (40, "#,##0.00;[Red]\\-#,##0.00"),
    (41, "_ * #,##0_ ;_ * \\-#,##0_ ;_ * \"-\"_ ;_ @_ "),
    (42, "_ \"₹\"\\ * #,##0_ ;_ \"₹\"\\ * \\-#,##0_ ;_ \"₹\"\\ * \"-\"_ ;_ @_ "),
    (43, "_ * #,##0.00_ ;_ * \\-#,##0.00_ ;_ * \"-\"??_ ;_ @_ "),
    (44, "_ \"₹\"\\ * #,##0.00_ ;_ \"₹\"\\ * \\-#,##0.00_ ;_ \"₹\"\\ * \"-\"??_ ;_ @_ "),
];

impl Locale {
    /// United States English: the default.
    pub fn en_us() -> Locale {
        let mut l = Locale {
            name: "en-US".into(),
            date_order: DateOrder::Mdy,
            date_sep: '/',
            decimal: '.',
            thousands: ',',
            grouping: Grouping::Thousands,
            currency: "$".into(),
            iso_keeps_format: true,
            builtins: Vec::new(),
        };
        l.set_date_builtins("m/d/yyyy", false, false);
        l.set_money(&EN_US_MONEY);
        l
    }

    /// Indian English as Windows ships it: dd-MM-yyyy, `-` separator, 3;2 grouping, hh:mm times.
    pub fn en_in() -> Locale {
        let mut l = Locale {
            name: "en-IN".into(),
            date_order: DateOrder::Dmy,
            date_sep: '-',
            decimal: '.',
            thousands: ',',
            grouping: Grouping::Indian,
            currency: "₹".into(),
            iso_keeps_format: false,
            builtins: Vec::new(),
        };
        l.set_date_builtins("dd/mm/yyyy", true, true);
        l.set_money(&EN_IN_MONEY);
        l
    }

    /// A named preset (`en-US`, `en-IN`; any case, `_` or `-`).
    pub fn preset(name: &str) -> Option<Locale> {
        match name.trim().replace('_', "-").to_ascii_lowercase().as_str() {
            "en-us" | "us" => Some(Locale::en_us()),
            "en-in" | "in" => Some(Locale::en_in()),
            _ => None,
        }
    }

    /// The locale from Windows' regional settings (`HKCU\Control Panel\International`): `get`
    /// returns a value by name (`LocaleName`, `iDate`, `sDate`, `sShortDate`, `sTimeFormat`,
    /// `sGrouping`, `sDecimal`, `sThousand`). Missing or unusable values keep the preset of the
    /// locale's name (en-US for names without one). The currency symbol and the currency and
    /// accounting built-ins come from the preset: only en-US and en-IN are known.
    pub fn from_windows(get: impl Fn(&str) -> Option<String>) -> Locale {
        let name = get("LocaleName").map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        let mut l = name.as_deref().and_then(Locale::preset).unwrap_or_else(Locale::en_us);
        if let Some(n) = name {
            l.name = n;
        }
        match get("iDate").as_deref().map(str::trim) {
            Some("0") => l.date_order = DateOrder::Mdy,
            Some("1") => l.date_order = DateOrder::Dmy,
            Some("2") => l.date_order = DateOrder::Ymd,
            _ => {}
        }
        let first = |key: &str| get(key).and_then(|s| s.chars().next());
        if let Some(c) = first("sDate").filter(|c| !c.is_ascii_alphanumeric() && *c != ':') {
            l.date_sep = c;
        }
        let decimal = first("sDecimal").filter(|c| !c.is_ascii_digit());
        let thousands = first("sThousand").filter(|c| !c.is_ascii_digit());
        if let (Some(d), Some(t)) = (decimal, thousands)
            && d != t
        {
            l.decimal = d;
            l.thousands = t;
        }
        if let Some(g) = get("sGrouping") {
            l.grouping = if g.trim().starts_with("3;2") { Grouping::Indian } else { Grouping::Thousands };
        }
        if let Some(short) = get("sShortDate").filter(|s| !s.trim().is_empty()) {
            let sep = get("sDate").unwrap_or_else(|| l.date_sep.to_string());
            let code = windows_date_to_code(&short, &sep);
            let time = get("sTimeFormat").unwrap_or_default();
            let hour_pad = time.contains("hh") || time.contains("HH");
            l.set_date_builtins(&code, short.contains("dd"), hour_pad);
        }
        l
    }

    /// The locale-dependent code of a built-in number format, in file form; `None` for ids that
    /// read the same everywhere (and for unknown ids).
    pub fn builtin(&self, id: u32) -> Option<&str> {
        self.builtins.iter().find(|(i, _)| *i == id).map(|(_, c)| c.as_str())
    }

    /// The format a typed date gets: the short date (built-in 14).
    pub fn short_date(&self) -> &str {
        self.builtin(14).unwrap_or("m/d/yyyy")
    }

    fn set(&mut self, id: u32, code: String) {
        match self.builtins.iter_mut().find(|(i, _)| *i == id) {
            Some(slot) => slot.1 = code,
            None => self.builtins.push((id, code)),
        }
    }

    fn set_money(&mut self, table: &[(u32, &str)]) {
        for (id, code) in table {
            self.set(*id, (*code).to_string());
        }
    }

    /// Built-ins 14–22 (and 27–36, which Excel shows as the short date or time outside en-US)
    /// from the short date code and whether days and hours have two digits.
    fn set_date_builtins(&mut self, short: &str, day_pad: bool, hour_pad: bool) {
        let d = if day_pad { "dd" } else { "d" };
        let h = if hour_pad { "hh" } else { "h" };
        self.set(14, short.to_string());
        self.set(15, format!("{d}-mmm-yy"));
        self.set(16, format!("{d}-mmm"));
        self.set(17, "mmm-yy".into());
        self.set(18, format!("{h}:mm AM/PM"));
        self.set(19, format!("{h}:mm:ss AM/PM"));
        self.set(20, format!("{h}:mm"));
        self.set(21, format!("{h}:mm:ss"));
        self.set(22, format!("{short} {h}:mm"));
        for id in [27, 28, 29, 30, 31, 36] {
            self.set(id, short.to_string());
        }
        for id in 32..=35 {
            self.set(id, format!("{h}:mm:ss"));
        }
        if self.name.eq_ignore_ascii_case("en-US") {
            self.builtins.retain(|(id, _)| !matches!(id, 27..=36));
        }
    }
}

/// A Windows date picture (`dd-MM-yyyy`) as an Excel format code in file form (`dd/mm/yyyy`): the
/// separator `sep` becomes `/` (shown as the locale's separator), other characters are escaped.
pub fn windows_date_to_code(picture: &str, sep: &str) -> String {
    let chars: Vec<char> = picture.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while let Some(&c) = chars.get(i) {
        let run = chars.get(i..).map_or(0, |r| r.iter().take_while(|&&x| x == c).count()).max(1);
        match c {
            'd' => out.push_str(&"d".repeat(run.min(4))),
            'M' => out.push_str(&"m".repeat(run.min(4))),
            'y' => out.push_str(if run <= 2 { "yy" } else { "yyyy" }),
            '\'' => {
                // A quoted literal: up to the next quote.
                let end = chars.get(i + 1..).and_then(|r| r.iter().position(|&x| x == '\'')).map_or(chars.len(), |p| i + 1 + p);
                let lit: String = chars.get(i + 1..end).map(|r| r.iter().collect()).unwrap_or_default();
                if !lit.is_empty() {
                    out.push('"');
                    out.push_str(&lit.replace('"', ""));
                    out.push('"');
                }
                i = end + 1;
                continue;
            }
            _ if !sep.is_empty() && picture.get(char_byte(picture, i)..).is_some_and(|r| r.starts_with(sep)) => {
                out.push('/');
                i += sep.chars().count();
                continue;
            }
            ' ' => out.push(' '),
            _ => {
                out.push('\\');
                out.push(c);
                i += 1;
                continue;
            }
        }
        i += run;
    }
    out
}

/// Byte offset of the `i`-th character of `s` (its length past the end).
fn char_byte(s: &str, i: usize) -> usize {
    s.char_indices().nth(i).map_or(s.len(), |(b, _)| b)
}

static DEFAULT: LazyLock<Locale> = LazyLock::new(Locale::en_us);
static CURRENT: RwLock<Option<&'static Locale>> = RwLock::new(None);

thread_local! {
    static SCOPED: Cell<Option<&'static Locale>> = const { Cell::new(None) };
}

/// The locale in effect on this thread: a [`scoped`] one, else the process-wide one.
pub fn locale() -> &'static Locale {
    if let Ok(Some(l)) = SCOPED.try_with(Cell::get) {
        return l;
    }
    let current = match CURRENT.read() {
        Ok(g) => *g,
        Err(poisoned) => *poisoned.into_inner(),
    };
    current.unwrap_or(&DEFAULT)
}

/// Sets the process-wide locale. Meant to be called once at start-up: each call keeps its locale
/// for the rest of the process (so the format codes it lends out stay valid).
pub fn set_locale(l: Locale) {
    let leaked: &'static Locale = Box::leak(Box::new(l));
    match CURRENT.write() {
        Ok(mut g) => *g = Some(leaked),
        Err(poisoned) => *poisoned.into_inner() = Some(leaked),
    }
}

/// Restores the previous thread locale when dropped.
pub struct ScopedLocale {
    previous: Option<&'static Locale>,
}

impl Drop for ScopedLocale {
    fn drop(&mut self) {
        let previous = self.previous;
        let _ = SCOPED.try_with(|s| s.set(previous));
    }
}

/// Uses `l` on this thread only until the returned guard drops (tests, previews). Like
/// [`set_locale`], the locale is kept for the rest of the process.
pub fn scoped(l: Locale) -> ScopedLocale {
    let leaked: &'static Locale = Box::leak(Box::new(l));
    let previous = SCOPED.try_with(|s| s.replace(Some(leaked))).ok().flatten();
    ScopedLocale { previous }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indian_grouping_positions() {
        let seps: Vec<usize> = (1..12).filter(|&p| Grouping::Indian.separator_at(p)).collect();
        assert_eq!(seps, [3, 5, 7, 9, 11]);
        let seps: Vec<usize> = (1..12).filter(|&p| Grouping::Thousands.separator_at(p)).collect();
        assert_eq!(seps, [3, 6, 9]);
    }

    #[test]
    fn windows_pictures() {
        assert_eq!(windows_date_to_code("dd-MM-yyyy", "-"), "dd/mm/yyyy");
        assert_eq!(windows_date_to_code("M/d/yyyy", "/"), "m/d/yyyy");
        assert_eq!(windows_date_to_code("dd.MM.yy", "."), "dd/mm/yy");
        assert_eq!(windows_date_to_code("yyyy-MM-dd", "-"), "yyyy/mm/dd");
        assert_eq!(windows_date_to_code("dd/MM/yyyy", "-"), "dd\\/mm\\/yyyy");
        assert_eq!(windows_date_to_code("d. M. yyyy", ". "), "d/m/yyyy");
        assert_eq!(windows_date_to_code("dd'x'MM", "-"), "dd\"x\"mm");
    }

    #[test]
    fn from_windows_en_in() {
        let reg = [
            ("LocaleName", "en-IN"),
            ("iDate", "1"),
            ("sDate", "-"),
            ("sShortDate", "dd-MM-yyyy"),
            ("sTimeFormat", "hh:mm:ss tt"),
            ("sGrouping", "3;2;0"),
            ("sDecimal", "."),
            ("sThousand", ","),
        ];
        let l = Locale::from_windows(|k| reg.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string()));
        assert_eq!(l, Locale::en_in());
        assert_eq!(l.builtin(14), Some("dd/mm/yyyy"));
        assert_eq!(l.builtin(20), Some("hh:mm"));
        assert_eq!(l.builtin(22), Some("dd/mm/yyyy hh:mm"));
        assert_eq!(l.builtin(40), Some("#,##0.00;[Red]\\-#,##0.00"));
        assert_eq!(l.builtin(3), None);
    }

    #[test]
    fn from_windows_en_us_matches_the_preset() {
        let reg = [
            ("LocaleName", "en-US"),
            ("iDate", "0"),
            ("sDate", "/"),
            ("sShortDate", "M/d/yyyy"),
            ("sTimeFormat", "h:mm:ss tt"),
            ("sGrouping", "3;0"),
            ("sDecimal", "."),
            ("sThousand", ","),
        ];
        let l = Locale::from_windows(|k| reg.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string()));
        assert_eq!(l, Locale::en_us());
        // Nothing usable: en-US.
        assert_eq!(Locale::from_windows(|_| None), Locale::en_us());
        // A user's own settings override the preset.
        let custom = [("LocaleName", "en-US"), ("iDate", "1"), ("sDate", "."), ("sShortDate", "dd.MM.yyyy"), ("sGrouping", "3;2;0")];
        let l = Locale::from_windows(|k| custom.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string()));
        assert_eq!((l.date_order, l.date_sep, l.grouping), (DateOrder::Dmy, '.', Grouping::Indian));
        assert_eq!(l.short_date(), "dd/mm/yyyy");
        // The same character for decimals and thousands is ignored.
        let bad = [("sDecimal", ","), ("sThousand", ",")];
        let l = Locale::from_windows(|k| bad.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string()));
        assert_eq!((l.decimal, l.thousands), ('.', ','));
    }

    #[test]
    fn scoped_locale_is_per_thread_and_restored() {
        assert_eq!(locale().name, "en-US");
        {
            let _g = scoped(Locale::en_in());
            assert_eq!(locale().name, "en-IN");
            std::thread::spawn(|| assert_eq!(locale().name, "en-US")).join().unwrap();
            {
                let _inner = scoped(Locale::en_us());
                assert_eq!(locale().name, "en-US");
            }
            assert_eq!(locale().name, "en-IN");
        }
        assert_eq!(locale().name, "en-US");
    }

    #[test]
    fn presets() {
        assert_eq!(Locale::preset("EN_in"), Some(Locale::en_in()));
        assert_eq!(Locale::preset("en-US"), Some(Locale::en_us()));
        assert_eq!(Locale::preset("xx"), None);
        assert!(is_locale_dependent_builtin(14) && is_locale_dependent_builtin(43) && !is_locale_dependent_builtin(3));
    }
}
