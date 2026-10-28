//! Sharing with people, links and notifications (Dexterity Sheets).
//!
//! - **Share dialog:** everyone at the office or specific people, each as viewer or editor, an
//!   optional end date, and whether viewers may save a copy. The office server enforces who may
//!   open and edit (`server::permission`); "allow downloading" is enforced here, by refusing
//!   Save As and Export for a viewer.
//! - **Links:** `dexterity://open/<server>/<book>` opens the workbook in the app after the
//!   person's own access is checked; `http://<server>/open/<book>` is the browser fallback
//!   (the office server serves it on its usual port). A second copy of the app hands a link to
//!   the one already running (a local port) and closes.
//! - **Notifications:** what the server tells a person (shared with you, access changed or
//!   removed, changes while you were away) and what the app notices itself (a new version,
//!   unsaved work to recover): a Windows notification that opens the workbook when clicked, and
//!   a history panel. People choose which kinds they get, mute single workbooks, and get them
//!   at once or as an hourly summary. Never one per cell edit.

use std::collections::BTreeMap;
use std::sync::mpsc::Receiver;

use egui::{RichText, Ui, vec2};
use gridcraft_collab::proto::{self, Access, ClientMsg, Notice};
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};

use crate::SheetApp;

/// Notification kinds a person can turn off: (kind, label).
pub const KINDS: &[(&str, &str)] = &[
    ("shared", "A spreadsheet is shared with me"),
    ("permission", "My access to a spreadsheet changes"),
    ("revoked", "My access is removed"),
    ("mention", "I am mentioned in a comment"),
    ("changed", "Others changed a shared spreadsheet while I was away"),
    ("template", "A template I used has a new approved version"),
    ("automation", "An automation rule reports something"),
    ("update", "A new Dexterity Sheets version is available"),
    ("recovery", "Unsaved work can be recovered"),
];
const HISTORY: usize = 200;
const SUMMARY_EVERY_MS: f64 = 3600.0 * 1000.0;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NotifyPrefs {
    pub enabled: bool,
    /// Kinds turned off.
    pub off: Vec<String>,
    /// Workbook ids muted.
    pub muted: Vec<String>,
    /// `now` or `hourly` (one summary an hour).
    pub frequency: String,
    pub history: Vec<Notice>,
    pub read: usize,
}

impl Default for NotifyPrefs {
    fn default() -> NotifyPrefs {
        NotifyPrefs { enabled: true, off: Vec::new(), muted: Vec::new(), frequency: "now".into(), history: Vec::new(), read: 0 }
    }
}

#[derive(Default)]
pub struct Share {
    pub dialog: bool,
    form: Access,
    expires: String,
    form_for: Option<u64>,
    pending_access: Option<(String, Access)>,
    pending_link: Option<(String, String)>,
    inbox: Option<Receiver<String>>,
    inbox_tried: bool,
    loaded: bool,
    pub prefs: NotifyPrefs,
    pub panel: bool,
    summary: Vec<Notice>,
    summary_at: f64,
}

fn prefs_path() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("APPDATA")?;
    Some(std::path::PathBuf::from(base).join("Dexterity Sheets").join("notifications.json"))
}

fn save(p: &NotifyPrefs) {
    if !crate::dex_ui::desktop() {
        return;
    }
    let Some(path) = prefs_path() else { return };
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if let Ok(b) = serde_json::to_vec_pretty(p) {
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, b).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

/// The local port a running app takes links on, one per Windows account on the PC.
fn inbox_port() -> u16 {
    let user = std::env::var("USERNAME").or_else(|_| std::env::var("USER")).unwrap_or_default();
    let h = user.bytes().fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(u32::from(b)));
    47_820 + (h % 80) as u16
}

