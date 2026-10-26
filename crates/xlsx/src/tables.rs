//! Fixed lookup tables: built-in number formats, the legacy indexed colour palette, paper sizes,
//! functions that need the `_xlfn.` prefix in files, and unit conversions.

/// Built-in number format codes (en-US) by id. Ids not listed (23–36 are East Asian
/// locale formats) read as General unless the current locale defines them.
const BUILTIN_FORMATS: [(u32, &str); 36] = [
    (0, "General"),
    (1, "0"),
    (2, "0.00"),
    (3, "#,##0"),
    (4, "#,##0.00"),
    (5, "\"$\"#,##0_);\\(\"$\"#,##0\\)"),
    (6, "\"$\"#,##0_);[Red]\\(\"$\"#,##0\\)"),
    (7, "\"$\"#,##0.00_);\\(\"$\"#,##0.00\\)"),
    (8, "\"$\"#,##0.00_);[Red]\\(\"$\"#,##0.00\\)"),
    (9, "0%"),
    (10, "0.00%"),
    (11, "0.00E+00"),
    (12, "# ?/?"),
    (13, "# ??/??"),
    (14, "m/d/yyyy"),
    (15, "d-mmm-yy"),
    (16, "d-mmm"),
    (17, "mmm-yy"),
    (18, "h:mm AM/PM"),
    (19, "h:mm:ss AM/PM"),
    (20, "h:mm"),
    (21, "h:mm:ss"),
    (22, "m/d/yyyy h:mm"),
    (37, "#,##0 ;(#,##0)"),
    (38, "#,##0 ;[Red](#,##0)"),
    (39, "#,##0.00;(#,##0.00)"),
    (40, "#,##0.00;[Red](#,##0.00)"),
    (41, "_(* #,##0_);_(* \\(#,##0\\);_(* \"-\"_);_(@_)"),
    (42, "_(\"$\"* #,##0_);_(\"$\"* \\(#,##0\\);_(\"$\"* \"-\"_);_(@_)"),
    (43, "_(* #,##0.00_);_(* \\(#,##0.00\\);_(* \"-\"??_);_(@_)"),
    (44, "_(\"$\"* #,##0.00_);_(\"$\"* \\(#,##0.00\\);_(\"$\"* \"-\"??_);_(@_)"),
    (45, "mm:ss"),
    (46, "[h]:mm:ss"),
    // ECMA-376 lists `mmss.0`; Excel shows id 47 as `mm:ss.0` (37:43.7, not 3743.7).
    (47, "mm:ss.0"),
    (48, "##0.0E+0"),
    (49, "@"),
];

/// The code a built-in id read from a file stands for: Excel shows the locale-dependent ids
/// (5–8, 14–22, 37–47) with the reader's regional settings, so they read as the current locale's
/// code (`40` is `#,##0.00;[Red](#,##0.00)` in en-US, `#,##0.00;[Red]\-#,##0.00` in en-IN).
pub fn builtin_format(id: u32) -> Option<&'static str> {
    if let Some(code) = gridcraft_core::locale::locale().builtin(id) {
        return Some(code);
    }
    BUILTIN_FORMATS.iter().find(|(i, _)| *i == id).map(|(_, c)| *c)
}

/// Id to write for a format code when it is a locale-independent built-in (0–4, 9–13, 48, 49).
/// Every other code is written as a custom format with its own text, whatever the current locale:
/// a bare locale-dependent id (5–8 currency, 14–22 dates and times, 37–44 accounting, 45–47)
/// shows the reader's version instead (en-IN Excel shows id 20 `h:mm` as `hh:mm` and id 43
/// without parentheses).
pub fn builtin_id(code: &str) -> Option<u32> {
    BUILTIN_FORMATS
        .iter()
        .find(|(i, c)| *c == code && !gridcraft_core::locale::is_locale_dependent_builtin(*i))
        .map(|(i, _)| *i)
}

pub use gridcraft_core::color::INDEXED;

