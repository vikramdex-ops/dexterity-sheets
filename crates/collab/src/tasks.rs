//! Assignments and the mention log (Dexterity Sheets).
//!
//! - `books/<id>/assignments.json`: the cells and ranges of a shared workbook assigned to people,
//!   each with an optional due date and its status (`open` or `done`).
//! - `mentions/<person>.jsonl`: every time a person was mentioned (in a comment, a note or a
//!   cell), kept so the read API can answer "who mentioned me this week" after the in-app
//!   notification is long gone. The newest [`MENTIONS_KEPT`] per person are kept.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::proto::Assignment;
use crate::{Result, server};

/// Most assignments one workbook holds.
pub const ASSIGNMENTS_MAX: usize = 5000;
/// Mentions kept per person (older ones are dropped when the log grows past twice this).
pub const MENTIONS_KEPT: usize = 500;
/// Longest assignment title.
pub const TITLE_MAX: usize = 300;

pub fn assignments_path(book_dir: &Path) -> PathBuf {
    book_dir.join("assignments.json")
}

/// A workbook's assignments (none when the file is missing or unreadable).
pub fn load_assignments(book_dir: &Path) -> Vec<Assignment> {
    fs::read(assignments_path(book_dir)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save_assignments(book_dir: &Path, items: &[Assignment]) -> Result<()> {
    server::write_atomic(&assignments_path(book_dir), &serde_json::to_vec_pretty(items)?)
}

/// `YYYY-MM-DD` when `s` is such a date (empty stays empty); `None` for anything else.
pub fn clean_due(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() {
        return Some(String::new());
    }
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok().map(|d| d.format("%Y-%m-%d").to_string())
}

/// `open` or `done` (a few common words for done are accepted); `None` for anything else.
pub fn clean_status(s: &str) -> Option<&'static str> {
    match s.trim().to_ascii_lowercase().as_str() {
        "" | "open" | "todo" | "to do" => Some("open"),
        "done" | "closed" | "complete" | "completed" | "resolved" | "finished" => Some("done"),
        _ => None,
    }
}

/// A cell or range in A1 form, upper case and without `$`: `B4`, `B4:D9`. `None` when it is not one.
pub fn clean_range(s: &str) -> Option<String> {
    let s = s.trim().replace('$', "").to_ascii_uppercase();
    if s.is_empty() || s.len() > 25 {
        return None;
    }
    let mut parts = s.split(':');
    let a = parts.next()?;
    let b = parts.next();
    if parts.next().is_some() {
        return None;
    }
    gridcraft_engine::core::CellRef::parse(a)?;
    if let Some(b) = b {
        gridcraft_engine::core::CellRef::parse(b)?;
    }
    Some(s)
}

/// One time a person was mentioned.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MentionRecord {
    /// The workbook's id and name.
    pub book: String,
    pub name: String,
    pub sheet: String,
    pub cell: String,
    /// Who wrote it: account (lower case) and the name others see.
    pub by: String,
    pub from: String,
    pub text: String,
    /// `comment`, `note` or `cell`.
    pub source: String,
    /// When (ms since 1970).
    pub at: i64,
}

fn mentions_path(dir: &Path, user: &str) -> PathBuf {
    dir.join("mentions").join(format!("{}.jsonl", server::user_file(user)))
}

/// Adds a mention to `user`'s log, dropping the oldest past [`MENTIONS_KEPT`].
pub fn log_mention(dir: &Path, user: &str, m: &MentionRecord) -> Result<()> {
    let path = mentions_path(dir, user);
    if let Some(d) = path.parent() {
        fs::create_dir_all(d)?;
    }
    let mut lines: Vec<String> = fs::read_to_string(&path).unwrap_or_default().lines().map(str::to_string).collect();
    lines.push(serde_json::to_string(m)?);
    if lines.len() > MENTIONS_KEPT * 2 {
        let cut = lines.len() - MENTIONS_KEPT;
        lines.drain(..cut);
    }
    let mut body = lines.join("\n");
    body.push('\n');
    server::write_atomic(&path, body.as_bytes())
}

/// `user`'s mentions after `since` (ms since 1970), oldest first.
pub fn mentions_of(dir: &Path, user: &str, since: i64) -> Vec<MentionRecord> {
    fs::read_to_string(mentions_path(dir, user))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str::<MentionRecord>(l).ok())
        .filter(|m| m.at > since)
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn dates_statuses_and_ranges_are_checked() {
        assert_eq!(clean_due(" 2026-12-01 ").as_deref(), Some("2026-12-01"));
        assert_eq!(clean_due("").as_deref(), Some(""));
        assert_eq!(clean_due("01/12/2026"), None);
        assert_eq!(clean_due("2026-02-30"), None);
        assert_eq!(clean_status("Done"), Some("done"));
        assert_eq!(clean_status(""), Some("open"));
        assert_eq!(clean_status("maybe"), None);
        assert_eq!(clean_range("$b$4").as_deref(), Some("B4"));
        assert_eq!(clean_range("b4:d9").as_deref(), Some("B4:D9"));
        assert_eq!(clean_range("B4:"), None);
        assert_eq!(clean_range("Sheet1!B4"), None);
        assert_eq!(clean_range("B4:C5:D6"), None);
    }

    #[test]
    fn the_mention_log_keeps_the_newest() {
        let dir = std::env::temp_dir().join(format!("dexsheets-mlog-{}-{}", std::process::id(), crate::now_ms()));
        // A log already at its limit (written in one go: a thousand synced writes are slow),
        // then a few more mentions the usual way.
        let full: String = (0..MENTIONS_KEPT * 2)
            .map(|i| serde_json::to_string(&MentionRecord { book: "b".into(), at: i as i64, ..MentionRecord::default() }).expect("json") + "\n")
            .collect();
        fs::create_dir_all(dir.join("mentions")).expect("dir");
        fs::write(mentions_path(&dir, r"dds\anita"), full).expect("log");
        for i in (MENTIONS_KEPT * 2)..(MENTIONS_KEPT * 2 + 5) {
            let m = MentionRecord { book: "b".into(), at: i as i64, ..MentionRecord::default() };
            log_mention(&dir, r"dds\anita", &m).expect("log");
        }
        let all = mentions_of(&dir, r"dds\anita", -1);
        assert!(all.len() <= MENTIONS_KEPT * 2 && all.len() >= MENTIONS_KEPT, "{}", all.len());
        assert_eq!(all.last().map(|m| m.at), Some((MENTIONS_KEPT * 2 + 4) as i64));
        assert_eq!(mentions_of(&dir, r"dds\anita", (MENTIONS_KEPT * 2 + 2) as i64).len(), 2);
        assert!(mentions_of(&dir, "nobody", 0).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }
}