/// A `dexterity:` link (from Explorer, a browser, a notification, a chat message): opens the
/// workbook here, or hands it to the copy of the app already running. Returns false for
/// anything that is not such a link.
pub fn open_link(app: &mut SheetApp, s: &str) -> bool {
    let s = s.trim();
    if !s.to_ascii_lowercase().starts_with("dexterity:") {
        return false;
    }
    #[cfg(not(target_arch = "wasm32"))]
    if crate::dex_ui::desktop() && app.collab.share.inbox.is_none() && forward(s) {
        // The running app has it; this copy has nothing to do.
        std::process::exit(0);
    }
    if s.trim_end_matches('/').ends_with("notifications") {
        app.collab.share.panel = true;
        return true;
    }
    match proto::parse_link(s) {
        Some((server, book)) => {
            app.collab.share.pending_link = Some((server.clone(), book));
            if !app.collab.signed_in() {
                app.collab.set_form_server(&server);
                app.open_backstage(crate::backstage::Page::Shared);
                app.toast = Some(("Sign in to the office server to open the shared spreadsheet.".into(), crate::now_ms()));
            }
        }
        None => app.message = Some(("Dexterity Sheets".into(), "That link is not a Dexterity Sheets link to a shared spreadsheet.".into())),
    }
    true
}

