//! The Dexterity Sheets server: accounts, shared workbooks, the ordered edit log and the audit
//! trail.
//!
//! On disk, under the data folder:
//! - `users.json`: accounts (salted PBKDF2 keys, never passwords).
//! - `books/<id>/book.json`: name, creator, dates.
//! - `books/<id>/snapshot-<seq>.xlsx`: the workbook as of edit `seq` (the newest one counts).
//! - `books/<id>/log.jsonl`: every edit, in order, flushed to disk before anyone hears of it.
//! - `books/<id>/audit.jsonl`: who changed which cell from what to what, one line per change.
//! - `books/<id>/versions/*.xlsx`: saved and automatic versions.
//!
//! A workbook's state is always its newest snapshot plus the log entries after it, so a crash at
//! any point loses nothing that was acknowledged.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use gridcraft_engine::core::{CellRef, Value};
use gridcraft_engine::model::Workbook;
use serde::{Deserialize, Serialize};

use crate::patch::{self, CellData, WbPatch};
use crate::proto::{
    self, Access, BookInfo, Change, ClientMsg, Conflict, FileSource, LastEdit, Logged, Notice, Peer, Role, Rule, ServerMsg, TemplateInfo, UpdateInfo,
    UserInfo, VersionInfo,
};
use crate::{Error, Result, auth, now_ms};

/// Write a fresh snapshot after this many edits.
const SNAPSHOT_EVERY: u64 = 300;
/// Keep an automatic version at most this often while a workbook is being edited.
const AUTO_VERSION_MS: i64 = 60 * 60 * 1000;
/// A workbook nobody has had open for this long is dropped from memory (it loads again when
/// opened), so a server that runs for months doesn't hold every workbook it ever served.
const IDLE_UNLOAD_MS: i64 = 10 * 60 * 1000;
const DAY_MS: i64 = 24 * 60 * 60 * 1000;
/// Most changes a history query returns.
const HISTORY_MAX: usize = 5000;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserRecord {
    pub user: String,
    pub display: String,
    pub role: Role,
    pub color: [u8; 3],
    pub salt: String,
    pub key: String,
    pub iterations: u32,
    #[serde(default)]
    pub disabled: bool,
}

