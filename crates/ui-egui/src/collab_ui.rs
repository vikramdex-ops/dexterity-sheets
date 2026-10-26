//! Dexterity Sheets LAN co-authoring in the window: signing in to the office server, the shared
//! workbook list (File › Shared), other people's selections on the grid, who last changed the
//! active cell (status bar), and the Edit History and Versions panes.
//!
//! The engine knows nothing of sharing: each frame [`tick`] sends what the person changed in a
//! shared workbook and applies what others changed, through `gridcraft_collab::sync`.

use std::sync::Arc;
use std::sync::mpsc::Receiver;

use egui::{Color32, FontId, Painter, RichText, Stroke, StrokeKind, Ui, pos2, vec2};
use gridcraft_collab::client::{Client, Config, Event};
use gridcraft_collab::discovery::{self, Found};
use gridcraft_collab::proto::{self, BookInfo, Change, ClientMsg, Peer, Role, ServerMsg, UserInfo, VersionInfo};
use gridcraft_collab::sync::SharedDoc;
use gridcraft_engine::core::{CellRef, RangeRef};
use gridcraft_engine::model::Sheet;
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
    let cfg = Config { server, user, password: std::mem::take(&mut c.form_password), app: format!("Dexterity Sheets {}", env!("CARGO_PKG_VERSION")), machine };
    c.client = Some(Client::connect(cfg, waker));
    c.status = Status::Connecting;
    c.error = None;
}

fn sign_out(app: &mut SheetApp) {
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
    // Workbooks that need to be fetched again (a missed edit, a restored version).
    let reload: Vec<String> = app.collab.docs.iter().filter(|d| d.needs_reload.is_some()).map(|d| d.book.id.clone()).collect();
    for id in reload {
        if !app.collab.opening.contains(&id) {
            app.collab.opening.push(id.clone());
            app.collab.send(ClientMsg::OpenBook { book: id });
        }
    }
    send_presence(app);
    if let Some(rx) = &app.collab.finding
        && let Ok(found) = rx.try_recv()
    {
        app.collab.found = found;
        app.collab.finding = None;
    }
    if app.collab.client.as_ref().is_some_and(|c| !c.online()) || !app.collab.opening.is_empty() {
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
}

fn send_local(app: &mut SheetApp) {
    let open: Vec<u64> = app.session.documents().iter().map(|d| d.uid).collect();
    let closed: Vec<String> = app.collab.docs.iter().filter(|d| !open.contains(&d.doc_uid)).map(|d| d.book.id.clone()).collect();
    for id in closed {
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
        ServerMsg::Books { books } => app.collab.books = books,
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
                    // Replace an untouched blank Book1, as opening a file does.
                    if app.session.documents().len() == 1
                        && app.collab.docs.is_empty()
                        && app.session.active().is_some_and(|d| d.path.is_none() && !d.is_dirty() && d.undo.is_empty() && d.wb.sheets.iter().all(|sh| sh.cells.is_empty()))
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
            match proto::unb64(&xlsx).map_err(|e| e.to_string()).and_then(|b| gridcraft_engine::io::open_bytes("version.xlsx", &b).map_err(|e| e.to_string())) {
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
        ServerMsg::BookGone { book } => {
            if let Some(i) = app.collab.docs.iter().position(|d| d.book.id == book) {
                let sd = app.collab.docs.remove(i);
                if let Some(doc) = app.session.document_mut(sd.doc_uid) {
                    doc.title = format!("{} (deleted on server)", sd.book.name);
                    doc.saved = Arc::new(gridcraft_engine::model::Workbook::new());
                }
                app.message = Some(("Dexterity Sheets".into(), format!("\"{}\" was deleted on the server. Your window keeps a copy: save it with Save As if you need it.", sd.book.name)));
            }
        }
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
        "collab.history" => {
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
            app.collab.send(ClientMsg::CreateBook { name, xlsx: None });
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
                    })
                })
                .collect();
            Ok(json!({
                "status": format!("{:?}", c.status),
                "me": c.me.as_ref().map(|m| m.user.clone()),
                "books": c.books.iter().map(|b| json!({"name": b.name, "id": b.id, "openBy": b.open_by})).collect::<Vec<_>>(),
                "docs": docs,
                "error": c.error,
            }))
        }
        // Who last changed a cell of the active shared workbook: {cell?: "A1", sheet?}.
        "collab.lastEdit" => {
            let Some(d) = app.session.active() else { return Some(Err("no workbook".into())) };
            let Some(sd) = app.collab.shared(d.uid) else { return Some(Err("not a shared workbook".into())) };
            let sheet = p.get("sheet").and_then(Json::as_str).map(str::to_string).or_else(|| d.wb.active().map(|s| s.name.clone())).unwrap_or_default();
            let at = p.get("cell").and_then(Json::as_str).and_then(CellRef::parse).unwrap_or(d.selection.active);
            Ok(match sd.last_edit(&sheet, at) {
                Some((u, t)) => json!({"user": u, "time": t}),
                None => Json::Null,
            })
        }
        // Save on a shared workbook keeps a version on the server: every edit is already saved.
        "file.save" if active_shared(app).is_some() => {
            let book = active_shared(app).map(|s| s.book.id.clone()).unwrap_or_default();
            let who = app.collab.me.as_ref().map(|m| m.display.clone()).unwrap_or_default();
            app.collab.send(ClientMsg::SaveVersion { book, label: format!("Saved by {who}") });
            app.toast = Some(("Every change is saved on the server as you work. A version was kept as well.".into(), crate::now_ms()));
            Ok(Json::Null)
        }
        _ => return None,
    };
    Some(r)
}