/// Paper sizes (`pageSetup paperSize`) we name. ID 12/13 are the JIS B4/B5 that
/// `pageLayout.pageSetup` calls `B4`/`B5`; 42 is ISO B4, and 43–88 are the Japanese postcards,
/// envelopes and B6. The engine's `known_paper` uses the same names and sizes.
const PAPERS: [(u32, &str); 23] = [
    (1, "Letter"),
    (3, "Tabloid"),
    (4, "Ledger"),
    (5, "Legal"),
    (6, "Statement"),
    (7, "Executive"),
    (8, "A3"),
    (9, "A4"),
    (11, "A5"),
    (12, "B4"),
    (13, "B5"),
    (20, "Envelope #10"),
    (27, "Envelope DL"),
    (34, "Envelope B5"),
    (42, "B4 (ISO)"),
    (43, "Japanese Postcard"),
    (69, "Japanese Double Postcard"),
    (70, "A6"),
    (71, "Japanese Envelope Kaku #2"),
    (72, "Japanese Envelope Kaku #3"),
    (73, "Japanese Envelope #3"),
    (74, "Japanese Envelope #4"),
    (88, "B6"),
];

pub fn paper_name(id: u32) -> Option<&'static str> {
    PAPERS.iter().find(|(i, _)| *i == id).map(|(_, n)| *n)
}

/// The file ID for a paper name. Spelling is loose like `known_paper`'s — spaces, hyphens,
/// underscores and parentheses don't matter, and the JIS/ISO and Japanese-envelope spellings the
/// page-setup commands take are folded onto the name in [`PAPERS`].
pub fn paper_id(name: &str) -> Option<u32> {
    let norm = |s: &str| s.trim().to_ascii_lowercase().replace([' ', '-', '_', '(', ')'], "");
    let normalized = norm(name);
    let alias = match normalized.as_str() {
        "b4jis" => "b4",
        "b5jis" => "b5",
        "b6jis" => "b6",
        "postcard" => "japanesepostcard",
        "doublepostcard" => "japanesedoublepostcard",
        "chou#3" => "japaneseenvelope#3",
        "chou#4" => "japaneseenvelope#4",
        "kaku#2" => "japaneseenvelopekaku#2",
        "kaku#3" => "japaneseenvelopekaku#3",
        other => other,
    };
    PAPERS.iter().find(|(_, n)| norm(n) == alias).map(|(i, _)| *i)
}

