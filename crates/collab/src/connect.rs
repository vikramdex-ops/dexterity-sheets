//! Dexterity Connect: what the office server tells the office chat (Dexterity Sheets).
//!
//! When an administrator sets a Connect hub and an integration token in the Server Manager
//! (`connect.json` in the data folder), each mention and assignment notice is also queued here
//! as a Direct message for the person (`POST /api/bot/message` on the hub), and the server keeps
//! a "Sheets pulse" card up to date (`POST /api/bot/widget`). The queue lives in
//! `connect-outbox.json`, so nothing is lost when the hub is down or the server restarts: the
//! server program posts it in the background and tries again later, never holding anything up.
//! The token is never logged and never sent anywhere but the hub.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Result, server};

/// Most messages waiting at once (the oldest go first when it is full).
pub const OUTBOX_MAX: usize = 1000;
/// A message not delivered after this long is dropped.
pub const GIVE_UP_MS: i64 = 3 * 24 * 60 * 60 * 1000;
/// The longest wait between two tries of one message.
const BACKOFF_MAX_MS: i64 = 60 * 60 * 1000;

/// `connect.json`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ConnectConfig {
    /// `https://<hub>:47900` (no path).
    pub hub: String,
    /// The integration token made in Connect's Control room.
    pub token: String,
    /// The hub certificate's SHA-256 fingerprint (hex, colons allowed), for a hub with its own
    /// certificate. Empty: the certificate must be trusted by Windows.
    pub fingerprint: String,
}

impl ConnectConfig {
    /// Set up enough to post.
    pub fn ready(&self) -> bool {
        !self.hub.trim().is_empty() && !self.token.trim().is_empty()
    }
}

pub fn config_path(dir: &Path) -> PathBuf {
    dir.join("connect.json")
}

/// The hub settings, if any.
pub fn load_config(dir: &Path) -> ConnectConfig {
    std::fs::read(config_path(dir)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Checks and saves the hub settings. An empty `token` keeps the one saved; an empty `hub`
/// turns posting off (and forgets the token).
pub fn save_config(dir: &Path, hub: &str, token: &str, fingerprint: &str) -> std::result::Result<(), String> {
    let hub = hub.trim().trim_end_matches('/');
    if hub.is_empty() {
        return server::write_atomic(&config_path(dir), b"{}\n").map_err(|e| e.to_string());
    }
    if !(hub.starts_with("https://") || hub.starts_with("http://")) || hub.contains(char::is_whitespace) || hub.len() > 300 {
        return Err("Type the hub's address as https://<hub>:47900".into());
    }
    let token = token.trim();
    if token.len() > 512 || token.contains(char::is_whitespace) {
        return Err("That does not look like a Connect integration token.".into());
    }
    let fp = normalize_fingerprint(fingerprint).ok_or("The fingerprint is 64 hexadecimal digits (colons and spaces are fine).")?;
    let old = load_config(dir);
    let c = ConnectConfig { hub: hub.to_string(), token: if token.is_empty() { old.token } else { token.to_string() }, fingerprint: fp };
    if c.token.is_empty() {
        return Err("Paste the integration token from Connect's Control room › Integrations.".into());
    }
    let bytes = serde_json::to_vec_pretty(&c).map_err(|e| e.to_string())?;
    server::write_atomic(&config_path(dir), &bytes).map_err(|e| e.to_string())
}

/// Lower-case hex without separators; empty stays empty; `None` when it is not 32 bytes of hex.
pub fn normalize_fingerprint(s: &str) -> Option<String> {
    let hex: String = s.chars().filter(|c| !matches!(c, ':' | ' ' | '-')).collect::<String>().to_ascii_lowercase();
    if hex.is_empty() {
        return Some(String::new());
    }
    (hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit())).then_some(hex)
}

/// A message waiting for the hub.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Outgoing {
    pub id: String,
    /// Windows accounts (`DOMAIN\user`).
    pub to: Vec<String>,
    pub text: String,
    /// The card under the text.
    pub title: String,
    pub lines: Vec<String>,
    pub created: i64,
    pub tries: u32,
    /// Not before this (ms since 1970).
    pub next: i64,
}

/// The queue, kept in `connect-outbox.json`.
#[derive(Debug, Default)]
pub struct Outbox {
    path: PathBuf,
    pub items: Vec<Outgoing>,
}

impl Outbox {
    pub fn load(dir: &Path) -> Outbox {
        let path = dir.join("connect-outbox.json");
        let items = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Outbox { path, items }
    }

    fn save(&self) -> Result<()> {
        server::write_atomic(&self.path, &serde_json::to_vec(&self.items)?)
    }

