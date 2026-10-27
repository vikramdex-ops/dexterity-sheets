//! Dexterity Sheets LAN co-authoring in the window: signing in to the office server, sharing any
//! workbook wherever it is saved (its file keeps everyone's edits), the shared workbook list
//! (File › Shared), other people's selections on the grid, who last changed the active cell
//! (status bar), and the Edit History and Versions panes.
//!
//! The engine knows nothing of sharing: each frame [`tick`] sends what the person changed in a
//! shared workbook and applies what others changed, through `gridcraft_collab::sync`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::mpsc::Receiver;

use egui::{Color32, FontId, Painter, RichText, Stroke, StrokeKind, Ui, pos2, vec2};
use gridcraft_collab::client::{Client, Config, Event};
use gridcraft_collab::discovery::{self, Found};
use gridcraft_collab::proto::{self, BookInfo, Change, ClientMsg, Peer, Role, ServerMsg, UpdateInfo, UserInfo, VersionInfo};
use gridcraft_collab::sync::SharedDoc;
use gridcraft_engine::core::{CellRef, RangeRef};
use gridcraft_engine::model::{Sheet, Workbook};
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};

use crate::SheetApp;
use crate::grid::Geo;
use crate::theme::{self, Tokens};

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Status {
    #[default]
    SignedOut,
    Connecting,
    Online,
    Reconnecting(String),
}

#[derive(Default, Serialize, Deserialize)]
struct Remembered {
    server: String,
    user: String,
}

#[derive(Default)]
pub struct HistoryPane {
    /// Only the active cell's edits (otherwise the whole workbook's).
    pub this_cell: bool,
    pub user: Option<String>,
    pub changes: Vec<Change>,
    /// What the shown list was asked for: (book, sheet, cell, user).
    asked: Option<(String, Option<String>, Option<String>, Option<String>)>,
}

#[derive(Default)]
pub struct Collab {
    pub client: Option<Client>,
    pub status: Status,
    pub me: Option<UserInfo>,
    pub users: Vec<UserInfo>,
    pub server_name: String,
    pub books: Vec<BookInfo>,
    pub docs: Vec<SharedDoc>,
    pub error: Option<String>,
    form_server: String,
    form_user: String,
    form_password: String,
    form_loaded: bool,
    found: Vec<Found>,
    finding: Option<Receiver<Vec<Found>>>,
    new_name: String,
    new_password: String,
    /// Workbooks asked for and not yet arrived.
    opening: Vec<String>,
    /// The last presence sent: (doc uid, sheet, range, editing).
    presence: Option<(u64, String, String, bool)>,
    presence_at: f64,
    pub history: HistoryPane,
    pub versions: Vec<VersionInfo>,
    version_label: String,
    /// A version asked for to open as a copy: (book, version) → title.
    want_copy: Option<String>,
    confirm_restore: Option<String>,
    /// A newer Dexterity Sheets the server offers.
    pub update: Option<UpdateInfo>,
    /// The update dialog is open.
    update_dialog: bool,
    /// Downloading or installing: what the dialog says.
    update_state: Option<String>,
    last_update_check: f64,
    /// Shared workbooks that are files on this PC, kept saved with everyone's edits.
    pub links: Vec<LinkedFile>,
    joining: Vec<Joining>,
    /// Share this window's workbook once signed in.
    share_after_sign_in: Option<u64>,
    /// Open files' keys (window uid → key; `None`: not a file that exists).
    keys: HashMap<u64, Option<String>>,
    /// Windows already compared with the shared files: not joined or told again.
    told: HashSet<u64>,
}

fn remembered_path() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("APPDATA")?;
    Some(std::path::PathBuf::from(base).join("Dexterity Sheets").join("shared.json"))
}

