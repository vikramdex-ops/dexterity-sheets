//! People in bulk: the template IT fills in, reading it back (or rows pasted from Excel), checking
//! every row before anything is created, readable passwords, and the sign-in sheet handed out
//! afterwards.

use gridcraft_collab::proto::Role;
use gridcraft_collab::server::{UserRecord, valid_username};
use gridcraft_engine::core::CellRef;
use gridcraft_engine::model::Workbook;
use gridcraft_engine::{DocState, Session, Trust};
use serde_json::json;

/// One person to add, as typed or read from a file. `line` is the row in the file (1-based), or
/// the position in the list for rows typed in the window.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    pub line: usize,
    pub user: String,
    pub display: String,
    pub role: String,
    pub password: String,
}

/// A row ready to create, or what is wrong with it.
#[derive(Clone, Debug, PartialEq)]
pub struct Checked {
    pub user: String,
    pub display: String,
    pub role: Role,
    /// The password to set: the one given, or a generated one.
    pub password: String,
    pub generated: bool,
    pub problems: Vec<String>,
}

pub const HEADERS: [&str; 4] = ["User name", "Full name", "Role", "Password"];

#[derive(Clone, Copy, PartialEq)]
enum Col {
    User,
    Display,
    Role,
    Password,
}

fn column_of(header: &str) -> Option<Col> {
    let h: String = header.trim().to_ascii_lowercase().chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    Some(match h.as_str() {
        "username" | "user" | "login" | "account" | "userid" | "loginname" => Col::User,
        "fullname" | "name" | "displayname" | "display" | "employeename" | "person" => Col::Display,
        "role" | "access" | "accesslevel" | "permission" | "rights" => Col::Role,
        "password" | "initialpassword" | "temporarypassword" | "pass" => Col::Password,
        _ => return None,
    })
}

fn put(row: &mut Row, col: Col, v: String) {
    match col {
        Col::User => row.user = v,
        Col::Display => row.display = v,
        Col::Role => row.role = v,
        Col::Password => row.password = v,
    }
}

/// Reads people from a table of text: the first row names the columns (any order; "User name",
/// "Full name", "Role", "Password" and common variants), every other non-empty row is a person.
/// Without a recognisable header the columns are taken in the template's order.
pub fn from_table(table: &[Vec<String>]) -> Result<Vec<Row>, String> {
    from_table_with(table, false)
}

/// The row that names the columns, if the table starts with one: (its index, the columns).
fn header_of(table: &[Vec<String>]) -> Option<(usize, Vec<Option<Col>>)> {
    let first = table.iter().position(|r| r.iter().any(|c| !c.trim().is_empty()))?;
    let header: Vec<Option<Col>> = table.get(first).map(|r| r.iter().map(|c| column_of(c)).collect()).unwrap_or_default();
    let named = header.iter().flatten().count();
    (named >= 2 || header.iter().flatten().any(|c| *c == Col::User || *c == Col::Display)).then_some((first, header))
}

/// `need_header`: a table without column names is not read (a notes sheet next to the list).
fn from_table_with(table: &[Vec<String>], need_header: bool) -> Result<Vec<Row>, String> {
    let Some(first) = table.iter().position(|r| r.iter().any(|c| !c.trim().is_empty())) else {
        return Err("The file has no rows.".into());
    };
    let (cols, start): (Vec<Option<Col>>, usize) = match header_of(table) {
        Some((at, header)) => (header, at + 1),
        None if need_header => return Err("No \"User name\" or \"Full name\" column was found. Start from the template (Download template).".into()),
        None => (vec![Some(Col::User), Some(Col::Display), Some(Col::Role), Some(Col::Password)], first),
    };
    if !cols.iter().flatten().any(|c| *c == Col::User || *c == Col::Display) {
        return Err("No \"User name\" or \"Full name\" column was found. Start from the template (Download template).".into());
    }
    let mut out = Vec::new();
    for (i, cells) in table.iter().enumerate().skip(start) {
        let mut row = Row { line: i + 1, ..Row::default() };
        for (c, v) in cells.iter().enumerate() {
            if let Some(Some(col)) = cols.get(c) {
                put(&mut row, *col, v.trim().to_string());
            }
        }
        if row.user.is_empty() && row.display.is_empty() && row.role.is_empty() && row.password.is_empty() {
            continue;
        }
        out.push(row);
    }
    if out.is_empty() {
        return Err("The file names the columns but has nobody under them.".into());
    }
    Ok(out)
}