    pub fn push(&mut self, m: Outgoing) {
        if self.items.len() >= OUTBOX_MAX {
            self.items.remove(0);
        }
        self.items.push(m);
        if let Err(e) = self.save() {
            log::warn!("Connect outbox: {e}");
        }
    }

    /// Messages due now, dropping those too old to matter.
    pub fn due(&mut self, now: i64) -> Vec<Outgoing> {
        let before = self.items.len();
        self.items.retain(|m| now - m.created < GIVE_UP_MS);
        if self.items.len() != before {
            log::warn!("Connect: {} notice(s) could not be delivered for three days and were dropped", before - self.items.len());
            let _ = self.save();
        }
        self.items.iter().filter(|m| m.next <= now).take(50).cloned().collect()
    }

    /// Delivered, or refused for good: off the queue.
    pub fn done(&mut self, id: &str) {
        self.items.retain(|m| m.id != id);
        if let Err(e) = self.save() {
            log::warn!("Connect outbox: {e}");
        }
    }

    /// Not delivered this time: tried again later, waiting longer each time (up to an hour).
    pub fn retry(&mut self, id: &str, now: i64) {
        if let Some(m) = self.items.iter_mut().find(|m| m.id == id) {
            m.tries = m.tries.saturating_add(1);
            let wait = 30_000i64.saturating_mul(1i64 << m.tries.min(10)).min(BACKOFF_MAX_MS);
            m.next = now + wait;
        }
        if let Err(e) = self.save() {
            log::warn!("Connect outbox: {e}");
        }
    }
}

/// The body of `POST /api/bot/message`.
pub fn message_body(m: &Outgoing) -> serde_json::Value {
    serde_json::json!({"text": m.text, "to": m.to, "card": {"title": m.title, "lines": m.lines}})
}

/// The body of `POST /api/bot/widget`: the "Sheets pulse" card.
pub fn widget_body(lines: &[String]) -> serde_json::Value {
    let lines: Vec<String> = lines.iter().take(8).map(|l| l.chars().take(200).collect()).collect();
    serde_json::json!({"key": "sheets-pulse", "title": "Sheets pulse", "lines": lines})
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn settings_are_checked_and_the_token_is_kept_when_left_empty() {
        let dir = std::env::temp_dir().join(format!("dexsheets-connect-{}-{}", std::process::id(), crate::now_ms()));
        std::fs::create_dir_all(&dir).expect("dir");
        assert!(save_config(&dir, "hub:47900", "t", "").is_err(), "needs https://");
        assert!(save_config(&dir, "https://hub:47900", "", "").is_err(), "a first token is required");
        save_config(&dir, "https://hub:47900/", "tok_123", "AA:bb").expect_err("short fingerprint");
        save_config(&dir, "https://hub:47900/", "tok_123", &"ab:".repeat(32)).expect("saved");
        let c = load_config(&dir);
        assert_eq!((c.hub.as_str(), c.token.as_str(), c.fingerprint.len()), ("https://hub:47900", "tok_123", 64));
        save_config(&dir, "https://hub2:47900", "", "").expect("keeps token");
        assert_eq!(load_config(&dir).token, "tok_123");
        save_config(&dir, "", "", "").expect("off");
        assert!(!load_config(&dir).ready());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_outbox_survives_a_restart_and_backs_off() {
        let dir = std::env::temp_dir().join(format!("dexsheets-outbox-{}-{}", std::process::id(), crate::now_ms()));
        std::fs::create_dir_all(&dir).expect("dir");
        let mut o = Outbox::load(&dir);
        o.push(Outgoing { id: "a".into(), to: vec![r"DDS\anita".into()], text: "hi".into(), created: 1000, ..Outgoing::default() });
        o.push(Outgoing { id: "b".into(), created: 1000, ..Outgoing::default() });
        assert_eq!(o.due(2000).len(), 2);
        o.retry("a", 2000);
        let mut o = Outbox::load(&dir);
        assert_eq!(o.due(2000).iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), vec!["b"], "a waits after a failure");
        assert_eq!(o.due(2000 + 60_000).len(), 2);
        o.done("b");
        assert_eq!(Outbox::load(&dir).items.len(), 1);
        assert!(o.due(1000 + GIVE_UP_MS).is_empty(), "given up after three days");
        let body = message_body(&Outgoing { to: vec!["x".into()], text: "t".into(), title: "T".into(), ..Outgoing::default() });
        assert_eq!(body["to"][0], "x");
        assert_eq!(widget_body(&vec!["l".repeat(300); 10])["lines"].as_array().map(Vec::len), Some(8));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