fn load_remembered() -> Remembered {
    remembered_path().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save_remembered(r: &Remembered) {
    if let Some(p) = remembered_path() {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(b) = serde_json::to_vec_pretty(r) {
            let _ = std::fs::write(p, b);
        }
    }
}

fn color(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

fn when(ms: i64) -> String {
    use chrono::TimeZone;
    match chrono::Local.timestamp_millis_opt(ms).single() {
        Some(t) => {
            let today = chrono::Local::now().date_naive();
            if t.date_naive() == today { t.format("%H:%M").to_string() } else { t.format("%d %b %Y %H:%M").to_string() }
        }
        None => String::new(),
    }
}

impl Collab {
    pub fn display_of(&self, user: &str) -> String {
        self.users.iter().find(|u| u.user == user).map(|u| u.display.clone()).unwrap_or_else(|| user.to_string())
    }
    pub fn shared(&self, uid: u64) -> Option<&SharedDoc> {
        self.docs.iter().find(|d| d.doc_uid == uid)
    }
    fn send(&self, m: ClientMsg) {
        if let Some(c) = &self.client {
            c.send(m);
        }
    }
    pub fn signed_in(&self) -> bool {
        self.client.is_some() && self.me.is_some()
    }
}

/// The shared workbook shown in the active window, if any.
pub fn active_shared(app: &SheetApp) -> Option<&SharedDoc> {
    let uid = app.session.active()?.uid;
    app.collab.shared(uid)
}

fn sign_in(c: &mut Collab, ctx: &egui::Context) {
    let server = c.form_server.trim().to_string();
    let user = c.form_user.trim().to_ascii_lowercase();
    if server.is_empty() || user.is_empty() || c.form_password.is_empty() {
        c.error = Some("Enter the server, your user name and your password.".into());
        return;
    }
    save_remembered(&Remembered { server: server.clone(), user: user.clone() });
    let ctx = ctx.clone();
    let waker: gridcraft_collab::client::Waker = Arc::new(move || ctx.request_repaint());
    let machine = std::env::var("COMPUTERNAME").unwrap_or_default();
    let cfg = Config {
        server,
        user,
        password: std::mem::take(&mut c.form_password),
        app: format!("Dexterity Sheets {}", gridcraft_collab::APP_VERSION.trim()),
        machine,
    };
    c.client = Some(Client::connect(cfg, waker));
    c.status = Status::Connecting;
    c.error = None;
}

fn sign_out(app: &mut SheetApp) {
    // A shared file gets its last edits and its window is the file again.
    let linked: Vec<(String, u64)> =
        app.collab.links.iter().filter_map(|l| app.collab.docs.iter().find(|d| d.book.id == l.book).map(|d| (l.book.clone(), d.doc_uid))).collect();
    for (book, uid) in linked {
        if let Some((path, None)) = unlink(app, &book) {
            app.collab.docs.retain(|d| d.book.id != book);
            back_to_file(app, uid, &path);
        }
    }
    app.collab.links.clear();
    app.collab.joining.clear();
    let c = &mut app.collab;
    // Shared workbooks stay open as unsaved local copies.
    for d in c.docs.drain(..) {
        if let Some(doc) = app.session.document_mut(d.doc_uid) {
            doc.title = format!("{} (offline copy)", d.book.name);
            doc.saved = Arc::new(gridcraft_engine::model::Workbook::new());
        }
    }
    c.client = None;
    c.me = None;
    c.status = Status::SignedOut;
    c.books.clear();
}

/// Each frame: network events in, the person's edits and position out.
pub fn tick(app: &mut SheetApp, ctx: &egui::Context) {
    if app.collab.client.is_none() && app.collab.docs.is_empty() {
        if let Some(rx) = &app.collab.finding
            && let Ok(found) = rx.try_recv()
        {
            app.collab.found = found;
            app.collab.finding = None;
        }
        return;
    }
    // The person's edits first, so they are pending before anyone else's are applied.
    send_local(app);
    let events: Vec<Event> = match &app.collab.client {
        Some(c) => c.events.try_iter().collect(),
        None => Vec::new(),
    };
    for e in events {
        handle(app, e);
    }
    keep_files(app);
    // Workbooks that need to be fetched again (a missed edit, a restored version).
    let reload: Vec<String> = app.collab.docs.iter().filter(|d| d.needs_reload.is_some()).map(|d| d.book.id.clone()).collect();
    for id in reload {
        if !app.collab.opening.contains(&id) {
            app.collab.opening.push(id.clone());
            app.collab.send(ClientMsg::OpenBook { book: id });
        }
    }
    send_presence(app);
    // Ask again every four hours, so a long-running window still hears of new versions.
    if app.collab.client.as_ref().is_some_and(Client::online) && crate::now_ms() - app.collab.last_update_check > 4.0 * 3600.0 * 1000.0 {
        app.collab.last_update_check = crate::now_ms();
        app.collab.send(ClientMsg::CheckUpdate { version: gridcraft_collab::APP_VERSION.trim().into() });
    }
    if let Some(rx) = &app.collab.finding
        && let Ok(found) = rx.try_recv()
    {
        app.collab.found = found;
        app.collab.finding = None;
    }
    // A shared file waiting to be written needs a frame even when nobody touches the window.
    if app.collab.client.as_ref().is_some_and(|c| !c.online())
        || !app.collab.opening.is_empty()
        || app.collab.links.iter().any(|l| !l.up_to_date() && !l.blocked)
    {
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
}

fn send_local(app: &mut SheetApp) {
    let open: Vec<u64> = app.session.documents().iter().map(|d| d.uid).collect();
    let closed: Vec<String> = app.collab.docs.iter().filter(|d| !open.contains(&d.doc_uid)).map(|d| d.book.id.clone()).collect();
    for id in closed {
        // A shared file's window closed: the file gets the last edits.
        let _ = unlink(app, &id);
        app.collab.send(ClientMsg::CloseBook { book: id.clone() });
        app.collab.docs.retain(|d| d.book.id != id);
    }
    let online = app.collab.client.as_ref().is_some_and(Client::online);
    let mut out = Vec::new();
    let mut denied = None;
    for sd in app.collab.docs.iter_mut() {
        let Some(doc) = app.session.document_mut(sd.doc_uid) else { continue };
        match sd.local_changes(doc) {
            Ok(Some((cseq, patch))) => out.push(ClientMsg::Patch { book: sd.book.id.clone(), cseq, patch }),
            Ok(None) => {}
            Err(e) => denied = Some(e.to_string()),
        }
    }
    if let Some(e) = denied {
        app.toast = Some((e, crate::now_ms()));
    }
    // Offline edits stay pending and are sent after the reconnect.
    if online {
        for m in out {
            app.collab.send(m);
        }
    }
}

fn send_presence(app: &mut SheetApp) {
    let Some(d) = app.session.active() else { return };
    let Some(sd) = app.collab.shared(d.uid) else { return };
    let sheet = d.wb.active().map(|s| s.name.clone()).unwrap_or_default();
    let range = d.selection.ranges.iter().map(RangeRef::a1).collect::<Vec<_>>().join(",");
    let now = (d.uid, sheet, range, app.editor.is_some());
    if app.collab.presence.as_ref() == Some(&now) || crate::now_ms() - app.collab.presence_at < 80.0 {
        return;
    }
    let book = sd.book.id.clone();
    app.collab.presence_at = crate::now_ms();
    app.collab.send(ClientMsg::Presence { book, sheet: now.1.clone(), range: now.2.clone(), editing: now.3 });
    app.collab.presence = Some(now);
}

fn handle(app: &mut SheetApp, e: Event) {
    match e {
        Event::Connected { me, users, server } => {
            app.collab.status = Status::Online;
            app.collab.me = Some(me);
            app.collab.users = users;
            app.collab.server_name = server;
            app.collab.error = None;
            app.collab.presence = None;
            app.collab.send(ClientMsg::ListBooks);
            app.collab.send(ClientMsg::CheckUpdate { version: gridcraft_collab::APP_VERSION.trim().into() });
            app.collab.last_update_check = crate::now_ms();
            // After a reconnect: fetch every open shared workbook again and resend what the
            // server never ordered.
            let ids: Vec<String> = app.collab.docs.iter().map(|d| d.book.id.clone()).collect();
            for id in ids {
                if !app.collab.opening.contains(&id) {
                    app.collab.opening.push(id.clone());
                }
                app.collab.send(ClientMsg::OpenBook { book: id });
            }
        }
        Event::Disconnected(why) => {
            app.collab.status = Status::Reconnecting(why);
            app.collab.opening.clear();
        }
        Event::Failed(why) => {
            app.collab.client = None;
            app.collab.me = None;
            app.collab.status = Status::SignedOut;
            app.collab.error = Some(why);
        }
        Event::Message(m) => message(app, m),
    }
}

fn message(app: &mut SheetApp, m: ServerMsg) {
    let instance = app.collab.client.as_ref().map(|c| c.instance.clone()).unwrap_or_default();
    match m {
        ServerMsg::Books { books } => {
            app.collab.books = books;
            if let Some(uid) = app.collab.share_after_sign_in.take()
                && let Err(e) = share_doc(app, uid, None)
            {
                app.message = Some(("Dexterity Sheets".into(), e));
            }
        }
        ServerMsg::Opened { book, snapshot_seq, xlsx, log, last_edits, peers } => {
            app.collab.opening.retain(|b| *b != book.id);
            let Ok(bytes) = proto::unb64(&xlsx) else { return };
            let read_only = app.collab.me.as_ref().is_some_and(|m| !m.role.can_edit());
            if let Some(sd) = app.collab.docs.iter_mut().find(|d| d.book.id == book.id) {
                let mine: Vec<u64> = log.iter().filter_map(|l| proto::own_cseq(l.origin.as_deref(), &instance)).collect();
                let Some(doc) = app.session.document_mut(sd.doc_uid) else { return };
                sd.peers = peers;
                for e in last_edits {
                    sd.last_edits.insert((e.sheet, CellRef::new(e.r, e.c)), (e.user, e.time));
                }
                match sd.resync(doc, snapshot_seq, &bytes, &log, book, &mine) {
                    Ok(resend) => {
                        for (cseq, patch) in resend {
                            let id = sd.book.id.clone();
                            if let Some(c) = &app.collab.client {
                                c.send(ClientMsg::Patch { book: id, cseq, patch });
                            }
                        }
                    }
                    Err(e) => app.message = Some(("Dexterity Sheets".into(), format!("Could not bring \"{}\" up to date: {e}", sd.book.name))),
                }
                return;
            }
            match SharedDoc::open(book, snapshot_seq, &bytes, &log, last_edits, peers, read_only) {
                Ok((sd, mut doc)) => {
                    doc.title = sd.book.name.clone();
                    let book = sd.book.clone();
                    arrived(app, &book, &doc);
                    // Replace an untouched blank Book1, as opening a file does.
                    if app.session.documents().len() == 1
                        && app.collab.docs.is_empty()
                        && app.session.active().is_some_and(|d| {
                            d.path.is_none() && !d.is_dirty() && d.undo.is_empty() && d.wb.sheets.iter().all(|sh| sh.cells.is_empty())
                        })
                    {
                        app.session.close_document(0);
                    }
                    app.session.add_document(doc);
                    app.collab.docs.push(sd);
                    app.backstage = None;
                    app.presence_reset();
                }
                Err(e) => app.message = Some(("Dexterity Sheets".into(), format!("Could not open the shared workbook: {e}"))),
            }
        }
        ServerMsg::Patched { book, seq, time, user, origin, patch } => {
            let cseq = proto::own_cseq(origin.as_deref(), &instance);
            let Some(sd) = app.collab.docs.iter_mut().find(|d| d.book.id == book) else { return };
            let Some(doc) = app.session.document_mut(sd.doc_uid) else { return };
            let mut outgoing = None;
            match sd.ordered(doc, seq, &user, time, cseq, &patch) {
                Ok(fx) => outgoing = fx.outgoing,
                Err(e) => sd.rejected(&e.to_string()),
            }
            if let Some((cseq, patch)) = outgoing {
                app.collab.send(ClientMsg::Patch { book: book.clone(), cseq, patch });
            }
            if cseq.is_none() && app.collab.history.asked.as_ref().is_some_and(|a| a.0 == book) {
                // The pane refreshes with the new edit.
                app.collab.history.asked = None;
            }
        }
        ServerMsg::Rejected { book, reason, .. } => {
            if let Some(sd) = app.collab.docs.iter_mut().find(|d| d.book.id == book) {
                sd.rejected(&reason);
            }
            app.toast = Some((format!("The server refused an edit ({reason}); reloading the workbook."), crate::now_ms()));
        }
        ServerMsg::PeerMoved { book, peer } => {
            if let Some(sd) = app.collab.docs.iter_mut().find(|d| d.book.id == book) {
                sd.peers.retain(|p| p.conn != peer.conn);
                sd.peers.push(peer);
            }
        }
        ServerMsg::PeerLeft { book, conn } => {
            if let Some(sd) = app.collab.docs.iter_mut().find(|d| d.book.id == book) {
                sd.peers.retain(|p| p.conn != conn);
            }
        }
        ServerMsg::History { changes, .. } => app.collab.history.changes = changes,
        ServerMsg::Versions { versions, .. } => app.collab.versions = versions,
        ServerMsg::Version { xlsx, .. } => {
            let title = app.collab.want_copy.take().unwrap_or_else(|| "Earlier version".into());
            match proto::unb64(&xlsx)
                .map_err(|e| e.to_string())
                .and_then(|b| gridcraft_engine::io::open_bytes("version.xlsx", &b).map_err(|e| e.to_string()))
            {
                Ok((wb, _)) => {
                    let doc = gridcraft_engine::DocState::new(wb, None, title, gridcraft_engine::Trust::FromFile);
                    app.session.add_document(doc);
                }
                Err(e) => app.message = Some(("Dexterity Sheets".into(), format!("Could not open that version: {e}"))),
            }
        }
        ServerMsg::Reload { book, reason } => {
            app.toast = Some((reason, crate::now_ms()));
            if let Some(sd) = app.collab.docs.iter_mut().find(|d| d.book.id == book) {
                sd.needs_reload = Some("reload".into());
            }
        }
        ServerMsg::BookGone { book, by, stopped } => {
            let file = unlink(app, &book);
            if let Some(i) = app.collab.docs.iter().position(|d| d.book.id == book) {
                let sd = app.collab.docs.remove(i);
                let by = by.unwrap_or_else(|| "Someone".into());
                if let Some((path, None)) = &file {
                    // This PC's file has every edit: the window is that file again.
                    back_to_file(app, sd.doc_uid, path);
                    app.message = Some((
                        "Dexterity Sheets".into(),
                        format!("{by} stopped sharing \"{}\". {} has everyone's edits up to now.", sd.book.name, file_name(path)),
                    ));
                } else {
                    if let Some(doc) = app.session.document_mut(sd.doc_uid) {
                        doc.title = format!("{} ({})", sd.book.name, if stopped { "no longer shared" } else { "deleted on server" });
                        doc.saved = Arc::new(gridcraft_engine::model::Workbook::new());
                    }
                    app.message = Some((
                        "Dexterity Sheets".into(),
                        if stopped {
                            format!(
                                "{by} stopped sharing \"{}\"; their file has everyone's edits. Your window keeps a copy: save it with Save As if you need it.",
                                sd.book.name
                            )
                        } else {
                            format!("{by} deleted \"{}\" on the server. Your window keeps a copy: save it with Save As if you need it.", sd.book.name)
                        },
                    ));
                }
            }
            save_record(&book, None);
        }
        ServerMsg::Update { update } => {
            let update = update.filter(|u| proto::version_newer(&u.version, gridcraft_collab::APP_VERSION.trim()));
            if let Some(u) = &update
                && app.collab.update.as_ref().map(|o| &o.version) != Some(&u.version)
            {
                app.toast = Some((
                    format!("Dexterity Sheets {} is available. Click \"Update\" in the status bar to install it.", u.version),
                    crate::now_ms(),
                ));
            }
            app.collab.update = update;
        }
        ServerMsg::UpdateFile { info, data } => install_update(app, &info, &data),
        ServerMsg::Ok { what } => app.toast = Some((what, crate::now_ms())),
        ServerMsg::Error { message } => app.message = Some(("Dexterity Sheets".into(), message)),
        ServerMsg::Challenge { .. } | ServerMsg::Welcome { .. } | ServerMsg::Pong => {}
    }
}

impl SheetApp {
    fn presence_reset(&mut self) {
        self.collab.presence = None;
    }
}

/// Commands the window handles for sharing. `None` = not a sharing command.
pub fn run_command(app: &mut SheetApp, id: &str, p: &Json) -> Option<Result<Json, String>> {
    let r = match id {
        "collab.shared" => {
            app.open_backstage(crate::backstage::Page::Shared);
            Ok(Json::Null)
        }
        // {cell?: true} shows the active cell's history, otherwise the whole workbook's.
        "collab.history" => {
            if let Some(c) = p.get("cell").and_then(Json::as_bool) {
                app.collab.history.this_cell = c;
            }
            app.grid.pane = Some("history".into());
            app.collab.history.asked = None;
            Ok(Json::Null)
        }
        "collab.versions" => {
            app.grid.pane = Some("versions".into());
            if let Some(sd) = active_shared(app) {
                let book = sd.book.id.clone();
                app.collab.send(ClientMsg::Versions { book });
            }
            Ok(Json::Null)
        }
        "collab.share" => share_active(app, p.get("name").and_then(Json::as_str)),
        "collab.stopSharing" => stop_sharing(app),
        // Help > Check for Updates asks the office server.
        "collab.checkUpdate" | "app.checkForUpdates" | "help.checkForUpdates" => {
            if !app.collab.signed_in() {
                app.open_backstage(crate::backstage::Page::Shared);
                return Some(Err("Updates come from the office server: sign in first (File > Shared).".into()));
            }
            app.collab.send(ClientMsg::CheckUpdate { version: gridcraft_collab::APP_VERSION.trim().into() });
            app.collab.last_update_check = crate::now_ms();
            app.collab.update_dialog = true;
            Ok(json!({"current": gridcraft_collab::APP_VERSION.trim(), "available": app.collab.update.as_ref().map(|u| u.version.clone())}))
        }
        "collab.installUpdate" => match app.collab.update.clone() {
            Some(u) => {
                app.collab.update_state = Some(format!("Downloading {}...", u.version));
                app.collab.send(ClientMsg::GetUpdate { version: u.version });
                Ok(Json::Null)
            }
            None => Err("No update is available.".into()),
        },
        // Scriptable sign-in, for tests and the control channel.
        "collab.signIn" => {
            let s = |k: &str| p.get(k).and_then(Json::as_str).unwrap_or("").to_string();
            app.collab.form_server = s("server");
            app.collab.form_user = s("user");
            app.collab.form_password = s("password");
            app.collab.form_loaded = true;
            let ctx = app.collab_ctx.clone();
            match ctx {
                Some(ctx) => {
                    sign_in(&mut app.collab, &ctx);
                    match app.collab.error.clone() {
                        Some(e) => Err(e),
                        None => Ok(json!({"status": "connecting"})),
                    }
                }
                None => Err("the window is not ready yet".into()),
            }
        }
        "collab.signOut" => {
            sign_out(app);
            Ok(Json::Null)
        }
        // Opens a shared workbook by name or id.
        "collab.open" => {
            let want = p.get("book").and_then(Json::as_str).unwrap_or("").to_string();
            match app.collab.books.iter().find(|b| b.id == want || b.name.eq_ignore_ascii_case(&want)).map(|b| b.id.clone()) {
                Some(id) => {
                    if !app.collab.opening.contains(&id) {
                        app.collab.opening.push(id.clone());
                        app.collab.send(ClientMsg::OpenBook { book: id.clone() });
                    }
                    Ok(json!({"book": id}))
                }
                None => Err(format!("no shared workbook \"{want}\" (the list has {})", app.collab.books.len())),
            }
        }
        "collab.create" => {
            let name = p.get("name").and_then(Json::as_str).unwrap_or("").to_string();
            app.collab.send(ClientMsg::CreateBook { name, xlsx: None, source: None });
            Ok(Json::Null)
        }
        "collab.status" => {
            let c = &app.collab;
            let docs: Vec<Json> = c
                .docs
                .iter()
                .map(|d| {
                    json!({
                        "book": d.book.name, "id": d.book.id, "seq": d.seq, "pending": d.pending_len(),
                        "peers": d.peers.iter().map(|p| json!({"user": p.user, "sheet": p.sheet, "range": p.range, "editing": p.editing})).collect::<Vec<_>>(),
                        "active": app.session.active().is_some_and(|a| a.uid == d.doc_uid),
                        "source": d.book.source.as_ref().map(|s| json!({"path": s.path, "user": s.user, "machine": s.machine})),
                        "file": c.links.iter().find(|l| l.book == d.book.id).map(|l| json!({
                            "path": l.path, "saved": l.up_to_date() && l.error.is_none(), "error": l.error,
                        })),
                    })
                })
                .collect();
            Ok(json!({
                "status": format!("{:?}", c.status),
                "me": c.me.as_ref().map(|m| m.user.clone()),
                "books": c.books.iter().map(|b| json!({"name": b.name, "id": b.id, "openBy": b.open_by, "file": b.source.as_ref().map(|s| s.path.clone())})).collect::<Vec<_>>(),
                "docs": docs,
                "error": c.error,
            }))
        }
        // Who last changed a cell of the active shared workbook: {cell?: "A1", sheet?}.
        "collab.lastEdit" => {
            let Some(d) = app.session.active() else { return Some(Err("no workbook".into())) };
            let Some(sd) = app.collab.shared(d.uid) else { return Some(Err("not a shared workbook".into())) };
            let sheet =
                p.get("sheet").and_then(Json::as_str).map(str::to_string).or_else(|| d.wb.active().map(|s| s.name.clone())).unwrap_or_default();
            let at = p.get("cell").and_then(Json::as_str).and_then(CellRef::parse).unwrap_or(d.selection.active);
            Ok(match sd.last_edit(&sheet, at) {
                Some((u, t)) => json!({"user": u, "time": t}),
                None => Json::Null,
            })
        }
        // Save on a shared workbook keeps a version on the server: every edit is already saved.
        // A shared file is also written to its file now.
        "file.save" if active_shared(app).is_some() && p.get("path").is_none() => {
            let book = active_shared(app).map(|s| s.book.id.clone()).unwrap_or_default();
            let who = app.collab.me.as_ref().map(|m| m.display.clone()).unwrap_or_default();
            app.collab.send(ClientMsg::SaveVersion { book: book.clone(), label: format!("Saved by {who}") });
            let wb = app.session.active().map(|d| d.wb.clone());
            let mut note = "Every change is saved on the server as you work. A version was kept as well.".to_string();
            if let Some(l) = app.collab.links.iter_mut().find(|l| l.book == book)
                && let Some(wb) = wb
            {
                l.step(&wb);
                l.flush();
                note = match &l.error {
                    Some(e) => e.clone(),
                    None => format!("Saved to {} and kept a version on the server.", l.path),
                };
            }
            app.toast = Some((note, crate::now_ms()));
            Ok(Json::Null)
        }
        _ => return None,
    };
    Some(r)
}

fn share_active(app: &mut SheetApp, name: Option<&str>) -> Result<Json, String> {
    let uid = app.session.active().ok_or("no workbook open")?.uid;
    if !app.collab.signed_in() {
        // Sign in first; the workbook is shared right after.
        app.collab.share_after_sign_in = Some(uid);
        app.open_backstage(crate::backstage::Page::Shared);
        app.toast = Some(("Sign in to the office server: the workbook is shared as soon as you are in.".into(), crate::now_ms()));
        return Ok(json!({"after": "signIn"}));
    }
    share_doc(app, uid, name)
}

/// Shares the workbook in window `uid`. A saved file stays linked: this app keeps writing
/// everyone's edits to it, wherever it is (a project folder, a network drive).
fn share_doc(app: &mut SheetApp, uid: u64, name: Option<&str>) -> Result<Json, String> {
    let d = app.session.documents().iter().find(|d| d.uid == uid).ok_or("that workbook is no longer open")?;
    if app.collab.shared(uid).is_some() {
        return Err("This workbook is already shared.".into());
    }
    if app.collab.me.as_ref().is_some_and(|m| !m.role.can_edit()) {
        return Err("Your account is view-only: ask your administrator to share it, or for edit access.".into());
    }
    let path = d.path.clone().filter(|p| std::path::Path::new(p).is_file());
    let key = path.as_deref().and_then(key_of);
    // The file is already shared (by someone else, or before): join it.
    if let Some(k) = &key
        && let Some(book) = app.collab.books.iter().find(|b| b.source.as_ref().is_some_and(|s| &s.key == k)).cloned()
    {
        join_file(app, uid, &book, true);
        return Ok(json!({"joined": book.name}));
    }
    let title = d.title.clone();
    let stem = path
        .as_deref()
        .and_then(|p| std::path::Path::new(p).file_stem().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| title.trim_end_matches(".xlsx").to_string());
    let name = name.map(str::to_string).filter(|n| !n.trim().is_empty()).unwrap_or(stem);
    let bytes = gridcraft_engine::io::save_bytes(&d.wb, "share.xlsx").map_err(|e| e.to_string())?;
    let wb = d.wb.clone();
    let me = app.collab.me.as_ref().map(|m| m.user.clone()).unwrap_or_default();
    let (source, link, note) = match (&path, &key) {
        (Some(p), Some(k)) if can_write_back(p) => (
            Some(proto::FileSource { path: p.clone(), key: k.clone(), user: me, machine: machine() }),
            Some((p.clone(), stamp_of(p))),
            format!("Sharing {}… your file keeps everyone's edits.", file_name(p)),
        ),
        (Some(p), Some(k)) => (
            // A format this app can't write (.xls, .ods): shared, and the file is left alone.
            Some(proto::FileSource { path: p.clone(), key: k.clone(), user: me, machine: machine() }),
            None,
            format!("Sharing {}… it can't be written back in its format: use Save As .xlsx to keep the team's edits as a file.", file_name(p)),
        ),
        _ => (None, None, "Sharing… tip: save the workbook in a folder first, and that file keeps everyone's edits.".to_string()),
    };
    app.collab.joining.push(Joining { key: source.as_ref().map(|s| s.key.clone()), name: name.clone(), doc_uid: uid, wb, link });
    app.collab.send(ClientMsg::CreateBook { name, xlsx: Some(proto::b64(&bytes)), source });
    app.toast = Some((note, crate::now_ms()));
    Ok(Json::Null)
}

// ---------------------------------------------------------------------------------------------
// Files: a workbook shared from a folder stays that file
//
// Sharing a saved workbook links the shared workbook to its file: the app of the person who
// shared it keeps the file saved with everyone's edits, so the file in their folder (or on the
// network drive) is always the team's latest. Opening a file that is being shared joins it
// instead of starting a second copy. Stop sharing hands the file back with every edit in it.

/// The file as it was last written here: (size, modified in ms since 1970).
type Stamp = (u64, i64);

/// A shared workbook that is a file this app keeps saved.
pub struct LinkedFile {
    pub book: String,
    pub path: String,
    /// The workbook as last written to the file.
    written: Arc<Workbook>,
    /// The newest content, and when it last changed.
    latest: Arc<Workbook>,
    changed_at: f64,
    /// What the file looked like after our last write: a different file means another program
    /// changed it, and it is then never overwritten.
    stamp: Option<Stamp>,
    writing: Option<(Arc<Workbook>, Receiver<Result<Stamp, String>>)>,
    retry_at: f64,
    /// The file was changed outside: writes stopped.
    blocked: bool,
    pub saved_at: Option<i64>,
    pub error: Option<String>,
}

/// A workbook on its way to being shared or joined; when it arrives it takes this window's place.
struct Joining {
    /// The file's key, or for a workbook that is no file, its name.
    key: Option<String>,
    name: String,
    doc_uid: u64,
    /// The window's content when asked: if the person typed in it meanwhile, it is kept.
    wb: Arc<Workbook>,
    /// This app keeps the file saved: (path, the file's stamp now).
    link: Option<(String, Option<Stamp>)>,
}

/// Files this PC wrote for shared workbooks (book id → key and stamp), kept across restarts so
/// that reopening a file can tell whether something else changed it meanwhile.
#[derive(Default, Serialize, Deserialize)]
struct LinkRecords {
    books: std::collections::BTreeMap<String, LinkRecord>,
}

#[derive(Clone, Serialize, Deserialize)]
struct LinkRecord {
    key: String,
    path: String,
    size: u64,
    modified: i64,
}

fn records_path() -> Option<std::path::PathBuf> {
    remembered_path().map(|p| p.with_file_name("linked-files.json"))
}

fn load_records() -> LinkRecords {
    records_path().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save_record(book: &str, rec: Option<LinkRecord>) {
    let Some(p) = records_path() else { return };
    let mut all = load_records();
    match rec {
        Some(r) => all.books.insert(book.to_string(), r),
        None => all.books.remove(book),
    };
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(b) = serde_json::to_vec_pretty(&all) {
        let _ = gridcraft_engine::io::write_file(&p.display().to_string(), &b);
    }
}

fn machine() -> String {
    std::env::var("COMPUTERNAME").unwrap_or_default()
}

/// The key that identifies the file at `path` on every PC, if it exists.
fn key_of(path: &str) -> Option<String> {
    let canonical = std::fs::canonicalize(path).ok()?;
    Some(proto::file_key(&machine(), &canonical.display().to_string()))
}

fn stamp_of(path: &str) -> Option<Stamp> {
    let m = std::fs::metadata(path).ok()?;
    let modified = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis();
    Some((m.len(), i64::try_from(modified).unwrap_or(i64::MAX)))
}

/// Formats this app can write back to the file a workbook was shared from.
fn can_write_back(path: &str) -> bool {
    use gridcraft_engine::io::FileKind;
    matches!(FileKind::from_path(path), Some(FileKind::Xlsx | FileKind::Csv | FileKind::Tsv))
}

fn file_name(path: &str) -> String {
    std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string())
}

/// Writes the workbook to its file (temporary file, then rename) and returns the new stamp.
fn write_workbook(wb: &Workbook, path: &str) -> Result<Stamp, String> {
    let bytes = gridcraft_engine::io::save_file_bytes(wb, path).map_err(|e| e.to_string())?;
    gridcraft_engine::io::write_file(path, &bytes).map_err(|e| e.to_string())?;
    stamp_of(path).ok_or_else(|| format!("{path}: written, but it can't be read back"))
}

impl LinkedFile {
    fn new(book: &str, path: &str, wb: &Arc<Workbook>, stamp: Option<Stamp>) -> LinkedFile {
        LinkedFile {
            book: book.to_string(),
            path: path.to_string(),
            // Nothing written yet: the file is brought up to date at once.
            written: Arc::new(Workbook::new()),
            latest: wb.clone(),
            changed_at: 0.0,
            stamp,
            writing: None,
            retry_at: 0.0,
            blocked: false,
            saved_at: None,
            error: None,
        }
    }

    pub fn up_to_date(&self) -> bool {
        Arc::ptr_eq(&self.written, &self.latest) && self.writing.is_none()
    }

    /// The file is still what this app last wrote (or is gone, which a write recreates).
    fn unchanged_outside(&mut self) -> bool {
        if self.blocked {
            return false;
        }
        let now = stamp_of(&self.path);
        if self.stamp.is_some() && now.is_some() && now != self.stamp {
            self.blocked = true;
            self.error = Some(format!(
                "{} was changed by another program, so it is no longer overwritten. Everyone's edits are safe in the shared workbook: use Save As to keep them as a file.",
                file_name(&self.path)
            ));
            return false;
        }
        true
    }

    fn wrote(&mut self, wb: Arc<Workbook>, r: Result<Stamp, String>) {
        match r {
            Ok(stamp) => {
                self.written = wb;
                self.stamp = Some(stamp);
                self.saved_at = Some(crate::now_ms() as i64);
                self.error = None;
                if let Some(key) = key_of(&self.path) {
                    save_record(&self.book, Some(LinkRecord { key, path: self.path.clone(), size: stamp.0, modified: stamp.1 }));
                }
            }
            Err(e) => {
                // Locked (open in another program), the drive offline: try again shortly.
                self.error = Some(format!("{} could not be saved: {e}. Trying again…", file_name(&self.path)));
                self.retry_at = crate::now_ms() + 10_000.0;
            }
        }
    }

    /// Writes now, waiting for it: for a window that closes and for the app quitting.
    fn flush(&mut self) {
        if let Some((wb, rx)) = self.writing.take()
            && let Ok(r) = rx.recv()
        {
            self.wrote(wb, r);
        }
        if Arc::ptr_eq(&self.written, &self.latest) || !self.unchanged_outside() {
            return;
        }
        let wb = self.latest.clone();
        let r = write_workbook(&wb, &self.path);
        self.wrote(wb, r);
    }

    /// Each frame: notices new content and writes it a moment after the last change, in the
    /// background so a large workbook never stalls typing.
    fn step(&mut self, wb: &Arc<Workbook>) {
        let now = crate::now_ms();
        if !Arc::ptr_eq(wb, &self.latest) {
            self.latest = wb.clone();
            self.changed_at = now;
        }
        if let Some((_, rx)) = &self.writing
            && let Ok(r) = rx.try_recv()
            && let Some((done, _)) = self.writing.take()
        {
            self.wrote(done, r);
        }
        if self.writing.is_some() || Arc::ptr_eq(&self.written, &self.latest) || now < self.retry_at || now - self.changed_at < 1000.0 {
            return;
        }
        if !self.unchanged_outside() {
            return;
        }
        let wb = self.latest.clone();
        let path = self.path.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let job = wb.clone();
        match std::thread::Builder::new().name("dexsheets-save-file".into()).spawn(move || {
            let _ = tx.send(write_workbook(&job, &path));
        }) {
            Ok(_) => self.writing = Some((wb, rx)),
            Err(_) => {
                let r = write_workbook(&wb, &self.path);
                self.wrote(wb, r);
            }
        }
    }
}

impl Drop for Collab {
    /// Quitting: every shared file gets its last edits.
    fn drop(&mut self) {
        for l in &mut self.links {
            l.flush();
        }
    }
}

/// Keeps every linked file saved, and joins open files that are being shared.
fn keep_files(app: &mut SheetApp) {
    let mut told = None;
    for l in app.collab.links.iter_mut() {
        let Some(sd) = app.collab.docs.iter().find(|d| d.book.id == l.book) else { continue };
        let Some(doc) = app.session.documents().iter().find(|d| d.uid == sd.doc_uid) else { continue };
        let was_blocked = l.blocked;
        l.step(&doc.wb);
        if l.blocked && !was_blocked {
            told = l.error.clone();
        }
    }
    if let Some(m) = told {
        app.message = Some(("Dexterity Sheets".into(), m));
    }
    if matches!(app.collab.status, Status::Online) && app.collab.me.is_some() {
        join_open_files(app);
    }
}

/// An open file that is also a shared workbook joins it, so nobody edits a copy of their own.
fn join_open_files(app: &mut SheetApp) {
    let mut found = Vec::new();
    for d in app.session.documents() {
        if app.collab.shared(d.uid).is_some() || app.collab.told.contains(&d.uid) || app.collab.joining.iter().any(|j| j.doc_uid == d.uid) {
            continue;
        }
        let Some(path) = d.path.clone() else { continue };
        let key = app.collab.keys.entry(d.uid).or_insert_with(|| key_of(&path)).clone();
        let Some(key) = key else { continue };
        if let Some(b) = app.collab.books.iter().find(|b| b.source.as_ref().is_some_and(|s| s.key == key)) {
            found.push((d.uid, b.clone()));
        }
    }
    for (uid, book) in found {
        app.collab.told.insert(uid);
        join_file(app, uid, &book, false);
    }
}

/// Joins `book`, the shared workbook of the file open in window `uid`.
fn join_file(app: &mut SheetApp, uid: u64, book: &BookInfo, asked: bool) {
    let Some(d) = app.session.documents().iter().find(|d| d.uid == uid) else { return };
    let Some(src) = book.source.clone() else { return };
    let path = d.path.clone().unwrap_or_default();
    let by = app.collab.display_of(&src.user);
    let me = app.collab.me.as_ref().map(|m| m.user.clone()).unwrap_or_default();
    let mine = src.user == me && src.machine.eq_ignore_ascii_case(&machine());
    if d.is_dirty() {
        app.message = Some((
            "Dexterity Sheets".into(),
            format!(
                "{} is shared with the team as \"{}\" (shared by {by}). This window has changes that are not in it, so it was left as it is.\n\nSave your changes under another name (Save As), then open the file again to work with the team.",
                file_name(&path),
                book.name
            ),
        ));
        return;
    }
    // Only the person who shared the file saves it, and only if nothing else changed it since.
    let link = if mine {
        let rec = load_records().books.get(&book.id).cloned();
        let now = stamp_of(&path);
        match rec {
            Some(r) if now == Some((r.size, r.modified)) => Some((path.clone(), now)),
            _ => {
                app.message = Some((
                    "Dexterity Sheets".into(),
                    format!(
                        "{} changed since it was last saved from the shared workbook \"{}\", so it was not replaced.\n\nBoth are open: your file in this window, the team's shared workbook in another. Copy what you need into the shared one; then Share › Stop sharing writes it to your file.",
                        file_name(&path),
                        book.name
                    ),
                ));
                app.collab.opening.push(book.id.clone());
                app.collab.send(ClientMsg::OpenBook { book: book.id.clone() });
                return;
            }
        }
    } else {
        None
    };
    app.collab.joining.push(Joining { key: Some(src.key.clone()), name: book.name.clone(), doc_uid: uid, wb: d.wb.clone(), link });
    app.collab.opening.push(book.id.clone());
    app.collab.send(ClientMsg::OpenBook { book: book.id.clone() });
    if !asked {
        app.toast = Some((
            if mine {
                format!("{} is shared with your team: you are back in it, with everyone's edits.", file_name(&path))
            } else {
                format!("{} is being edited live by the team (shared by {by}): you joined them.", file_name(&path))
            },
            crate::now_ms(),
        ));
    }
}

/// A shared workbook arrived: if it is one this app asked to share or join from a window, it
/// takes that window's place (and its file stays linked).
fn arrived(app: &mut SheetApp, book: &BookInfo, doc: &gridcraft_engine::DocState) {
    let me = app.collab.me.as_ref().map(|m| m.user.clone()).unwrap_or_default();
    let Some(i) = app.collab.joining.iter().position(|j| match &j.key {
        Some(k) => book.source.as_ref().is_some_and(|s| &s.key == k),
        None => book.source.is_none() && book.created_by == me && book.name == j.name,
    }) else {
        return;
    };
    let j = app.collab.joining.remove(i);
    if let Some((path, stamp)) = &j.link {
        app.collab.links.retain(|l| l.book != book.id);
        app.collab.links.push(LinkedFile::new(&book.id, path, &doc.wb, *stamp));
    }
    // The window it came from closes, unless the person typed in it meanwhile.
    if let Some(at) = app.session.document_index(j.doc_uid)
        && app.session.documents().get(at).is_some_and(|d| Arc::ptr_eq(&d.wb, &j.wb))
    {
        app.session.close_document(at);
    }
}

/// The window's linked file (path), when this app keeps it saved.
pub fn linked_file(app: &SheetApp, uid: u64) -> Option<&LinkedFile> {
    let sd = app.collab.shared(uid)?;
    app.collab.links.iter().find(|l| l.book == sd.book.id)
}

/// A shared workbook stops being shared here (closed, signed out, stopped, deleted): its file
/// gets the last edits. Returns the file's path.
fn unlink(app: &mut SheetApp, book: &str) -> Option<(String, Option<String>)> {
    let i = app.collab.links.iter().position(|l| l.book == book)?;
    let mut l = app.collab.links.remove(i);
    if let Some(sd) = app.collab.docs.iter().find(|d| d.book.id == book)
        && let Some(doc) = app.session.documents().iter().find(|d| d.uid == sd.doc_uid)
    {
        l.step(&doc.wb);
    }
    l.flush();
    let ok = l.up_to_date() && !l.blocked;
    Some((l.path.clone(), (!ok).then(|| l.error.clone().unwrap_or_else(|| "the file could not be saved".into()))))
}

/// Turns a shared window back into its file, after [`unlink`].
fn back_to_file(app: &mut SheetApp, uid: u64, path: &str) {
    if let Some(doc) = app.session.document_mut(uid) {
        doc.path = Some(path.to_string());
        doc.title = file_name(path);
        doc.saved = doc.wb.clone();
    }
    app.collab.told.insert(uid);
}

/// The Share menu's sharing entries (title bar).
pub fn share_menu(app: &mut SheetApp, ui: &mut Ui) {
    let uid = app.session.active().map(|d| d.uid);
    let shared = active_shared(app).is_some();
    let linked = uid.and_then(|u| linked_file(app, u)).is_some();
    if !shared && ui.button("Share with my team…").on_hover_text("Everyone edits it live; a saved file keeps everyone's edits").clicked() {
        app.run_or_alert("collab.share", json!({}));
    }
    if linked && ui.button("Stop sharing (keep my file)").clicked() {
        app.run_or_alert("collab.stopSharing", json!({}));
    }
    if ui.button("Shared workbooks…").clicked() {
        app.run_or_alert("collab.shared", json!({}));
    }
}

/// Stops sharing the active workbook: its file gets every edit, the server's copy goes to its
/// trash folder, and everyone else keeps a copy in their window.
fn stop_sharing(app: &mut SheetApp) -> Result<Json, String> {
    let d = app.session.active().ok_or("no workbook open")?;
    let uid = d.uid;
    let sd = app.collab.shared(uid).ok_or("This workbook is not shared.")?;
    let book = sd.book.clone();
    if linked_file(app, uid).is_none() {
        return Err("Only the person who shared this file can stop sharing it, from the PC it was shared from.".into());
    }
    if let Some(l) = app.collab.links.iter().find(|l| l.book == book.id)
        && l.blocked
    {
        return Err(l.error.clone().unwrap_or_default());
    }
    let (path, err) = unlink(app, &book.id).ok_or("not linked")?;
    if let Some(e) = err {
        // Nothing is lost: the workbook stays shared until its file can be written.
        if let Some(sd) = app.collab.docs.iter().find(|d| d.book.id == book.id)
            && let Some(doc) = app.session.documents().iter().find(|d| d.uid == sd.doc_uid)
        {
            let stamp = stamp_of(&path);
            app.collab.links.push(LinkedFile::new(&book.id, &path, &doc.wb, stamp));
        }
        return Err(format!("Still shared: {e}"));
    }
    app.collab.send(ClientMsg::DeleteBook { book: book.id.clone(), stopped: true });
    app.collab.docs.retain(|d| d.book.id != book.id);
    save_record(&book.id, None);
    back_to_file(app, uid, &path);
    app.toast = Some((format!("Stopped sharing. {} has everyone's edits.", file_name(&path)), crate::now_ms()));
    Ok(json!({"file": path}))
}

// ---------------------------------------------------------------------------------------------
// File › Shared

pub fn shared_page(app: &mut SheetApp, ui: &mut Ui, close: &mut bool) {
    let t = Tokens::get(ui.ctx());
    if !app.collab.form_loaded {
        let r = load_remembered();
        app.collab.form_server = r.server;
        app.collab.form_user = r.user;
        app.collab.form_loaded = true;
        start_finding(&mut app.collab);
    }
    if let Some(err) = app.collab.error.clone() {
        ui.label(RichText::new(err).color(t.danger));
        ui.add_space(8.0);
    }
    match (&app.collab.status, app.collab.me.clone()) {
        (Status::SignedOut, _) => sign_in_form(app, ui, &t),
        (Status::Connecting, _) => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(format!("Signing in to {}…", app.collab.client.as_ref().map(|c| c.config.server.clone()).unwrap_or_default()));
            });
            if ui.button("Cancel").clicked() {
                app.collab.client = None;
                app.collab.status = Status::SignedOut;
            }
        }
        (_, Some(me)) => books_list(app, ui, &t, &me, close),
        (_, None) => {
            ui.spinner();
        }
    }
}