#[cfg(not(target_arch = "wasm32"))]
fn forward(link: &str) -> bool {
    use std::io::Write;
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], inbox_port()));
    match std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(300)) {
        Ok(mut s) => s.write_all(format!("{link}\n").as_bytes()).is_ok(),
        Err(_) => false,
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn start_inbox(ctx: Option<egui::Context>) -> Option<Receiver<String>> {
    use std::io::{BufRead, Read};
    let listener = std::net::TcpListener::bind(("127.0.0.1", inbox_port())).ok()?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("dexsheets-links".into())
        .spawn(move || {
            for s in listener.incoming().flatten() {
                let _ = s.set_read_timeout(Some(std::time::Duration::from_secs(2)));
                let mut line = String::new();
                if std::io::BufReader::new(s.take(4096)).read_line(&mut line).is_ok_and(|n| n > 0) {
                    let _ = tx.send(line.trim().to_string());
                    if let Some(c) = &ctx {
                        c.request_repaint();
                    }
                }
            }
        })
        .ok()?;
    Some(rx)
}

/// Every frame: the link inbox, a link waiting for sign-in, the hourly summary.
pub fn tick(app: &mut SheetApp) {
    if !app.collab.share.loaded {
        app.collab.share.loaded = true;
        if crate::dex_ui::desktop() {
            app.collab.share.prefs =
                prefs_path().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    if !app.collab.share.inbox_tried && crate::dex_ui::desktop() {
        app.collab.share.inbox_tried = true;
        app.collab.share.inbox = start_inbox(app.collab_ctx.clone());
    }
    let mut got = Vec::new();
    if let Some(rx) = &app.collab.share.inbox {
        while let Ok(l) = rx.try_recv() {
            got.push(l);
        }
    }
    for l in got {
        open_link(app, &l);
    }
    if app.collab.signed_in()
        && let Some((_, book)) = app.collab.share.pending_link.take()
    {
        app.collab.open_book(&book);
    }
    let now = crate::clock::monotonic_ms();
    if !app.collab.share.summary.is_empty() && now - app.collab.share.summary_at >= SUMMARY_EVERY_MS {
        app.collab.share.summary_at = now;
        let n = std::mem::take(&mut app.collab.share.summary);
        let names: Vec<String> =
            n.iter().filter(|x| !x.name.is_empty()).map(|x| x.name.clone()).collect::<std::collections::BTreeSet<_>>().into_iter().take(3).collect();
        native(&format!("{} update(s) in shared spreadsheets", n.len()), &[names.join(", ")], "dexterity://notifications");
    }
}

/// A notice from the server (or the app): kept in the history, shown as a Windows notification
/// if the person wants this kind and has not muted the workbook.
pub fn notice(app: &mut SheetApp, n: Notice) {
    let p = &mut app.collab.share.prefs;
    p.history.insert(0, n.clone());
    p.history.truncate(HISTORY);
    let wanted = p.enabled && !p.off.contains(&n.kind) && !(!n.book.is_empty() && p.muted.contains(&n.book));
    let hourly = p.frequency == "hourly";
    save(&app.collab.share.prefs);
    app.toast = Some((notice_line(&n), crate::now_ms()));
    if !wanted {
        return;
    }
    if hourly {
        app.collab.share.summary.push(n);
        return;
    }
    let server = app.collab.server_address();
    let launch = if n.book.is_empty() { "dexterity://notifications".to_string() } else { proto::link(&server, &n.book) };
    let mut lines = Vec::new();
    if !n.name.is_empty() {
        lines.push(n.name.clone());
    }
    if !n.permission.is_empty() {
        lines.push(format!("Permission: {}", if n.permission == "editor" { "Editor" } else { "Viewer" }));
    }
    native(&title_of(&n), &lines, &launch);
}

/// A notice the app makes itself (a new version, recovery).
pub fn local(app: &mut SheetApp, kind: &str, text: &str) {
    let n = Notice {
        id: format!("n-{}-{kind}", crate::now_ms() as i64),
        time: crate::now_ms() as i64,
        kind: kind.into(),
        text: text.into(),
        ..Notice::default()
    };
    notice(app, n);
}

fn title_of(n: &Notice) -> String {
    match n.kind.as_str() {
        "shared" if !n.by.is_empty() => format!("{} shared a spreadsheet with you", n.by),
        "permission" => format!("Your access to {} changed", n.name),
        "revoked" => "Your access to a shared spreadsheet has changed".into(),
        "changed" if !n.by.is_empty() => format!("{} updated a shared spreadsheet", n.by),
        _ => n.text.clone(),
    }
}

fn notice_line(n: &Notice) -> String {
    let t = title_of(n);
    if n.name.is_empty() || t.contains(&n.name) { t } else { format!("{t}: {}", n.name) }
}

/// A Windows notification under the app's own name and icon (the Start menu shortcut's
/// AppUserModelID), opening `launch` when clicked. Fails quietly elsewhere: the history panel
/// and the in-app line always have it.
fn native(title: &str, lines: &[String], launch: &str) {
    #[cfg(windows)]
    if crate::dex_ui::desktop() {
        let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;");
        let mut texts = format!("<text>{}</text>", esc(title));
        for l in lines.iter().take(2) {
            texts.push_str(&format!("<text>{}</text>", esc(l)));
        }
        let open = if launch.contains("/open/") { "Open Spreadsheet" } else { "Open" };
        let xml = format!(
            "<toast activationType=\"protocol\" launch=\"{l}\"><visual><binding template=\"ToastGeneric\">{texts}</binding></visual><actions><action content=\"{open}\" activationType=\"protocol\" arguments=\"{l}\"/><action content=\"View Details\" activationType=\"protocol\" arguments=\"dexterity://notifications\"/></actions></toast>",
            l = esc(launch)
        );
        let script = format!(
            "$ErrorActionPreference='Stop';[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] > $null;[Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom.XmlDocument, ContentType = WindowsRuntime] > $null;$x = New-Object Windows.Data.Xml.Dom.XmlDocument;$x.LoadXml('{}');[Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('DexterityDesign.DexteritySheets').Show([Windows.UI.Notifications.ToastNotification]::new($x))",
            xml.replace('\'', "''")
        );
        let utf16: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let encoded = gridcraft_engine::io::base64_encode(&utf16);
        use std::os::windows::process::CommandExt;
        let r = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-EncodedCommand", &encoded])
            .creation_flags(0x0800_0000)
            .spawn();
        if let Err(e) = r {
            log::warn!("notification: {e}");
        }
    }
    #[cfg(not(windows))]
    log::info!("notification: {title} {lines:?} {launch}");
}

fn shared_doc_book(app: &SheetApp) -> Option<proto::BookInfo> {
    let d = app.session.active()?;
    app.collab.shared(d.uid).map(|sd| sd.book.clone())
}

/// The sharing commands. `None` = not one of them.
pub fn run_command(app: &mut SheetApp, id: &str, p: &Json) -> Option<Result<Json, String>> {
    // Allow downloading: a viewer of a workbook whose owner turned it off can't save a copy.
    if matches!(id, "file.saveAs" | "file.saveBytes" | "file.exportPdf" | "file.export" | "file.saveCopy")
        && let Some(b) = shared_doc_book(app)
        && b.permission == "viewer"
        && !b.access.download
    {
        return Some(Err(format!("Whoever shared \"{}\" doesn't allow copies to be saved or exported.", b.name)));
    }
    Some(match id {
        "collab.shareDialog" => {
            app.collab.share.dialog = true;
            app.collab.share.form_for = None;
            Ok(Json::Null)
        }
        // {mode?: office|people, everyone?: editor|viewer, people?: {user: editor|viewer}, expires?: "YYYY-MM-DD"|null, download?}
        "collab.setAccess" => {
            let mut a = shared_doc_book(app).map(|b| b.access).unwrap_or_default();
            if let Some(m) = p.get("mode").and_then(Json::as_str) {
                a.mode = m.into();
            }
            if let Some(e) = p.get("everyone").and_then(Json::as_str) {
                a.everyone = e.into();
            }
            if let Some(o) = p.get("people").and_then(Json::as_object) {
                a.people = o.iter().filter_map(|(k, v)| v.as_str().map(|v| (k.to_ascii_lowercase(), v.to_string()))).collect();
            }
            if let Some(e) = p.get("expires") {
                a.expires = e.as_str().and_then(date_ms);
            }
            if let Some(d) = p.get("download").and_then(Json::as_bool) {
                a.download = d;
            }
            apply_access(app, a)
        }
        "collab.copyLink" => match shared_doc_book(app) {
            Some(b) => {
                let server = app.collab.server_address();
                let (link, web) = (proto::link(&server, &b.id), proto::web_link(&server, &b.id));
                if let Some(ctx) = &app.collab_ctx {
                    ctx.copy_text(link.clone());
                }
                Ok(json!({"link": link, "webLink": web}))
            }
            None => Err("Share the workbook first: the link points at the shared copy on the office server.".into()),
        },
        "collab.openLink" => {
            let l = p.get("link").and_then(Json::as_str).unwrap_or("");
            if open_link(app, l) { Ok(Json::Null) } else { Err("not a dexterity: link".into()) }
        }
        // {enabled?, off?: [kind], frequency?: now|hourly, mute?: book, unmute?: book} → the history and settings
        "collab.notifications" => {
            let pr = &mut app.collab.share.prefs;
            if let Some(e) = p.get("enabled").and_then(Json::as_bool) {
                pr.enabled = e;
            }
            if let Some(o) = p.get("off").and_then(Json::as_array) {
                pr.off = o.iter().filter_map(Json::as_str).map(str::to_string).collect();
            }
            if let Some(f) = p.get("frequency").and_then(Json::as_str) {
                pr.frequency = if f == "hourly" { "hourly".into() } else { "now".into() };
            }
            if let Some(b) = p.get("mute").and_then(Json::as_str)
                && !pr.muted.iter().any(|x| x == b)
            {
                pr.muted.push(b.into());
            }
            if let Some(b) = p.get("unmute").and_then(Json::as_str) {
                pr.muted.retain(|x| x != b);
            }
            save(pr);
            if p.as_object().is_none_or(|o| o.is_empty()) {
                app.collab.share.panel = true;
            }
            let pr = &app.collab.share.prefs;
            Ok(json!({"enabled": pr.enabled, "off": pr.off, "frequency": pr.frequency, "muted": pr.muted, "history": pr.history}))
        }
        _ => return None,
    })
}

fn date_ms(s: &str) -> Option<i64> {
    use chrono::TimeZone;
    let d = chrono::NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").ok()?;
    let end = d.and_hms_opt(23, 59, 59)?;
    chrono::Local.from_local_datetime(&end).single().map(|t| t.timestamp_millis())
}

/// Sets who has access: now for a shared workbook, or right after sharing for one that isn't.
fn apply_access(app: &mut SheetApp, a: Access) -> Result<Json, String> {
    if !app.collab.signed_in() {
        return Err("Sign in to the office server first (File › Shared).".into());
    }
    match shared_doc_book(app) {
        Some(b) => {
            app.collab.send(ClientMsg::SetAccess { book: b.id.clone(), access: a });
            Ok(json!({"book": b.id}))
        }
        None => {
            let name = app.session.active().map(|d| d.title.trim_end_matches(".xlsx").to_string()).unwrap_or_default();
            app.collab.share.pending_access = Some((name, a));
            app.run("collab.share", json!({}))
        }
    }
}

/// A shared workbook arrived: if it was shared from the dialog, its access is set now.
pub fn opened(app: &mut SheetApp, book: &proto::BookInfo) {
    let me = app.collab.me.as_ref().map(|m| m.user.clone()).unwrap_or_default();
    if book.created_by == me
        && let Some((name, a)) = app.collab.share.pending_access.take()
    {
        if book.name.starts_with(&name) || name.is_empty() {
            app.collab.send(ClientMsg::SetAccess { book: book.id.clone(), access: a });
        } else {
            app.collab.share.pending_access = Some((name, a));
        }
    }
}

/// The status bar's notification count (opens the panel).
pub fn bell(app: &mut SheetApp, ui: &mut Ui) {
    let p = &app.collab.share.prefs;
    let unread = p.history.len().saturating_sub(p.read);
    let label = if unread > 0 { format!("Notifications ({unread})") } else { "Notifications".into() };
    if ui
        .add(egui::Label::new(RichText::new(label).small()).sense(egui::Sense::click()))
        .on_hover_text("What was shared with you, and other news")
        .clicked()
    {
        app.collab.share.panel = !app.collab.share.panel;
    }
}

/// The Share dialog and the notifications panel.
pub fn windows(app: &mut SheetApp, ctx: &egui::Context) {
    share_dialog(app, ctx);
    panel(app, ctx);
}

fn share_dialog(app: &mut SheetApp, ctx: &egui::Context) {
    if !app.collab.share.dialog {
        return;
    }
    let book = shared_doc_book(app);
    let uid = app.session.active().map(|d| d.uid);
    if app.collab.share.form_for != uid {
        app.collab.share.form_for = uid;
        app.collab.share.form = book.as_ref().map(|b| b.access.clone()).unwrap_or_default();
        app.collab.share.expires = app
            .collab
            .share
            .form
            .expires
            .map(|ms| {
                use chrono::TimeZone;
                chrono::Local.timestamp_millis_opt(ms).single().map(|t| t.format("%Y-%m-%d").to_string()).unwrap_or_default()
            })
            .unwrap_or_default();
    }
    let title = app.session.active().map(|d| d.display_title()).unwrap_or_default();
    let me = app.collab.me.as_ref().map(|m| m.user.clone()).unwrap_or_default();
    let users: Vec<(String, String)> = app.collab.users.iter().filter(|u| u.user != me).map(|u| (u.user.clone(), u.display.clone())).collect();
    let signed_in = app.collab.signed_in();
    let mut open = true;
    let mut apply = false;
    let mut copy = false;
    egui::Window::new("Share spreadsheet")
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.set_max_width(460.0);
            let f = &mut app.collab.share.form;
            egui::Grid::new("dex_share").num_columns(2).spacing(vec2(12.0, 8.0)).show(ui, |ui| {
                ui.label("Spreadsheet");
                ui.strong(&title);
                ui.end_row();
                ui.label("Access");
                ui.vertical(|ui| {
                    ui.radio_value(&mut f.mode, "office".to_string(), "Everyone at the office");
                    ui.radio_value(&mut f.mode, "people".to_string(), "Specific people");
                });
                ui.end_row();
                if f.mode == "office" {
                    ui.label("Permission");
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut f.everyone, "editor".to_string(), "Editor");
                        ui.radio_value(&mut f.everyone, "viewer".to_string(), "Viewer");
                    });
                    ui.end_row();
                }
                ui.label("Link expires");
                ui.add(egui::TextEdit::singleline(&mut app.collab.share.expires).hint_text("never, or YYYY-MM-DD").desired_width(140.0));
                ui.end_row();
                ui.label("Downloads");
                ui.checkbox(&mut f.download, "Viewers may save a copy or export it");
                ui.end_row();
            });
            if f.mode == "people" {
                ui.add_space(4.0);
                ui.label("People (the creator and administrators always have access):");
                egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                    if users.is_empty() {
                        ui.label(RichText::new("Sign in to the office server to see your colleagues.").weak());
                    }
                    for (u, display) in &users {
                        ui.horizontal(|ui| {
                            let mut on = f.people.contains_key(u);
                            if ui.checkbox(&mut on, display).changed() {
                                if on {
                                    f.people.insert(u.clone(), "editor".into());
                                } else {
                                    f.people.remove(u);
                                }
                            }
                            if let Some(perm) = f.people.get_mut(u) {
                                ui.radio_value(perm, "editor".to_string(), "Editor");
                                ui.radio_value(perm, "viewer".to_string(), "Viewer");
                            }
                        });
                    }
                });
            }
            ui.separator();
            ui.horizontal(|ui| {
                let label = if book.is_some() { "Save sharing" } else { "Share" };
                if ui
                    .add_enabled(signed_in, egui::Button::new(label))
                    .on_disabled_hover_text("Sign in to the office server first (File › Shared)")
                    .clicked()
                {
                    apply = true;
                }
                if ui.add_enabled(book.is_some(), egui::Button::new("Copy link")).on_disabled_hover_text("Share it first").clicked() {
                    copy = true;
                }
            });
            if let Some(b) = &book {
                let server = app.collab.server_address();
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "Paste the link in a chat or e-mail. It opens the spreadsheet in Dexterity Sheets; whoever clicks it needs access.",
                    )
                    .weak(),
                );
                ui.monospace(proto::link(&server, &b.id));
                ui.label(RichText::new(format!("In a browser: {}", proto::web_link(&server, &b.id))).weak());
            }
        });
    if apply {
        let mut a = app.collab.share.form.clone();
        let e = app.collab.share.expires.trim().to_string();
        a.expires = if e.is_empty() || e.eq_ignore_ascii_case("never") { None } else { date_ms(&e) };
        if !e.is_empty() && !e.eq_ignore_ascii_case("never") && a.expires.is_none() {
            app.message = Some(("Share spreadsheet".into(), "Type the end date as YYYY-MM-DD, or leave it empty for never.".into()));
        } else if let Err(e) = apply_access(app, a) {
            app.message = Some(("Share spreadsheet".into(), e));
        }
    }
    if copy {
        match run_command(app, "collab.copyLink", &json!({})) {
            Some(Ok(_)) => app.toast = Some(("Link copied.".into(), crate::now_ms())),
            Some(Err(e)) => app.message = Some(("Share spreadsheet".into(), e)),
            None => {}
        }
    }
    if !open {
        app.collab.share.dialog = false;
    }
}

