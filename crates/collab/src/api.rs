//! The office server's read API (Dexterity Sheets): plain HTTP and JSON on TCP 47812, for
//! Dexterity Connect (Nib answers "what's assigned to me in Sheets?" from it).
//!
//! Every request carries `Authorization: Bearer <token>`. Tokens are made and revoked by an
//! administrator in the Server Manager; the token is shown once and only its SHA-256 is kept
//! (`api-tokens.json`). Read only: nothing here changes a workbook.
//!
//! | Request | Answer |
//! | --- | --- |
//! | `GET /api/v1/health` | `{name, version}` |
//! | `GET /api/v1/people/{user}/assigned` | `[{book, sheet, cell, title, status, due, link}]` |
//! | `GET /api/v1/people/{user}/mentions?since=<ISO 8601 or ms>` | `[{book, sheet, cell, from, text, at, link}]` |
//! | `GET /api/v1/books` | `[{id, name, sharer, online: [user], link}]` |
//!
//! `{user}` is `DOMAIN\user`, URL-encoded (`dex%5Cpriya`), matched without regard to case. `401`
//! for a missing or refused token, `404` for any other path. The request is read with hard
//! limits (16 KB of headers, no body, 5 seconds), one thread per connection, at most
//! [`MAX_CONNECTIONS`] at once.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::server::Server;
use crate::{Result, auth, identity, server};

pub const DEFAULT_PORT: u16 = 47812;
/// The request line and headers together.
pub const MAX_HEAD: usize = 16 * 1024;
pub const MAX_CONNECTIONS: usize = 32;
/// The whole request must arrive within this.
const READ_TIMEOUT: Duration = Duration::from_secs(5);
/// Most rows one answer holds.
const ROWS_MAX: usize = 500;

/// A read-API token as kept on the server: never the token itself.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TokenInfo {
    pub id: String,
    /// What it is for ("Dexterity Connect").
    pub name: String,
    /// SHA-256 of the token, hex.
    pub hash: String,
    pub created: i64,
    /// When it was last used (ms since 1970), kept roughly.
    pub last_used: i64,
}

pub fn tokens_path(dir: &Path) -> PathBuf {
    dir.join("api-tokens.json")
}