/// People from the sheet that names its columns (the template's "People" sheet); a workbook with
/// no such sheet is read from its first sheet in the template's column order.
pub fn from_workbook(wb: &Workbook) -> Result<Vec<Row>, String> {
    let tables: Vec<Vec<Vec<String>>> = wb.sheets.iter().filter_map(|sh| sheet_table(wb, sh)).collect();
    if let Some(t) = tables.iter().find(|t| header_of(t).is_some()) {
        return from_table_with(t, true);
    }
    match tables.first() {
        Some(t) => from_table_with(t, false),
        None => Err("The workbook is empty.".into()),
    }
}

fn sheet_table(wb: &Workbook, sh: &gridcraft_engine::model::Sheet) -> Option<Vec<Vec<String>>> {
    let used = sh.used_range()?;
    let rows = (used.end.row - used.start.row + 1).min(10_000);
    let cols = (used.end.col - used.start.col + 1).min(50);
    Some(
        (0..rows)
            .map(|r| (0..cols).map(|c| gridcraft_engine::display::value_text(wb, sh, CellRef::new(used.start.row + r, used.start.col + c))).collect())
            .collect(),
    )
}

/// Reads an .xlsx, .xls, .ods, .csv or .tsv file of people.
pub fn from_file(name: &str, bytes: &[u8]) -> Result<Vec<Row>, String> {
    let (wb, _) = gridcraft_engine::io::open_bytes(name, bytes).map_err(|e| format!("{name} could not be read: {e}"))?;
    from_workbook(&wb)
}

/// Rows copied from Excel (tab-separated) or typed one per line.
pub fn from_pasted(text: &str) -> Result<Vec<Row>, String> {
    let table: Vec<Vec<String>> = text
        .lines()
        .map(|l| if l.contains('\t') { l.split('\t').map(str::to_string).collect() } else { l.split(',').map(str::to_string).collect() })
        .collect();
    from_table(&table)
}

/// "Anita Rao" → "anita.rao".
pub fn user_from_name(display: &str) -> String {
    let parts: Vec<String> = display
        .split_whitespace()
        .map(|w| w.chars().filter(char::is_ascii_alphanumeric).collect::<String>().to_ascii_lowercase())
        .filter(|w| !w.is_empty())
        .collect();
    parts.join(".").chars().take(64).collect()
}

pub fn role_of(text: &str) -> Option<Role> {
    match text.trim().to_ascii_lowercase().as_str() {
        "" | "editor" | "edit" | "user" | "member" => Some(Role::Editor),
        "admin" | "administrator" => Some(Role::Admin),
        "viewer" | "view" | "read" | "read only" | "readonly" | "view only" => Some(Role::Viewer),
        _ => None,
    }
}

pub fn role_name(r: Role) -> &'static str {
    match r {
        Role::Admin => "Administrator",
        Role::Editor => "Editor",
        Role::Viewer => "Viewer (read only)",
    }
}

/// A password people can read out and type: 10 characters without look-alikes (0/O, 1/l/I),
/// always with a digit.
pub fn new_password() -> String {
    const LETTERS: &[u8] = b"abcdefghjkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ";
    const DIGITS: &[u8] = b"23456789";
    let bytes = gridcraft_collab::auth::random_bytes::<10>();
    let mut out: String = bytes.iter().take(7).map(|b| char::from(LETTERS[usize::from(*b) % LETTERS.len()])).collect();
    out.push('-');
    for b in bytes.iter().skip(7).take(2) {
        out.push(char::from(DIGITS[usize::from(*b) % DIGITS.len()]));
    }
    out
}

/// Checks every row against the rules and the accounts that already exist. Nothing is created
/// unless every row is right.
pub fn check(rows: &[Row], existing: &[UserRecord]) -> Vec<Checked> {
    let mut seen: Vec<String> = Vec::new();
    rows.iter()
        .map(|r| {
            let mut problems = Vec::new();
            let mut user = r.user.trim().to_ascii_lowercase();
            if user.is_empty() {
                user = user_from_name(&r.display);
            }
            if user.is_empty() {
                problems.push("needs a user name or a full name".to_string());
            } else if !valid_username(&user) {
                problems.push(format!("\"{user}\": user names use letters, digits, '.', '_' and '-' (no spaces)"));
            } else if existing.iter().any(|u| u.user == user) {
                problems.push(format!("{user} already has an account (use Reset password to change it)"));
            } else if seen.contains(&user) {
                problems.push(format!("{user} is in the list twice"));
            }
            seen.push(user.clone());
            let role = role_of(&r.role).unwrap_or_else(|| {
                problems.push(format!("role \"{}\" should be Administrator, Editor or Viewer", r.role.trim()));
                Role::Editor
            });
            let (password, generated) = if r.password.is_empty() { (new_password(), true) } else { (r.password.clone(), false) };
            if !generated && password.chars().count() < 6 {
                problems.push("the password needs at least 6 characters (or leave it empty for one to be made)".to_string());
            }
            let display = if r.display.trim().is_empty() { user.clone() } else { r.display.trim().to_string() };
            Checked { user, display, role, password, generated, problems }
        })
        .collect()
}