fn start_finding(c: &mut Collab) {
    let (tx, rx) = std::sync::mpsc::channel();
    c.finding = Some(rx);
    let _ = std::thread::Builder::new().name("dexsheets-discover".into()).spawn(move || {
        let _ = tx.send(discovery::find(std::time::Duration::from_millis(900)));
    });
}

fn sign_in_form(app: &mut SheetApp, ui: &mut Ui, t: &Tokens) {
    ui.label(RichText::new("Share any workbook with your team, wherever it is saved, and edit it together live. A file you share keeps everyone's edits; the office server also keeps every change with the name and time, so you can always see who changed what.").color(t.text_dim));
    ui.add_space(14.0);
    let c = &mut app.collab;
    ui.label(RichText::new("Servers on this network").font(theme::ui_bold(13.5)));
    ui.horizontal(|ui| {
        if c.finding.is_some() {
            ui.spinner();
            ui.label("Looking…");
        } else if c.found.is_empty() {
            ui.label(RichText::new("None answered. Type the server's computer name or IP address below.").color(t.text_dim));
        }
        if c.finding.is_none() && ui.small_button("Look again").clicked() {
            start_finding(c);
        }
    });
    for f in c.found.clone() {
        let picked = c.form_server == f.address;
        if ui.selectable_label(picked, format!("{}   ({})", f.name, f.address)).clicked() {
            c.form_server = f.address.clone();
        }
    }
    ui.add_space(10.0);
    let mut submit = false;
    egui::Grid::new("collab_signin").num_columns(2).spacing(vec2(14.0, 8.0)).show(ui, |ui| {
        ui.label("Server");
        ui.add(egui::TextEdit::singleline(&mut c.form_server).hint_text("computer name or IP, e.g. DDS-SERVER").desired_width(280.0));
        ui.end_row();
        ui.label("User name");
        ui.add(egui::TextEdit::singleline(&mut c.form_user).desired_width(280.0));
        ui.end_row();
        ui.label("Password");
        let r = ui.add(egui::TextEdit::singleline(&mut c.form_password).password(true).desired_width(280.0));
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            submit = true;
        }
        ui.end_row();
    });
    ui.add_space(10.0);
    if ui.add(egui::Button::new(RichText::new("Sign in").color(t.on_accent)).fill(t.accent).min_size(vec2(110.0, 30.0))).clicked() {
        submit = true;
    }
    if submit {
        let ctx = ui.ctx().clone();
        sign_in(c, &ctx);
    }
    ui.add_space(10.0);
    ui.label(RichText::new("Accounts are created by your administrator on the server. There is no guest access.").color(t.text_dim).small());
}

