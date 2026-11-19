//! The office suite around Dexterity Sheets (Dexterity Sheets only): Dexterity Connect, Slate,
//! Sleuth, Datum and Project Opener. Each integration stays out of the way until it is wanted and
//! shows only when its app is installed on this PC.
//!
//! - **Links to a cell:** `dexterity://open/<server>/<book>?at=<Sheet>!<Cell or range>` opens
//!   the shared workbook and goes to that place. The sheet is percent-encoded; an Excel-style
//!   quoted sheet (`'Line List'!B4`) is read too. *Copy Link to This Cell* in the cell menu.
//! - **Connect:** *Share to Connect…* (title-bar Share menu) and *Discuss This Row in Connect*
//!   (cell menu) open Connect's share window (`dexconnect://share?...`) with the workbook, the
//!   row's `header: value` pairs and a link back to the cell. Connect asks before it sends.
//! - **Notes:** the *Notes* button at the bottom right opens the workbook's notes file in Slate
//!   (or in a small pane here when Slate is not installed). *Add Note for This Row* appends a
//!   heading with the row and a link to it.
//! - **Office links on cells:** *Find in Sleuth* and *Open in Project Opener* in the cell menu;
//!   links in cells with the suite's schemes (`dexterity`, `dexconnect`, `sleuth`, `datum`,
//!   `projectopener`) are handed to Windows. No other scheme is opened by this module.
//!
//! Which apps are installed is read once, off the UI thread, from the registry (`HKCR\<scheme>`
//! with a `URL Protocol` value) and from Slate's install folders. `docs/dexterity-suite.md`.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

use egui::{RichText, Ui};
use gridcraft_collab::proto;
use gridcraft_engine::core::CellRef;
use serde_json::{Value as Json, json};

use crate::SheetApp;

/// The URL schemes a link in a cell may open outside the workbook (besides http, https and
/// mailto, which upstream's Open Hyperlink prompt handles).
pub const SCHEMES: [&str; 5] = ["dexterity", "dexconnect", "sleuth", "datum", "projectopener"];

/// Most `header: value` pairs a row summary carries, and the most characters of each value.
const ROW_PAIRS: usize = 12;
const VALUE_CHARS: usize = 80;
/// Connect cuts the whole message at 8,000 characters; a summary stays well inside it.
const SUMMARY_CHARS: usize = 1500;
/// Sleuth reads at most 200 characters of a search.
const SEARCH_CHARS: usize = 200;
/// Notes files and Project Opener's workspace bigger than this are not read.
const FILE_CAP: u64 = 8 * 1024 * 1024;

/// The office apps found on this PC.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Installed {
    /// `dexconnect:` is registered.
    pub connect: bool,
    pub sleuth: bool,
    pub datum: bool,
    pub project_opener: bool,
    /// Where `slate.exe` is.
    pub slate: Option<PathBuf>,
    /// Project Opener's workspace file (its projects, to resolve a project number in a cell).
    pub workspace: Option<PathBuf>,
}

/// A project in Project Opener's workspace: (id, name, job number).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub job_number: String,
}

/// The in-app notes pane, used when Slate is not installed.
#[derive(Default)]
pub struct NotesPane {
    pub path: PathBuf,
    pub title: String,
    pub text: String,
    /// What the file held when it was read (or last written): a save never overwrites a change
    /// another program made since.
    disk: String,
    pub error: Option<String>,
}

#[derive(Default)]
pub struct Suite {
    installed: Option<Installed>,
    probe: Option<Receiver<Installed>>,
    projects: Vec<Project>,
    projects_stamp: Option<std::time::SystemTime>,
    projects_checked: Option<f64>,
    pub notes: Option<NotesPane>,
    /// Every address handed to Windows (or, outside the desktop app, that would have been).
    pub opened: Vec<String>,
}

/// Sets which office apps count as installed, instead of looking (tests, other hosts).
pub fn set_installed(app: &mut SheetApp, i: Installed) {
    app.collab.suite.installed = Some(i);
    app.collab.suite.probe = None;
    app.collab.suite.projects_stamp = None;
    app.collab.suite.projects_checked = None;
}

/// The office apps found so far (nothing until the background look has finished).
pub fn installed(app: &mut SheetApp) -> Installed {
    let s = &mut app.collab.suite;
    if let Some(rx) = &s.probe
        && let Ok(i) = rx.try_recv()
    {
        s.installed = Some(i);
        s.probe = None;
    }
    if s.installed.is_none() && s.probe.is_none() {
        if crate::dex_ui::desktop() {
            s.probe = start_probe(app.collab_ctx.clone());
        }
        if s.probe.is_none() {
            s.installed = Some(Installed::default());
        }
    }
    app.collab.suite.installed.clone().unwrap_or_default()
}

#[cfg(windows)]
fn start_probe(ctx: Option<egui::Context>) -> Option<Receiver<Installed>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("dexsheets-suite".into())
        .spawn(move || {
            let i = probe();
            let _ = tx.send(i);
            if let Some(c) = ctx {
                c.request_repaint();
            }
        })
        .ok()?;
    Some(rx)
}