fn session_with(sheet_name: &str) -> Session {
    let mut s = Session::new();
    let mut wb = Workbook::new();
    if wb.sheets.is_empty() {
        wb.sheets.push(std::sync::Arc::new(gridcraft_engine::model::Sheet::new("Sheet1")));
    }
    s.add_document(DocState::new(wb, None, "people".into(), Trust::Local));
    let _ = s.execute("sheet.rename", json!({"name": sheet_name}));
    s
}

fn header_row(s: &mut Session, headers: &[&str], widths: &[f64]) -> Result<(), String> {
    for (i, h) in headers.iter().enumerate() {
        let col = char::from(b'A' + u8::try_from(i).unwrap_or(0));
        s.execute("cell.set", json!({"cell": format!("{col}1"), "input": h})).map_err(|e| e.to_string())?;
        if let Some(w) = widths.get(i) {
            let _ = s.execute("home.columnWidth", json!({"range": format!("{col}1"), "width": w}));
        }
    }
    let last = char::from(b'A' + u8::try_from(headers.len().saturating_sub(1)).unwrap_or(0));
    let _ = s.execute("home.bold", json!({"range": format!("A1:{last}1"), "on": true}));
    let _ = s.execute("home.fillColor", json!({"range": format!("A1:{last}1"), "color": "#DCE6F4"}));
    let _ = s.execute("view.freezeTopRow", json!({}));
    Ok(())
}

fn workbook_bytes(s: &Session, name: &str) -> Result<Vec<u8>, String> {
    let doc = s.active().ok_or("no workbook")?;
    gridcraft_engine::io::save_bytes(&doc.wb, name).map_err(|e| e.to_string())
}

/// The template: a "People" sheet with the four columns (Role offers a drop-down) and a "How to
/// fill this in" sheet. No example rows, so nothing made up can be imported by mistake.
pub fn template_xlsx() -> Result<Vec<u8>, String> {
    let mut s = session_with("People");
    header_row(&mut s, &HEADERS, &[130.0, 190.0, 110.0, 130.0])?;
    let _ = s.execute(
        "data.validation",
        json!({"range": "C2:C1000", "type": "list", "formula1": "\"Administrator,Editor,Viewer\"", "dropdown": true,
               "errorStyle": "stop", "errorTitle": "Role", "errorMessage": "Choose Administrator, Editor or Viewer."}),
    );
    let _ = s.execute("home.insertSheet", json!({"name": "How to fill this in"}));
    let help = [
        "One person per row on the People sheet. Keep the first row (the column names) as it is.",
        "User name: what they type to sign in. Letters, digits, '.', '_' or '-', no spaces (anita.rao). Empty: made from the full name.",
        "Full name: shown next to their edits and cursor (Anita Rao).",
        "Role: Editor (edit, create and share workbooks), Viewer (read only) or Administrator (also manages accounts). Empty: Editor.",
        "Password: at least 6 characters. Empty: a readable one is made for them.",
        "Then in Dexterity Sheets Server Manager: People > Add many > Import from file, check the list, and click Create.",
        "After creating, save the sign-in sheet to hand out the passwords, and delete it once everyone has signed in.",
    ];
    for (i, line) in help.iter().enumerate() {
        let _ = s.execute("cell.set", json!({"cell": format!("A{}", i + 1), "input": line}));
    }
    let _ = s.execute("home.columnWidth", json!({"range": "A1", "width": 900}));
    let _ = s.execute("sheet.activate", json!({"sheet": "People"}));
    workbook_bytes(&s, "Dexterity Sheets people template.xlsx")
}