fn share_active(app: &mut SheetApp, name: Option<&str>) -> Result<Json, String> {
    if !app.collab.signed_in() {
        app.open_backstage(crate::backstage::Page::Shared);
        return Err("Sign in to the office server first (File › Shared).".into());
    }
    let d = app.session.active().ok_or("no workbook open")?;
    if app.collab.shared(d.uid).is_some() {
        return Err("This workbook is already shared.".into());
    }
    let name = name.map(str::to_string).unwrap_or_else(|| d.title.trim_end_matches(".xlsx").to_string());
    let bytes = gridcraft_engine::io::save_bytes(&d.wb, "share.xlsx").map_err(|e| e.to_string())?;
    app.collab.send(ClientMsg::CreateBook { name, xlsx: Some(proto::b64(&bytes)) });
    app.toast = Some(("Sharing… the shared copy opens in its own window.".into(), crate::now_ms()));
    Ok(Json::Null)
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
    ui.label(RichText::new("Work on shared workbooks with your team. Every change is saved on the office server with your name and the time, so you can always see who changed what.").color(t.text_dim));
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
                app.collab.send(ClientMsg::CreateBook { name, xlsx: None });
            }
            let can_share = app.session.active().is_some_and(|d| app.collab.shared(d.uid).is_none());
            if ui.add_enabled(can_share, egui::Button::new("Share the current workbook")).on_hover_text("Uploads the workbook in the active window as a new shared workbook").clicked() {
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
            ui.label(RichText::new(&b.name).font(theme::ui_bold(13.5)));
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
                app.collab.send(ClientMsg::DeleteBook { book: id.to_string() });
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
            ui.add(egui::TextEdit::singleline(&mut app.collab.new_password).password(true).hint_text("new password (6+ characters)").desired_width(220.0));
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
        let r = ui.add(egui::Label::new(small(txt, t.text_dim)).selectable(false).sense(egui::Sense::click())).on_hover_text("Show this cell's history");
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
    egui::ComboBox::from_id_salt("collab_history_user").selected_text(user.as_deref().map(|u| app.collab.display_of(u)).unwrap_or_else(|| "Everyone".into())).show_ui(ui, |ui| {
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
            let old = if c.old.is_empty() { "(empty)".to_string() } else { c.old.clone() };
            let new = if c.new.is_empty() { "(empty)".to_string() } else { c.new.clone() };
            ui.label(RichText::new(format!("{old}  →  {new}")).monospace().size(11.5));
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
    ui.label(RichText::new("The server keeps a version every hour while people edit, whenever someone presses Save, and before any restore.").color(t.text_dim).small());
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