pub fn load_tokens(dir: &Path) -> Vec<TokenInfo> {
    std::fs::read(tokens_path(dir)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save_tokens(dir: &Path, t: &[TokenInfo]) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    server::write_atomic(&tokens_path(dir), &serde_json::to_vec_pretty(t)?)
}

/// Makes a token named `name`; returns it (shown once, never stored) and its id.
pub fn create_token(dir: &Path, name: &str) -> Result<(String, String)> {
    let token = format!("dsk_{}", auth::hex(&auth::random_bytes::<32>()));
    let id = auth::hex(&auth::random_bytes::<6>());
    let mut all = load_tokens(dir);
    let name = server::clean_display(name).unwrap_or_else(|| "Dexterity Connect".into());
    all.push(TokenInfo { id: id.clone(), name, hash: crate::proto::sha256_hex(token.as_bytes()), created: crate::now_ms(), last_used: 0 });
    save_tokens(dir, &all)?;
    Ok((token, id))
}

/// Revokes the token with `id`: requests with it are refused from now on.
pub fn revoke_token(dir: &Path, id: &str) -> Result<()> {
    let mut all = load_tokens(dir);
    let before = all.len();
    all.retain(|t| t.id != id);
    if all.len() == before {
        return Err(crate::Error::Denied("no such token".into()));
    }
    save_tokens(dir, &all)
}

/// The token's entry when `token` is one of the server's.
fn check_token(dir: &Path, token: &str) -> Option<String> {
    if token.is_empty() || token.len() > 512 {
        return None;
    }
    let hash = crate::proto::sha256_hex(token.as_bytes());
    let all = load_tokens(dir);
    // Compared in full whatever matches first, so timing says nothing about the hash.
    let mut found = None;
    for t in &all {
        let same = t.hash.len() == hash.len() && t.hash.bytes().zip(hash.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0;
        if same {
            found = Some(t.id.clone());
        }
    }
    found
}

/// Notes when a token was last used, at most once a minute per token.
fn mark_used(dir: &Path, id: &str) {
    let mut all = load_tokens(dir);
    let now = crate::now_ms();
    if let Some(t) = all.iter_mut().find(|t| t.id == id && now - t.last_used > 60_000) {
        t.last_used = now;
        let _ = save_tokens(dir, &all);
    }
}

/// A parsed request: the method, the path (still encoded) and the query.
#[derive(Debug, PartialEq)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub query: String,
    pub bearer: Option<String>,
}

/// Parses the request line and headers. `Err(status)` for what is refused before routing.
pub fn parse_head(head: &[u8]) -> std::result::Result<Request, u16> {
    let text = std::str::from_utf8(head).map_err(|_| 400u16)?;
    let mut lines = text.split("\r\n");
    let first = lines.next().ok_or(400u16)?;
    let mut parts = first.split(' ');
    let (method, target, version) = (parts.next().ok_or(400u16)?, parts.next().ok_or(400u16)?, parts.next().ok_or(400u16)?);
    if parts.next().is_some() || !version.starts_with("HTTP/1.") || !target.starts_with('/') {
        return Err(400);
    }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let mut bearer = None;
    for l in lines.take(100) {
        if l.is_empty() {
            break;
        }
        let Some((k, v)) = l.split_once(':') else { return Err(400) };
        if k.trim().eq_ignore_ascii_case("authorization") {
            let v = v.trim();
            if let (Some(scheme), Some(rest)) = (v.get(..7), v.get(7..))
                && scheme.eq_ignore_ascii_case("bearer ")
            {
                bearer = Some(rest.trim().to_string());
            }
        }
    }
    Ok(Request { method: method.to_string(), path: path.to_string(), query: query.to_string(), bearer })
}

/// `%XX` decoded (and `+` left as it is); `None` when it is not valid UTF-8 or a `%` is broken.
pub fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        if c == b'%' {
            let hex = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(c);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// A `since` value: ISO 8601 (`2026-11-12T10:00:00Z`, a date alone) or ms since 1970.
pub fn parse_since(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return Some(0);
    }
    if s.chars().all(|c| c.is_ascii_digit()) {
        return s.parse().ok();
    }
    if let Ok(t) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(t.timestamp_millis());
    }
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok().and_then(|d| d.and_hms_opt(0, 0, 0)).map(|t| t.and_utc().timestamp_millis())
}

/// The value of `key` in a query string, decoded.
fn query_value(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        (k == key).then(|| percent_decode(&v.replace('+', " "))).flatten()
    })
}

/// Answers one request (status, JSON body) without touching the network: the routing and the
/// token check, for the listener and for tests.
pub fn answer(server: &Server, req: &Request) -> (u16, Value) {
    let dir = server.data_dir();
    if req.method != "GET" {
        return (405, json!({"error": "only GET"}));
    }
    let Some(token_id) = req.bearer.as_deref().and_then(|t| check_token(&dir, t)) else {
        return (401, json!({"error": "a valid token is needed (Server Manager › Dexterity Connect)"}));
    };
    mark_used(&dir, &token_id);
    let segs: Vec<&str> = req.path.trim_end_matches('/').split('/').collect();
    match segs.as_slice() {
        ["", "api", "v1", "health"] => (200, json!({"name": server.name(), "version": crate::APP_VERSION.trim()})),
        ["", "api", "v1", "books"] => (200, Value::Array(server.api_books().into_iter().take(ROWS_MAX).collect())),
        ["", "api", "v1", "people", who, what @ ("assigned" | "mentions")] => {
            let Some(user) = percent_decode(who).map(|u| identity::account_key(u.trim())).filter(|u| !u.is_empty() && u.len() <= 300) else {
                return (404, json!({"error": "not found"}));
            };
            if *what == "assigned" {
                (200, Value::Array(server.api_assigned(&user).into_iter().take(ROWS_MAX).collect()))
            } else {
                let Some(since) = parse_since(&query_value(&req.query, "since").unwrap_or_default()) else {
                    return (400, json!({"error": "since is an ISO 8601 time or milliseconds since 1970"}));
                };
                let mut rows = server.api_mentions(&user, since);
                // The newest when there are too many.
                if rows.len() > ROWS_MAX {
                    rows.drain(..rows.len() - ROWS_MAX);
                }
                (200, Value::Array(rows))
            }
        }
        _ => (404, json!({"error": "not found"})),
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        431 => "Request Header Fields Too Large",
        503 => "Service Unavailable",
        _ => "Error",
    }
}