impl UserRecord {
    pub fn info(&self) -> UserInfo {
        UserInfo { user: self.user.clone(), display: self.display.clone(), role: self.role, color: self.color }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct BookMeta {
    id: String,
    name: String,
    created: i64,
    created_by: String,
    modified: i64,
    modified_by: String,
    #[serde(default)]
    seq: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source: Option<FileSource>,
    #[serde(default)]
    access: Access,
    /// The last edit each person had when they closed it (for "changed while you were away").
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    seen: BTreeMap<String, u64>,
}

struct Book {
    meta: BookMeta,
    wb: Workbook,
    snapshot_seq: u64,
    snapshot: Arc<Vec<u8>>,
    tail: Vec<Logged>,
    log: File,
    audit: File,
    last_edits: HashMap<(String, CellRef), (String, i64)>,
    /// The last edit of each cell's content since the workbook was loaded: (seq, client instance,
    /// user). An edit made without having seen it is a conflict.
    writes: HashMap<(String, CellRef), (u64, String, String)>,
    last_version: i64,
    /// Last opened or edited.
    touched: i64,
}

struct Conn {
    user: String,
    instance: String,
    machine: String,
    /// The app version and update channel it last asked about.
    version: String,
    channel: String,
    tx: Sender<Arc<Vec<u8>>>,
    open: Vec<String>,
    peer: HashMap<String, Peer>,
}

pub struct State {
    dir: PathBuf,
    name: String,
    users: Vec<UserRecord>,
    metas: BTreeMap<String, BookMeta>,
    books: HashMap<String, Book>,
    conns: HashMap<u64, Conn>,
    /// The newest installer in `updates/` as last announced, and its file.
    update: Option<(UpdateInfo, PathBuf)>,
    /// Every installer in `updates/`, newest first, with the administrator's channel and status.
    available: Vec<(UpdateInfo, PathBuf)>,
    policy: String,
    /// sha256 of installers already hashed: (path, size, modified) → hash.
    hashes: HashMap<(PathBuf, u64, std::time::SystemTime), String>,
}

pub struct Server {
    state: Mutex<State>,
    next_conn: AtomicU64,
}

fn lock(m: &Mutex<State>) -> MutexGuard<'_, State> {
    // A thread that panicked while holding the lock leaves consistent data (every change is
    // written to disk before memory), so carry on with it.
    m.lock().unwrap_or_else(|p| p.into_inner())
}

pub fn users_path(dir: &Path) -> PathBuf {
    dir.join("users.json")
}

pub fn load_users(dir: &Path) -> Result<Vec<UserRecord>> {
    let p = users_path(dir);
    if !p.exists() {
        return Ok(Vec::new());
    }
    Ok(serde_json::from_slice(&fs::read(p)?)?)
}

pub fn save_users(dir: &Path, users: &[UserRecord]) -> Result<()> {
    fs::create_dir_all(dir)?;
    write_atomic(&users_path(dir), &serde_json::to_vec_pretty(users)?)
}

/// Opens an append-only log (edits, audit). A line cut short by a crash has no newline: it is ended
/// first, so the next entry starts on a line of its own instead of being glued to the broken one
/// (which would make that acknowledged entry unreadable at the next start).
fn open_log(path: &Path) -> Result<File> {
    let mut f = OpenOptions::new().create(true).read(true).append(true).open(path)?;
    let len = f.metadata()?.len();
    if len > 0 {
        let mut last = [0u8; 1];
        f.seek(SeekFrom::Start(len - 1))?;
        f.read_exact(&mut last)?;
        if last[0] != b'\n' {
            f.write_all(b"\n")?;
            f.sync_data()?;
        }
    }
    Ok(f)
}

/// Rewrites `log.jsonl` without the entries up to `keep_after` (they are in every kept snapshot)
/// and without unreadable lines (never acknowledged).
fn compact_log(dir: &Path, b: &mut Book, keep_after: u64) -> Result<()> {
    let path = dir.join("log.jsonl");
    let Ok(f) = File::open(&path) else { return Ok(()) };
    let mut kept = Vec::new();
    let mut dropped = 0usize;
    for line in BufReader::new(f).lines().map_while(std::result::Result::ok) {
        match serde_json::from_str::<Logged>(&line) {
            Ok(e) if e.seq > keep_after => {
                kept.extend_from_slice(line.as_bytes());
                kept.push(b'\n');
            }
            _ => dropped += 1,
        }
    }
    if dropped == 0 {
        return Ok(());
    }
    let tmp = dir.join("log.jsonl.tmp");
    {
        let mut t = File::create(&tmp)?;
        t.write_all(&kept)?;
        t.sync_all()?;
    }
    // Windows can't replace a file that is open: let go of it while it is swapped.
    let parked = dir.join("log.parked");
    b.log = OpenOptions::new().create(true).append(true).open(&parked)?;
    let swapped = fs::rename(&tmp, &path);
    b.log = open_log(&path)?;
    let _ = fs::remove_file(&parked);
    swapped?;
    Ok(())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    {
        let mut f = File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

/// An installer the server may hand to office PCs: an `.msi` of the app, never the server's own
/// (`DexSheetsServer-…msi`), which releases carry too and IT may drop into the same folder.
pub fn is_app_installer(file: &str) -> bool {
    let f = file.to_ascii_lowercase();
    f.ends_with(".msi") && !f.contains("server")
}

pub fn valid_username(u: &str) -> bool {
    !u.is_empty() && u.len() <= 64 && u.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Adds an account or replaces its password, display name and role.
pub fn set_user(dir: &Path, user: &str, display: &str, password: &str, role: Role) -> Result<()> {
    let user = user.trim().to_ascii_lowercase();
    if !valid_username(&user) {
        return Err(Error::Denied("user names use letters, digits, '.', '_' and '-'".into()));
    }
    if password.chars().count() < 6 {
        return Err(Error::Denied("passwords need at least 6 characters".into()));
    }
    let mut users = load_users(dir)?;
    let salt = auth::random_bytes::<16>();
    let key = auth::pbkdf2(password, &salt, auth::ITERATIONS);
    let color = proto::PALETTE[users.len() % proto::PALETTE.len()];
    let rec = UserRecord {
        user: user.clone(),
        display: if display.trim().is_empty() { user.clone() } else { display.trim().to_string() },
        role,
        color,
        salt: auth::hex(&salt),
        key: auth::hex(&key),
        iterations: auth::ITERATIONS,
        disabled: false,
    };
    match users.iter_mut().find(|u| u.user == user) {
        Some(u) => {
            let color = u.color;
            *u = rec;
            u.color = color;
        }
        None => users.push(rec),
    }
    save_users(dir, &users)
}

fn slug(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    let s = s.trim_matches('-').chars().take(40).collect::<String>();
    format!("{}-{}", if s.is_empty() { "book" } else { &s }, auth::hex(&auth::random_bytes::<3>()))
}

fn a1(at: CellRef) -> String {
    gridcraft_engine::core::CellRef::new(at.row, at.col).to_string()
}

impl Server {
    pub fn open(dir: PathBuf, name: String) -> Result<Server> {
        fs::create_dir_all(dir.join("books"))?;
        let users = load_users(&dir)?;
        let mut metas = BTreeMap::new();
        for e in fs::read_dir(dir.join("books"))?.flatten() {
            let p = e.path().join("book.json");
            if let Ok(bytes) = fs::read(&p)
                && let Ok(m) = serde_json::from_slice::<BookMeta>(&bytes)
            {
                metas.insert(m.id.clone(), m);
            }
        }
        fs::create_dir_all(dir.join("updates"))?;
        let mut state = State {
            dir,
            name,
            users,
            metas,
            books: HashMap::new(),
            conns: HashMap::new(),
            update: None,
            available: Vec::new(),
            policy: String::new(),
            hashes: HashMap::new(),
        };
        state.update = state.scan_updates();
        Ok(Server { state: Mutex::new(state), next_conn: AtomicU64::new(1) })
    }

    pub fn serve(self: Arc<Self>, listener: TcpListener) {
        for stream in listener.incoming() {
            match stream {
                Ok(s) => {
                    let me = self.clone();
                    let _ = std::thread::Builder::new().name("dexsheets-conn".into()).spawn(move || me.handle(s));
                }
                Err(e) => log::warn!("accept: {e}"),
            }
        }
    }

    fn handle(self: Arc<Self>, stream: TcpStream) {
        let peer_addr = stream.peer_addr().map(|a| a.to_string()).unwrap_or_default();
        // A browser following a shared workbook's web link (same port: no firewall change).
        let mut head = [0u8; 4];
        let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
        if stream.peek(&mut head).is_ok_and(|n| n == 4) && &head == b"GET " {
            self.serve_http(stream);
            return;
        }
        let _ = stream.set_nodelay(true);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
        let Ok(mut reader) = stream.try_clone() else { return };
        let (user, instance, machine) = match self.handshake(&mut reader) {
            Ok(x) => x,
            // A connection closed before a full hello is a port check (the Server Manager's
            // "is it running" test, a monitoring tool), not a login: nothing to log or slow down.
            Err(e) if is_probe(&e) => return,
            Err(e) => {
                log::info!("login refused from {peer_addr}: {e}");
                // Slow down password guessing.
                std::thread::sleep(Duration::from_millis(800));
                let mut w = &stream;
                let _ = proto::write_frame(&mut w, &ServerMsg::Error { message: e.to_string() });
                return;
            }
        };
        // No read timeout once logged in: clients ping every 10 s, a dead peer fails the write.
        let _ = reader.set_read_timeout(Some(Duration::from_secs(45)));
        let conn = self.next_conn.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = channel::<Arc<Vec<u8>>>();
        let Ok(writer) = stream.try_clone() else { return };
        let _ = std::thread::Builder::new().name("dexsheets-writer".into()).spawn(move || write_loop(writer, rx));
        {
            let mut st = lock(&self.state);
            let me = st.users.iter().find(|u| u.user == user).map(UserRecord::info);
            let Some(me) = me else { return };
            let users = st.users.iter().filter(|u| !u.disabled).map(UserRecord::info).collect();
            st.conns.insert(
                conn,
                Conn {
                    user: user.clone(),
                    instance,
                    machine: machine.clone(),
                    version: String::new(),
                    channel: String::new(),
                    tx: tx.clone(),
                    open: Vec::new(),
                    peer: HashMap::new(),
                },
            );
            send(&tx, &ServerMsg::Welcome { me, conn, users });
            let role = st.users.iter().find(|u| u.user == user).map_or(Role::Viewer, |u| u.role);
            for notice in st.notices_at_sign_in(&user, role) {
                send(&tx, &ServerMsg::Notice { notice });
            }
        }
        log::info!("{user} connected from {peer_addr} ({machine})");
        loop {
            let msg = match proto::read_frame::<_, ClientMsg>(&mut reader) {
                Ok(m) => m,
                Err(e) => {
                    log::info!("{user} disconnected: {e}");
                    break;
                }
            };
            let reply = {
                let mut st = lock(&self.state);
                st.dispatch(conn, msg)
            };
            if let Err(e) = reply {
                send(&tx, &ServerMsg::Error { message: e.to_string() });
            }
        }
        let mut st = lock(&self.state);
        if let Some(c) = st.conns.remove(&conn) {
            for b in c.open {
                st.broadcast(&b, &ServerMsg::PeerLeft { book: b.clone(), conn }, None);
            }
        }
        let _ = stream.shutdown(std::net::Shutdown::Both);
    }

    fn handshake(&self, s: &mut TcpStream) -> Result<(String, String, String)> {
        let (user, instance, machine) = match proto::read_frame::<_, ClientMsg>(s)? {
            ClientMsg::Hello { protocol, user, instance, machine, .. } => {
                if protocol != proto::PROTOCOL {
                    return Err(Error::Denied(format!(
                        "this server speaks protocol {}, the app speaks {protocol}: install the same version",
                        proto::PROTOCOL
                    )));
                }
                (user.trim().to_ascii_lowercase(), instance, machine)
            }
            _ => return Err(Error::Protocol("expected Hello".into())),
        };
        let (rec, server) = {
            let mut st = lock(&self.state);
            // Accounts changed with the server's `user` command take effect at the next login.
            if let Ok(users) = load_users(&st.dir) {
                st.users = users;
            }
            (st.users.iter().find(|u| u.user == user).cloned(), st.name.clone())
        };
        let nonce = auth::hex(&auth::random_bytes::<24>());
        // An unknown user gets a challenge like anyone else, so names can't be probed.
        let (salt, iterations) = match &rec {
            Some(r) => (r.salt.clone(), r.iterations),
            None => (auth::hex(&auth::hmac_sha256(b"no-such-user", user.as_bytes())[..16]), auth::ITERATIONS),
        };
        proto::write_frame(s, &ServerMsg::Challenge { salt, iterations, nonce: nonce.clone(), server })?;
        let proof = match proto::read_frame::<_, ClientMsg>(s)? {
            ClientMsg::Login { proof } => proof,
            _ => return Err(Error::Protocol("expected Login".into())),
        };
        let Some(rec) = rec else { return Err(Error::Denied("Wrong user name or password.".into())) };
        let key = auth::unhex(&rec.key).unwrap_or_default();
        let expect = auth::hmac_sha256(&key, &auth::unhex(&nonce).unwrap_or_default());
        if !auth::same(&expect, &auth::unhex(&proof).unwrap_or_default()) {
            return Err(Error::Denied("Wrong user name or password.".into()));
        }
        if rec.disabled {
            return Err(Error::Denied("This account is disabled. Ask an administrator.".into()));
        }
        Ok((user, instance, machine))
    }
}

fn write_loop(mut w: TcpStream, rx: Receiver<Arc<Vec<u8>>>) {
    while let Ok(frame) = rx.recv() {
        if w.write_all(&frame).is_err() {
            let _ = w.shutdown(std::net::Shutdown::Both);
            return;
        }
    }
}

fn encode(msg: &ServerMsg) -> Option<Arc<Vec<u8>>> {
    let mut buf = Vec::new();
    proto::write_frame(&mut buf, msg).ok()?;
    Some(Arc::new(buf))
}

fn send(tx: &Sender<Arc<Vec<u8>>>, msg: &ServerMsg) {
    if let Some(f) = encode(msg) {
        let _ = tx.send(f);
    }
}

impl State {
    fn user(&self, conn: u64) -> Result<(&Conn, &UserRecord)> {
        let c = self.conns.get(&conn).ok_or_else(|| Error::Protocol("unknown connection".into()))?;
        let u = self.users.iter().find(|u| u.user == c.user).ok_or_else(|| Error::Denied("account removed".into()))?;
        Ok((c, u))
    }

    fn reply(&self, conn: u64, msg: &ServerMsg) {
        if let Some(c) = self.conns.get(&conn) {
            send(&c.tx, msg);
        }
    }

    /// Sends to everyone who has `book` open, except `skip`.
    fn broadcast(&self, book: &str, msg: &ServerMsg, skip: Option<u64>) {
        let Some(frame) = encode(msg) else { return };
        for (id, c) in &self.conns {
            if Some(*id) != skip && c.open.iter().any(|b| b == book) {
                let _ = c.tx.send(frame.clone());
            }
        }
    }

    /// Everyone's workbook list changes together (a new workbook, a rename, who has what open).
    /// Everyone's list of the shared workbooks they may open.
    fn broadcast_books(&self) {
        for c in self.conns.values() {
            let Some(u) = self.users.iter().find(|u| u.user == c.user) else { continue };
            let books = self.books_for(&u.user, u.role);
            if let Some(frame) = encode(&ServerMsg::Books { books }) {
                let _ = c.tx.send(frame);
            }
        }
    }

    fn books_for(&self, user: &str, role: Role) -> Vec<BookInfo> {
        self.metas.values().filter_map(|m| self.info_for(m, user, role)).collect()
    }

    /// The workbook as `user` sees it in the list, or `None` when they may not open it.
    fn info_for(&self, m: &BookMeta, user: &str, role: Role) -> Option<BookInfo> {
        let permission = permission(m, user, role, now_ms())?;
        let mut i = self.info(m);
        i.permission = permission.into();
        Some(i)
    }

    /// Fails unless `user` may open the workbook (and, with `edit`, change it).
    fn require(&self, user: &str, role: Role, book: &str, edit: bool) -> Result<()> {
        let m = self.metas.get(book).ok_or_else(|| Error::Denied("no such workbook".into()))?;
        match permission(m, user, role, now_ms()) {
            Some("editor") => Ok(()),
            Some(_) if !edit => Ok(()),
            Some(_) => Err(Error::Denied("you can view this workbook but not change it".into())),
            None => {
                let by = self.users.iter().find(|u| u.user == m.created_by).map_or_else(|| m.created_by.clone(), |u| u.display.clone());
                Err(Error::Denied(format!("you don't have access to \"{}\": ask {by} to share it with you", m.name)))
            }
        }
    }

    /// Tells `user` now if they are connected, otherwise at their next sign-in.
    fn notify(&self, user: &str, mut n: Notice) {
        n.time = now_ms();
        n.id = format!("n-{}-{}", n.time, n.kind);
        let online: Vec<&Conn> = self.conns.values().filter(|c| c.user == user).collect();
        if online.is_empty() {
            let path = self.dir.join("notices").join(format!("{user}.jsonl"));
            match serde_json::to_value(&n) {
                Ok(v) => {
                    if let Err(e) = append_line(&path, &v) {
                        log::warn!("notice for {user}: {e}");
                    }
                }
                Err(e) => log::warn!("notice for {user}: {e}"),
            }
            return;
        }
        if let Some(frame) = encode(&ServerMsg::Notice { notice: n }) {
            for c in online {
                let _ = c.tx.send(frame.clone());
            }
        }
    }

    /// At sign-in: what was kept for the person while they were away, and a summary of the
    /// shared workbooks others changed since they last had them open (one notice per workbook,
    /// never one per edit).
    fn notices_at_sign_in(&mut self, user: &str, role: Role) -> Vec<Notice> {
        let path = self.dir.join("notices").join(format!("{user}.jsonl"));
        let mut out: Vec<Notice> = fs::read_to_string(&path).unwrap_or_default().lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
        let _ = fs::remove_file(&path);
        for m in self.metas.values() {
            let Some(seen) = m.seen.get(user) else { continue };
            if m.seq > *seen && m.modified_by != user && permission(m, user, role, now_ms()).is_some() {
                let by = self.users.iter().find(|u| u.user == m.modified_by).map_or_else(|| m.modified_by.clone(), |u| u.display.clone());
                out.push(Notice {
                    id: format!("n-{}-changed-{}", now_ms(), m.id),
                    time: m.modified,
                    kind: "changed".into(),
                    book: m.id.clone(),
                    name: m.name.clone(),
                    by,
                    permission: String::new(),
                    text: format!("{} edit(s) since you last had it open", m.seq - seen),
                });
            }
        }
        out.truncate(200);
        out
    }

    /// The newest installer in `updates/`. Files are named with their version as a `-`-separated
    /// part (`DexteritySheets-0.13.0-x64.msi`, `dexterity-sheets-0.13.0-windows-x64.msi`); notes
    /// sit next to them as `<same name>.md` or `.txt`.
    /// Reads `updates/`: every app installer, with the channel and status the administrator gave
    /// it in `updates/releases.json`. Returns the newest stable version anyone may be offered
    /// (the announcement for apps that predate channels).
    fn scan_updates(&mut self) -> Option<(UpdateInfo, PathBuf)> {
        let dir = self.dir.join("updates");
        let releases: serde_json::Value = fs::read(dir.join("releases.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        self.policy = releases.pointer("/policy/mode").and_then(serde_json::Value::as_str).unwrap_or("prompt").to_string();
        let mut found: Vec<(UpdateInfo, PathBuf)> = Vec::new();
        for e in fs::read_dir(&dir).ok()?.flatten() {
            let path = e.path();
            if !path.file_name().is_some_and(|n| is_app_installer(&n.to_string_lossy())) {
                continue;
            }
            let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            let Some(version) = stem.split('-').find(|p| p.starts_with(|c: char| c.is_ascii_digit()) && p.contains('.')).map(str::to_string) else {
                continue;
            };
            let Ok(meta) = e.metadata() else { continue };
            let modified = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
            let key = (path.clone(), meta.len(), modified);
            let sha256 = match self.hashes.get(&key) {
                Some(h) => h.clone(),
                None => {
                    use sha2::Digest;
                    let Ok(bytes) = fs::read(&path) else { continue };
                    let h = auth::hex(&sha2::Sha256::digest(bytes));
                    self.hashes.insert(key, h.clone());
                    h
                }
            };
            let notes = ["md", "txt"].iter().find_map(|x| fs::read_to_string(path.with_extension(x)).ok()).unwrap_or_default();
            let file = path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
            let rel = releases.get("releases").and_then(|r| r.get(&version));
            let field = |k: &str, default: &str| rel.and_then(|r| r.get(k)).and_then(serde_json::Value::as_str).unwrap_or(default).to_string();
            let info = UpdateInfo {
                version,
                notes,
                file,
                size: meta.len(),
                sha256,
                channel: field("channel", "stable"),
                status: field("status", "ok"),
                note: field("note", ""),
            };
            // Two files of one version: keep one.
            if !found.iter().any(|(u, _)| u.version == info.version) {
                found.push((info, path));
            }
        }
        found.sort_by(|a, b| {
            if proto::version_newer(&a.0.version, &b.0.version) {
                std::cmp::Ordering::Less
            } else if proto::version_newer(&b.0.version, &a.0.version) {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Equal
            }
        });
        self.available = found;
        self.available.iter().find(|(u, _)| u.channel == "stable" && offerable(u)).cloned()
    }

    fn default_channel(&self) -> String {
        let releases: serde_json::Value =
            fs::read(self.dir.join("updates").join("releases.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        match releases.pointer("/policy/defaultChannel").and_then(serde_json::Value::as_str) {
            Some("testing") => "testing".into(),
            _ => "stable".into(),
        }
    }

    /// What an app running `version` on `channel` is told: the newest version it may move to,
    /// its own version's status, and the older versions it may roll back to.
    fn offer(&self, version: &str, channel: &str) -> ServerMsg {
        let on_channel = |u: &UpdateInfo| channel == "testing" || u.channel == "stable";
        let update =
            self.available.iter().map(|(u, _)| u).find(|u| offerable(u) && on_channel(u) && proto::version_newer(&u.version, version)).cloned();
        let current = self.available.iter().map(|(u, _)| u).find(|u| u.version == version).cloned();
        let older = self
            .available
            .iter()
            .map(|(u, _)| u)
            .filter(|u| u.status != "blocked" && proto::version_newer(version, &u.version))
            .take(5)
            .cloned()
            .collect();
        ServerMsg::Update { update, current, older, policy: self.policy.clone() }
    }

    /// Records the app version a person's PC runs (`clients.json`, for the admin dashboard).
    fn note_client(&self, conn: u64) {
        let Some(c) = self.conns.get(&conn) else { return };
        let path = self.dir.join("clients.json");
        let mut all: serde_json::Value = fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_else(|| serde_json::json!({}));
        if !all.get("clients").is_some_and(serde_json::Value::is_object) {
            all = serde_json::json!({"clients": {}});
        }
        if let Some(map) = all.get_mut("clients").and_then(serde_json::Value::as_object_mut) {
            map.insert(
                format!("{}|{}", c.user, c.machine),
                serde_json::json!({"user": c.user, "machine": c.machine, "version": c.version, "channel": c.channel, "lastSeen": now_ms()}),
            );
        }
        match serde_json::to_vec_pretty(&all) {
            Ok(b) => {
                if let Err(e) = write_atomic(&path, &b) {
                    log::warn!("clients.json: {e}");
                }
            }
            Err(e) => log::warn!("clients.json: {e}"),
        }
    }

    fn book_dir(&self, id: &str) -> PathBuf {
        self.dir.join("books").join(id)
    }

    fn info(&self, m: &BookMeta) -> BookInfo {
        let mut open_by: Vec<String> = self
            .conns
            .values()
            .filter(|c| c.open.contains(&m.id))
            .filter_map(|c| self.users.iter().find(|u| u.user == c.user).map(|u| u.display.clone()))
            .collect();
        open_by.sort();
        open_by.dedup();
        let seq = self.books.get(&m.id).map_or(m.seq, |b| b.meta.seq);
        BookInfo {
            id: m.id.clone(),
            name: m.name.clone(),
            created: m.created,
            created_by: m.created_by.clone(),
            modified: m.modified,
            modified_by: m.modified_by.clone(),
            seq,
            open_by,
            source: m.source.clone().map(Box::new),
            access: m.access.clone(),
            permission: String::new(),
        }
    }

    fn load_book(&mut self, id: &str) -> Result<()> {
        if self.books.contains_key(id) {
            return Ok(());
        }
        let meta = self.metas.get(id).cloned().ok_or_else(|| Error::Denied("no such workbook".into()))?;
        let dir = self.book_dir(id);
        // The newest snapshot that opens.
        let mut snaps: Vec<(u64, PathBuf)> = fs::read_dir(&dir)?
            .flatten()
            .filter_map(|e| {
                let n = e.file_name().to_string_lossy().to_string();
                let seq = n.strip_prefix("snapshot-")?.strip_suffix(".xlsx")?.parse().ok()?;
                Some((seq, e.path()))
            })
            .collect();
        snaps.sort_by_key(|s| std::cmp::Reverse(s.0));
        let mut loaded = None;
        for (seq, p) in &snaps {
            match fs::read(p)
                .map_err(Error::from)
                // An .xlsx is a zip: anything else is damage, never "a workbook" (the reader would
                // otherwise take stray text as a CSV and serve a near-empty sheet).
                .and_then(|b| if b.starts_with(b"PK\x03\x04") { Ok(b) } else { Err(Error::Engine("not an .xlsx file".into())) })
                .and_then(|b| gridcraft_engine::io::open_bytes("book.xlsx", &b).map(|(wb, _)| (wb, b)).map_err(Error::from))
            {
                Ok((wb, bytes)) => {
                    loaded = Some((*seq, wb, bytes));
                    break;
                }
                Err(e) => log::error!("{}: snapshot {} unreadable ({e}); trying an older one", meta.name, p.display()),
            }
        }
        let (snapshot_seq, mut wb, bytes) = loaded.ok_or_else(|| Error::Engine(format!("{}: no readable snapshot", meta.name)))?;
        let mut tail = Vec::new();
        let mut writes = HashMap::new();
        let mut seq = snapshot_seq;
        let log_path = dir.join("log.jsonl");
        if let Ok(f) = File::open(&log_path) {
            for line in BufReader::new(f).lines().map_while(std::result::Result::ok) {
                // A line cut short by a crash is the last one and was never acknowledged.
                let Ok(entry) = serde_json::from_str::<Logged>(&line) else { continue };
                if entry.seq <= snapshot_seq {
                    continue;
                }
                if entry.seq != seq + 1 {
                    log::warn!("{}: log jumps from {seq} to {}", meta.name, entry.seq);
                }
                let inst = entry.origin.as_deref().and_then(|o| o.rsplit_once(':')).map(|(i, _)| i.to_string()).unwrap_or_default();
                for c in describe(&wb, &entry.patch, entry.seq, entry.time, &entry.user, "") {
                    if let Some(at) = CellRef::parse(&c.cell) {
                        writes.insert((c.sheet, at), (entry.seq, inst.clone(), entry.user.clone()));
                    }
                }
                if let Err(e) = patch::apply(&mut wb, &entry.patch) {
                    log::error!("{}: edit {} could not be replayed: {e}", meta.name, entry.seq);
                }
                seq = entry.seq;
                tail.push(entry);
            }
        }
        let mut last_edits = HashMap::new();
        if let Ok(f) = File::open(dir.join("audit.jsonl")) {
            for line in BufReader::new(f).lines().map_while(std::result::Result::ok) {
                if let Ok(c) = serde_json::from_str::<Change>(&line)
                    && let Ok(at) = CellRef::parse(&c.cell).ok_or(())
                {
                    last_edits.insert((c.sheet, at), (c.user, c.time));
                }
            }
        }
        let log = open_log(&log_path)?;
        let audit = open_log(&dir.join("audit.jsonl"))?;
        let mut meta = meta;
        meta.seq = seq;
        let now = now_ms();
        self.books.insert(
            id.to_string(),
            Book { meta, wb, snapshot_seq, snapshot: Arc::new(bytes), tail, log, audit, last_edits, writes, last_version: now, touched: now },
        );
        Ok(())
    }

    /// Drops workbooks nobody has had open for [`IDLE_UNLOAD_MS`] from memory, after a final
    /// snapshot. Their files stay; opening one loads it again.
    fn unload_idle(&mut self) {
        let now = now_ms();
        let open: HashSet<&String> = self.conns.values().flat_map(|c| c.open.iter()).collect();
        let idle: Vec<String> =
            self.books.iter().filter(|(id, b)| !open.contains(id) && now - b.touched > IDLE_UNLOAD_MS).map(|(id, _)| id.clone()).collect();
        for id in idle {
            if self.books.get(&id).is_some_and(|b| b.meta.seq != b.snapshot_seq)
                && let Err(e) = self.snapshot(&id)
            {
                // Kept in memory; every edit is in its log anyway. Tried again next time.
                log::error!("{id}: snapshot before unloading failed: {e}");
                continue;
            }
            if let Some(m) = self.books.get(&id).map(|b| b.meta.clone()) {
                let _ = self.save_meta(&m);
                self.metas.insert(id.clone(), m);
            }
            self.books.remove(&id);
            log::info!("{id}: unloaded from memory (nobody has had it open for a while)");
        }
    }

    /// Automatic versions thin out with age: every one from the last 14 days, one a day for 90
    /// days, one a week after that. Versions someone kept by name, on Save, or before a restore are
    /// never removed.
    fn prune_versions(&self, id: &str) {
        let dir = self.book_dir(id).join("versions");
        let now = now_ms();
        let mut kept: HashSet<(bool, i64)> = HashSet::new();
        // Newest first: the newest of each day (or week) is the one kept.
        for v in self.versions(id).into_iter().filter(|v| v.label == "automatic") {
            let age = now - v.time;
            if age < 14 * DAY_MS {
                continue;
            }
            let bucket = if age < 90 * DAY_MS { (false, v.time / DAY_MS) } else { (true, v.time / (7 * DAY_MS)) };
            if !kept.insert(bucket) {
                let _ = fs::remove_file(dir.join(format!("{}.xlsx", v.id)));
                let _ = fs::remove_file(dir.join(format!("{}.json", v.id)));
            }
        }
    }

    fn save_meta(&self, m: &BookMeta) -> Result<()> {
        write_atomic(&self.book_dir(&m.id).join("book.json"), &serde_json::to_vec_pretty(m)?)
    }

    fn create_book(&mut self, user: &str, name: &str, xlsx: Option<&[u8]>, source: Option<FileSource>) -> Result<BookMeta> {
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::Denied("give the workbook a name".into()));
        }
        let taken = |n: &str| self.metas.values().any(|m| m.name.eq_ignore_ascii_case(n));
        let name = if !taken(name) {
            name.to_string()
        } else if source.is_some() {
            // Two files called "Line list.xlsx" in different folders can both be shared.
            (2..1000).map(|i| format!("{name} ({i})")).find(|n| !taken(n)).ok_or_else(|| Error::Denied("too many workbooks with that name".into()))?
        } else {
            return Err(Error::Denied(format!("a shared workbook called \"{name}\" already exists")));
        };
        let name = name.as_str();
        let wb = match xlsx {
            Some(b) => gridcraft_engine::io::open_bytes("upload.xlsx", b)?.0,
            None => {
                let mut wb = Workbook::new();
                if wb.sheets.is_empty() {
                    wb.sheets.push(Arc::new(gridcraft_engine::model::Sheet::new("Sheet1")));
                }
                wb
            }
        };
        let bytes = gridcraft_engine::io::save_bytes(&wb, "book.xlsx")?;
        let id = slug(name);
        let dir = self.book_dir(&id);
        fs::create_dir_all(dir.join("versions"))?;
        write_atomic(&dir.join("snapshot-0.xlsx"), &bytes)?;
        let now = now_ms();
        let meta = BookMeta {
            id: id.clone(),
            name: name.to_string(),
            created: now,
            created_by: user.to_string(),
            modified: now,
            modified_by: user.to_string(),
            seq: 0,
            source,
            access: Access::default(),
            seen: BTreeMap::new(),
        };
        self.save_meta(&meta)?;
        self.metas.insert(id, meta.clone());
        Ok(meta)
    }

    fn snapshot(&mut self, id: &str) -> Result<()> {
        let dir = self.book_dir(id);
        let Some(b) = self.books.get_mut(id) else { return Ok(()) };
        let bytes = gridcraft_engine::io::save_bytes(&b.wb, "book.xlsx")?;
        // Everyone's replica is "snapshot + edits after it", so the server's own copy must be
        // exactly what the snapshot reads back as.
        let (wb, _) = gridcraft_engine::io::open_bytes("book.xlsx", &bytes)?;
        write_atomic(&dir.join(format!("snapshot-{}.xlsx", b.meta.seq)), &bytes)?;
        b.wb = wb;
        b.snapshot_seq = b.meta.seq;
        b.snapshot = Arc::new(bytes);
        b.tail.clear();
        // Keep the newest three snapshots.
        let mut snaps: Vec<(u64, PathBuf)> = fs::read_dir(&dir)?
            .flatten()
            .filter_map(|e| {
                let n = e.file_name().to_string_lossy().to_string();
                Some((n.strip_prefix("snapshot-")?.strip_suffix(".xlsx")?.parse().ok()?, e.path()))
            })
            .collect();
        snaps.sort_by_key(|s| std::cmp::Reverse(s.0));
        for (_, p) in snaps.iter().skip(3) {
            let _ = fs::remove_file(p);
        }
        // The edit log only needs what comes after the oldest snapshot still kept (a damaged newer
        // snapshot falls back to an older one plus the log), so it never grows without end.
        if let Some(oldest) = snaps.iter().take(3).map(|s| s.0).min()
            && let Err(e) = compact_log(&dir, b, oldest)
        {
            log::warn!("{id}: edit log not compacted: {e}");
        }
        Ok(())
    }

    fn save_version(&mut self, id: &str, user: &str, label: &str) -> Result<VersionInfo> {
        self.snapshot(id)?;
        let dir = self.book_dir(id).join("versions");
        fs::create_dir_all(&dir)?;
        let b = self.books.get_mut(id).ok_or_else(|| Error::Denied("no such workbook".into()))?;
        let time = now_ms();
        b.last_version = time;
        let clean: String = label.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' }).take(60).collect();
        let vid = format!("{time}-{}-{}", b.meta.seq, user);
        write_atomic(&dir.join(format!("{vid}.xlsx")), &b.snapshot)?;
        let info = VersionInfo {
            id: vid.clone(),
            seq: b.meta.seq,
            time,
            user: user.to_string(),
            label: clean.trim().to_string(),
            size: b.snapshot.len() as u64,
        };
        write_atomic(&dir.join(format!("{vid}.json")), &serde_json::to_vec(&info)?)?;
        Ok(info)
    }

    fn versions(&self, id: &str) -> Vec<VersionInfo> {
        let mut out: Vec<VersionInfo> = fs::read_dir(self.book_dir(id).join("versions"))
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .filter_map(|e| serde_json::from_slice(&fs::read(e.path()).ok()?).ok())
            .collect();
        out.sort_by_key(|v: &VersionInfo| std::cmp::Reverse(v.time));
        out
    }

    fn version_path(&self, id: &str, version: &str) -> Result<PathBuf> {
        if !version.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')) {
            return Err(Error::Denied("bad version id".into()));
        }
        let p = self.book_dir(id).join("versions").join(format!("{version}.xlsx"));
        if p.exists() { Ok(p) } else { Err(Error::Denied("no such version".into())) }
    }

    fn dispatch(&mut self, conn: u64, msg: ClientMsg) -> Result<()> {
        let (user, role, display) = {
            let (_, u) = self.user(conn)?;
            (u.user.clone(), u.role, u.display.clone())
        };
        match msg {
            ClientMsg::Ping => self.reply(conn, &ServerMsg::Pong),
            ClientMsg::CheckUpdate { version, channel } => {
                let channel = channel.filter(|c| c == "stable" || c == "testing").unwrap_or_else(|| self.default_channel());
                if let Some(c) = self.conns.get_mut(&conn) {
                    c.version = version.clone();
                    c.channel = channel.clone();
                }
                self.note_client(conn);
                let msg = self.offer(&version, &channel);
                self.reply(conn, &msg);
            }
            ClientMsg::GetUpdate { version } => {
                // Any installer the server holds, for an update or a rollback, but never a blocked one.
                let Some((info, path)) = self.available.iter().find(|(u, _)| u.version == version && u.status != "blocked").cloned() else {
                    return Err(Error::Denied(format!("version {version} is not available on this server")));
                };
                let data = fs::read(&path)?;
                log::info!("{user} downloads update {}", info.version);
                self.reply(conn, &ServerMsg::UpdateFile { info, data: proto::b64(&data) });
            }
            ClientMsg::Hello { .. } | ClientMsg::Login { .. } => {}
            ClientMsg::ChangePassword { old_proof: _, new_password } => {
                // The connection already proved the old password at login.
                set_user(&self.dir, &user, &display, &new_password, role)?;
                self.users = load_users(&self.dir)?;
                self.reply(conn, &ServerMsg::Ok { what: "Password changed.".into() });
            }
            ClientMsg::Feedback { report } => {
                let machine = self.conns.get(&conn).map(|c| c.machine.clone()).unwrap_or_default();
                let time = now_ms();
                let mut v = serde_json::to_value(&report)?;
                if let Some(o) = v.as_object_mut() {
                    o.insert("id".into(), serde_json::json!(format!("fb-{time}-{conn}")));
                    o.insert("time".into(), serde_json::json!(time));
                    o.insert("user".into(), serde_json::json!(user));
                    o.insert("machine".into(), serde_json::json!(machine));
                }
                append_line(&self.dir.join("feedback").join("feedback.jsonl"), &v)?;
                log::info!("{user} reported a problem with {} ({})", report.version, report.features.join(", "));
                self.reply(conn, &ServerMsg::Ok { what: "Thank you: your report reached the administrator.".into() });
            }
            ClientMsg::Health { version, kind, detail } => {
                let machine = self.conns.get(&conn).map(|c| c.machine.clone()).unwrap_or_default();
                let line =
                    serde_json::json!({"time": now_ms(), "user": user, "machine": machine, "version": version, "kind": kind, "detail": detail});
                append_line(&self.dir.join("health.jsonl"), &line)?;
            }
            ClientMsg::Usage { version, counts } => {
                if let Err(e) = add_usage(&self.dir.join("adoption.json"), &version, &counts) {
                    log::warn!("adoption.json: {e}");
                }
            }
            ClientMsg::ListBooks => {
                let books = self.books_for(&user, role);
                self.reply(conn, &ServerMsg::Books { books });
            }
            ClientMsg::CreateBook { name, xlsx, source, sha256 } => {
                if !role.can_edit() {
                    return Err(Error::Denied("view-only accounts can't create workbooks".into()));
                }
                // A file that is already shared is joined, never shared twice.
                if let Some(src) = &source
                    && let Some(id) = self.metas.values().find(|m| m.source.as_ref().is_some_and(|s| s.key == src.key)).map(|m| m.id.clone())
                {
                    return self.dispatch(conn, ClientMsg::OpenBook { book: id });
                }
                let source = source.map(|s| FileSource { user: user.clone(), ..s });
                let bytes = match xlsx {
                    Some(s) => Some(proto::unb64_checked(&s, sha256.as_deref().unwrap_or(""))?),
                    None => None,
                };
                let meta = self.create_book(&user, &name, bytes.as_deref(), source)?;
                log::info!("{user} created \"{}\"", meta.name);
                self.broadcast_books();
                return self.dispatch(conn, ClientMsg::OpenBook { book: meta.id });
            }
            ClientMsg::OpenBook { book } => {
                self.require(&user, role, &book, false)?;
                self.load_book(&book)?;
                if let Some(b) = self.books.get_mut(&book) {
                    b.touched = now_ms();
                }
                if let Some(c) = self.conns.get_mut(&conn)
                    && !c.open.contains(&book)
                {
                    c.open.push(book.clone());
                }
                let peers: Vec<Peer> = self.conns.iter().filter(|(id, _)| **id != conn).filter_map(|(_, c)| c.peer.get(&book).cloned()).collect();
                let b = self.books.get(&book).ok_or_else(|| Error::Denied("no such workbook".into()))?;
                let last_edits = b
                    .last_edits
                    .iter()
                    .map(|((sheet, at), (user, time))| LastEdit { sheet: sheet.clone(), r: at.row, c: at.col, user: user.clone(), time: *time })
                    .collect();
                let mut info = self.info(&b.meta);
                info.permission = permission(&b.meta, &user, role, now_ms()).unwrap_or("viewer").into();
                let msg = ServerMsg::Opened {
                    book: info,
                    snapshot_seq: b.snapshot_seq,
                    xlsx: proto::b64(&b.snapshot),
                    log: b.tail.clone(),
                    last_edits,
                    peers,
                    sha256: proto::sha256_hex(&b.snapshot),
                };
                self.reply(conn, &msg);
                log::info!("{user} opened \"{}\"", b.meta.name);
                self.broadcast_books();
            }
            ClientMsg::CloseBook { book } => {
                let seq = self.books.get(&book).map(|b| b.meta.seq);
                if let (Some(seq), Some(b)) = (seq, self.books.get_mut(&book)) {
                    // The loaded copy too: edits write `metas` from it.
                    b.meta.seen.insert(user.clone(), seq);
                }
                if let (Some(seq), Some(m)) = (seq, self.metas.get_mut(&book)) {
                    m.seen.insert(user.clone(), seq);
                    let m = m.clone();
                    if let Err(e) = self.save_meta(&m) {
                        log::warn!("{book}: {e}");
                    }
                }
                if let Some(c) = self.conns.get_mut(&conn) {
                    c.open.retain(|b| *b != book);
                    c.peer.remove(&book);
                }
                self.broadcast(&book, &ServerMsg::PeerLeft { book: book.clone(), conn }, None);
                self.broadcast_books();
            }
            ClientMsg::RenameBook { book, name } => {
                let creator = self.metas.get(&book).map(|m| m.created_by.clone()).unwrap_or_default();
                if role != Role::Admin && creator != user {
                    return Err(Error::Denied("only the workbook's creator or an administrator can rename it".into()));
                }
                let name = name.trim().to_string();
                if name.is_empty() || self.metas.values().any(|m| m.id != book && m.name.eq_ignore_ascii_case(&name)) {
                    return Err(Error::Denied("that name is empty or taken".into()));
                }
                let mut m = self.metas.get(&book).cloned().ok_or_else(|| Error::Denied("no such workbook".into()))?;
                m.name = name;
                self.save_meta(&m)?;
                if let Some(b) = self.books.get_mut(&book) {
                    b.meta.name = m.name.clone();
                }
                self.metas.insert(book, m);
                self.broadcast_books();
            }
            ClientMsg::DeleteBook { book, stopped } => {
                let creator = self.metas.get(&book).map(|m| m.created_by.clone()).unwrap_or_default();
                if role != Role::Admin && creator != user {
                    return Err(Error::Denied("only the workbook's creator or an administrator can delete it".into()));
                }
                let from = self.book_dir(&book);
                let trash = self.dir.join("trash");
                fs::create_dir_all(&trash)?;
                // Close its log files first: Windows can't move a folder with open files in it.
                // A failed move leaves it listed; it is loaded again when next opened.
                self.books.remove(&book);
                fs::rename(&from, trash.join(format!("{book}-{}", now_ms())))?;
                self.metas.remove(&book);
                if stopped {
                    log::info!("{user} stopped sharing {book} (its server copy is kept in the trash folder)");
                } else {
                    log::info!("{user} deleted workbook {book} (kept in the trash folder)");
                }
                self.broadcast(&book, &ServerMsg::BookGone { book: book.clone(), by: Some(display.clone()), stopped }, None);
                self.broadcast_books();
            }
            ClientMsg::Patch { book, cseq, patch, base } => {
                if let Err(e) = self.take_patch(conn, &user, &display, role, &book, cseq, patch, base) {
                    self.reply(conn, &ServerMsg::Rejected { book, cseq, reason: e.to_string() });
                }
            }
            ClientMsg::Presence { book, sheet, range, editing } => {
                let Some(u) = self.users.iter().find(|u| u.user == user) else { return Ok(()) };
                let peer = Peer { user: user.clone(), display: u.display.clone(), color: u.color, sheet, range, editing, conn };
                if let Some(c) = self.conns.get_mut(&conn) {
                    c.peer.insert(book.clone(), peer.clone());
                }
                self.broadcast(&book, &ServerMsg::PeerMoved { book: book.clone(), peer }, Some(conn));
            }
            ClientMsg::History { book, sheet, cell, user: who, limit } => {
                self.require(&user, role, &book, false)?;
                let path = self.book_dir(&book).join("audit.jsonl");
                let mut changes: Vec<Change> = Vec::new();
                if let Ok(f) = File::open(path) {
                    for line in BufReader::new(f).lines().map_while(std::result::Result::ok) {
                        let Ok(c) = serde_json::from_str::<Change>(&line) else { continue };
                        if sheet.as_ref().is_some_and(|s| *s != c.sheet)
                            || cell.as_ref().is_some_and(|x| !x.eq_ignore_ascii_case(&c.cell))
                            || who.as_ref().is_some_and(|w| *w != c.user)
                        {
                            continue;
                        }
                        changes.push(c);
                    }
                }
                changes.reverse();
                changes.truncate(limit.clamp(1, HISTORY_MAX));
                self.reply(conn, &ServerMsg::History { book, changes });
            }
            ClientMsg::Versions { book } => {
                self.require(&user, role, &book, false)?;
                let versions = self.versions(&book);
                self.reply(conn, &ServerMsg::Versions { book, versions });
            }
            ClientMsg::GetRules { book } => {
                self.require(&user, role, &book, false)?;
                let rules = load_rules(&self.book_dir(&book));
                self.reply(conn, &ServerMsg::Rules { book, rules });
            }
            ClientMsg::SetRules { book, rules } => {
                let m = self.metas.get(&book).cloned().ok_or_else(|| Error::Denied("no such workbook".into()))?;
                if role != Role::Admin && m.created_by != user {
                    return Err(Error::Denied("only the workbook's creator or an administrator can change its automation".into()));
                }
                let old = load_rules(&self.book_dir(&book));
                let mut clean = Vec::new();
                for (i, mut r) in rules.into_iter().take(50).enumerate() {
                    check_rule(&r, &self.users)?;
                    if r.id.is_empty() {
                        r.id = format!("r{}-{i}", now_ms());
                    }
                    // The server keeps when it last ran.
                    r.last_run = old.iter().find(|o| o.id == r.id).and_then(|o| o.last_run);
                    clean.push(r);
                }
                save_rules(&self.book_dir(&book), &clean)?;
                log::info!("{user} set {} automation rule(s) on \"{}\"", clean.len(), m.name);
                self.reply(conn, &ServerMsg::Rules { book, rules: clean });
            }
            ClientMsg::RunRule { book, id } => {
                let m = self.metas.get(&book).cloned().ok_or_else(|| Error::Denied("no such workbook".into()))?;
                if role != Role::Admin && m.created_by != user {
                    return Err(Error::Denied("only the workbook's creator or an administrator can run its automation".into()));
                }
                let rule = load_rules(&self.book_dir(&book)).into_iter().find(|r| r.id == id).ok_or_else(|| Error::Denied("no such rule".into()))?;
                let summary = self.run_rule(&book, &rule, None)?;
                self.reply(conn, &ServerMsg::Ok { what: summary });
            }
            ClientMsg::ListTemplates => {
                let templates = list_templates(&self.dir);
                self.reply(conn, &ServerMsg::Templates { templates });
            }
            ClientMsg::GetTemplate { id } => {
                let dir = self.dir.join("templates");
                let info = list_templates(&self.dir).into_iter().find(|t| t.id == id).ok_or_else(|| Error::Denied("no such template".into()))?;
                let bytes = fs::read(dir.join(format!("{id}.xlsx")))?;
                // Who used it hears about its next approved version.
                let used = dir.join(format!("{id}.users"));
                let mut users: Vec<String> = fs::read_to_string(&used).unwrap_or_default().lines().map(str::to_string).collect();
                if !users.contains(&user) && users.len() < 10_000 {
                    users.push(user.clone());
                    let _ = write_atomic(&used, users.join("\n").as_bytes());
                }
                self.reply(conn, &ServerMsg::Template { info, sha256: proto::sha256_hex(&bytes), xlsx: proto::b64(&bytes) });
            }
            ClientMsg::PublishTemplate { id, name, category, description, mandatory, xlsx, sha256 } => {
                if role != Role::Admin {
                    return Err(Error::Denied(
                        "only an administrator publishes office templates: ask yours, or keep it as a personal template".into(),
                    ));
                }
                let name = name.trim().to_string();
                if name.is_empty() {
                    return Err(Error::Denied("give the template a name".into()));
                }
                let bytes = proto::unb64_checked(&xlsx, sha256.as_deref().unwrap_or(""))?;
                // Must open as a workbook: a template nobody can use is refused here, not later.
                gridcraft_engine::io::open_bytes("template.xlsx", &bytes)?;
                let dir = self.dir.join("templates");
                fs::create_dir_all(&dir)?;
                let existing = id.as_ref().and_then(|i| list_templates(&self.dir).into_iter().find(|t| &t.id == i));
                let id = existing.as_ref().map_or_else(|| slug(&name), |t| t.id.clone());
                let info = TemplateInfo {
                    id: id.clone(),
                    name: name.clone(),
                    category: category.trim().to_string(),
                    description: description.trim().to_string(),
                    mandatory: mandatory.trim().to_string(),
                    version: existing.as_ref().map_or(1, |t| t.version.saturating_add(1)),
                    updated: now_ms(),
                    updated_by: display.clone(),
                    size: bytes.len() as u64,
                };
                write_atomic(&dir.join(format!("{id}.xlsx")), &bytes)?;
                write_atomic(&dir.join(format!("{id}.json")), &serde_json::to_vec_pretty(&info)?)?;
                log::info!("{user} published template \"{name}\" version {}", info.version);
                // A new version: tell those who used the old one (their spreadsheets are not touched).
                if existing.is_some() {
                    let users: Vec<String> =
                        fs::read_to_string(dir.join(format!("{id}.users"))).unwrap_or_default().lines().map(str::to_string).collect();
                    for u in users.iter().filter(|u| **u != user) {
                        self.notify(u, Notice {
                            kind: "template".into(),
                            name: name.clone(),
                            by: display.clone(),
                            text: format!("A new approved version (v{}) of the template \"{name}\" is available. Spreadsheets already made from it are not changed.", info.version),
                            ..Notice::default()
                        });
                    }
                }
                let templates = list_templates(&self.dir);
                if let Some(frame) = encode(&ServerMsg::Templates { templates }) {
                    for c in self.conns.values() {
                        let _ = c.tx.send(frame.clone());
                    }
                }
            }
            ClientMsg::DeleteTemplate { id } => {
                if role != Role::Admin {
                    return Err(Error::Denied("only an administrator removes office templates".into()));
                }
                let dir = self.dir.join("templates");
                let trash = self.dir.join("trash").join("templates");
                fs::create_dir_all(&trash)?;
                for ext in ["xlsx", "json", "users"] {
                    let from = dir.join(format!("{id}.{ext}"));
                    if from.exists() {
                        fs::rename(&from, trash.join(format!("{id}-{}.{ext}", now_ms())))?;
                    }
                }
                let templates = list_templates(&self.dir);
                self.reply(conn, &ServerMsg::Templates { templates });
            }
            ClientMsg::SetAccess { book, access } => {
                let m = self.metas.get(&book).cloned().ok_or_else(|| Error::Denied("no such workbook".into()))?;
                if role != Role::Admin && m.created_by != user {
                    return Err(Error::Denied("only the workbook's creator or an administrator can change who has access".into()));
                }
                let clean = |p: &str| if p == "viewer" { "viewer".to_string() } else { "editor".to_string() };
                let access = Access {
                    mode: if access.mode == "people" { "people".into() } else { "office".into() },
                    everyone: clean(&access.everyone),
                    people: access
                        .people
                        .iter()
                        .map(|(u, p)| (u.trim().to_ascii_lowercase(), clean(p)))
                        .filter(|(u, _)| self.users.iter().any(|r| r.user == *u))
                        .take(1000)
                        .collect(),
                    expires: access.expires,
                    download: access.download,
                };
                let now = now_ms();
                let before: Vec<(String, Option<&'static str>)> =
                    self.users.iter().filter(|u| u.user != user).map(|u| (u.user.clone(), permission(&m, &u.user, u.role, now))).collect();
                let mut next = m.clone();
                next.access = access;
                self.save_meta(&next)?;
                self.metas.insert(book.clone(), next.clone());
                if let Some(b) = self.books.get_mut(&book) {
                    b.meta.access = next.access.clone();
                }
                // Tell the people whose access changed: newly shared, a different permission,
                // or no longer shared (they keep any copy of their own).
                for (who, was) in before {
                    let role_of = self.users.iter().find(|u| u.user == who).map_or(Role::Viewer, |u| u.role);
                    let now_p = permission(&next, &who, role_of, now);
                    // Only the people it concerns: named before or now, or who have had it open.
                    let involved = m.access.people.contains_key(&who) || next.access.people.contains_key(&who) || m.seen.contains_key(&who);
                    // Access they never used (the whole office could open it) counts as newly shared.
                    let fresh = !m.seen.contains_key(&who) && !m.access.people.contains_key(&who);
                    let was = if fresh && now_p.is_some() { None } else { was };
                    if was == now_p || !involved {
                        continue;
                    }
                    let (kind, text) = match (was, now_p) {
                        (None, Some(p)) => {
                            ("shared", format!("{display} shared a spreadsheet with you · {}", if p == "editor" { "can edit" } else { "can view" }))
                        }
                        (Some(_), Some(p)) => {
                            ("permission", format!("You can now {} {}", if p == "editor" { "edit" } else { "only view" }, next.name))
                        }
                        (Some(_), None) => ("revoked", format!("Your access to {} has changed: it is no longer shared with you", next.name)),
                        (None, None) => continue,
                    };
                    self.notify(
                        &who,
                        Notice {
                            kind: kind.into(),
                            book: book.clone(),
                            name: next.name.clone(),
                            by: display.clone(),
                            permission: now_p.unwrap_or("").into(),
                            text,
                            ..Notice::default()
                        },
                    );
                }
                // Anyone who lost access and has it open: it closes for them.
                let gone: Vec<u64> = self
                    .conns
                    .iter()
                    .filter(|(_, c)| c.open.contains(&book))
                    .filter(|(_, c)| self.users.iter().find(|u| u.user == c.user).is_none_or(|u| permission(&next, &u.user, u.role, now).is_none()))
                    .map(|(id, _)| *id)
                    .collect();
                for id in gone {
                    if let Some(c) = self.conns.get_mut(&id) {
                        c.open.retain(|b| *b != book);
                        if let Some(frame) = encode(&ServerMsg::BookGone { book: book.clone(), by: Some(display.clone()), stopped: false }) {
                            let _ = c.tx.send(frame);
                        }
                    }
                }
                log::info!("{user} changed who has access to \"{}\"", next.name);
                self.broadcast_books();
                self.reply(conn, &ServerMsg::Ok { what: "Sharing updated.".into() });
            }
            ClientMsg::SaveVersion { book, label } => {
                self.require(&user, role, &book, true)?;
                self.load_book(&book)?;
                let v = self.save_version(&book, &user, &label)?;
                log::info!("{user} saved a version of {book} at edit {}", v.seq);
                let versions = self.versions(&book);
                self.reply(conn, &ServerMsg::Versions { book, versions });
            }
            ClientMsg::GetVersion { book, version } => {
                self.require(&user, role, &book, false)?;
                let bytes = fs::read(self.version_path(&book, &version)?)?;
                self.reply(conn, &ServerMsg::Version { book, version, xlsx: proto::b64(&bytes) });
            }
            ClientMsg::RestoreVersion { book, version } => {
                self.require(&user, role, &book, true)?;
                if !role.can_edit() {
                    return Err(Error::Denied("view-only accounts can't restore versions".into()));
                }
                self.load_book(&book)?;
                let bytes = fs::read(self.version_path(&book, &version)?)?;
                let (wb, _) = gridcraft_engine::io::open_bytes("book.xlsx", &bytes)?;
                // Keep what is being replaced, then make the restored workbook the new snapshot.
                self.save_version(&book, &user, "before restore")?;
                let dir = self.book_dir(&book);
                let Some(b) = self.books.get_mut(&book) else { return Ok(()) };
                b.meta.seq += 1;
                let entry = Logged {
                    seq: b.meta.seq,
                    time: now_ms(),
                    user: user.clone(),
                    origin: Some(format!("restore:{version}")),
                    patch: WbPatch::default(),
                };
                writeln!(b.log, "{}", serde_json::to_string(&entry)?)?;
                b.log.sync_data()?;
                b.wb = wb;
                b.meta.modified = entry.time;
                b.meta.modified_by = user.clone();
                let meta = b.meta.clone();
                let _ = dir;
                self.snapshot(&book)?;
                self.save_meta(&meta)?;
                self.metas.insert(book.clone(), meta);
                log::info!("{user} restored {book} to version {version}");
                self.broadcast(&book, &ServerMsg::Reload { book: book.clone(), reason: format!("{display} restored an earlier version") }, None);
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    fn take_patch(
        &mut self,
        conn: u64,
        user: &str,
        display: &str,
        role: Role,
        book: &str,
        cseq: u64,
        patch: WbPatch,
        base: Option<u64>,
    ) -> Result<()> {
        if !role.can_edit() {
            return Err(Error::Denied("view-only account".into()));
        }
        self.require(user, role, book, true)?;
        self.load_book(book)?;
        let instance = self.conns.get(&conn).map(|c| c.instance.clone()).unwrap_or_default();
        let b = self.books.get_mut(book).ok_or_else(|| Error::Denied("no such workbook".into()))?;
        let time = now_ms();
        let changes = describe(&b.wb, &patch, b.meta.seq + 1, time, user, display);
        let mut next = b.wb.clone();
        patch::apply(&mut next, &patch)?;
        // Cells someone else changed after the last edit this client had seen.
        let mut conflicts = Vec::new();
        if let Some(base) = base {
            for c in &changes {
                let Some(at) = CellRef::parse(&c.cell) else { continue };
                if let Some((seq, inst, other)) = b.writes.get(&(c.sheet.clone(), at))
                    && *seq > base
                    && *inst != instance
                {
                    let shown = self.users.iter().find(|u| u.user == *other).map_or_else(|| other.clone(), |u| u.display.clone());
                    conflicts.push(Conflict {
                        sheet: c.sheet.clone(),
                        cell: c.cell.clone(),
                        user: other.clone(),
                        display: shown,
                        theirs: c.old.clone(),
                        yours: c.new.clone(),
                    });
                }
            }
        }
        let entry = Logged { seq: b.meta.seq + 1, time, user: user.to_string(), origin: Some(format!("{instance}:{cseq}")), patch };
        // On disk before anyone hears of it.
        let line = serde_json::to_string(&entry)?;
        writeln!(b.log, "{line}")?;
        b.log.sync_data()?;
        for c in &changes {
            if let Ok(l) = serde_json::to_string(c) {
                let _ = writeln!(b.audit, "{l}");
            }
            if let Ok(at) = CellRef::parse(&c.cell).ok_or(()) {
                b.last_edits.insert((c.sheet.clone(), at), (c.user.clone(), c.time));
                b.writes.insert((c.sheet.clone(), at), (entry.seq, instance.clone(), user.to_string()));
            }
        }
        let _ = b.audit.flush();
        b.wb = next;
        b.meta.seq = entry.seq;
        b.meta.modified = time;
        b.meta.modified_by = user.to_string();
        b.tail.push(entry.clone());
        b.touched = time;
        let (seq, snap_due, version_due) = (b.meta.seq, b.meta.seq - b.snapshot_seq >= SNAPSHOT_EVERY, time - b.last_version >= AUTO_VERSION_MS);
        if !conflicts.is_empty() {
            log::info!("{book}: {user}'s edit {seq} replaced {} cell(s) changed meanwhile by others", conflicts.len());
        }
        // Change rules see people's edits (never the automation's own, so rules can't loop).
        let fired: Vec<(Rule, Change)> = if user == AUTOMATION {
            Vec::new()
        } else {
            let first = self.books.get(book).and_then(|b| b.wb.sheets.first().map(|s| s.name.clone())).unwrap_or_default();
            load_rules(&self.book_dir(book))
                .into_iter()
                .filter(|r| r.enabled)
                .flat_map(|r| {
                    changes
                        .iter()
                        .filter(|c| {
                            c.sheet == first
                                && matches!(&r.trigger, proto::Trigger::Change { column, equals }
                                    if column_of(&c.cell).eq_ignore_ascii_case(column.trim()) && norm(&c.new) == norm(equals) && norm(&c.old) != norm(equals))
                        })
                        .map(|c| (r.clone(), c.clone()))
                        .collect::<Vec<_>>()
                })
                .take(20)
                .collect()
        };
        let msg =
            ServerMsg::Patched { book: book.to_string(), seq, time, user: user.to_string(), origin: entry.origin, patch: entry.patch, conflicts };
        self.broadcast(book, &msg, None);
        if version_due {
            if let Err(e) = self.save_version(book, "auto", "automatic") {
                log::error!("{book}: automatic version failed: {e}");
            }
            self.prune_versions(book);
        } else if snap_due && let Err(e) = self.snapshot(book) {
            log::error!("{book}: snapshot failed: {e}");
        }
        if let Some(m) = self.books.get(book).map(|b| b.meta.clone()) {
            if seq % 20 == 0 {
                let _ = self.save_meta(&m);
            }
            self.metas.insert(book.to_string(), m);
        }
        for (rule, change) in fired {
            if let Err(e) = self.run_rule(book, &rule, Some((&change, display))) {
                log::warn!("{book}: rule \"{}\": {e}", rule.name);
            }
        }
        Ok(())
    }

    /// Schedule rules whose time has come (housekeeping, every half minute).
    fn run_due_rules(&mut self) {
        use chrono::{Datelike, Timelike};
        let now = chrono::Local::now();
        let ids: Vec<String> = self.metas.keys().cloned().collect();
        for id in ids {
            let dir = self.book_dir(&id);
            let mut rules = load_rules(&dir);
            let mut ran = false;
            for i in 0..rules.len() {
                let Some(r) = rules.get(i).cloned() else { continue };
                let proto::Trigger::Schedule { every, day, time } = &r.trigger else { continue };
                if !r.enabled {
                    continue;
                }
                let (h, m) =
                    time.split_once(':').and_then(|(h, m)| Some((h.trim().parse::<u32>().ok()?, m.trim().parse::<u32>().ok()?))).unwrap_or((9, 0));
                let Some(today_at) = now.with_hour(h.min(23)).and_then(|t| t.with_minute(m.min(59))).and_then(|t| t.with_second(0)) else { continue };
                let due = now >= today_at
                    && r.last_run.is_none_or(|t| t < today_at.timestamp_millis())
                    && (every != "weekly" || now.weekday().num_days_from_monday() == u32::from(*day));
                if !due {
                    continue;
                }
                match self.run_rule(&id, &r, None) {
                    Ok(s) => log::info!("{id}: rule \"{}\" ran: {s}", r.name),
                    Err(e) => log::warn!("{id}: rule \"{}\": {e}", r.name),
                }
                if let Some(x) = rules.get_mut(i) {
                    x.last_run = Some(now_ms());
                }
                ran = true;
            }
            if ran && let Err(e) = save_rules(&dir, &rules) {
                log::warn!("{id}: automation.json: {e}");
            }
        }
    }

    /// Runs a rule's steps on the server's copy of the workbook; returns what it found.
    fn run_rule(&mut self, book: &str, rule: &Rule, change: Option<(&Change, &str)>) -> Result<String> {
        self.load_book(book)?;
        let (name, wb) = self.books.get(book).map(|b| (b.meta.name.clone(), b.wb.clone())).ok_or_else(|| Error::Denied("no such workbook".into()))?;
        let mut found: Vec<String> = Vec::new();
        let mut rows: Vec<Vec<String>> = Vec::new();
        if let Some((c, by)) = change {
            found.push(format!("{} changed {}!{} to \"{}\"", by, c.sheet, c.cell, c.new));
        }
        for a in &rule.actions {
            match a {
                proto::Action::QualityCheck => {
                    let mut s = gridcraft_engine::Session::new();
                    s.add_document(gridcraft_engine::DocState::new(wb.clone(), None, name.clone(), gridcraft_engine::Trust::FromFile));
                    match s.execute("data.qualityCheck", serde_json::json!({"limit": 500})) {
                        Ok(r) => {
                            let total = r.get("total").and_then(serde_json::Value::as_u64).unwrap_or(0);
                            found.push(format!("{total} quality issue(s)"));
                            for i in r.get("issues").and_then(serde_json::Value::as_array).into_iter().flatten().take(500) {
                                let g = |k: &str| i.get(k).and_then(serde_json::Value::as_str).unwrap_or("").to_string();
                                rows.push(vec!["Quality".into(), g("cell"), g("message"), g("suggestion")]);
                            }
                        }
                        Err(e) => found.push(format!("quality check could not run ({e})")),
                    }
                }
                proto::Action::Overdue { date_column, status_column, done } => {
                    let over = overdue(&wb, date_column, status_column, done);
                    found.push(format!("{} overdue item(s)", over.len()));
                    for (row, date, status) in over.into_iter().take(500) {
                        rows.push(vec!["Overdue".into(), format!("row {row}"), format!("due {date}"), format!("status \"{status}\"")]);
                    }
                }
                proto::Action::Stamp { column } => {
                    if let Some((c, _)) = change
                        && let (Some(at), Some(col)) = (CellRef::parse(&c.cell), col_index(column))
                    {
                        let stamp = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
                        let patch = WbPatch {
                            sheets: vec![patch::SheetPatch {
                                name: c.sheet.clone(),
                                cells: vec![patch::CellChange {
                                    r: at.row,
                                    c: col,
                                    cell: Some(CellData { v: Value::Text(stamp.into()), f: None, s: None }),
                                }],
                                ..Default::default()
                            }],
                            ..Default::default()
                        };
                        self.take_patch(0, AUTOMATION, "Automation", Role::Admin, book, 0, patch, None)?;
                        found.push(format!("completion time written in {column}{}", at.row + 1));
                    }
                }
                proto::Action::Report | proto::Action::Notify { .. } => {}
            }
        }
        let summary = format!("{}: {}", rule.name, if found.is_empty() { "nothing to report".into() } else { found.join("; ") });
        let people: Vec<String> =
            rule.actions.iter().flat_map(|a| if let proto::Action::Notify { people } = a { people.clone() } else { Vec::new() }).collect();
        // The report: a new shared workbook only the rule's people (and its creator) can open.
        let mut report_id = String::new();
        if rule.actions.iter().any(|a| matches!(a, proto::Action::Report)) {
            let mut csv = String::from("Step,Where,Found,Detail\r\n");
            csv.push_str(&format!("Summary,,\"{}\",\r\n", summary.replace('"', "\"\"")));
            for r in &rows {
                csv.push_str(&r.iter().map(|x| format!("\"{}\"", x.replace('"', "\"\""))).collect::<Vec<_>>().join(","));
                csv.push_str("\r\n");
            }
            let (report, _) = gridcraft_engine::io::open_bytes_with("report.csv", csv.as_bytes(), None, true)?;
            let bytes = gridcraft_engine::io::save_bytes(&report, "report.xlsx")?;
            let title = format!("{} report {} ({})", name, chrono::Local::now().format("%Y-%m-%d %H%M"), rule.name);
            let owner = self.metas.get(book).map(|m| m.created_by.clone()).unwrap_or_default();
            let mut meta = self.create_book(&owner, &title, Some(&bytes), None)?;
            meta.access =
                Access { mode: "people".into(), people: people.iter().map(|p| (p.clone(), "viewer".to_string())).collect(), ..Access::default() };
            self.save_meta(&meta)?;
            report_id = meta.id.clone();
            self.metas.insert(meta.id.clone(), meta);
            self.broadcast_books();
        }
        for p in people {
            self.notify(
                &p,
                Notice {
                    kind: "automation".into(),
                    book: if report_id.is_empty() { book.to_string() } else { report_id.clone() },
                    name: name.clone(),
                    by: "Automation".into(),
                    text: summary.clone(),
                    ..Notice::default()
                },
            );
        }
        Ok(summary)
    }

    /// Writes metadata and a final snapshot of every loaded workbook (shutdown).
    pub fn flush_all(&mut self) {
        let ids: Vec<String> = self.books.keys().cloned().collect();
        for id in ids {
            let dirty = self.books.get(&id).is_some_and(|b| b.meta.seq != b.snapshot_seq);
            if dirty && let Err(e) = self.snapshot(&id) {
                log::error!("{id}: snapshot failed: {e}");
            }
            if let Some(m) = self.books.get(&id).map(|b| b.meta.clone()) {
                let _ = self.save_meta(&m);
            }
        }
    }
}

impl Server {
    pub fn flush_all(&self) {
        lock(&self.state).flush_all();
    }

    /// Housekeeping, every half minute: workbooks nobody has open leave memory.
    pub fn unload_idle(&self) {
        let mut st = lock(&self.state);
        st.run_due_rules();
        st.unload_idle();
    }

    /// Looks for a new installer in `updates/` and tells everyone connected about it.
    /// The web link's page: opens the app on the shared workbook, or offers the installer. It
    /// names nothing (no sign-in here): the app checks the person's access when it opens.
    fn serve_http(&self, mut stream: TcpStream) {
        let mut buf = vec![0u8; 8192];
        let n = stream.read(&mut buf).unwrap_or(0);
        let req = String::from_utf8_lossy(buf.get(..n).unwrap_or_default()).to_string();
        let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
        let host = req
            .lines()
            .find_map(|l| l.strip_prefix("Host:").or_else(|| l.strip_prefix("host:")))
            .map(|h| h.trim().to_string())
            .filter(|h| h.chars().all(|c| c.is_ascii_alphanumeric() || "-._:[]".contains(c)))
            .unwrap_or_default();
        let respond = |stream: &mut TcpStream, status: &str, kind: &str, extra: &str, body: &[u8]| {
            let head = format!(
                "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n{extra}Cache-Control: no-store\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(body);
        };
        if let Some(book) = path.strip_prefix("/open/").filter(|b| !b.is_empty() && b.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')) {
            let app_link = proto::link(&host, book);
            let page = format!(
                "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\"><title>Open in Dexterity Sheets</title>\
<style>body{{font-family:Segoe UI,Arial,sans-serif;max-width:34rem;margin:4rem auto;padding:0 1rem;color:#1b1f2a}}a.b{{display:inline-block;background:#1e2a78;color:#fff;padding:.7rem 1.2rem;border-radius:.4rem;text-decoration:none;margin:.3rem 0}}p{{line-height:1.5}}</style></head>\
<body><h1>A shared spreadsheet</h1><p>Someone shared a spreadsheet with you on the office server. It opens in Dexterity Sheets, which checks that you have access.</p>\
<p><a class=\"b\" href=\"{app_link}\">Open in Dexterity Sheets</a></p><p>Not installed yet? <a href=\"/download\">Get Dexterity Sheets</a> from this server, install it, sign in, then open the link again.</p>\
<script>location.href={app_json};</script></body></html>",
                app_json = serde_json::to_string(&app_link).unwrap_or_default()
            );
            respond(&mut stream, "200 OK", "text/html; charset=utf-8", "", page.as_bytes());
        } else if path == "/download" {
            let offer = lock(&self.state).update.clone();
            match offer.and_then(|(u, p)| fs::read(p).ok().map(|b| (u, b))) {
                Some((u, bytes)) => {
                    let name: String = u.file.chars().filter(|c| c.is_ascii_alphanumeric() || ".-_".contains(*c)).collect();
                    respond(&mut stream, "200 OK", "application/x-msi", &format!("Content-Disposition: attachment; filename=\"{name}\"\r\n"), &bytes);
                }
                None => respond(&mut stream, "404 Not Found", "text/plain; charset=utf-8", "", b"No installer on this server yet: ask IT."),
            }
        } else {
            respond(&mut stream, "404 Not Found", "text/plain; charset=utf-8", "", b"Not found");
        }
    }

    pub fn poll_updates(&self) {
        let mut st = lock(&self.state);
        let before: Vec<(String, String, String)> =
            st.available.iter().map(|(u, _)| (u.version.clone(), u.channel.clone(), u.status.clone())).collect();
        let policy = st.policy.clone();
        let found = st.scan_updates();
        st.update = found;
        let after: Vec<(String, String, String)> =
            st.available.iter().map(|(u, _)| (u.version.clone(), u.channel.clone(), u.status.clone())).collect();
        if before == after && policy == st.policy {
            return;
        }
        if let Some((u, _)) = &st.update {
            log::info!("update {} is available to clients", u.version);
        }
        // Each app hears what applies to its own version and channel (a release marked blocked
        // reaches the people running it at once).
        for c in st.conns.values() {
            let msg = if c.version.is_empty() {
                ServerMsg::Update { update: st.update.as_ref().map(|(u, _)| u.clone()), current: None, older: Vec::new(), policy: st.policy.clone() }
            } else {
                st.offer(&c.version, &c.channel)
            };
            if let Some(frame) = encode(&msg) {
                let _ = c.tx.send(frame);
            }
        }
    }

    /// The folder new installers go in.
    pub fn updates_dir(&self) -> PathBuf {
        lock(&self.state).dir.join("updates")
    }
}

/// The audit lines for a patch: one per cell whose content a person changed (values that were
/// only recalculated are not anyone's edit), plus one per sheet-level change.
fn describe(wb: &Workbook, p: &WbPatch, seq: u64, time: i64, user: &str, display: &str) -> Vec<Change> {
    let mut out = Vec::new();
    let mk = |sheet: &str, cell: String, old: String, new: String| Change {
        seq,
        time,
        user: user.to_string(),
        display: display.to_string(),
        sheet: sheet.to_string(),
        cell,
        old,
        new,
    };
    if let Some(st) = &p.structure {
        let after: Vec<String> = st
            .slots
            .iter()
            .map(|s| match s {
                patch::Slot::Existing { name, .. } | patch::Slot::New { name } => name.clone(),
            })
            .collect();
        out.push(mk("", String::new(), format!("sheets: {}", st.base.join(", ")), format!("sheets: {}", after.join(", "))));
    }
    if !p.fields.is_empty() {
        let mut fields: Vec<&str> = p.fields.keys().map(|k| friendly_field(k)).collect();
        fields.dedup();
        out.push(mk("", String::new(), String::new(), format!("changed {}", fields.join(", "))));
    }
    for sp in &p.sheets {
        let sheet = wb.sheets.iter().find(|s| s.name == sp.name);

        for ch in &sp.cells {
            let at = CellRef::new(ch.r, ch.c);
            let old = sheet.and_then(|s| s.cells.get(at)).map(|c| CellData {
                v: c.value.clone(),
                f: c.formula.as_ref().map(|f| patch::FormulaData { text: f.text.clone(), array: f.array, dynamic: f.dynamic }),
                s: (c.style != gridcraft_engine::model::StyleId::DEFAULT).then(|| wb.styles.get(c.style).clone()),
            });
            match (&old, &ch.cell) {
                (Some(o), Some(n)) if o.same_content(n) => continue,
                (Some(o), Some(n)) if o.f.is_some() && o.f == n.f && o.s == n.s => continue,
                _ => {}
            }
            let old_in = old.as_ref().map(CellData::input).unwrap_or_default();
            let new_in = ch.cell.as_ref().map(CellData::input).unwrap_or_default();
            if old_in == new_in {
                out.push(mk(&sp.name, a1(at), old_in, format!("{new_in} (format)")));
            } else {
                out.push(mk(&sp.name, a1(at), old_in, new_in));
            }
        }

        let mut fields: Vec<&str> = sp.fields.keys().chain(sp.keyed.keys()).map(|k| friendly_field(k)).collect();
        fields.sort_unstable();
        fields.dedup();
        if !fields.is_empty() {
            out.push(mk(&sp.name, String::new(), String::new(), format!("changed {}", fields.join(", "))));
        }
    }
    out
}

/// What a sheet or workbook field is called in the Edit History.
fn friendly_field(k: &str) -> &str {
    match k {
        "cols" => "column widths",
        "rows" => "row heights",
        "default_col_width" | "default_row_height" => "default sizes",
        "merges" => "merged cells",
        "freeze" => "freeze panes",
        "tab_color" => "tab colour",
        "visibility" => "sheet visibility",
        "show_gridlines" | "show_headings" | "show_formulas" | "show_zeros" => "display options",
        "cond_formats" => "conditional formatting",
        "validations" => "data validation",
        "tables" => "tables",
        "autofilter" => "filter",
        "comments" => "comments and notes",
        "hyperlinks" => "links",
        "charts" => "charts",
        "images" | "cell_pictures" => "pictures",
        "shapes" => "shapes",
        "sparklines" => "sparklines",
        "pivots" => "pivot tables",
        "slicers" => "slicers",
        "protection" | "edit_ranges" => "protection",
        "print" => "page setup",
        "names" => "named ranges",
        "theme" => "theme",
        "calc" => "calculation options",
        "props" => "document properties",
        "cell_styles" => "cell styles",
        other => other,
    }
}

/// Starts the server: TCP on `port` and LAN discovery. Returns once listening.
pub fn start(dir: PathBuf, name: String, port: u16) -> Result<Arc<Server>> {
    start_on(dir, name, port).map(|(s, _)| s)
}

/// As [`start`], also returning the port (useful with port 0).
pub fn start_on(dir: PathBuf, name: String, port: u16) -> Result<(Arc<Server>, u16)> {
    let server = Arc::new(Server::open(dir, name.clone())?);
    let listener = TcpListener::bind(("0.0.0.0", port))?;
    let port = listener.local_addr()?.port();
    let s = server.clone();
    std::thread::Builder::new().name("dexsheets-accept".into()).spawn(move || s.serve(listener))?;
    let version = env!("CARGO_PKG_VERSION").to_string();
    std::thread::Builder::new().name("dexsheets-discovery".into()).spawn(move || crate::discovery::answer(name, port, version))?;
    let s = server.clone();
    std::thread::Builder::new().name("dexsheets-updates".into()).spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(30));
            s.poll_updates();
            s.unload_idle();
        }
    })?;
    Ok((server, port))
}

/// Who automation edits are attributed to.
const AUTOMATION: &str = "automation";

fn load_rules(dir: &Path) -> Vec<Rule> {
    fs::read(dir.join("automation.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save_rules(dir: &Path, rules: &[Rule]) -> Result<()> {
    write_atomic(&dir.join("automation.json"), &serde_json::to_vec_pretty(rules)?)
}

/// A rule is refused if it names a column that doesn't exist or a person who isn't an account.
fn check_rule(r: &Rule, users: &[UserRecord]) -> Result<()> {
    let col = |c: &str| col_index(c).is_some();
    match &r.trigger {
        proto::Trigger::Schedule { every, day, time } => {
            if every != "daily" && every != "weekly"
                || *day > 6
                || !time
                    .split_once(':')
                    .is_some_and(|(h, m)| h.trim().parse::<u32>().is_ok_and(|h| h < 24) && m.trim().parse::<u32>().is_ok_and(|m| m < 60))
            {
                return Err(Error::Denied(format!("rule \"{}\": the schedule needs daily or weekly, a day 0-6 and a time like 09:00", r.name)));
            }
        }
        proto::Trigger::Change { column, .. } => {
            if !col(column) {
                return Err(Error::Denied(format!("rule \"{}\": \"{column}\" is not a column", r.name)));
            }
        }
    }
    for a in &r.actions {
        match a {
            proto::Action::Notify { people } => {
                if let Some(p) = people.iter().find(|p| !users.iter().any(|u| &u.user == *p)) {
                    return Err(Error::Denied(format!("rule \"{}\": no account \"{p}\"", r.name)));
                }
            }
            proto::Action::Overdue { date_column, status_column, .. } if !col(date_column) || !col(status_column) => {
                return Err(Error::Denied(format!("rule \"{}\": the overdue check needs two columns such as D and F", r.name)));
            }
            proto::Action::Stamp { column } if !col(column) || !matches!(r.trigger, proto::Trigger::Change { .. }) => {
                return Err(Error::Denied(format!("rule \"{}\": a completion time needs a column and a change trigger", r.name)));
            }
            _ => {}
        }
    }
    Ok(())
}

/// Column index of "F" / "AB" (0-based), up to XFD.
fn col_index(c: &str) -> Option<u32> {
    let c = c.trim();
    if c.is_empty() || c.len() > 3 || !c.chars().all(|x| x.is_ascii_alphabetic()) {
        return None;
    }
    let n = c.chars().try_fold(0u32, |n, x| n.checked_mul(26)?.checked_add(u32::from(x.to_ascii_uppercase() as u8 - b'A') + 1))?;
    (1..=16_384).contains(&n).then(|| n - 1)
}

fn column_of(a1: &str) -> String {
    a1.chars().take_while(char::is_ascii_alphabetic).collect()
}

fn norm(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// (row number, due date, status) of rows on the first sheet whose date has passed and whose
/// status is not `done`. Dates are cell serial numbers (formatted as dates).
fn overdue(wb: &Workbook, date_col: &str, status_col: &str, done: &str) -> Vec<(u32, String, String)> {
    let (Some(dc), Some(sc), Some(sh)) = (col_index(date_col), col_index(status_col), wb.sheets.first()) else { return Vec::new() };
    let today = (now_ms() as f64 / 86_400_000.0).floor() + 25_569.0;
    let mut out = Vec::new();
    for (at, cell) in sh.cells.iter() {
        if at.col != dc || out.len() >= 10_000 {
            continue;
        }
        let Value::Number(d) = cell.value else { continue };
        if d >= today || d < 1.0 {
            continue;
        }
        let status = sh.cell(CellRef::new(at.row, sc)).map(|c| patch::value_text(&c.value)).unwrap_or_default();
        if norm(&status) == norm(done) {
            continue;
        }
        let date = chrono::NaiveDate::from_ymd_opt(1899, 12, 30)
            .and_then(|b| b.checked_add_days(chrono::Days::new(d as u64)))
            .map(|x| x.format("%Y-%m-%d").to_string())
            .unwrap_or_default();
        out.push((at.row + 1, date, status));
    }
    out.sort();
    out
}

/// The template library (`templates/<id>.json` beside `<id>.xlsx`), by category then name.
fn list_templates(dir: &Path) -> Vec<TemplateInfo> {
    let mut out: Vec<TemplateInfo> = fs::read_dir(dir.join("templates"))
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
                .filter_map(|e| fs::read(e.path()).ok().and_then(|b| serde_json::from_slice::<TemplateInfo>(&b).ok()))
                .filter(|t| dir.join("templates").join(format!("{}.xlsx", t.id)).is_file())
                .take(5000)
                .collect()
        })
        .unwrap_or_default();
    out.sort_by_key(|t| (t.category.to_lowercase(), t.name.to_lowercase()));
    out
}

/// What `user` may do with a workbook: `editor`, `viewer`, or nothing (`None`). The creator and
/// administrators always edit; a view-only account never edits; a link past its expiry date
/// opens for nobody else.
fn permission(m: &BookMeta, user: &str, role: Role, now: i64) -> Option<&'static str> {
    if role == Role::Admin || m.created_by == user {
        return Some("editor");
    }
    if m.access.expires.is_some_and(|t| now > t) {
        return None;
    }
    let granted = if m.access.mode == "people" { m.access.people.get(user).map(String::as_str)? } else { m.access.everyone.as_str() };
    Some(if granted == "editor" && role.can_edit() { "editor" } else { "viewer" })
}

/// Offered as an update: not blocked or deprecated.
fn offerable(u: &UpdateInfo) -> bool {
    u.status != "blocked" && u.status != "deprecated"
}

/// Appends one JSON line (feedback, health), creating the folder.
fn append_line(path: &Path, v: &serde_json::Value) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let mut f = fs::OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(f, "{}", serde_json::to_string(v)?)?;
    f.sync_data()?;
    Ok(())
}

/// Adds an app's command counts to `adoption.json` (per day and version; 90 days kept).
fn add_usage(path: &Path, version: &str, counts: &std::collections::BTreeMap<String, u64>) -> Result<()> {
    let mut all: serde_json::Value = fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_else(|| serde_json::json!({}));
    if !all.get("days").is_some_and(serde_json::Value::is_object) {
        all = serde_json::json!({"days": {}});
    }
    let today = chrono::Local::now().date_naive();
    let day = today.format("%Y-%m-%d").to_string();
    if let Some(days) = all.get_mut("days").and_then(serde_json::Value::as_object_mut) {
        let oldest = (today - chrono::Duration::days(90)).format("%Y-%m-%d").to_string();
        days.retain(|d, _| *d >= oldest);
        let per_day = days.entry(day).or_insert_with(|| serde_json::json!({}));
        if let Some(per_day) = per_day.as_object_mut() {
            let per_version = per_day.entry(version.to_string()).or_insert_with(|| serde_json::json!({}));
            if let Some(m) = per_version.as_object_mut() {
                // At most 2000 distinct commands per version and day: the catalogue is smaller.
                for (id, n) in counts.iter().take(2000) {
                    let was = m.get(id).and_then(serde_json::Value::as_u64).unwrap_or(0);
                    m.insert(id.clone(), serde_json::json!(was.saturating_add(*n)));
                }
            }
        }
    }
    write_atomic(path, &serde_json::to_vec_pretty(&all)?)
}

/// True when a connection ended before sending a complete hello.
fn is_probe(e: &Error) -> bool {
    matches!(e, Error::Io(io) if io.kind() == std::io::ErrorKind::UnexpectedEof)
}

#[cfg(test)]
mod housekeeping_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;
    use gridcraft_engine::core::Value;
    use gridcraft_engine::{DocState, Session, Trust};

    #[test]
    fn a_port_check_is_not_a_refused_login() {
        // Connect and hang up (the Server Manager's running check), or hang up mid-header.
        for sent in [&b""[..], &b"\x00\x00"[..]] {
            let e = proto::read_frame::<_, ClientMsg>(&mut &sent[..]).expect_err("no hello");
            assert!(is_probe(&e), "{e}");
        }
        // Junk and wrong passwords are still logged and slowed down.
        let mut junk = 4u32.to_be_bytes().to_vec();
        junk.extend_from_slice(b"\x00nope");
        let e = proto::read_frame::<_, ClientMsg>(&mut junk.as_slice()).expect_err("junk");
        assert!(!is_probe(&e), "{e}");
        assert!(!is_probe(&Error::Denied("wrong password".into())));
    }

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("dexsheets-house-{name}-{}", now_ms()));
        set_user(&d, "vikram", "Vikram", "pipe-123", Role::Admin).expect("user");
        d
    }

    /// The log entry that sets `cell` to `n`, on top of `wb`; `wb` becomes the result.
    fn entry(wb: &mut Arc<Workbook>, seq: u64, cell: &str, n: f64) -> Logged {
        let mut s = Session::new();
        s.add_document(DocState::new((**wb).clone(), None, "t".into(), Trust::Local));
        s.execute("cell.set", serde_json::json!({"cell": cell, "input": n.to_string()})).expect("set");
        let after = s.active().expect("doc").wb.clone();
        let p = patch::diff(wb, &after, &mut patch::DiffCache::default()).expect("diff");
        *wb = after;
        Logged { seq, time: now_ms(), user: "vikram".into(), origin: None, patch: p }
    }

    fn value(st: &State, id: &str, at: &str) -> Value {
        let b = st.books.get(id).expect("loaded");
        b.wb.sheet(0).expect("sheet").value(CellRef::parse(at).expect("cell"))
    }

    fn fresh(d: &Path) -> (State, String, Arc<Workbook>) {
        let mut st = Server::open(d.to_path_buf(), "T".into()).expect("open").state.into_inner().unwrap_or_else(|p| p.into_inner());
        let id = st.create_book("vikram", "Book", None, None).expect("create").id;
        st.load_book(&id).expect("load");
        let wb = Arc::new(st.books.get(&id).expect("b").wb.clone());
        (st, id, wb)
    }

    fn reopen(d: &Path, id: &str) -> State {
        let mut st = Server::open(d.to_path_buf(), "T".into()).expect("open").state.into_inner().unwrap_or_else(|p| p.into_inner());
        st.load_book(id).expect("load");
        st
    }

    #[test]
    fn a_line_cut_short_by_a_crash_never_swallows_the_next_edit() {
        let d = dir("cut");
        let (st, id, mut wb) = fresh(&d);
        drop(st);
        let log = d.join("books").join(&id).join("log.jsonl");
        let first = entry(&mut wb, 1, "A1", 1.0);
        // Edit 1 written; edit 2 cut off by a power cut halfway through (never acknowledged).
        fs::write(&log, format!("{}\n{{\"seq\":2,\"ti", serde_json::to_string(&first).expect("json"))).expect("write");
        let mut st = reopen(&d, &id);
        assert_eq!(st.books.get(&id).expect("b").meta.seq, 1);
        // The next acknowledged edit, appended exactly as a live edit is.
        let mut wb1 = Arc::new(st.books.get(&id).expect("b").wb.clone());
        let second = entry(&mut wb1, 2, "A2", 2.0);
        let b = st.books.get_mut(&id).expect("b");
        writeln!(b.log, "{}", serde_json::to_string(&second).expect("json")).expect("append");
        drop(st);
        let st = reopen(&d, &id);
        assert_eq!(st.books.get(&id).expect("b").meta.seq, 2, "the edit after the broken line survives a restart");
        assert_eq!(value(&st, &id, "A2"), Value::Number(2.0));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn the_log_is_compacted_and_an_older_snapshot_still_restores_everything() {
        let d = dir("compact");
        let (mut st, id, mut wb) = fresh(&d);
        let book = d.join("books").join(&id);
        let mut seq = 0;
        for round in 0..4 {
            let mut lines = String::new();
            for _ in 0..5 {
                seq += 1;
                lines += &format!("{}\n", serde_json::to_string(&entry(&mut wb, seq, &format!("A{seq}"), seq as f64)).expect("json"));
            }
            let b = st.books.get_mut(&id).expect("b");
            b.log.write_all(lines.as_bytes()).expect("append");
            // Reload so the server's copy has the edits, then snapshot (as every 300 edits).
            st.books.remove(&id);
            st.load_book(&id).expect("load");
            if round > 0 {
                st.snapshot(&id).expect("snapshot");
            }
        }
        let log = fs::read_to_string(book.join("log.jsonl")).expect("log");
        let first_kept = log.lines().filter_map(|l| serde_json::from_str::<Logged>(l).ok()).map(|e| e.seq).min();
        assert_eq!(first_kept, Some(11), "edits in every kept snapshot (5..=20 kept as 10, 15, 20) leave the log");
        drop(st);
        let st = reopen(&d, &id);
        assert_eq!(st.books.get(&id).expect("b").meta.seq, 20);
        assert_eq!(value(&st, &id, "A20"), Value::Number(20.0));
        drop(st);
        // The newest snapshot is damaged: the oldest kept one plus the log still has everything.
        fs::write(book.join("snapshot-20.xlsx"), b"not a workbook").expect("damage");
        fs::write(book.join("snapshot-15.xlsx"), b"not a workbook").expect("damage");
        let st = reopen(&d, &id);
        assert_eq!(st.books.get(&id).expect("b").meta.seq, 20);
        assert_eq!(value(&st, &id, "A13"), Value::Number(13.0));
        assert_eq!(value(&st, &id, "A20"), Value::Number(20.0));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn idle_workbooks_leave_memory_and_load_again() {
        let d = dir("idle");
        let (mut st, id, _) = fresh(&d);
        st.unload_idle();
        assert!(st.books.contains_key(&id), "just used: stays");
        st.books.get_mut(&id).expect("b").touched = 0;
        st.unload_idle();
        assert!(!st.books.contains_key(&id), "nobody had it open for a while: unloaded");
        st.load_book(&id).expect("loads again");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn automatic_versions_thin_out_but_kept_ones_stay() {
        let d = dir("versions");
        let (st, id, _) = fresh(&d);
        let vdir = d.join("books").join(&id).join("versions");
        fs::create_dir_all(&vdir).expect("dir");
        let now = now_ms();
        let add = |n: usize, age_days: i64, hour: i64, label: &str| {
            let v = VersionInfo {
                id: format!("v{n}"),
                seq: 1,
                time: now - age_days * DAY_MS - hour * 3_600_000,
                user: "auto".into(),
                label: label.into(),
                size: 1,
            };
            fs::write(vdir.join(format!("v{n}.json")), serde_json::to_vec(&v).expect("json")).expect("w");
            fs::write(vdir.join(format!("v{n}.xlsx")), b"x").expect("w");
        };
        add(1, 1, 0, "automatic");
        add(2, 1, 1, "automatic");
        add(3, 30, 0, "automatic");
        add(4, 30, 1, "automatic");
        add(5, 30, 2, "Issued for review Rev B");
        add(6, 200, 0, "automatic");
        add(7, 201, 0, "automatic");
        st.prune_versions(&id);
        let left: HashSet<String> = st.versions(&id).into_iter().map(|v| v.id).collect();
        let want: HashSet<String> = ["v1", "v2", "v3", "v5", "v6"].iter().map(|s| s.to_string()).collect();
        assert_eq!(left, want);
        assert!(!vdir.join("v4.xlsx").exists());
        let _ = fs::remove_dir_all(&d);
    }
}