fn books_list(app: &mut SheetApp, ui: &mut Ui, t: &Tokens, me: &UserInfo, close: &mut bool) {
    let online = matches!(app.collab.status, Status::Online);
    ui.horizontal(|ui| {
        ui.label(RichText::new("●").color(if online { Color32::from_rgb(0x2c, 0xa0, 0x2c) } else { t.danger }));
        let role = match me.role {
            Role::Admin => "administrator",
            Role::Editor => "editor",
            Role::Viewer => "view only",
        };
        ui.label(format!("Signed in as {} ({}) on {}", me.display, role, app.collab.server_name));
        if let Status::Reconnecting(why) = &app.collab.status {
            ui.label(RichText::new(format!("— reconnecting ({why})")).color(t.danger));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Sign out").clicked() {
                sign_out(app);
            }
            if ui.button("Refresh").clicked() {
                app.collab.send(ClientMsg::ListBooks);
            }
        });
    });
    ui.add_space(14.0);
    if me.role.can_edit() {
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut app.collab.new_name).hint_text("Name of a new shared workbook").desired_width(300.0));
            if ui.button("New shared workbook").clicked() && !app.collab.new_name.trim().is_empty() {
                let name = std::mem::take(&mut app.collab.new_name);
                app.collab.send(ClientMsg::CreateBook { name, xlsx: None, source: None });
            }
            let can_share = app.session.active().is_some_and(|d| app.collab.shared(d.uid).is_none());
            if ui
                .add_enabled(can_share, egui::Button::new("Share the current workbook"))
                .on_hover_text(
                    "Shares the workbook in the active window. If it is a saved file, the file stays where it is and keeps everyone's edits.",
                )
                .clicked()
            {
                let name = (!app.collab.new_name.trim().is_empty()).then(|| std::mem::take(&mut app.collab.new_name));
                let r = share_active(app, name.as_deref());
                if let Err(e) = r {
                    app.collab.error = Some(e);
                }
            }
        });
        ui.add_space(12.0);
    }
    ui.label(RichText::new("Shared workbooks").font(theme::ui_bold(15.0)));
    ui.add_space(6.0);
    if app.collab.books.is_empty() {
        ui.label(RichText::new("No shared workbooks yet.").color(t.text_dim));
    }
    let mut open = None;
    let mut delete = None;
    egui::Grid::new("collab_books").num_columns(4).striped(true).spacing(vec2(18.0, 8.0)).show(ui, |ui| {
        ui.label(RichText::new("Name").strong());
        ui.label(RichText::new("Last change").strong());
        ui.label(RichText::new("Open now by").strong());
        ui.label("");
        ui.end_row();
        let mut books = app.collab.books.clone();
        books.sort_by_key(|b| std::cmp::Reverse(b.modified));
        for b in books {
            ui.vertical(|ui| {
                let one_line = |r: RichText| egui::Label::new(r).wrap_mode(egui::TextWrapMode::Extend);
                ui.add(one_line(RichText::new(&b.name).font(theme::ui_bold(13.5))));
                if let Some(src) = &b.source {
                    // The folder and the file name; the whole path on hover.
                    let p = std::path::Path::new(&src.path);
                    let short = match p.parent().and_then(|f| f.file_name()) {
                        Some(folder) => format!("{}\\{}", folder.to_string_lossy(), file_name(&src.path)),
                        None => file_name(&src.path),
                    };
                    ui.add(one_line(RichText::new(format!("{}'s file · {short}", app.collab.display_of(&src.user))).color(t.text_dim).small()))
                        .on_hover_text(format!("Shared from {} on {}. Opening that file joins this workbook.", src.path, src.machine));
                }
            });
            ui.label(format!("{} by {}", when(b.modified), app.collab.display_of(&b.modified_by)));
            ui.label(if b.open_by.is_empty() { "—".to_string() } else { b.open_by.join(", ") });
            ui.horizontal(|ui| {
                if ui.button("Open").clicked() {
                    open = Some(b.id.clone());
                }
                if (me.role == Role::Admin || b.created_by == me.user) && ui.small_button("Delete…").clicked() {
                    delete = Some((b.id.clone(), b.name.clone()));
                }
            });
            ui.end_row();
        }
    });
    if let Some(id) = open {
        if let Some(sd) = app.collab.docs.iter().find(|d| d.book.id == id) {
            if let Some(i) = app.session.document_index(sd.doc_uid) {
                app.session.set_active(i);
            }
            *close = true;
        } else if !app.collab.opening.contains(&id) {
            app.collab.opening.push(id.clone());
            app.collab.send(ClientMsg::OpenBook { book: id });
        }
    }
    if let Some((id, name)) = delete {
        app.collab.confirm_restore = Some(format!("delete:{id}:{name}"));
    }
    if let Some(c) = app.collab.confirm_restore.clone()
        && let Some(rest) = c.strip_prefix("delete:")
    {
        let (id, name) = rest.split_once(':').unwrap_or((rest, ""));
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("Delete \"{name}\" for everyone? The server keeps it in its trash folder.")).color(t.danger));
            if ui.button("Delete").clicked() {
                app.collab.send(ClientMsg::DeleteBook { book: id.to_string(), stopped: false });
                app.collab.confirm_restore = None;
            }
            if ui.button("Cancel").clicked() {
                app.collab.confirm_restore = None;
            }
        });
    }
    if !app.collab.opening.is_empty() {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label("Opening…");
        });
    }
    ui.add_space(22.0);
    ui.collapsing("Change my password", |ui| {
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut app.collab.new_password)
                    .password(true)
                    .hint_text("new password (6+ characters)")
                    .desired_width(220.0),
            );
            if ui.button("Change").clicked() && app.collab.new_password.chars().count() >= 6 {
                let p = std::mem::take(&mut app.collab.new_password);
                app.collab.send(ClientMsg::ChangePassword { old_proof: String::new(), new_password: p });
            }
        });
    });
}