/// Functions introduced after Excel 2007 that files store with the `_xlfn.` prefix. Entries with
/// `_xlws.` get `_xlfn._xlws.`.
const FUTURE_FUNCTIONS: &[&str] = &[
    "ACOT",
    "ACOTH",
    "AGGREGATE",
    "ARABIC",
    "ARRAYTOTEXT",
    "BASE",
    "BETA.DIST",
    "BETA.INV",
    "BINOM.DIST",
    "BINOM.DIST.RANGE",
    "BINOM.INV",
    "BITAND",
    "BITLSHIFT",
    "BITOR",
    "BITRSHIFT",
    "BITXOR",
    "BYCOL",
    "BYROW",
    "CEILING.MATH",
    "CEILING.PRECISE",
    "CHISQ.DIST",
    "CHISQ.DIST.RT",
    "CHISQ.INV",
    "CHISQ.INV.RT",
    "CHISQ.TEST",
    "CHOOSECOLS",
    "CHOOSEROWS",
    "COMBINA",
    "CONCAT",
    "CONFIDENCE.NORM",
    "CONFIDENCE.T",
    "COT",
    "COTH",
    "COVARIANCE.P",
    "COVARIANCE.S",
    "CSC",
    "CSCH",
    "DAYS",
    "DECIMAL",
    "DROP",
    "ENCODEURL",
    "ERF.PRECISE",
    "ERFC.PRECISE",
    "EXPAND",
    "EXPON.DIST",
    "F.DIST",
    "F.DIST.RT",
    "F.INV",
    "F.INV.RT",
    "F.TEST",
    "FIELDVALUE",
    "FILTERXML",
    "FLOOR.MATH",
    "FLOOR.PRECISE",
    "FORECAST.ETS",
    "FORECAST.ETS.CONFINT",
    "FORECAST.ETS.SEASONALITY",
    "FORECAST.ETS.STAT",
    "FORECAST.LINEAR",
    "FORMULATEXT",
    "GAMMA",
    "GAMMA.DIST",
    "GAMMA.INV",
    "GAMMALN.PRECISE",
    "GAUSS",
    "GROUPBY",
    "HSTACK",
    "HYPGEOM.DIST",
    "IFNA",
    "IFS",
    "IMAGE",
    "IMCOSH",
    "IMCOT",
    "IMCSC",
    "IMCSCH",
    "IMSEC",
    "IMSECH",
    "IMSINH",
    "IMTAN",
    "ISFORMULA",
    "ISOMITTED",
    "ISOWEEKNUM",
    "LAMBDA",
    "LET",
    "LOGNORM.DIST",
    "LOGNORM.INV",
    "MAKEARRAY",
    "MAP",
    "MAXIFS",
    "MINIFS",
    "MODE.MULT",
    "MODE.SNGL",
    "MUNIT",
    "NEGBINOM.DIST",
    "NETWORKDAYS.INTL",
    "NORM.DIST",
    "NORM.INV",
    "NORM.S.DIST",
    "NORM.S.INV",
    "NUMBERVALUE",
    "PDURATION",
    "PERCENTILE.EXC",
    "PERCENTILE.INC",
    "PERCENTOF",
    "PERCENTRANK.EXC",
    "PERCENTRANK.INC",
    "PERMUTATIONA",
    "PHI",
    "PIVOTBY",
    "POISSON.DIST",
    "QUARTILE.EXC",
    "QUARTILE.INC",
    "RANDARRAY",
    "RANK.AVG",
    "RANK.EQ",
    "REDUCE",
    "REGEXEXTRACT",
    "REGEXREPLACE",
    "REGEXTEST",
    "RRI",
    "SCAN",
    "SEC",
    "SECH",
    "SEQUENCE",
    "SHEET",
    "SHEETS",
    "SINGLE",
    "SKEW.P",
    "SORTBY",
    "STDEV.P",
    "STDEV.S",
    "SWITCH",
    "T.DIST",
    "T.DIST.2T",
    "T.DIST.RT",
    "T.INV",
    "T.INV.2T",
    "T.TEST",
    "TAKE",
    "TEXTAFTER",
    "TEXTBEFORE",
    "TEXTJOIN",
    "TEXTSPLIT",
    "TOCOL",
    "TOROW",
    "TRIMRANGE",
    "UNICHAR",
    "UNICODE",
    "UNIQUE",
    "VALUETOTEXT",
    "VAR.P",
    "VAR.S",
    "VSTACK",
    "WEBSERVICE",
    "WEIBULL.DIST",
    "WORKDAY.INTL",
    "WRAPCOLS",
    "WRAPROWS",
    "XLOOKUP",
    "XMATCH",
    "XOR",
    "Z.TEST",
    "_xlws.FILTER",
    "_xlws.SORT",
];

/// The name to write for a function (`XLOOKUP` → `_xlfn.XLOOKUP`, `FILTER` → `_xlfn._xlws.FILTER`).
pub fn file_function_name(name: &str) -> Option<String> {
    let u = name.to_ascii_uppercase();
    for f in FUTURE_FUNCTIONS {
        if let Some(ws) = f.strip_prefix("_xlws.") {
            if ws == u {
                return Some(format!("_xlfn._xlws.{u}"));
            }
        } else if *f == u {
            return Some(format!("_xlfn.{u}"));
        }
    }
    None
}

// ---------------------------------------------------------------- units

/// Column width as stored in a file (characters including padding) → model width (px).
pub fn col_width_to_px(w: f64) -> f32 {
    if !w.is_finite() || w <= 0.0 {
        return 0.0;
    }
    let w = w.min(255.0);
    (((256.0 * w + (128.0f64 / 7.0).trunc()) / 256.0) * 7.0).trunc() as f32
}