/// The sign-in sheet for the people just created: who they are, their password and the server.
pub fn sign_in_sheet_xlsx(created: &[Checked], server: &str) -> Result<Vec<u8>, String> {
    let mut s = session_with("Sign-in details");
    header_row(&mut s, &["User name", "Full name", "Role", "Password", "Server"], &[130.0, 190.0, 140.0, 130.0, 200.0])?;
    for (i, c) in created.iter().enumerate() {
        let r = i + 2;
        for (col, v) in [("A", c.user.as_str()), ("B", c.display.as_str()), ("C", role_name(c.role)), ("D", c.password.as_str()), ("E", server)] {
            // A leading apostrophe keeps a password like "123456" as text.
            s.execute("cell.set", json!({"cell": format!("{col}{r}"), "input": format!("'{v}")})).map_err(|e| e.to_string())?;
        }
    }
    workbook_bytes(&s, "sign-in.xlsx")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(rows: &[&[&str]]) -> Vec<Vec<String>> {
        rows.iter().map(|r| r.iter().map(|c| c.to_string()).collect()).collect()
    }

    #[test]
    fn headers_in_any_order_and_variants() {
        let rows =
            from_table(&t(&[&["Role", "Name", "Login"], &["viewer", "Anita Rao", "anita"], &["", "", ""], &["", "Rahul Mehta", ""]])).expect("rows");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0], Row { line: 2, user: "anita".into(), display: "Anita Rao".into(), role: "viewer".into(), password: String::new() });
        assert_eq!(rows[1].line, 4);
    }

    #[test]
    fn no_header_takes_the_template_order() {
        let rows = from_pasted("anita\tAnita Rao\tEditor\tpipe-123\nrahul\tRahul Mehta").expect("rows");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].password, "pipe-123");
        assert_eq!(rows[1].display, "Rahul Mehta");
    }

    #[test]
    fn checks_every_rule() {
        let rows = vec![
            Row { line: 2, user: String::new(), display: "Anita Rao".into(), role: String::new(), password: String::new() },
            Row { line: 3, user: "anita.rao".into(), display: String::new(), role: "boss".into(), password: "123".into() },
            Row { line: 4, user: "Has Space".into(), ..Row::default() },
        ];
        let c = check(&rows, &[]);
        assert_eq!(c[0].user, "anita.rao");
        assert!(c[0].problems.is_empty(), "{:?}", c[0].problems);
        assert!(c[0].generated && c[0].password.len() == 10);
        assert_eq!(c[0].role, Role::Editor);
        assert_eq!(c[1].problems.len(), 3, "{:?}", c[1].problems);
        assert_eq!(c[2].problems.len(), 1);
    }

    #[test]
    fn passwords_are_readable_and_differ() {
        let a = new_password();
        assert_eq!(a.len(), 10);
        assert!(a.chars().any(|c| c.is_ascii_digit()));
        assert!(!a.contains(['0', 'O', '1', 'l', 'I']));
        assert_ne!(a, new_password());
    }

    #[test]
    fn template_reads_back_as_an_empty_list_and_a_filled_one_imports() {
        let bytes = template_xlsx().expect("template");
        assert!(from_file("t.xlsx", &bytes).is_err(), "no example rows to import by mistake");
        let (wb, _) = gridcraft_engine::io::open_bytes("t.xlsx", &bytes).expect("open");
        let mut s = Session::new();
        s.add_document(DocState::new(wb, None, "t".into(), Trust::Local));
        s.execute("cell.set", json!({"cell": "B2", "input": "Rahul Mehta"})).expect("set");
        s.execute("cell.set", json!({"cell": "C2", "input": "Viewer"})).expect("set");
        let filled = gridcraft_engine::io::save_bytes(&s.active().expect("doc").wb, "f.xlsx").expect("save");
        let rows = from_file("f.xlsx", &filled).expect("rows");
        let c = check(&rows, &[]);
        assert_eq!((c[0].user.as_str(), c[0].role), ("rahul.mehta", Role::Viewer));
    }

    #[test]
    fn sign_in_sheet_keeps_passwords_as_text() {
        let c = check(&[Row { line: 1, user: "a1".into(), display: "A".into(), role: String::new(), password: "123456".into() }], &[]);
        let bytes = sign_in_sheet_xlsx(&c, "DDS-SERVER").expect("sheet");
        let (wb, _) = gridcraft_engine::io::open_bytes("s.xlsx", &bytes).expect("open");
        let sh = wb.sheets.first().expect("sheet");
        assert_eq!(gridcraft_engine::display::value_text(&wb, sh, CellRef::new(1, 3)), "123456");
    }
}