// ---------------------------------------------------------------------------------------------
// Grid: other people's selections

pub fn paint_peers(p: &Painter, geo: &Geo, sh: &Sheet, app: &SheetApp, rows: (u32, u32), cols: (u32, u32)) {
    let Some(sd) = active_shared(app) else { return };
    let me = app.collab.client.as_ref().map(|c| c.instance.clone());
    let _ = me;
    for peer in &sd.peers {
        if peer.sheet != sh.name {
            continue;
        }
        let c = color(peer.color);
        for (i, part) in peer.range.split(',').enumerate() {
            let Some(r) = RangeRef::parse(part.trim()) else { continue };
            let visible = r.start.row <= rows.1 && r.end.row >= rows.0 && r.start.col <= cols.1 && r.end.col >= cols.0;
            if !visible {
                continue;
            }
            let rect = geo.range_rect(sh, r);
            p.rect_filled(rect, 0.0, c.gamma_multiply(0.10));
            p.rect_stroke(rect, 0.0, Stroke::new(2.0, c), StrokeKind::Inside);
            if i == 0 {
                let label = if peer.editing { format!("{} (typing)", peer.display) } else { peer.display.clone() };
                let font = FontId::proportional(11.0);
                let galley = p.layout_no_wrap(label, font, Color32::WHITE);
                let tag = egui::Rect::from_min_size(pos2(rect.left(), rect.top() - galley.size().y - 2.0), galley.size() + vec2(8.0, 2.0));
                p.rect_filled(tag, 2.0, c);
                p.galley(tag.min + vec2(4.0, 1.0), galley, Color32::WHITE);
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Status bar

pub fn status(app: &mut SheetApp, ui: &mut Ui, t: &Tokens) {
    let Some(d) = app.session.active() else { return };
    let Some(sd) = app.collab.shared(d.uid) else { return };
    let small = |s: String, c: Color32| RichText::new(s).font(theme::ui_font(12.5)).color(c);
    ui.add_space(12.0);
    let (dot, text) = match &app.collab.status {
        Status::Online if sd.has_pending() => (Color32::from_rgb(0xe0, 0xa0, 0x00), "Shared · saving…".to_string()),
        Status::Online => (Color32::from_rgb(0x2c, 0xa0, 0x2c), "Shared · saved".to_string()),
        Status::Reconnecting(_) => (t.danger, format!("Offline · {} edit(s) waiting", sd.pending_len())),
        _ => (t.danger, "Not signed in".to_string()),
    };
    ui.add(egui::Label::new(small("●".into(), dot)).selectable(false));
    ui.add(egui::Label::new(small(text, t.text_dim)).selectable(false));
    if sd.read_only {
        ui.add(egui::Label::new(small(" · view only".into(), t.danger)).selectable(false));
    }
    if let Some(l) = app.collab.links.iter().find(|l| l.book == sd.book.id) {
        let (txt, c) = match &l.error {
            Some(_) => (format!(" · {} not updated", file_name(&l.path)), t.danger),
            None if l.up_to_date() => (format!(" · in {}", file_name(&l.path)), t.text_dim),
            None => (format!(" · saving to {}…", file_name(&l.path)), t.text_dim),
        };
        let hover = l.error.clone().unwrap_or_else(|| format!("Everyone's edits are saved to your file: {}", l.path));
        ui.add(egui::Label::new(small(txt, c)).selectable(false)).on_hover_text(hover);
    } else if let Some(src) = &sd.book.source {
        ui.add(egui::Label::new(small(format!(" · {}'s file", app.collab.display_of(&src.user)), t.text_dim)).selectable(false))
            .on_hover_text(format!("Shared from {} on {}; it keeps everyone's edits.", src.path, src.machine));
    }
    // Who else is here.
    let mut seen: Vec<String> = Vec::new();
    for p in &sd.peers {
        if seen.contains(&p.user) {
            continue;
        }
        seen.push(p.user.clone());
        ui.add_space(6.0);
        let r = ui.add(egui::Label::new(small(format!("● {}", p.display), color(p.color))).selectable(false).sense(egui::Sense::click()));
        let r = r.on_hover_text(format!("{} is on {}!{}{}", p.display, p.sheet, p.range, if p.editing { " (typing)" } else { "" }));
        if r.clicked() {
            go_to_peer(app, p.clone());
            return;
        }
    }
    // Who last changed the active cell.
    let sheet = d.wb.active().map(|s| s.name.clone()).unwrap_or_default();
    if let Some((user, time)) = sd.last_edit(&sheet, d.selection.active) {
        ui.add_space(12.0);
        let txt = format!("{}: last changed by {}, {}", d.selection.active.a1(), app.collab.display_of(user), when(*time));
        let r =
            ui.add(egui::Label::new(small(txt, t.text_dim)).selectable(false).sense(egui::Sense::click())).on_hover_text("Show this cell's history");
        if r.clicked() {
            app.collab.history.this_cell = true;
            app.collab.history.asked = None;
            app.grid.pane = Some("history".into());
        }
    }
}

fn go_to_peer(app: &mut SheetApp, p: Peer) {
    let _ = app.run("sheet.activate", json!({"sheet": p.sheet}));
    let first = p.range.split(',').next().unwrap_or("").to_string();
    if !first.is_empty() {
        let _ = app.run("selection.set", json!({"range": first}));
        app.grid.ensure_visible = true;
    }
}

// ---------------------------------------------------------------------------------------------
// Panes

pub fn history_pane(app: &mut SheetApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(d) = app.session.active() else { return };
    let Some(sd) = app.collab.shared(d.uid) else {
        ui.label(RichText::new("Edit history is kept for shared workbooks. Open one from File › Shared, or share this workbook there.").italics());
        return;
    };
    let book = sd.book.id.clone();
    let sheet = d.wb.active().map(|s| s.name.clone()).unwrap_or_default();
    let cell = d.selection.active.a1();
    ui.horizontal(|ui| {
        ui.selectable_value(&mut app.collab.history.this_cell, true, format!("This cell ({cell})"));
        ui.selectable_value(&mut app.collab.history.this_cell, false, "Whole workbook");
    });
    let mut user = app.collab.history.user.clone();
    egui::ComboBox::from_id_salt("collab_history_user")
        .selected_text(user.as_deref().map(|u| app.collab.display_of(u)).unwrap_or_else(|| "Everyone".into()))
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut user, None, "Everyone");
            for u in &app.collab.users {
                ui.selectable_value(&mut user, Some(u.user.clone()), &u.display);
            }
        });
    app.collab.history.user = user;
    let want = (
        book.clone(),
        app.collab.history.this_cell.then(|| sheet.clone()),
        app.collab.history.this_cell.then(|| cell.clone()),
        app.collab.history.user.clone(),
    );
    if app.collab.history.asked.as_ref() != Some(&want) {
        app.collab.send(ClientMsg::History { book, sheet: want.1.clone(), cell: want.2.clone(), user: want.3.clone(), limit: 500 });
        app.collab.history.asked = Some(want);
    }
    ui.separator();
    if app.collab.history.changes.is_empty() {
        ui.label(RichText::new("No changes recorded yet.").color(t.text_dim));
    }
    let mut go = None;
    for c in app.collab.history.changes.clone() {
        egui::Frame::NONE.inner_margin(4).show(ui, |ui| {
            ui.horizontal(|ui| {
                let u = app.collab.users.iter().find(|u| u.user == c.user).map(|u| color(u.color)).unwrap_or(t.text);
                ui.label(RichText::new(&c.display).color(u).strong());
                ui.label(RichText::new(when(c.time)).color(t.text_dim).small());
            });
            let place = if c.cell.is_empty() { c.sheet.clone() } else { format!("{}!{}", c.sheet, c.cell) };
            if !c.cell.is_empty() {
                if ui.link(&place).clicked() {
                    go = Some((c.sheet.clone(), c.cell.clone()));
                }
            } else if !place.is_empty() {
                ui.label(&place);
            }
            if c.cell.is_empty() {
                // A sheet- or workbook-level change: there is no old value to show.
                ui.label(RichText::new(&c.new).italics().size(11.5));
            } else {
                let old = if c.old.is_empty() { "(empty)".to_string() } else { c.old.clone() };
                let new = if c.new.is_empty() { "(empty)".to_string() } else { c.new.clone() };
                ui.label(RichText::new(format!("{old}  →  {new}")).monospace().size(11.5));
            }
        });
        ui.separator();
    }
    if let Some((sheet, cell)) = go {
        let _ = app.run("sheet.activate", json!({"sheet": sheet}));
        let _ = app.run("selection.set", json!({"range": cell}));
        app.grid.ensure_visible = true;
    }
}

pub fn versions_pane(app: &mut SheetApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(d) = app.session.active() else { return };
    let Some(sd) = app.collab.shared(d.uid) else {
        ui.label(RichText::new("Versions are kept for shared workbooks.").italics());
        return;
    };
    let (book, name) = (sd.book.id.clone(), sd.book.name.clone());
    let can_edit = !sd.read_only;
    ui.label(
        RichText::new("The server keeps a version every hour while people edit, whenever someone presses Save, and before any restore.")
            .color(t.text_dim)
            .small(),
    );
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut app.collab.version_label).hint_text("label, e.g. Issued for review Rev B").desired_width(180.0));
        if ui.button("Keep version").clicked() {
            let label = std::mem::take(&mut app.collab.version_label);
            app.collab.send(ClientMsg::SaveVersion { book: book.clone(), label });
        }
    });
    if ui.small_button("Refresh").clicked() {
        app.collab.send(ClientMsg::Versions { book: book.clone() });
    }
    ui.separator();
    for v in app.collab.versions.clone() {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new(if v.label.is_empty() { "(no label)".to_string() } else { v.label.clone() }).strong());
                ui.label(RichText::new(format!("{} · {} · edit {}", when(v.time), app.collab.display_of(&v.user), v.seq)).color(t.text_dim).small());
            });
        });
        ui.horizontal(|ui| {
            if ui.small_button("Open a copy").clicked() {
                app.collab.want_copy = Some(format!("{name} ({})", when(v.time)));
                app.collab.send(ClientMsg::GetVersion { book: book.clone(), version: v.id.clone() });
            }
            if can_edit {
                if app.collab.confirm_restore.as_deref() == Some(v.id.as_str()) {
                    ui.label(RichText::new("Replace the workbook for everyone?").color(t.danger));
                    if ui.small_button("Restore").clicked() {
                        app.collab.send(ClientMsg::RestoreVersion { book: book.clone(), version: v.id.clone() });
                        app.collab.confirm_restore = None;
                    }
                    if ui.small_button("Cancel").clicked() {
                        app.collab.confirm_restore = None;
                    }
                } else if ui.small_button("Restore…").clicked() {
                    app.collab.confirm_restore = Some(v.id.clone());
                }
            }
        });
        ui.separator();
    }
}