/// Model width (px) → stored column width.
pub fn px_to_col_width(px: f32) -> f64 {
    let px = (px.max(0.0) as f64).round().min(1790.0);
    let back = |w: f64| col_width_to_px(w) as f64;
    let chars = if px > 12.0 { ((px - 5.0) / 7.0 * 100.0 + 0.5).trunc() / 100.0 } else { px / 12.0 };
    let w = ((chars * 7.0 + 5.0) / 7.0 * 256.0).trunc() / 256.0;
    if back(w) == px {
        return w;
    }
    // Smallest stored width that reads back as `px`.
    let s = (px * 256.0 / 7.0 - 18.0).ceil().max(0.0);
    s / 256.0
}

/// Row height in typographic points → model px.
pub fn pt_to_px(pt: f64) -> f32 {
    if !pt.is_finite() || pt <= 0.0 { 0.0 } else { (pt.min(409.5) * 4.0 / 3.0) as f32 }
}

pub fn px_to_pt(px: f32) -> f64 {
    ((px.max(0.0) as f64) * 0.75 * 100.0).round() / 100.0
}

/// EMU per px at 96 dpi.
pub const EMU_PER_PX: f64 = 9525.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths() {
        assert_eq!(col_width_to_px(9.140625), 64.0);
        assert_eq!(px_to_col_width(64.0), 9.140625);
        for px in 0..400 {
            assert_eq!(col_width_to_px(px_to_col_width(px as f32)), px as f32, "px {px}");
        }
        assert_eq!(pt_to_px(15.0), 20.0);
        assert_eq!(px_to_pt(20.0), 15.0);
    }

    #[test]
    fn functions() {
        assert_eq!(file_function_name("xlookup").as_deref(), Some("_xlfn.XLOOKUP"));
        assert_eq!(file_function_name("forecast.ets.confint").as_deref(), Some("_xlfn.FORECAST.ETS.CONFINT"));
        assert_eq!(file_function_name("FILTER").as_deref(), Some("_xlfn._xlws.FILTER"));
        assert_eq!(file_function_name("PERCENTOF").as_deref(), Some("_xlfn.PERCENTOF"));
        assert_eq!(file_function_name("TRIMRANGE").as_deref(), Some("_xlfn.TRIMRANGE"));
        assert_eq!(file_function_name("SUM"), None);
        assert_eq!(builtin_format(14), Some("m/d/yyyy"));
        assert_eq!(builtin_format(47), Some("mm:ss.0"));
        assert_eq!(builtin_id("0.00%"), Some(10));
        assert_eq!(builtin_id(builtin_format(5).unwrap()), None);
        for id in [5, 14, 20, 22, 37, 40, 43, 45, 46, 47] {
            assert_eq!(builtin_id(builtin_format(id).unwrap()), None, "{id}");
        }
        for id in [0, 1, 2, 3, 4, 9, 10, 11, 12, 13, 48, 49] {
            assert_eq!(builtin_id(builtin_format(id).unwrap()), Some(id), "{id}");
        }
        assert_eq!(paper_name(9), Some("A4"));
        assert_eq!(paper_id("a4"), Some(9));
        // 12/13 are the JIS B4/B5 Japanese Excel uses; 42 is ISO B4.
        assert_eq!(paper_name(12), Some("B4"));
        assert_eq!(paper_id("b5"), Some(13));
        assert_eq!(paper_id("B5 (JIS)"), Some(13));
        assert_eq!(paper_id("B4 (ISO)"), Some(42));
        // The Japanese papers page setup names.
        assert_eq!(paper_name(43), Some("Japanese Postcard"));
        assert_eq!(paper_name(88), Some("B6"));
        for (name, id) in [
            ("Japanese Postcard", 43),
            ("japanese double postcard", 69),
            ("A6", 70),
            ("Japanese Envelope Kaku #2", 71),
            ("kaku #3", 72),
            ("Japanese Envelope #3", 73),
            ("chou #4", 74),
            ("B6 (JIS)", 88),
        ] {
            assert_eq!(paper_id(name), Some(id), "{name}");
        }
        assert_eq!(paper_id("Napkin"), None);
    }
}