fn respond(s: &mut TcpStream, status: u16, body: &Value) {
    let body = serde_json::to_vec(body).unwrap_or_default();
    let mut head = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n",
        reason(status),
        body.len()
    );
    if status == 401 {
        head.push_str("WWW-Authenticate: Bearer\r\n");
    }
    head.push_str("\r\n");
    let _ = s.write_all(head.as_bytes());
    let _ = s.write_all(&body);
    let _ = s.flush();
}

/// Reads the head (up to the blank line). `Err(status)`: too large, too slow or cut short.
fn read_head(s: &mut TcpStream) -> std::result::Result<Vec<u8>, u16> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 2048];
    let started = crate::now_ms();
    loop {
        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            buf.truncate(end);
            return Ok(buf);
        }
        if buf.len() > MAX_HEAD {
            return Err(431);
        }
        if crate::now_ms() - started > READ_TIMEOUT.as_millis() as i64 {
            return Err(408);
        }
        match s.read(&mut chunk) {
            Ok(0) => return Err(400),
            Ok(n) => buf.extend_from_slice(chunk.get(..n).unwrap_or_default()),
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => return Err(408),
            Err(_) => return Err(400),
        }
    }
}

fn handle(server: &Server, mut s: TcpStream) {
    let _ = s.set_read_timeout(Some(READ_TIMEOUT));
    let _ = s.set_write_timeout(Some(READ_TIMEOUT));
    let (status, body) = match read_head(&mut s).and_then(|h| parse_head(&h)) {
        Ok(req) => answer(server, &req),
        Err(400) => return,
        Err(status) => (status, json!({"error": reason(status)})),
    };
    respond(&mut s, status, &body);
    // Unread bytes (an oversized request) would make Windows reset the connection and lose the
    // answer: say we are done, then take what is still coming, briefly and up to a limit.
    let _ = s.shutdown(std::net::Shutdown::Write);
    let _ = s.set_read_timeout(Some(Duration::from_millis(500)));
    let _ = std::io::copy(&mut (&s).take(256 * 1024), &mut std::io::sink());
    let _ = s.shutdown(std::net::Shutdown::Both);
}

/// Listens on `port` (all interfaces; 0 picks one) and answers in the background. Returns the port.
pub fn start(server: Arc<Server>, port: u16) -> Result<u16> {
    let listener = TcpListener::bind(("0.0.0.0", port))?;
    let port = listener.local_addr()?.port();
    let busy = Arc::new(AtomicUsize::new(0));
    std::thread::Builder::new().name("dexsheets-api".into()).spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            if busy.load(Ordering::SeqCst) >= MAX_CONNECTIONS {
                respond(&mut s, 503, &json!({"error": "busy"}));
                continue;
            }
            busy.fetch_add(1, Ordering::SeqCst);
            let (srv, b) = (server.clone(), busy.clone());
            let spawned = std::thread::Builder::new().name("dexsheets-api-conn".into()).spawn(move || {
                handle(&srv, s);
                b.fetch_sub(1, Ordering::SeqCst);
            });
            if spawned.is_err() {
                busy.fetch_sub(1, Ordering::SeqCst);
            }
        }
    })?;
    Ok(port)
}