// ---------------------------------------------------------------------------------------------
// Updates

/// Saves the installer the server sent, checks it, starts it and closes the app; the installer
/// replaces this version and opens the new one.
fn install_update(app: &mut SheetApp, info: &UpdateInfo, data: &str) {
    use sha2::Digest;
    let fail = |app: &mut SheetApp, why: String| {
        app.collab.update_state = None;
        app.message = Some(("Dexterity Sheets".into(), format!("The update could not be installed: {why}")));
    };
    let bytes = match proto::unb64(data) {
        Ok(b) => b,
        Err(e) => return fail(app, e.to_string()),
    };
    let got = gridcraft_collab::auth::hex(&sha2::Sha256::digest(&bytes));
    if got != info.sha256 {
        return fail(app, "the download is damaged (checksum mismatch). Try again.".into());
    }
    let dir = std::env::temp_dir().join("DexteritySheets-update");
    let _ = std::fs::create_dir_all(&dir);
    let name: String = info.file.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')).collect();
    let path = dir.join(if name.to_ascii_lowercase().ends_with(".msi") { name } else { format!("DexteritySheets-{}.msi", info.version) });
    if let Err(e) = std::fs::write(&path, &bytes) {
        return fail(app, e.to_string());
    }
    let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default();
    // msiexec replaces the installed version; afterwards the app opens again.
    let log = path.with_extension("log");
    let script =
        format!("start \"\" /wait msiexec /i \"{}\" /passive /norestart /l*v \"{}\" & start \"\" \"{}\"", path.display(), log.display(), exe);
    // cmd.exe doesn't understand the backslash-escaped quotes `arg` would add: pass the line raw.
    // No console window (CREATE_NO_WINDOW); msiexec shows its own progress bar.
    #[cfg(windows)]
    let started = {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("cmd").raw_arg(format!("/C \"{script}\"")).creation_flags(0x0800_0000).spawn()
    };
    #[cfg(not(windows))]
    let started: std::io::Result<std::process::Child> =
        Err(std::io::Error::other(format!("automatic install runs on Windows; the installer is at {} ({script})", path.display())));
    match started {
        Ok(_) => {
            for l in &mut app.collab.links {
                l.flush();
            }
            app.collab.update_state = Some(format!("Installing {}... the app closes now and reopens when done.", info.version));
            if let Some(ctx) = app.collab_ctx.clone() {
                app.request_quit(&ctx);
            }
        }
        Err(e) => fail(app, format!("{e} (installer saved at {})", path.display())),
    }
}