#[cfg(not(windows))]
fn start_probe(_ctx: Option<egui::Context>) -> Option<Receiver<Installed>> {
    None
}

/// For each scheme: `HKCR\<scheme>` has a `URL Protocol` value, so the scheme opens an app (HKCR
/// is HKCU's and HKLM's classes together, so per-user and per-machine installs both count). The
/// queries run at the same time: each `reg.exe` takes a moment to start.
#[cfg(windows)]
fn registered(schemes: &[&str]) -> Vec<bool> {
    use std::os::windows::process::CommandExt;
    let children: Vec<_> = schemes
        .iter()
        .map(|s| {
            std::process::Command::new("reg")
                .args(["query", &format!("HKCR\\{s}"), "/v", "URL Protocol"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .creation_flags(0x0800_0000)
                .spawn()
                .ok()
        })
        .collect();
    children.into_iter().map(|c| c.and_then(|mut c| c.wait().ok()).is_some_and(|s| s.success())).collect()
}

#[cfg(windows)]
fn probe() -> Installed {
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
    // Where Slate's installer puts it (per user), or a per-machine copy; the same places
    // Dexterity Connect looks.
    let slate = [env("LOCALAPPDATA").map(|p| p.join("Programs").join("Slate")), env("ProgramFiles").map(|p| p.join("Slate"))]
        .into_iter()
        .flatten()
        .map(|d| d.join("slate.exe"))
        .find(|p| p.is_file());
    let r = registered(&["dexconnect", "sleuth", "datum", "projectopener"]);
    let on = |i: usize| r.get(i).copied().unwrap_or(false);
    let project_opener = on(3);
    let workspace = env("APPDATA").map(|p| p.join("DexterityDesign").join("ProjectOpener").join("workspace.json")).filter(|p| p.is_file());
    Installed { connect: on(0), sleuth: on(1), datum: on(2), project_opener, slate, workspace: workspace.filter(|_| project_opener) }
}

// ---------------------------------------------------------------------------------------------
// Links

/// Percent-encodes everything but RFC 3986's unreserved characters.
pub fn encode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { char::from(b).to_string() } else { format!("%{b:02X}") }).collect()
}

/// Decodes `%XX` (and nothing else: `+` stays `+`). `None` for a damaged escape or bad UTF-8.
pub fn decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut raw = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        if b == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            raw.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            raw.push(b);
            i += 1;
        }
    }
    String::from_utf8(raw).ok()
}

/// `dexterity://open/<server>/<book>?at=<Sheet>!<Cell or range>`.
pub fn link_at(server: &str, book: &str, sheet: &str, at: &str) -> String {
    if at.is_empty() || sheet.is_empty() {
        return proto::link(server, book);
    }
    format!("{}?at={}!{}", proto::link(server, book), encode(sheet), at)
}

/// A cell or a range as a link carries it: `B4`, `$B$4`, `A1:C9`.
fn valid_place(s: &str) -> bool {
    let mut parts = s.split(':');
    let ok = |p: &str| {
        let p = p.replace('$', "");
        CellRef::parse(&p).is_some() && p.len() <= 12
    };
    match (parts.next(), parts.next(), parts.next()) {
        (Some(a), None, _) => ok(a),
        (Some(a), Some(b), None) => ok(a) && ok(b),
        _ => false,
    }
}

/// The `?at=` part of a link: `(sheet, cell or range)`. Reads a percent-encoded sheet
/// (`Line%20List!B4`) and an Excel-style quoted one (`'Line List'!B4`, `'O''Brien'!A1`).
pub fn parse_at(at: &str) -> Option<(String, String)> {
    let at = decode(at.trim().trim_end_matches('/'))?;
    let (sheet, place) = at.rsplit_once('!')?;
    let sheet = match sheet.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
        Some(q) => q.replace("''", "'"),
        None => sheet.to_string(),
    };
    let ok =
        !sheet.is_empty() && sheet.chars().count() <= 31 && !sheet.chars().any(|c| c.is_control() || "[]*?/\\:".contains(c)) && valid_place(place);
    ok.then(|| (sheet, place.to_ascii_uppercase()))
}

/// `(server, book, (sheet, place))` of a `dexterity://open/...` link. A link whose `?at=` is
/// damaged still opens the workbook.
pub fn parse_link(s: &str) -> Option<(String, String, Option<proto::LinkTarget>)> {
    let s = s.trim();
    let (base, query) = match s.split_once('?') {
        Some((b, q)) => (b, Some(q)),
        None => (s, None),
    };
    let (server, book) = proto::parse_link(base)?;
    let at = query.and_then(|q| q.split('&').find_map(|kv| kv.strip_prefix("at="))).and_then(parse_at);
    Some((server, book, at))
}