fn panel(app: &mut SheetApp, ctx: &egui::Context) {
    if !app.collab.share.panel {
        return;
    }
    let mut open = true;
    let mut go: Option<String> = None;
    let mut mute: Option<(String, bool)> = None;
    let mut changed = false;
    egui::Window::new("Notifications").collapsible(false).open(&mut open).default_width(440.0).show(ctx, |ui| {
        let p = &mut app.collab.share.prefs;
        egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
            if p.history.is_empty() {
                ui.label(
                    RichText::new(
                        "Nothing yet. You are told here when someone shares a spreadsheet with you, your access changes, or a new version is ready.",
                    )
                    .weak(),
                );
            }
            for n in &p.history {
                ui.horizontal(|ui| {
                    ui.label(when(n.time));
                    ui.label(notice_line(n));
                    if !n.book.is_empty() && n.kind != "revoked" && ui.small_button("Open").clicked() {
                        go = Some(n.book.clone());
                    }
                    if !n.book.is_empty() {
                        let muted = p.muted.contains(&n.book);
                        if ui.small_button(if muted { "Unmute" } else { "Mute" }).on_hover_text("Notifications about this spreadsheet").clicked() {
                            mute = Some((n.book.clone(), !muted));
                        }
                    }
                });
            }
        });
        ui.separator();
        ui.collapsing("Settings", |ui| {
            changed |= ui.checkbox(&mut p.enabled, "Show Windows notifications").changed();
            ui.horizontal(|ui| {
                ui.label("When:");
                changed |= ui.radio_value(&mut p.frequency, "now".to_string(), "Right away").changed();
                changed |= ui.radio_value(&mut p.frequency, "hourly".to_string(), "One summary an hour").changed();
            });
            for (k, label) in KINDS {
                let mut on = !p.off.iter().any(|x| x == k);
                if ui.checkbox(&mut on, *label).changed() {
                    p.off.retain(|x| x != k);
                    if !on {
                        p.off.push((*k).to_string());
                    }
                    changed = true;
                }
            }
            if !p.muted.is_empty() {
                ui.label(format!("{} spreadsheet(s) muted (Unmute next to their notifications).", p.muted.len()));
            }
        });
        p.read = p.history.len();
    });
    if changed || mute.is_some() {
        if let Some((b, on)) = mute {
            app.collab.share.prefs.muted.retain(|x| *x != b);
            if on {
                app.collab.share.prefs.muted.push(b);
            }
        }
        save(&app.collab.share.prefs);
    }
    if let Some(b) = go {
        app.collab.open_book(&b);
    }
    if !open {
        app.collab.share.panel = false;
        save(&app.collab.share.prefs);
    }
}