/// The status-bar badge and the update dialog. Drawn whatever workbook is open.
pub fn update_badge(app: &mut SheetApp, ui: &mut Ui, t: &Tokens) {
    if let Some(u) = app.collab.update.clone() {
        ui.add_space(12.0);
        let r = ui
            .add(
                egui::Label::new(RichText::new(format!("Update to {}", u.version)).font(theme::ui_bold(12.5)).color(t.accent))
                    .selectable(false)
                    .sense(egui::Sense::click()),
            )
            .on_hover_text("A newer Dexterity Sheets is available from the office server");
        if r.clicked() {
            app.collab.update_dialog = true;
        }
    }
    if !app.collab.update_dialog {
        return;
    }
    let mut open = true;
    let mut install = false;
    egui::Window::new("Dexterity Sheets updates").collapsible(false).resizable(false).open(&mut open).anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0)).show(ui.ctx(), |ui| {
        ui.set_max_width(420.0);
        ui.label(format!("You have version {}.", gridcraft_collab::APP_VERSION.trim()));
        match &app.collab.update {
            Some(u) => {
                ui.label(RichText::new(format!("Version {} is available ({:.1} MB).", u.version, u.size as f64 / 1_048_576.0)).strong());
                if !u.notes.trim().is_empty() {
                    ui.add_space(6.0);
                    egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                        ui.label(u.notes.trim());
                    });
                }
                ui.add_space(8.0);
                match &app.collab.update_state {
                    Some(state) => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(state);
                        });
                    }
                    None => {
                        ui.label(RichText::new("Shared workbooks are saved on the server, so nothing is lost. Other unsaved workbooks will ask to be saved first.").color(t.text_dim).small());
                        if ui.button("Update now").clicked() {
                            install = true;
                        }
                    }
                }
            }
            None if app.collab.signed_in() => {
                ui.label("You have the latest version.");
            }
            None => {
                ui.label("Sign in to the office server (File > Shared) to check for updates.");
            }
        }
    });
    if install {
        let _ = run_command(app, "collab.installUpdate", &json!({}));
    }
    if !open {
        app.collab.update_dialog = false;
    }
}