/// Connect's share window: `dexconnect://share?from=…&title=…&text=…&link=…`.
pub fn connect_share(title: &str, text: &str, link: &str) -> String {
    let mut u = format!("dexconnect://share?from={}", encode("Dexterity Sheets"));
    for (k, v) in [("title", title), ("text", text), ("link", link)] {
        if !v.is_empty() {
            u.push_str(&format!("&{k}={}", encode(v)));
        }
    }
    u
}

/// `sleuth://search?q=…`, the text trimmed and cut to what Sleuth reads.
pub fn sleuth_search(text: &str) -> String {
    let t: String = text.trim().chars().filter(|c| !c.is_control()).take(SEARCH_CHARS).collect();
    format!("sleuth://search?q={}", encode(&t))
}

/// The scheme of a link to one of the office apps, lower case. `None` for any other address.
pub fn suite_scheme(target: &str) -> Option<&'static str> {
    let t = target.trim();
    if t.chars().any(char::is_control) {
        return None;
    }
    let (scheme, rest) = t.split_once(':')?;
    let lower = scheme.to_ascii_lowercase();
    SCHEMES.iter().copied().find(|s| *s == lower).filter(|_| rest.len() > 2)
}

/// The project a cell's text names in Project Opener: its job number or its whole name (case
/// ignored), or the one project whose name starts with the text and a space ("P12513" for
/// "P12513 As-built isometrics"). `None` when nothing or more than one project fits.
pub fn resolve_project<'a>(projects: &'a [Project], text: &str) -> Option<&'a Project> {
    let t = text.trim();
    if t.is_empty() || t.chars().count() > 200 {
        return None;
    }
    let exact: Vec<&Project> = projects
        .iter()
        .filter(|p| p.name.eq_ignore_ascii_case(t) || (!p.job_number.is_empty() && p.job_number.trim().eq_ignore_ascii_case(t)))
        .collect();
    if let [p] = exact.as_slice() {
        return Some(p);
    }
    if !exact.is_empty() {
        return None;
    }
    let lower = format!("{} ", t.to_lowercase());
    let starts: Vec<&Project> = projects.iter().filter(|p| p.name.to_lowercase().starts_with(&lower)).collect();
    match starts.as_slice() {
        [p] => Some(p),
        _ => None,
    }
}