fn when(ms: i64) -> String {
    use chrono::TimeZone;
    chrono::Local.timestamp_millis_opt(ms).single().map(|t| t.format("%d %b %H:%M").to_string()).unwrap_or_default()
}

/// Kinds and their state for tests and the control channel.
pub fn kinds() -> BTreeMap<&'static str, &'static str> {
    KINDS.iter().copied().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_round_trip_and_reject_junk() {
        let l = proto::link("PC-SERVER:47810", "project-tracker-383b13");
        assert_eq!(l, "dexterity://open/PC-SERVER:47810/project-tracker-383b13");
        assert_eq!(proto::parse_link(&l), Some(("PC-SERVER:47810".into(), "project-tracker-383b13".into())));
        assert_eq!(proto::parse_link("dexterity://open/10.0.0.5:47810/book-1/"), Some(("10.0.0.5:47810".into(), "book-1".into())));
        for bad in ["https://example.com/x", "dexterity://open/", "dexterity://open/a b/c", "dexterity://open/srv/<script>"] {
            assert_eq!(proto::parse_link(bad), None, "{bad}");
        }
        assert_eq!(proto::web_link("srv:47810", "b"), "http://srv:47810/open/b");
    }

    #[test]
    fn end_dates_are_the_end_of_that_day() {
        let a = date_ms("2030-01-31").unwrap();
        let b = date_ms("2030-02-01").unwrap();
        assert!(b - a > 0 && b - a <= 86_400_000);
        assert_eq!(date_ms("31/01/2030"), None);
    }

    #[test]
    fn notice_titles_name_who_and_what() {
        let n = Notice { kind: "shared".into(), by: "Vikram".into(), name: "Project_Tracker".into(), ..Notice::default() };
        assert_eq!(title_of(&n), "Vikram shared a spreadsheet with you");
        assert_eq!(notice_line(&n), "Vikram shared a spreadsheet with you: Project_Tracker");
        assert!(kinds().contains_key("changed"));
    }
}
