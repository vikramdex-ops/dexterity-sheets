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

use gridcraft_engine::core::CellRef;
use gridcraft_engine::model::Workbook;
use serde::{Deserialize, Serialize};

use crate::patch::{self, CellData, WbPatch};
use crate::proto::{
    self, BookInfo, Change, ClientMsg, Conflict, FileSource, LastEdit, Logged, Peer, Role, ServerMsg, UpdateInfo, UserInfo, VersionInfo,
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
        let mut state = State { dir, name, users, metas, books: HashMap::new(), conns: HashMap::new(), update: None, hashes: HashMap::new() };
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
            st.conns.insert(conn, Conn { user: user.clone(), instance, tx: tx.clone(), open: Vec::new(), peer: HashMap::new() });
            send(&tx, &ServerMsg::Welcome { me, conn, users });
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
    fn broadcast_books(&self) {
        let books = self.metas.values().map(|m| self.info(m)).collect();
        let Some(frame) = encode(&ServerMsg::Books { books }) else { return };
        for c in self.conns.values() {
            let _ = c.tx.send(frame.clone());
        }
    }

    /// The newest installer in `updates/`. Files are named with their version as a `-`-separated
    /// part (`DexteritySheets-0.13.0-x64.msi`, `dexterity-sheets-0.13.0-windows-x64.msi`); notes
    /// sit next to them as `<same name>.md` or `.txt`.
    fn scan_updates(&mut self) -> Option<(UpdateInfo, PathBuf)> {
        let dir = self.dir.join("updates");
        let mut best: Option<(String, PathBuf, u64, std::time::SystemTime)> = None;
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
            if best.as_ref().is_none_or(|(v, ..)| proto::version_newer(&version, v)) {
                best = Some((version, path, meta.len(), modified));
            }
        }
        let (version, path, size, modified) = best?;
        let key = (path.clone(), size, modified);
        let sha256 = match self.hashes.get(&key) {
            Some(h) => h.clone(),
            None => {
                use sha2::Digest;
                let h = auth::hex(&sha2::Sha256::digest(fs::read(&path).ok()?));
                self.hashes.insert(key, h.clone());
                h
            }
        };
        let notes = ["md", "txt"].iter().find_map(|x| fs::read_to_string(path.with_extension(x)).ok()).unwrap_or_default();
        let file = path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
        Some((UpdateInfo { version, notes, file, size, sha256 }, path))
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
            ClientMsg::CheckUpdate { version } => {
                let update = self.update.as_ref().filter(|(u, _)| proto::version_newer(&u.version, &version)).map(|(u, _)| u.clone());
                self.reply(conn, &ServerMsg::Update { update });
            }
            ClientMsg::GetUpdate { version } => {
                let Some((info, path)) = self.update.clone().filter(|(u, _)| u.version == version) else {
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
            ClientMsg::ListBooks => {
                let books = self.metas.values().map(|m| self.info(m)).collect();
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
                let msg = ServerMsg::Opened {
                    book: self.info(&b.meta),
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
                let versions = self.versions(&book);
                self.reply(conn, &ServerMsg::Versions { book, versions });
            }
            ClientMsg::SaveVersion { book, label } => {
                self.load_book(&book)?;
                let v = self.save_version(&book, &user, &label)?;
                log::info!("{user} saved a version of {book} at edit {}", v.seq);
                let versions = self.versions(&book);
                self.reply(conn, &ServerMsg::Versions { book, versions });
            }
            ClientMsg::GetVersion { book, version } => {
                let bytes = fs::read(self.version_path(&book, &version)?)?;
                self.reply(conn, &ServerMsg::Version { book, version, xlsx: proto::b64(&bytes) });
            }
            ClientMsg::RestoreVersion { book, version } => {
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
        Ok(())
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
        lock(&self.state).unload_idle();
    }

    /// Looks for a new installer in `updates/` and tells everyone connected about it.
    pub fn poll_updates(&self) {
        let mut st = lock(&self.state);
        let found = st.scan_updates();
        let changed = found.as_ref().map(|(u, _)| &u.version) != st.update.as_ref().map(|(u, _)| &u.version);
        st.update = found;
        if changed && let Some((u, _)) = &st.update {
            log::info!("update {} is available to clients", u.version);
            if let Some(frame) = encode(&ServerMsg::Update { update: Some(u.clone()) }) {
                for c in st.conns.values() {
                    let _ = c.tx.send(frame.clone());
                }
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