/// The projects in Project Opener's `workspace.json`.
pub fn read_projects(bytes: &[u8]) -> Vec<Project> {
    let Ok(v) = serde_json::from_slice::<Json>(bytes) else { return Vec::new() };
    let s = |p: &Json, k: &str| p.get(k).and_then(Json::as_str).unwrap_or("").to_string();
    v.get("projects")
        .and_then(Json::as_array)
        .map(|a| {
            a.iter()
                .filter(|p| p.is_object())
                .map(|p| Project { id: s(p, "id"), name: s(p, "name"), job_number: s(p, "job_number") })
                .filter(|p| !p.id.is_empty() && !p.name.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// The projects, re-read when the workspace file changed (looked at every few seconds at most).
fn projects(app: &mut SheetApp) -> Vec<Project> {
    let i = installed(app);
    let s = &mut app.collab.suite;
    let Some(ws) = i.workspace.filter(|_| i.project_opener) else { return Vec::new() };
    let now = crate::clock::monotonic_ms();
    if s.projects_checked.is_none_or(|at| now - at >= 3000.0) {
        s.projects_checked = Some(now);
        let meta = std::fs::metadata(&ws).ok();
        let stamp = meta.as_ref().and_then(|m| m.modified().ok());
        if stamp != s.projects_stamp || s.projects_stamp.is_none() {
            s.projects_stamp = stamp;
            s.projects = match meta {
                Some(m) if m.len() <= FILE_CAP => std::fs::read(&ws).map(|b| read_projects(&b)).unwrap_or_default(),
                _ => Vec::new(),
            };
        }
    }
    s.projects.clone()
}

/// Hands a suite link to Windows, which starts the app that registered its scheme. Refuses
/// every other address. Outside the desktop app the address is only recorded.
fn launch(app: &mut SheetApp, url: &str) -> Result<Json, String> {
    if suite_scheme(url).is_none() {
        return Err("Only links to the Dexterity office apps are opened here.".into());
    }
    app.collab.suite.opened.push(url.to_string());
    #[cfg(windows)]
    if crate::dex_ui::desktop() {
        use std::os::windows::process::CommandExt;
        // Explorer opens a protocol address with its registered app; no command shell reads it.
        std::process::Command::new("explorer.exe")
            .arg(url)
            .creation_flags(0x0800_0000)
            .spawn()
            .map_err(|e| format!("Windows could not open the link: {e}"))?;
    }
    Ok(json!({"url": url}))
}

// ---------------------------------------------------------------------------------------------
// Where things are

/// The active workbook's place: the server link base (when shared), its file (when saved or
/// shared from a file here), its name and the sheet.
struct Place {
    shared: Option<(String, String)>,
    file: Option<String>,
    name: String,
    sheet: String,
}

fn place(app: &SheetApp) -> Option<Place> {
    let d = app.session.active()?;
    let sheet = d.wb.active().map(|s| s.name.clone()).unwrap_or_default();
    let sd = app.collab.shared(d.uid);
    let shared = sd.map(|sd| (app.collab.server_address(), sd.book.id.clone()));
    let file = d.path.clone().or_else(|| crate::collab_ui::linked_file(app, d.uid).map(|l| l.path.clone()));
    let stem = |s: &str| Path::new(s).file_stem().and_then(|s| s.to_str()).map(str::to_string);
    let name = match (sd, &file) {
        (Some(sd), _) if !sd.book.name.is_empty() => sd.book.name.clone(),
        (_, Some(f)) => stem(f).unwrap_or_else(|| d.display_title()),
        _ => {
            let t = d.display_title();
            stem(&t).unwrap_or(t)
        }
    };
    Some(Place { shared, file, name, sheet })
}

/// The selection as a link carries it: the active cell, or the one range selected.
fn selected_place(app: &SheetApp) -> Option<String> {
    let d = app.session.active()?;
    match d.selection.ranges.as_slice() {
        [r] if r.start != r.end => Some(r.a1()),
        _ => Some(d.selection.active.a1()),
    }
}

/// A link to `at` on the active sheet: the server link when the workbook is shared, otherwise
/// its file (people who can reach that folder can open it; there is no cell in that link).
fn link_for(p: &Place, at: &str) -> Option<(String, bool)> {
    match (&p.shared, &p.file) {
        (Some((server, book)), _) => Some((link_at(server, book, &p.sheet, at), true)),
        (None, Some(f)) => Some((f.clone(), false)),
        (None, None) => None,
    }
}

/// Excel's way of writing a place on a sheet: `'Line List'!B4`.
fn sheet_place(sheet: &str, at: &str) -> String {
    format!("{}!{at}", gridcraft_engine::formula::quote_sheet(sheet))
}

/// The active row as `header: value` pairs: the headers are the first used row of the sheet (or
/// the row's table's header row); a row without headers names its columns.
pub fn row_summary(app: &SheetApp) -> String {
    let Some(d) = app.session.active() else { return String::new() };
    let Some(sh) = d.wb.active() else { return String::new() };
    let row = d.selection.active.row;
    let Some(used) = sh.used_range() else { return String::new() };
    let header_row = sh.table_at(d.selection.active).map(|t| t.range.start.row).unwrap_or(used.start.row);
    let mut out: Vec<String> = Vec::new();
    let mut len = 0;
    for col in used.start.col..=used.end.col.min(used.start.col.saturating_add(255)) {
        let c = CellRef::new(row, col);
        let value = gridcraft_engine::display::cell_text(&d.wb, sh, c);
        let value = value.split_whitespace().collect::<Vec<_>>().join(" ");
        if value.is_empty() {
            continue;
        }
        let value: String = if value.chars().count() > VALUE_CHARS { value.chars().take(VALUE_CHARS - 1).chain(['…']).collect() } else { value };
        let header = if row > header_row { gridcraft_engine::display::cell_text(&d.wb, sh, CellRef::new(header_row, col)) } else { String::new() };
        let header = header.split_whitespace().collect::<Vec<_>>().join(" ");
        let header: String = header.chars().take(40).collect();
        let pair = if header.is_empty() { format!("{}: {value}", gridcraft_engine::core::col_to_letters(col)) } else { format!("{header}: {value}") };
        len += pair.len();
        out.push(pair);
        if out.len() >= ROW_PAIRS || len >= SUMMARY_CHARS {
            break;
        }
    }
    out.join("\n")
}

/// The active cell's text (trimmed), what Sleuth and Project Opener look up.
fn cell_text(app: &SheetApp) -> String {
    let Some(d) = app.session.active() else { return String::new() };
    let Some(sh) = d.wb.active() else { return String::new() };
    gridcraft_engine::display::cell_text(&d.wb, sh, d.selection.active).trim().to_string()
}

/// A suite link in the active cell: its hyperlink, or the address in its `HYPERLINK` formula.
fn cell_link(app: &SheetApp) -> Option<String> {
    let d = app.session.active()?;
    let sh = d.wb.active()?;
    let at = d.selection.active;
    if let Some(h) = sh.hyperlinks.get(&at) {
        return suite_scheme(&h.target).map(|_| h.target.trim().to_string());
    }
    let f = sh.cell(at)?.formula.as_ref()?;
    let rest = f.text.trim_start_matches('=').trim_start();
    let upper = rest.get(..10)?.to_ascii_uppercase();
    if upper != "HYPERLINK(" {
        return None;
    }
    let lit = rest.get(10..)?.trim_start().strip_prefix('"')?;
    let mut s = String::new();
    let mut chars = lit.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '"' {
            if chars.peek() == Some(&'"') {
                chars.next();
                s.push('"');
                continue;
            }
            return suite_scheme(&s).map(|_| s.trim().to_string());
        }
        s.push(c);
    }
    None
}

// ---------------------------------------------------------------------------------------------
// Notes

/// Where the active workbook's notes are kept: `<folder>\<book>.notes.md` beside a saved file
/// (so they travel with it, wherever it is saved), or for a workbook that lives only on the
/// office server, `%APPDATA%\Dexterity Sheets\Notes\<server>\<book>.notes.md` on this PC.
fn notes_path(app: &SheetApp) -> Result<(PathBuf, String), String> {
    let p = place(app).ok_or("No workbook is open.")?;
    if let Some(f) = &p.file {
        let f = Path::new(f);
        let stem = f.file_stem().and_then(|s| s.to_str()).unwrap_or("Workbook");
        let dir = f.parent().unwrap_or(Path::new("."));
        return Ok((dir.join(format!("{stem}.notes.md")), p.name));
    }
    if let Some((server, book)) = &p.shared {
        let base = std::env::var_os("APPDATA").ok_or("The notes folder (%APPDATA%) is not set on this PC.")?;
        let safe = |s: &str| s.chars().map(|c| if c.is_ascii_alphanumeric() || "-._".contains(c) { c } else { '_' }).collect::<String>();
        let dir = PathBuf::from(base).join("Dexterity Sheets").join("Notes").join(safe(server));
        return Ok((dir.join(format!("{}.notes.md", safe(book))), p.name));
    }
    Err("Save the workbook first: its notes are kept beside the file.".into())
}

fn read_small(path: &Path) -> Result<String, String> {
    match std::fs::metadata(path) {
        Ok(m) if m.len() > FILE_CAP => Err(format!("{} is too big to open here (over 8 MB).", path.display())),
        Ok(_) => std::fs::read(path).map(|b| String::from_utf8_lossy(&b).into_owned()).map_err(|e| format!("Could not read {}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(format!("Could not read {}: {e}", path.display())),
    }
}

/// Writes through a temporary file, so a failed write never leaves half a file.
fn write_file(path: &Path, text: &str) -> Result<(), String> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("Could not create {}: {e}", d.display()))?;
    }
    let tmp = path.with_extension("md.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("Could not write {}: {e}", path.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("Could not write {}: {e}", path.display())
    })
}

fn notes_header(name: &str) -> String {
    format!("# Notes: {name}\n\n")
}

/// The heading and lines a row note starts with.
fn row_note(app: &SheetApp) -> Option<String> {
    let p = place(app)?;
    let at = app.session.active()?.selection.active.a1();
    let when = chrono::Local::now().format("%d %b %Y %H:%M");
    let mut s = format!("\n## {} ({when})\n\n", sheet_place(&p.sheet, &at));
    let summary = row_summary(app);
    for l in summary.lines() {
        s.push_str(&format!("- {l}\n"));
    }
    match link_for(&p, &at) {
        Some((l, true)) => s.push_str(&format!("\n[Open in Dexterity Sheets]({l})\n")),
        Some((l, false)) => s.push_str(&format!("\nIn {l}, {}\n", sheet_place(&p.sheet, &at))),
        None => {}
    }
    s.push('\n');
    Some(s)
}

/// Opens the notes (Slate, or the pane here), after appending `add` when given.
fn open_notes(app: &mut SheetApp, add: Option<String>) -> Result<Json, String> {
    let (path, name) = notes_path(app)?;
    let slate = installed(app).slate;
    // The pane holds this file with edits not saved yet: the row goes there, not under it.
    if let Some(pane) = app.collab.suite.notes.as_mut().filter(|n| n.path == path && n.text != n.disk) {
        if let Some(a) = add {
            pane.text.push_str(&a);
        }
        return Ok(json!({"path": path.display().to_string(), "in": "pane"}));
    }
    let mut text = read_small(&path)?;
    if text.is_empty() || add.is_some() {
        if text.is_empty() {
            text = notes_header(&name);
        }
        if let Some(a) = &add {
            if !text.ends_with('\n') {
                text.push('\n');
            }
            text.push_str(a);
        }
        // Slate opens only a file that is there, so the notes are written before it starts.
        write_file(&path, &text)?;
    }
    if let Some(exe) = slate {
        app.collab.suite.opened.push(format!("slate:{}", path.display()));
        #[cfg(windows)]
        if crate::dex_ui::desktop() {
            use std::os::windows::process::CommandExt;
            std::process::Command::new(&exe).arg(&path).creation_flags(0x0800_0000).spawn().map_err(|e| format!("Could not start Slate: {e}"))?;
        }
        let _ = exe;
        app.collab.suite.notes = None;
        return Ok(json!({"path": path.display().to_string(), "in": "slate"}));
    }
    app.collab.suite.notes = Some(NotesPane { path: path.clone(), title: name, text: text.clone(), disk: text, error: None });
    Ok(json!({"path": path.display().to_string(), "in": "pane"}))
}

/// Saves the pane, unless another program changed the file since it was read.
fn save_pane(app: &mut SheetApp) -> Result<(), String> {
    let Some(pane) = app.collab.suite.notes.as_mut() else { return Ok(()) };
    let now = read_small(&pane.path)?;
    if now != pane.disk {
        return Err(
            "Another program changed the notes file since it was opened here. Reload to see its version (your text here is kept until you do)."
                .into(),
        );
    }
    write_file(&pane.path, &pane.text)?;
    pane.disk = pane.text.clone();
    pane.error = None;
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Commands

/// The suite's commands (UI-only; the window does each one). `None` = not one of them.
pub fn run_command(app: &mut SheetApp, id: &str, p: &Json) -> Option<Result<Json, String>> {
    let text_param = |p: &Json| p.get("text").and_then(Json::as_str).map(str::to_string);
    Some(match id {
        // What is installed: {connect, sleuth, datum, projectOpener, slate}.
        "suite.apps" => {
            let i = installed(app);
            Ok(json!({"connect": i.connect, "sleuth": i.sleuth, "datum": i.datum, "projectOpener": i.project_opener, "slate": i.slate.is_some()}))
        }
        // A link to the selected cell or range, on the clipboard. {} → {link, cellLink}
        "suite.copyCellLink" => {
            let Some(pl) = place(app) else { return Some(Err("No workbook is open.".into())) };
            let at = selected_place(app).unwrap_or_default();
            match link_for(&pl, &at) {
                Some((link, cell)) => {
                    let copied = if cell { link.clone() } else { format!("{link} ({})", sheet_place(&pl.sheet, &at)) };
                    if let Some(ctx) = &app.collab_ctx {
                        ctx.copy_text(copied.clone());
                    }
                    app.toast = Some((
                        if cell {
                            "Link copied: it opens the spreadsheet at this cell.".into()
                        } else {
                            "This file isn't shared, so its path and the cell were copied. Share it for a link that opens at the cell.".into()
                        },
                        crate::now_ms(),
                    ));
                    Ok(json!({"link": copied, "cellLink": cell}))
                }
                None => Err("Save or share the workbook first: a new workbook has no place to link to.".into()),
            }
        }
        // {row?: true} adds the active row's `header: value` pairs; {text?} replaces them.
        "suite.shareToConnect" | "suite.discussRow" => {
            if !installed(app).connect {
                return Some(Err("Dexterity Connect is not installed on this PC.".into()));
            }
            let Some(pl) = place(app) else { return Some(Err("No workbook is open.".into())) };
            let row = id == "suite.discussRow" || p.get("row").and_then(Json::as_bool) == Some(true);
            let at =
                if row { app.session.active().map(|d| d.selection.active.a1()).unwrap_or_default() } else { selected_place(app).unwrap_or_default() };
            let title = if pl.sheet.is_empty() { pl.name.clone() } else { format!("{} \u{2014} {}", pl.name, pl.sheet) };
            let mut text = text_param(p).unwrap_or_else(|| if row { row_summary(app) } else { String::new() });
            let link = match link_for(&pl, &at) {
                Some((l, true)) => l,
                Some((l, false)) => {
                    // A file link can't go to a cell: say where, and that it isn't shared.
                    let note =
                        format!("Local file (not shared), {}. Only people who can reach this folder can open it.", sheet_place(&pl.sheet, &at));
                    text = if text.is_empty() { note } else { format!("{text}\n\n{note}") };
                    l
                }
                None => String::new(),
            };
            launch(app, &connect_share(&title, &text, &link))
        }
        // {text?} or the active cell's text.
        "suite.findInSleuth" => {
            if !installed(app).sleuth {
                return Some(Err("Sleuth is not installed on this PC.".into()));
            }
            let t = text_param(p).unwrap_or_else(|| cell_text(app));
            if t.trim().is_empty() {
                return Some(Err("The cell is empty: there is nothing to search for.".into()));
            }
            launch(app, &sleuth_search(&t))
        }
        // {text?} or the active cell's text: a project's number or name.
        "suite.openInProjectOpener" => {
            if !installed(app).project_opener {
                return Some(Err("Project Opener is not installed on this PC.".into()));
            }
            let t = text_param(p).unwrap_or_else(|| cell_text(app));
            let projects = projects(app);
            match resolve_project(&projects, &t) {
                Some(pr) => {
                    let url = format!("projectopener://open/{}", encode(&pr.id));
                    launch(app, &url).map(|_| json!({"url": url, "project": pr.name}))
                }
                None => Err(format!("No single project in Project Opener is called \"{t}\" or has that job number.")),
            }
        }
        // {url?} or the active cell's link: opens a link to one of the office apps.
        "suite.openLink" => {
            let url = p.get("url").and_then(Json::as_str).map(str::to_string).or_else(|| cell_link(app)).unwrap_or_default();
            if url.trim().to_ascii_lowercase().starts_with("dexterity:") {
                return Some(if crate::dex_ui::share::open_link(app, &url) { Ok(json!({"url": url})) } else { Err("not a dexterity: link".into()) });
            }
            launch(app, url.trim())
        }
        // The workbook's notes, in Slate or the pane here.
        "suite.notes" => open_notes(app, None),
        // Appends the active row (a heading with its place, its values, a link) to the notes.
        "suite.noteForRow" => match row_note(app) {
            Some(n) => open_notes(app, Some(n)),
            None => Err("No workbook is open.".into()),
        },
        // The pane's text: {text?} sets it, {save: true} saves it, {close: true} closes it.
        "suite.notesPane" => {
            let Some(pane) = app.collab.suite.notes.as_mut() else { return Some(Err("The notes pane is not open.".into())) };
            if let Some(t) = text_param(p) {
                pane.text = t;
            }
            let save = p.get("save").and_then(Json::as_bool) == Some(true);
            let r = if save { save_pane(app) } else { Ok(()) };
            if let Err(e) = &r
                && let Some(pane) = app.collab.suite.notes.as_mut()
            {
                pane.error = Some(e.clone());
            }
            if let Err(e) = r {
                return Some(Err(e));
            }
            if p.get("close").and_then(Json::as_bool) == Some(true) {
                app.collab.suite.notes = None;
                return Some(Ok(Json::Null));
            }
            let pane = app.collab.suite.notes.as_ref();
            Ok(
                json!({"path": pane.map(|n| n.path.display().to_string()), "text": pane.map(|n| n.text.clone()), "saved": pane.is_some_and(|n| n.text == n.disk)}),
            )
        }
        _ => return None,
    })
}

/// A link in a cell that one of the office apps opens: handed to Windows. Returns false for any
/// other address (upstream's Open Hyperlink handles http, https and mailto, and refuses the rest).
pub fn follow_link(app: &mut SheetApp, target: &str) -> bool {
    if suite_scheme(target).is_none() {
        return false;
    }
    if let Err(e) = run_command(app, "suite.openLink", &json!({"url": target})).unwrap_or(Ok(Json::Null)) {
        app.message = Some(("Hyperlink".into(), e));
    }
    true
}

// ---------------------------------------------------------------------------------------------
// Menus, the Notes button, the pane

fn run(app: &mut SheetApp, id: &str, p: Json) {
    match app.run(id, p) {
        Ok(_) => {}
        Err(e) => app.message = Some(("Dexterity Sheets".into(), crate::clean_error(&e))),
    }
}

/// Title-bar Share menu: Share to Connect… and Discuss This Row in Connect.
pub fn share_menu(app: &mut SheetApp, ui: &mut Ui) {
    if app.session.active().is_none() || !installed(app).connect {
        return;
    }
    ui.separator();
    if ui.button("Share to Connect…").on_hover_text("Post a link to this spreadsheet in Dexterity Connect (you choose who, and confirm)").clicked()
    {
        run(app, "suite.shareToConnect", json!({}));
    }
    if ui.button("Discuss This Row in Connect…").on_hover_text("Post this row's values and a link to it in Dexterity Connect").clicked() {
        run(app, "suite.discussRow", json!({}));
    }
}

/// The cell menu's office items, after upstream's. Returns true when one was chosen.
pub fn cell_menu(app: &mut SheetApp, ui: &mut Ui) -> bool {
    let Some(pl) = place(app) else { return false };
    let i = installed(app);
    let text = cell_text(app);
    let linkable = pl.shared.is_some() || pl.file.is_some();
    let link = cell_link(app);
    let project = if i.project_opener && !text.is_empty() { resolve_project(&projects(app), &text).map(|p| p.name.clone()) } else { None };
    let short = |t: &str| if t.chars().count() > 24 { t.chars().take(23).chain(['…']).collect() } else { t.to_string() };
    let mut items: Vec<(String, &str, String)> = Vec::new();
    if let Some(l) = &link {
        let app_name = match suite_scheme(l) {
            Some("dexconnect") => "Dexterity Connect",
            Some("sleuth") => "Sleuth",
            Some("datum") => "Datum",
            Some("projectopener") => "Project Opener",
            _ => "Dexterity Sheets",
        };
        items.push((format!("Open Link in {app_name}"), "suite.openLink", l.clone()));
    }
    if linkable {
        items.push(("Copy Link to This Cell".into(), "suite.copyCellLink", "A link that opens this spreadsheet at this cell".into()));
    }
    if i.connect {
        items.push((
            "Discuss This Row in Connect…".into(),
            "suite.discussRow",
            "Post this row's values and a link to it in Dexterity Connect".into(),
        ));
    }
    if linkable {
        let label = if i.slate.is_some() { "Add Note in Slate for This Row" } else { "Add Note for This Row" };
        items.push((label.into(), "suite.noteForRow", "Add this row, with a link to it, to the workbook's notes".into()));
    }
    if i.sleuth && !text.is_empty() {
        items.push((
            format!("Find \u{201c}{}\u{201d} in Sleuth", short(&text)),
            "suite.findInSleuth",
            "Search the office's files for this text".into(),
        ));
    }
    if let Some(name) = project {
        items.push(("Open in Project Opener".into(), "suite.openInProjectOpener", format!("Open the project \u{201c}{name}\u{201d}")));
    }
    if items.is_empty() {
        return false;
    }
    ui.separator();
    let mut chosen = None;
    for (label, id, hover) in &items {
        if ui
            .add(egui::Button::new(label.as_str()).frame_when_inactive(false).min_size(egui::vec2(190.0, 20.0)))
            .on_hover_text(hover.as_str())
            .clicked()
        {
            chosen = Some(*id);
        }
    }
    match chosen {
        Some(id) => {
            run(app, id, json!({}));
            true
        }
        None => false,
    }
}

/// The status bar's Notes button (bottom right).
pub fn notes_button(app: &mut SheetApp, ui: &mut Ui, t: &crate::theme::Tokens) {
    if app.session.active().is_none() {
        return;
    }
    let slate = installed(app).slate.is_some();
    let hover = match notes_path(app) {
        Ok((p, _)) if slate => format!("Notes for this workbook, in Slate: {}", p.display()),
        Ok((p, _)) => format!("Notes for this workbook: {}", p.display()),
        Err(e) => e,
    };
    // A real button, not status text: an outlined pill with the note icon, a hover and pressed
    // fill, and drawn pressed while this workbook's notes pane is open.
    let open = app.collab.suite.notes.as_ref().is_some_and(|n| notes_path(app).is_ok_and(|(p, _)| p == n.path));
    let font = crate::theme::ui_font(12.5);
    let label = "Notes";
    let tw = ui.painter().layout_no_wrap(label.to_string(), font.clone(), t.text).size().x;
    let (rect, r) = ui.allocate_exact_size(egui::vec2(tw + 32.0, 20.0), egui::Sense::click());
    let fill = if open || r.is_pointer_button_down_on() {
        t.pressed
    } else if r.hovered() {
        t.hover
    } else {
        t.window
    };
    let stroke = if r.hovered() || open { t.text_dim } else { t.separator };
    ui.painter().rect_filled(rect, 4.0, fill);
    ui.painter().rect_stroke(rect, 4.0, egui::Stroke::new(1.0, stroke), egui::StrokeKind::Inside);
    let icon = egui::Rect::from_center_size(egui::pos2(rect.left() + 13.0, rect.center().y), egui::vec2(14.0, 14.0));
    crate::icons::paint(ui.painter(), icon, crate::icons::Icon::Note, t.text);
    ui.painter().text(egui::pos2(rect.left() + 24.0, rect.center().y), egui::Align2::LEFT_CENTER, label, font, t.text);
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    if r.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(hover).clicked() {
        run(app, "suite.notes", json!({}));
    }
    ui.add_space(10.0);
}

/// The notes pane (when Slate is not installed).
pub fn windows(app: &mut SheetApp, ctx: &egui::Context) {
    let Some(pane) = app.collab.suite.notes.as_mut() else { return };
    let mut open = true;
    let mut save = false;
    let mut reload = false;
    egui::Window::new(format!("Notes: {}", pane.title)).open(&mut open).default_width(420.0).show(ctx, |ui| {
        ui.label(RichText::new(pane.path.display().to_string()).weak().small());
        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
            ui.add(egui::TextEdit::multiline(&mut pane.text).desired_width(f32::INFINITY).desired_rows(12).code_editor());
        });
        if let Some(e) = &pane.error {
            ui.colored_label(ui.visuals().error_fg_color, e);
        }
        ui.horizontal(|ui| {
            let dirty = pane.text != pane.disk;
            if ui.add_enabled(dirty, egui::Button::new("Save")).on_disabled_hover_text("No changes to save").clicked() {
                save = true;
            }
            if pane.error.is_some() && ui.button("Reload").on_hover_text("Read the file again (drops the text here)").clicked() {
                reload = true;
            }
            ui.label(RichText::new("Install Slate to edit notes in a full editor.").weak().small());
        });
    });
    if (save || (!open && app.collab.suite.notes.as_ref().is_some_and(|n| n.text != n.disk)))
        && let Err(e) = save_pane(app)
    {
        if let Some(p) = app.collab.suite.notes.as_mut() {
            p.error = Some(e);
        }
        // Unsaved text is never dropped by closing: the pane stays with the reason.
        return;
    }
    if reload && let Some(p) = app.collab.suite.notes.as_mut() {
        match read_small(&p.path) {
            Ok(t) => {
                p.text = t.clone();
                p.disk = t;
                p.error = None;
            }
            Err(e) => p.error = Some(e),
        }
    }
    if !open {
        app.collab.suite.notes = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn places_and_ranges() {
        assert!(valid_place("B4") && valid_place("$B$4") && valid_place("A1:C9"));
        assert!(!valid_place("B4;rm") && !valid_place("A1:B2:C3") && !valid_place(""));
    }
}
