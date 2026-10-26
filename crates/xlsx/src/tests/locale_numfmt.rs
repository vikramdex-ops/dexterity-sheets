//! Number formats whose meaning depends on the reader's locale are written with their code: the
//! currency formats 5–8, the dates and times 14–22, the accounting formats 37–44 and 45–47.
//! Written as a bare built-in id, LibreOffice and Excel in other locales show their own version (a
//! 44 cell holding -0.14 reads `$ (0.14)` in US Excel but `- 0.14` in LibreOffice; en-IN Excel
//! shows id 20 `h:mm` as `hh:mm` and id 43 without parentheses).

use std::io::Read;
use std::sync::Arc;

use gridcraft_core::{CellRef, Value};
use gridcraft_model::*;

use crate::tables::builtin_format;
use crate::{read_xlsx, write_xlsx};

fn styles_xml(bytes: &[u8]) -> String {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut s = String::new();
    zip.by_name("xl/styles.xml").unwrap().read_to_string(&mut s).unwrap();
    s
}

/// `bytes` with the part `name` replaced by `content`.
fn replace_part(bytes: &[u8], name: &str, content: &str) -> Vec<u8> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut parts: Vec<(String, Vec<u8>)> = vec![];
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).unwrap();
        let mut b = vec![];
        f.read_to_end(&mut b).unwrap();
        let n = f.name().to_string();
        let b = if n == name { content.as_bytes().to_vec() } else { b };
        parts.push((n, b));
    }
    let refs: Vec<(&str, &[u8])> = parts.iter().map(|(n, b)| (n.as_str(), b.as_slice())).collect();
    crate::tests::make_zip(&refs)
}

/// Writes one cell formatted with `code`; returns the `numFmtId` its cell format points at, the
/// `styles.xml` and the code read back.
fn round_trip(code: &str) -> (u32, String, String) {
    let mut wb = Workbook::new();
    let style = wb.styles.derive(StyleId::DEFAULT, |s| s.num_fmt = NumFmt::new(code));
    let mut sheet = Sheet::new("Sheet1");
    sheet.cells.set(CellRef::new(0, 0), Cell { value: Value::Number(-0.14), formula: None, style });
    wb.sheets = vec![Arc::new(sheet)];
    let bytes = write_xlsx(&wb).unwrap();
    let xml = styles_xml(&bytes);
    let xfs = &xml[xml.find("<cellXfs").unwrap()..];
    let id = xfs
        .match_indices("numFmtId=\"")
        .map(|(i, m)| xfs[i + m.len()..].split('"').next().unwrap().parse::<u32>().unwrap())
        .find(|&id| id != 0)
        .unwrap();
    let (back, _) = read_xlsx(&bytes).unwrap();
    let st = back.styles.get(back.sheet(0).unwrap().cell(CellRef::new(0, 0)).unwrap().style);
    (id, xml, st.num_fmt.as_str().to_string())
}

#[test]
fn currency_and_accounting_formats_are_written_with_their_code() {
    for builtin in [5, 6, 7, 8, 14, 15, 16, 17, 18, 19, 20, 21, 22, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47] {
        let code = builtin_format(builtin).unwrap();
        let (id, xml, back) = round_trip(code);
        assert!(id >= 164, "format {builtin} written as built-in id {id}");
        assert!(xml.contains(&format!("<numFmt numFmtId=\"{id}\"")), "format {builtin}: no <numFmt> for id {id}");
        assert_eq!(back, code, "format {builtin}");
    }
}

#[test]
fn locale_independent_built_ins_keep_their_id() {
    for builtin in [1, 2, 3, 4, 9, 10, 11, 12, 13, 48, 49] {
        let (id, _, back) = round_trip(builtin_format(builtin).unwrap());
        assert_eq!(id, builtin);
        assert_eq!(back, builtin_format(builtin).unwrap());
    }
}

/// A file naming a locale-dependent id shows it the way Excel does on this computer: under
/// en-IN, 40 has a minus sign (not parentheses), 14 is dd-mm-yyyy, 20 is hh:mm.
#[test]
fn bare_locale_ids_read_as_the_current_locale_shows_them() {
    use gridcraft_core::locale::{Locale, scoped};
    let styles = |id: u32| {
        format!(
            "<styleSheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><fonts count=\"1\"><font/></fonts><fills count=\"1\"><fill/></fills><borders count=\"1\"><border/></borders><cellXfs count=\"2\"><xf numFmtId=\"0\"/><xf numFmtId=\"{id}\" applyNumberFormat=\"1\"/></cellXfs></styleSheet>"
        )
    };
    let read = |id: u32| {
        let mut wb = Workbook::new();
        let style = wb.styles.derive(StyleId::DEFAULT, |s| s.num_fmt = NumFmt::new("0.000"));
        let mut sheet = Sheet::new("Sheet1");
        sheet.cells.set(CellRef::new(0, 0), Cell { value: Value::Number(1.0), formula: None, style });
        wb.sheets = vec![Arc::new(sheet)];
        let bytes = write_xlsx(&wb).unwrap();
        let bytes = replace_part(&bytes, "xl/styles.xml", &styles(id));
        let (back, _) = read_xlsx(&bytes).unwrap();
        let cell = back.sheet(0).unwrap().cell(CellRef::new(0, 0)).unwrap().style;
        back.styles.get(cell).num_fmt.as_str().to_string()
    };
    assert_eq!(read(40), "#,##0.00;[Red](#,##0.00)");
    assert_eq!(read(14), "m/d/yyyy");
    {
        let _l = scoped(Locale::en_in());
        assert_eq!(read(40), "#,##0.00;[Red]\\-#,##0.00");
        assert_eq!(read(14), "dd/mm/yyyy");
        assert_eq!(read(20), "hh:mm");
        assert_eq!(read(47), "mm:ss.0");
        assert_eq!(read(3), "#,##0");
        // Saving does not depend on the locale: the code read is written as itself.
        let (id, xml, back) = round_trip("dd/mm/yyyy");
        assert!(id >= 164 && xml.contains("formatCode=\"dd/mm/yyyy\""), "{xml}");
        assert_eq!(back, "dd/mm/yyyy");
    }
    let (_, xml_us, _) = round_trip("h:mm");
    let _l = scoped(Locale::en_in());
    let (_, xml_in, _) = round_trip("h:mm");
    assert_eq!(xml_us, xml_in);
}
