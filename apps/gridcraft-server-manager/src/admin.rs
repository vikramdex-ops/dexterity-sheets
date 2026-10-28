//! The admin dashboard: which app versions the office PCs run, the release status and update
//! policy the server hands out, problem reports, app health and feature adoption. Everything is
//! read from files the server keeps in its data folder (`docs/dexterity-admin-data.md`); only
//! `updates/releases.json` is written here. A missing file is the same as an empty one and a line
//! that does not parse is skipped, so a page shows what the apps really reported and nothing else.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::{Read, Seek};
use std::path::Path;

use egui::{Color32, RichText, Ui, vec2};
use serde_json::{Value, json};

use crate::{App, BAD, DIM, OK, WARN, card, primary, system};

/// Where problem reports go when the App updates page names no repository.
pub const DEFAULT_REPO: &str = "vikramdex-ops/dexterity-sheets";
/// A whole-JSON file larger than this is refused rather than read.
const MAX_JSON: u64 = 32 * 1024 * 1024;
/// Only the last part of a long `.jsonl` file is read (the first, cut line is skipped).
const MAX_LINES: u64 = 32 * 1024 * 1024;
/// The issue body in the GitHub address, after URL encoding, stays under this many characters.
pub const BODY_CAP: usize = 6000;
/// Longest text kept from one field of a report (logs are at most 64 KB by the contract).
const MAX_FIELD: usize = 70 * 1024;
const DAY_MS: i64 = 86_400_000;

// ---------------------------------------------------------------------------------------------
// Versions
// ---------------------------------------------------------------------------------------------

/// Sort key of a version, by the rule the server uses to pick the newest installer
/// (`proto::version_newer`): dotted numbers compared part by part (a missing part is 0), then a
/// release above its pre-release (`1.3.0` above `1.3.0-beta`).
fn version_key(v: &str) -> (Vec<u64>, bool) {
    let v = v.trim().trim_start_matches(['v', 'V']);
    let (core, pre) = match v.split_once('-') {
        Some((c, _)) => (c, true),
        None => (v, false),
    };
    let mut parts: Vec<u64> = core.split('.').map(|p| p.trim().parse().unwrap_or(0)).collect();
    while parts.last() == Some(&0) {
        parts.pop();
    }
    (parts, !pre)
}

/// Orders two versions oldest first; versions the server treats as equal fall back to their text,
/// so the order is total.
pub fn cmp_versions(a: &str, b: &str) -> Ordering {
    version_key(a).cmp(&version_key(b)).then_with(|| a.cmp(b))
}

fn older(a: &str, b: &str) -> bool {
    version_key(a) < version_key(b)
}

/// The version in an installer's file name, as the server reads it: the first `-`-separated part
/// that starts with a digit and holds a dot (`dexterity-sheets-1.2.0-windows-x64.msi` → `1.2.0`).
pub fn installer_version(name: &str) -> Option<String> {
    let stem = Path::new(name).file_stem()?.to_string_lossy().into_owned();
    stem.split('-').find(|p| p.starts_with(|c: char| c.is_ascii_digit()) && p.contains('.')).map(str::to_string)
}

// ---------------------------------------------------------------------------------------------
// Files
// ---------------------------------------------------------------------------------------------

/// A whole file, `None` when it does not exist.
fn read_whole(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match std::fs::metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", path.display())),
        Ok(m) if m.len() > MAX_JSON => {
            return Err(format!("{} is too large to read ({}).", path.display(), system::human_size(m.len())));
        }
        Ok(_) => {}
    }
    match std::fs::read(path) {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// A JSON file; missing is `Null`, and a file that does not parse is reported.
fn read_json(path: &Path) -> Result<Value, String> {
    match read_whole(path)? {
        None => Ok(Value::Null),
        Some(b) if b.iter().all(u8::is_ascii_whitespace) => Ok(Value::Null),
        Some(b) => serde_json::from_slice(&b).map_err(|e| format!("{} can't be read: {e}", path.display())),
    }
}

/// The lines of a `.jsonl` file (the last [`MAX_LINES`] bytes of a long one); missing is empty.
fn read_lines(path: &Path) -> Result<String, String> {
    let mut f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(String::new()),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let from = len.saturating_sub(MAX_LINES);
    f.seek(std::io::SeekFrom::Start(from)).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut buf = Vec::new();
    f.take(MAX_LINES).read_to_end(&mut buf).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// Parses each non-blank line with `parse`; returns what parsed and how many lines did not.
fn parse_lines<T>(text: &str, parse: impl Fn(&Value) -> Option<T>) -> (Vec<T>, usize) {
    let mut out = Vec::new();
    let mut skipped = 0;
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        match serde_json::from_str::<Value>(line).ok().as_ref().and_then(&parse) {
            Some(t) => out.push(t),
            None => skipped += 1,
        }
    }
    (out, skipped)
}

/// Cuts text to at most `max` bytes, on a character boundary.
fn capped(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.get(..end).unwrap_or("").to_string()
}

fn text(v: &Value, key: &str) -> String {
    capped(v.get(key).and_then(Value::as_str).unwrap_or("").trim(), MAX_FIELD)
}

fn millis(v: &Value, key: &str) -> i64 {
    v.get(key).and_then(|t| t.as_i64().or_else(|| t.as_f64().filter(|f| f.is_finite()).map(|f| f as i64))).unwrap_or(0)
}

// ---------------------------------------------------------------------------------------------
// updates/releases.json
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Channel {
    #[default]
    Stable,
    Testing,
}

impl Channel {
    pub const ALL: [Channel; 2] = [Channel::Stable, Channel::Testing];
    pub fn id(self) -> &'static str {
        match self {
            Channel::Stable => "stable",
            Channel::Testing => "testing",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Channel::Stable => "Stable",
            Channel::Testing => "Testing",
        }
    }
    fn parse(s: &str) -> Option<Channel> {
        Channel::ALL.into_iter().find(|c| c.id().eq_ignore_ascii_case(s.trim()))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Status {
    #[default]
    Ok,
    KnownIssue,
    Deprecated,
    Blocked,
}

impl Status {
    pub const ALL: [Status; 4] = [Status::Ok, Status::KnownIssue, Status::Deprecated, Status::Blocked];
    pub fn id(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::KnownIssue => "knownIssue",
            Status::Deprecated => "deprecated",
            Status::Blocked => "blocked",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Status::Ok => "OK",
            Status::KnownIssue => "Known issue",
            Status::Deprecated => "Deprecated",
            Status::Blocked => "Blocked",
        }
    }
    fn hint(self) -> &'static str {
        match self {
            Status::Ok => "Offered to the PCs on its channel.",
            Status::KnownIssue => "Still offered; people running it see the note.",
            Status::Deprecated => "No longer offered; people running it are told to move on.",
            Status::Blocked => "Never offered; PCs running it are told to roll back to the newest OK version older than it.",
        }
    }
    fn parse(s: &str) -> Option<Status> {
        Status::ALL.into_iter().find(|x| x.id().eq_ignore_ascii_case(s.trim()))
    }
    /// Whether the server offers an installer with this status.
    fn offered(self) -> bool {
        matches!(self, Status::Ok | Status::KnownIssue)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Mode {
    #[default]
    Prompt,
    Auto,
    Manual,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Prompt, Mode::Auto, Mode::Manual];
    pub fn id(self) -> &'static str {
        match self {
            Mode::Prompt => "prompt",
            Mode::Auto => "auto",
            Mode::Manual => "manual",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Mode::Prompt => "Ask first: the app offers the update and the person installs it",
            Mode::Auto => "Install on its own: when no workbook has unsaved changes",
            Mode::Manual => "Never offer: people use Help › Check for Updates",
        }
    }
    fn parse(s: &str) -> Option<Mode> {
        Mode::ALL.into_iter().find(|m| m.id().eq_ignore_ascii_case(s.trim()))
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Release {
    pub channel: Channel,
    pub status: Status,
    pub note: String,
}

/// `updates/releases.json`: the update policy and each version's channel, status and note.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Releases {
    pub mode: Mode,
    pub default_channel: Channel,
    pub releases: BTreeMap<String, Release>,
}

impl Releases {
    /// A version's entry; an installer with none is stable and OK.
    pub fn get(&self, version: &str) -> Release {
        self.releases.get(version).cloned().unwrap_or_default()
    }

    /// The same settings without entries that say only what no entry says.
    fn normalized(&self) -> Releases {
        let mut r = self.clone();
        r.releases.retain(|v, rel| !v.trim().is_empty() && *rel != Release::default());
        for rel in r.releases.values_mut() {
            rel.note = rel.note.trim().to_string();
        }
        r
    }

    fn same_as(&self, other: &Releases) -> bool {
        self.normalized() == other.normalized()
    }
}

pub fn parse_releases(v: &Value) -> Releases {
    let policy = v.get("policy");
    let field = |k: &str| policy.and_then(|p| p.get(k)).and_then(Value::as_str).unwrap_or("");
    let mut out = Releases {
        mode: Mode::parse(field("mode")).unwrap_or_default(),
        default_channel: Channel::parse(field("defaultChannel")).unwrap_or_default(),
        releases: BTreeMap::new(),
    };
    if let Some(map) = v.get("releases").and_then(Value::as_object) {
        for (version, r) in map {
            let s = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or("");
            out.releases.insert(
                version.trim().to_string(),
                Release {
                    channel: Channel::parse(s("channel")).unwrap_or_default(),
                    status: Status::parse(s("status")).unwrap_or_default(),
                    note: capped(s("note"), 4000),
                },
            );
        }
    }
    out
}

pub fn releases_json(r: &Releases) -> Value {
    let r = r.normalized();
    let releases: serde_json::Map<String, Value> =
        r.releases.iter().map(|(v, rel)| (v.clone(), json!({"channel": rel.channel.id(), "status": rel.status.id(), "note": rel.note}))).collect();
    json!({"policy": {"mode": r.mode.id(), "defaultChannel": r.default_channel.id()}, "releases": releases})
}

fn releases_path(dir: &Path) -> std::path::PathBuf {
    dir.join("updates").join("releases.json")
}

pub fn load_releases(dir: &Path) -> Result<Releases, String> {
    read_json(&releases_path(dir)).map(|v| parse_releases(&v))
}

/// Writes `updates/releases.json` (a temporary file, then renamed over the old one).
pub fn save_releases(dir: &Path, r: &Releases) -> Result<(), String> {
    std::fs::create_dir_all(dir.join("updates")).map_err(|e| e.to_string())?;
    let bytes = serde_json::to_vec_pretty(&releases_json(r)).map_err(|e| e.to_string())?;
    system::write_atomic(&releases_path(dir), &bytes)
}

/// The version a PC on `channel` is offered: the newest installer on the server that is OK or
/// has a known issue, on the stable channel (or, for a testing PC, on either).
pub fn offered(versions: &[String], r: &Releases, channel: Channel) -> Option<String> {
    versions
        .iter()
        .filter(|v| {
            let rel = r.get(v);
            rel.status.offered() && (rel.channel == Channel::Stable || channel == Channel::Testing)
        })
        .max_by(|a, b| cmp_versions(a, b))
        .cloned()
}

/// Where PCs running a blocked version are told to go: the newest installer on the server that
/// is OK and older than it.
pub fn rollback_for(blocked: &str, versions: &[String], r: &Releases) -> Option<String> {
    versions.iter().filter(|v| older(v, blocked) && r.get(v).status == Status::Ok).max_by(|a, b| cmp_versions(a, b)).cloned()
}

// ---------------------------------------------------------------------------------------------
// clients.json
// ---------------------------------------------------------------------------------------------

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Client {
    pub user: String,
    pub machine: String,
    pub version: String,
    pub channel: String,
    pub last_seen: i64,
}

/// Every PC that reported, newest version first, then the one seen most recently.
pub fn parse_clients(v: &Value) -> Vec<Client> {
    let mut out: Vec<Client> = v
        .get("clients")
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter(|(_, c)| c.is_object())
                .map(|(key, c)| {
                    let (ku, km) = key.split_once('|').unwrap_or((key.as_str(), ""));
                    let or = |s: String, d: &str| if s.is_empty() { d.trim().to_string() } else { s };
                    Client {
                        user: or(text(c, "user"), ku),
                        machine: or(text(c, "machine"), km),
                        version: or(text(c, "version"), "unknown"),
                        channel: text(c, "channel"),
                        last_seen: millis(c, "lastSeen"),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by(|a, b| cmp_versions(&b.version, &a.version).then(b.last_seen.cmp(&a.last_seen)).then_with(|| a.user.cmp(&b.user)));
    out
}

pub fn load_clients(dir: &Path) -> Result<Vec<Client>, String> {
    read_json(&dir.join("clients.json")).map(|v| parse_clients(&v))
}

/// Per version, newest first: (version, people, PCs).
pub fn version_counts(clients: &[Client]) -> Vec<(String, usize, usize)> {
    let mut by: BTreeMap<&str, (BTreeSet<&str>, BTreeSet<&str>)> = BTreeMap::new();
    for c in clients {
        let e = by.entry(c.version.as_str()).or_default();
        e.0.insert(c.user.as_str());
        e.1.insert(c.machine.as_str());
    }
    let mut out: Vec<(String, usize, usize)> = by.into_iter().map(|(v, (u, m))| (v.to_string(), u.len(), m.len())).collect();
    out.sort_by(|a, b| cmp_versions(&b.0, &a.0));
    out
}

// ---------------------------------------------------------------------------------------------
// feedback/feedback.jsonl
// ---------------------------------------------------------------------------------------------

/// The features a report can name, with the words shown for them.
pub const FEATURES: [(&str, &str); 10] = [
    ("opening", "Opening files"),
    ("saving", "Saving"),
    ("formulas", "Formulas"),
    ("formatting", "Formatting"),
    ("importExport", "Import and export"),
    ("sharing", "Sharing"),
    ("notifications", "Notifications"),
    ("startup", "Starting the app"),
    ("performance", "Speed"),
    ("other", "Other"),
];

/// A feature id from the fixed list; anything else counts as `other`.
fn feature_id(s: &str) -> &'static str {
    FEATURES.iter().find(|(id, _)| id.eq_ignore_ascii_case(s.trim())).map_or("other", |(id, _)| *id)
}

pub fn feature_label(id: &str) -> &'static str {
    FEATURES.iter().find(|(f, _)| *f == id).map_or("Other", |(_, l)| *l)
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
pub enum Severity {
    #[default]
    Low,
    Medium,
    High,
    Blocking,
}

impl Severity {
    const ALL: [Severity; 4] = [Severity::Blocking, Severity::High, Severity::Medium, Severity::Low];
    pub fn id(self) -> &'static str {
        match self {
            Severity::Low => "low",
            Severity::Medium => "medium",
            Severity::High => "high",
            Severity::Blocking => "blocking",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Severity::Low => "Low",
            Severity::Medium => "Medium",
            Severity::High => "High",
            Severity::Blocking => "Blocks work",
        }
    }
    fn color(self) -> Color32 {
        match self {
            Severity::Blocking | Severity::High => BAD,
            Severity::Medium => WARN,
            Severity::Low => DIM,
        }
    }
    fn parse(s: &str) -> Severity {
        Severity::ALL.into_iter().find(|x| x.id().eq_ignore_ascii_case(s.trim())).unwrap_or_default()
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Report {
    pub id: String,
    pub time: i64,
    pub user: String,
    pub machine: String,
    pub version: String,
    pub os: String,
    pub features: Vec<&'static str>,
    pub description: String,
    pub expected: String,
    pub actual: String,
    pub severity: Severity,
    pub after_update: bool,
    pub rolled_back: bool,
    pub logs: Option<String>,
}

pub fn parse_report(v: &Value) -> Option<Report> {
    if !v.is_object() {
        return None;
    }
    let mut features: Vec<&'static str> = Vec::new();
    for f in v.get("features").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
        let id = feature_id(f);
        if !features.contains(&id) {
            features.push(id);
        }
    }
    if features.is_empty() {
        features.push("other");
    }
    let version = text(v, "version");
    Some(Report {
        id: text(v, "id"),
        time: millis(v, "time"),
        user: text(v, "user"),
        machine: text(v, "machine"),
        version: if version.is_empty() { "unknown".into() } else { version },
        os: text(v, "os"),
        features,
        description: text(v, "description"),
        expected: text(v, "expected"),
        actual: text(v, "actual"),
        severity: Severity::parse(v.get("severity").and_then(Value::as_str).unwrap_or("")),
        after_update: v.get("afterUpdate").and_then(Value::as_bool).unwrap_or(false),
        rolled_back: v.get("rolledBack").and_then(Value::as_bool).unwrap_or(false),
        logs: v.get("logs").and_then(Value::as_str).filter(|l| !l.trim().is_empty()).map(|l| capped(l, MAX_FIELD)),
    })
}

/// The reports (oldest first, as in the file) and how many lines were skipped.
pub fn load_feedback(dir: &Path) -> Result<(Vec<Report>, usize), String> {
    read_lines(&dir.join("feedback").join("feedback.jsonl")).map(|t| parse_lines(&t, parse_report))
}

/// Reports about one feature in one version.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Group {
    pub version: String,
    pub feature: &'static str,
    /// Indexes into the report list, newest report first.
    pub reports: Vec<usize>,
    pub highest: Severity,
    pub after_update: usize,
    pub rolled_back: usize,
}

/// Groups reports by version × feature (a report naming two features is in both groups): newest
/// version first, then the most severe, then the most reported.
pub fn group_feedback(reports: &[Report]) -> Vec<Group> {
    let mut by: BTreeMap<(String, &'static str), Vec<usize>> = BTreeMap::new();
    for (i, r) in reports.iter().enumerate() {
        for f in &r.features {
            by.entry((r.version.clone(), *f)).or_default().push(i);
        }
    }
    let mut out: Vec<Group> = by
        .into_iter()
        .map(|((version, feature), mut idx)| {
            idx.sort_by_key(|i| std::cmp::Reverse(reports.get(*i).map_or(0, |r| r.time)));
            let rs: Vec<&Report> = idx.iter().filter_map(|i| reports.get(*i)).collect();
            Group {
                version,
                feature,
                highest: rs.iter().map(|r| r.severity).max().unwrap_or_default(),
                after_update: rs.iter().filter(|r| r.after_update).count(),
                rolled_back: rs.iter().filter(|r| r.rolled_back).count(),
                reports: idx,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        cmp_versions(&b.version, &a.version)
            .then(b.highest.cmp(&a.highest))
            .then(b.reports.len().cmp(&a.reports.len()))
            .then_with(|| a.feature.cmp(b.feature))
    });
    out
}

/// Percent-encodes everything but letters, digits and `-_.~` (UTF-8 bytes as `%XX`).
pub fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// Encodes `text`, cut (on a character) so the encoded result, `suffix` included, fits in `cap`.
fn encode_capped(text: &str, cap: usize, suffix: &str) -> String {
    let full = url_encode(text);
    if full.len() <= cap {
        return full;
    }
    let tail = url_encode(suffix);
    let budget = cap.saturating_sub(tail.len());
    let mut out = String::new();
    let mut buf = [0u8; 4];
    for c in text.chars() {
        let e = url_encode(c.encode_utf8(&mut buf));
        if out.len() + e.len() > budget {
            break;
        }
        out.push_str(&e);
    }
    out.push_str(&tail);
    out
}

fn word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Replaces every whole-word occurrence of a name (ASCII case ignored) with `with`.
fn scrub_one(text: &str, name: &str, with: &str) -> String {
    let name = name.trim();
    if name.is_empty() {
        return text.to_string();
    }
    let lower = text.to_ascii_lowercase();
    let needle = name.to_ascii_lowercase();
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    let mut from = 0;
    while let Some(pos) = lower.get(from..).and_then(|s| s.find(&needle)) {
        let start = from + pos;
        let end = start + needle.len();
        let before = lower.get(..start).and_then(|s| s.chars().next_back());
        let after = lower.get(end..).and_then(|s| s.chars().next());
        if !before.is_some_and(word_char) && !after.is_some_and(word_char) {
            out.push_str(text.get(last..start).unwrap_or(""));
            out.push_str(with);
            last = end;
        }
        // Step past the first character of this match.
        from = start + lower.get(start..).and_then(|s| s.chars().next()).map_or(1, char::len_utf8);
    }
    out.push_str(text.get(last..).unwrap_or(""));
    out
}

/// Takes the names of people and PCs out of text written by someone in the office.
fn scrub(text: &str, people: &[String], pcs: &[String]) -> String {
    let mut all: Vec<(&str, &str)> = people.iter().map(|p| (p.as_str(), "[person]")).chain(pcs.iter().map(|m| (m.as_str(), "[PC]"))).collect();
    // Longer names first, so "Anita Rao" goes before "anita".
    all.sort_by_key(|(n, _)| std::cmp::Reverse(n.len()));
    all.dedup_by(|a, b| a.0.eq_ignore_ascii_case(b.0));
    all.into_iter().fold(text.to_string(), |t, (n, with)| scrub_one(&t, n, with))
}

/// Accepts `owner/name` (or its github.com address); anything else becomes [`DEFAULT_REPO`].
pub fn issue_repo(repo: &str) -> String {
    let r = repo.trim().trim_start_matches("https://github.com/").trim_end_matches('/');
    let ok = r.split('/').count() == 2
        && r.split('/').all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')));
    if ok { r.to_string() } else { DEFAULT_REPO.to_string() }
}

fn day(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).map(|d| d.with_timezone(&chrono::Local).format("%d %b %Y").to_string()).unwrap_or_default()
}

fn when(ms: i64) -> String {
    if ms <= 0 {
        return "unknown time".into();
    }
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|d| d.with_timezone(&chrono::Local).format("%d %b %Y %H:%M").to_string())
        .unwrap_or_else(|| "unknown time".into())
}

/// The address of a new GitHub issue holding a group's reports: version, feature, counts,
/// severities and what people wrote, with the names of people and PCs taken out (`names`: user
/// and display names known to the server, besides the reporters'). Logs stay on the server.
pub fn issue_url(repo: &str, group: &Group, reports: &[Report], names: &[String]) -> String {
    let rs: Vec<&Report> = group.reports.iter().filter_map(|i| reports.get(*i)).collect();
    let mut people: Vec<String> = reports.iter().map(|r| r.user.clone()).chain(names.iter().cloned()).collect();
    people.retain(|p| !p.trim().is_empty());
    let pcs: Vec<String> = reports.iter().map(|r| r.machine.clone()).filter(|m| !m.trim().is_empty()).collect();
    let clean = |t: &str| scrub(t, &people, &pcs);
    let n = rs.len();
    let feature = feature_label(group.feature);
    let title = format!("Feedback: {feature} in {} ({n} report{})", group.version, if n == 1 { "" } else { "s" });
    let severities: Vec<String> = Severity::ALL
        .into_iter()
        .filter_map(|s| {
            let c = rs.iter().filter(|r| r.severity == s).count();
            (c > 0).then(|| format!("{} {c}", s.id()))
        })
        .collect();
    let mut systems: Vec<String> = rs.iter().map(|r| clean(&r.os)).filter(|o| !o.is_empty()).collect();
    systems.sort();
    systems.dedup();
    let with_logs = rs.iter().filter(|r| r.logs.is_some()).count();
    let mut body = String::new();
    let _ = writeln!(
        body,
        "Problem reports sent from the office with Help › Report a Problem, gathered by the Server Manager. Names of people and PCs are left out.\n"
    );
    let _ = writeln!(body, "- **Version:** {}", group.version);
    let _ = writeln!(body, "- **Feature:** {feature} (`{}`)", group.feature);
    let _ = writeln!(body, "- **Reports:** {n}; {} after an update; {} rolled back", group.after_update, group.rolled_back);
    let _ = writeln!(body, "- **Severity:** {}", severities.join(", "));
    if !systems.is_empty() {
        let _ = writeln!(body, "- **System:** {}", systems.join("; "));
    }
    if with_logs > 0 {
        let _ = writeln!(body, "- **Logs:** {with_logs} report(s) include diagnostic logs; they stay on the office server.");
    }
    for (i, r) in rs.iter().enumerate() {
        let mut tags = vec![r.severity.id().to_string()];
        if r.after_update {
            tags.push("after an update".into());
        }
        if r.rolled_back {
            tags.push("rolled back".into());
        }
        let date = day(r.time);
        if !date.is_empty() {
            tags.push(date);
        }
        let _ = writeln!(body, "\n### {}. {}\n", i + 1, tags.join(" · "));
        for (label, t) in [("What happened", &r.description), ("Expected", &r.expected), ("Instead", &r.actual)] {
            if !t.is_empty() {
                let _ = writeln!(body, "**{label}:** {}\n", clean(t));
            }
        }
    }
    let suffix = format!("\n\n… (cut to fit; {n} report(s) in all, the rest are in the Server Manager's Feedback page)");
    format!(
        "https://github.com/{}/issues/new?title={}&labels={}&body={}",
        issue_repo(repo),
        url_encode(&title),
        url_encode(&format!("feedback,area:{}", group.feature)),
        encode_capped(&body, BODY_CAP, &suffix)
    )
}

// ---------------------------------------------------------------------------------------------
// health.jsonl
// ---------------------------------------------------------------------------------------------

const KINDS: [(&str, &str); 3] = [("updateFailed", "Update failed"), ("recovered", "Recovered after an abnormal end"), ("error", "Error")];

fn kind_label(kind: &str) -> &str {
    KINDS.iter().find(|(k, _)| *k == kind).map_or(kind, |(_, l)| *l)
}

fn kind_rank(kind: &str) -> usize {
    KINDS.iter().position(|(k, _)| *k == kind).unwrap_or(KINDS.len())
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Event {
    pub time: i64,
    pub user: String,
    pub machine: String,
    pub version: String,
    pub kind: String,
    pub detail: String,
}

pub fn parse_event(v: &Value) -> Option<Event> {
    let kind = text(v, "kind");
    if !v.is_object() || kind.is_empty() {
        return None;
    }
    let version = text(v, "version");
    Some(Event {
        time: millis(v, "time"),
        user: text(v, "user"),
        machine: text(v, "machine"),
        version: if version.is_empty() { "unknown".into() } else { version },
        kind: capped(&kind, 64),
        detail: capped(&text(v, "detail"), 4000),
    })
}

/// The events, newest first, and how many lines were skipped.
pub fn load_health(dir: &Path) -> Result<(Vec<Event>, usize), String> {
    read_lines(&dir.join("health.jsonl")).map(|t| {
        let (mut e, skipped) = parse_lines(&t, parse_event);
        e.sort_by_key(|x| std::cmp::Reverse(x.time));
        (e, skipped)
    })
}

/// Per kind and version, the events of the last 7 and the last 30 days.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct HealthRow {
    pub kind: String,
    pub version: String,
    pub week: usize,
    pub month: usize,
}

pub fn health_rows(events: &[Event], now: i64) -> Vec<HealthRow> {
    let mut by: BTreeMap<(String, String), (usize, usize)> = BTreeMap::new();
    for e in events {
        let age = now.saturating_sub(e.time);
        if age >= 30 * DAY_MS {
            continue;
        }
        let c = by.entry((e.kind.clone(), e.version.clone())).or_default();
        c.1 += 1;
        if age < 7 * DAY_MS {
            c.0 += 1;
        }
    }
    let mut out: Vec<HealthRow> = by.into_iter().map(|((kind, version), (week, month))| HealthRow { kind, version, week, month }).collect();
    out.sort_by(|a, b| kind_rank(&a.kind).cmp(&kind_rank(&b.kind)).then_with(|| a.kind.cmp(&b.kind)).then(cmp_versions(&b.version, &a.version)));
    out
}

// ---------------------------------------------------------------------------------------------
// adoption.json
// ---------------------------------------------------------------------------------------------

/// Command use per day and version: `days[date][version][command] = count`.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Adoption {
    pub days: BTreeMap<chrono::NaiveDate, BTreeMap<String, BTreeMap<String, u64>>>,
}

pub fn parse_adoption(v: &Value) -> Adoption {
    let mut out = Adoption::default();
    for (d, versions) in v.get("days").and_then(Value::as_object).into_iter().flatten() {
        let Ok(date) = chrono::NaiveDate::parse_from_str(d.trim(), "%Y-%m-%d") else { continue };
        let day = out.days.entry(date).or_default();
        for (version, commands) in versions.as_object().into_iter().flatten() {
            let per = day.entry(version.trim().to_string()).or_default();
            for (cmd, n) in commands.as_object().into_iter().flatten() {
                if let Some(n) = n.as_u64().filter(|n| *n > 0) {
                    let c = per.entry(capped(cmd.trim(), 128)).or_default();
                    *c = c.saturating_add(n);
                }
            }
        }
    }
    out
}

pub fn load_adoption(dir: &Path) -> Result<Adoption, String> {
    read_json(&dir.join("adoption.json")).map(|v| parse_adoption(&v))
}

/// Every version that counted any use, newest first.
pub fn adoption_versions(a: &Adoption) -> Vec<String> {
    let mut v: Vec<String> = a.days.values().flat_map(|d| d.keys().cloned()).collect::<BTreeSet<_>>().into_iter().collect();
    v.sort_by(|a, b| cmp_versions(b, a));
    v
}

/// Uses per command over the `days` days up to `today` (one version, or all), most used first.
pub fn command_use(a: &Adoption, today: chrono::NaiveDate, days: i64, version: Option<&str>) -> Vec<(String, u64)> {
    let mut by: BTreeMap<&str, u64> = BTreeMap::new();
    for (date, versions) in &a.days {
        if (today - *date).num_days() >= days {
            continue;
        }
        for (v, commands) in versions {
            if version.is_some_and(|want| want != v) {
                continue;
            }
            for (cmd, n) in commands {
                let c = by.entry(cmd.as_str()).or_default();
                *c = c.saturating_add(*n);
            }
        }
    }
    let mut out: Vec<(String, u64)> = by.into_iter().map(|(c, n)| (c.to_string(), n)).collect();
    out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out
}

/// How many of the most used commands are listed, and how many of the least used.
pub const TOP: usize = 30;
pub const RARE: usize = 10;

/// The least used commands outside the top list, least used first.
pub fn rarely_used(used: &[(String, u64)]) -> Vec<(String, u64)> {
    let rest = used.get(TOP..).unwrap_or(&[]);
    let mut out: Vec<(String, u64)> = rest.iter().rev().take(RARE).cloned().collect();
    out.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    out
}

// ---------------------------------------------------------------------------------------------
// The pages
// ---------------------------------------------------------------------------------------------

/// What the dashboard pages read, reloaded when one is opened and every few seconds while it is.
pub struct Dashboard {
    drawn_at: Option<f64>,
    loaded_at: f64,
    now: i64,
    today: chrono::NaiveDate,
    clients: Vec<Client>,
    clients_problem: Option<String>,
    /// As saved, and as being edited on the Releases page.
    saved: Releases,
    releases: Releases,
    releases_problem: Option<String>,
    reports: Vec<Report>,
    groups: Vec<Group>,
    feedback_skipped: usize,
    feedback_problem: Option<String>,
    selected: Option<(String, &'static str)>,
    events: Vec<Event>,
    health_skipped: usize,
    health_problem: Option<String>,
    adoption: Adoption,
    adoption_problem: Option<String>,
    adoption_days: i64,
    adoption_version: Option<String>,
}

impl Default for Dashboard {
    fn default() -> Dashboard {
        Dashboard {
            drawn_at: None,
            loaded_at: f64::MIN,
            now: 0,
            today: chrono::NaiveDate::default(),
            clients: Vec::new(),
            clients_problem: None,
            saved: Releases::default(),
            releases: Releases::default(),
            releases_problem: None,
            reports: Vec::new(),
            groups: Vec::new(),
            feedback_skipped: 0,
            feedback_problem: None,
            selected: None,
            events: Vec::new(),
            health_skipped: 0,
            health_problem: None,
            adoption: Adoption::default(),
            adoption_problem: None,
            adoption_days: 30,
            adoption_version: None,
        }
    }
}

impl Dashboard {
    fn load(&mut self, dir: &Path) {
        self.now = gridcraft_collab::now_ms();
        self.today = chrono::Local::now().date_naive();
        match load_clients(dir) {
            Ok(c) => {
                self.clients = c;
                self.clients_problem = None;
            }
            Err(e) => self.clients_problem = Some(e),
        }
        match load_releases(dir) {
            Ok(r) => {
                // Edits not yet saved are kept.
                if self.releases.same_as(&self.saved) {
                    self.releases = r.clone();
                }
                self.saved = r;
                self.releases_problem = None;
            }
            Err(e) => self.releases_problem = Some(e),
        }
        match load_feedback(dir) {
            Ok((r, skipped)) => {
                self.groups = group_feedback(&r);
                self.reports = r;
                self.feedback_skipped = skipped;
                self.feedback_problem = None;
            }
            Err(e) => self.feedback_problem = Some(e),
        }
        match load_health(dir) {
            Ok((e, skipped)) => {
                self.events = e;
                self.health_skipped = skipped;
                self.health_problem = None;
            }
            Err(e) => self.health_problem = Some(e),
        }
        match load_adoption(dir) {
            Ok(a) => {
                self.adoption = a;
                self.adoption_problem = None;
            }
            Err(e) => self.adoption_problem = Some(e),
        }
    }
}

/// "5 min ago" and the like; a date for anything older than two months.
pub fn ago(now: i64, then: i64) -> String {
    if then <= 0 {
        return "never".into();
    }
    let s = now.saturating_sub(then) / 1000;
    match s {
        ..60 => "just now".into(),
        60..3600 => format!("{} min ago", s / 60),
        3600..86_400 => format!("{} h ago", s / 3600),
        86_400..172_800 => "yesterday".into(),
        172_800..5_184_000 => format!("{} days ago", s / 86_400),
        _ => day(then),
    }
}

fn problem(ui: &mut Ui, p: &Option<String>) {
    if let Some(e) = p {
        ui.label(RichText::new(e).color(BAD));
    }
}

fn skipped(ui: &mut Ui, n: usize) {
    if n > 0 {
        ui.label(RichText::new(format!("{n} line(s) that could not be read were skipped.")).color(DIM).small());
    }
}

fn empty(ui: &mut Ui, title: &str, why: &str) {
    card(ui, |ui| {
        ui.label(RichText::new(title).strong());
        ui.label(RichText::new(why).color(DIM));
    });
}

/// A small coloured tag (status, channel).
fn tag(ui: &mut Ui, text: &str, color: Color32) {
    egui::Frame::new().stroke(egui::Stroke::new(1.0, color)).corner_radius(4.0).inner_margin(egui::Margin::symmetric(5, 1)).show(ui, |ui| {
        ui.label(RichText::new(text).color(color).small());
    });
}

fn status_color(s: Status) -> Color32 {
    match s {
        Status::Ok => OK,
        Status::KnownIssue | Status::Deprecated => WARN,
        Status::Blocked => BAD,
    }
}

impl App {
    /// Loads the dashboard's files when one of its pages is opened, then every 10 seconds.
    fn dashboard_refresh(&mut self, ui: &Ui) {
        let now = ui.input(|i| i.time);
        let d = &mut self.dash;
        if d.drawn_at.is_none_or(|t| now - t > 1.0) || now - d.loaded_at > 10.0 {
            d.load(&self.dir);
            d.loaded_at = now;
            self.installers = system::installers(&self.dir);
        }
        d.drawn_at = Some(now);
    }

    /// Versions of the installers on the server (each once), newest first.
    fn installer_versions(&self) -> Vec<String> {
        let mut v: Vec<String> = self.installers.iter().filter_map(|i| installer_version(&i.name)).collect::<BTreeSet<_>>().into_iter().collect();
        v.sort_by(|a, b| cmp_versions(b, a));
        v
    }

    pub(crate) fn versions_page(&mut self, ui: &mut Ui) {
        self.dashboard_refresh(ui);
        ui.heading("Installed versions");
        ui.label(
            RichText::new("The Dexterity Sheets version on each PC, as the apps tell this server when they sign in or look for updates.").color(DIM),
        );
        ui.add_space(8.0);
        let d = &self.dash;
        problem(ui, &d.clients_problem);
        if d.clients.is_empty() {
            empty(ui, "No PCs have reported a version yet", "Each app reports its version when it signs in to this server or looks for updates.");
            return;
        }
        let counts = version_counts(&d.clients);
        card(ui, |ui| {
            ui.label(RichText::new(format!("{} PC(s) on {} version(s)", d.clients.len(), counts.len())).strong());
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                for (v, people, pcs) in &counts {
                    egui::Frame::new()
                        .fill(ui.visuals().extreme_bg_color)
                        .stroke(egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color))
                        .corner_radius(6.0)
                        .inner_margin(egui::Margin::symmetric(12, 8))
                        .show(ui, |ui| {
                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(v).size(18.0).strong());
                                    let s = d.saved.get(v).status;
                                    if s != Status::Ok {
                                        tag(ui, s.label(), status_color(s));
                                    }
                                });
                                ui.label(RichText::new(format!("{pcs} PC(s), {people} people")).color(DIM));
                            });
                        });
                }
            });
        });
        card(ui, |ui| {
            egui::Grid::new("clients").num_columns(5).striped(true).spacing(vec2(22.0, 7.0)).show(ui, |ui| {
                for h in ["Version", "Person", "PC", "Channel", "Last seen"] {
                    ui.label(RichText::new(h).strong());
                }
                ui.end_row();
                let mut last: Option<&str> = None;
                for c in &d.clients {
                    if last == Some(c.version.as_str()) {
                        ui.label("");
                    } else {
                        ui.label(RichText::new(&c.version).strong());
                    }
                    last = Some(c.version.as_str());
                    ui.label(&c.user);
                    ui.label(&c.machine);
                    let channel = Channel::parse(&c.channel).map_or_else(
                        || if c.channel.is_empty() { format!("{} (default)", d.saved.default_channel.label()) } else { c.channel.clone() },
                        |ch| ch.label().to_string(),
                    );
                    ui.label(channel);
                    ui.label(ago(d.now, c.last_seen)).on_hover_text(when(c.last_seen));
                    ui.end_row();
                }
            });
        });
    }

    pub(crate) fn releases_page(&mut self, ui: &mut Ui) {
        self.dashboard_refresh(ui);
        ui.heading("Releases");
        ui.label(
            RichText::new("Which installer each PC is offered, and what people running a version are told. The server reads these settings within 30 seconds of Save.")
                .color(DIM),
        );
        ui.add_space(8.0);
        problem(ui, &self.dash.releases_problem);
        let admin = self.admin;
        if !admin {
            ui.label(RichText::new("View only: run the Server Manager as administrator to change releases.").color(WARN));
            ui.add_space(4.0);
        }
        let on_server = self.installer_versions();
        let mut rows: Vec<String> =
            on_server.iter().cloned().chain(self.dash.releases.releases.keys().cloned()).collect::<BTreeSet<_>>().into_iter().collect();
        rows.sort_by(|a, b| cmp_versions(b, a));
        let mut save = false;
        let mut discard = false;
        let dirty = !self.dash.releases.same_as(&self.dash.saved);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(admin && dirty, primary("Save"))
                .on_disabled_hover_text(if admin { "Nothing changed." } else { "Needs administrator rights." })
                .clicked()
            {
                save = true;
            }
            if ui.add_enabled(dirty, egui::Button::new("Discard changes")).clicked() {
                discard = true;
            }
            if dirty {
                ui.label(RichText::new("Not saved yet").color(WARN));
            }
        });
        ui.add_space(8.0);

        card(ui, |ui| {
            ui.label(RichText::new("Update policy").strong());
            ui.add_enabled_ui(admin, |ui| {
                ui.label(RichText::new("How a new version reaches the PCs").color(DIM));
                for m in Mode::ALL {
                    ui.radio_value(&mut self.dash.releases.mode, m, m.label());
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Channel of a PC that has not chosen one").color(DIM));
                    for c in Channel::ALL {
                        ui.radio_value(&mut self.dash.releases.default_channel, c, c.label());
                    }
                });
            })
            .response
            .on_disabled_hover_text("Needs administrator rights.");
        });

        let r = &self.dash.releases;
        card(ui, |ui| {
            ui.label(RichText::new(if dirty { "What the PCs will be offered once you save" } else { "What the PCs are offered" }).strong());
            ui.add_space(4.0);
            egui::Grid::new("offered").num_columns(2).spacing(vec2(16.0, 6.0)).show(ui, |ui| {
                for c in Channel::ALL {
                    ui.label(format!("{} PCs", c.label()));
                    match offered(&on_server, r, c) {
                        Some(v) => ui.label(RichText::new(v).strong()),
                        None if on_server.is_empty() => ui.label(RichText::new("nothing: there are no installers on this server").color(WARN)),
                        None => ui.label(RichText::new("nothing: no installer here is OK (or a known issue) on this channel").color(WARN)),
                    };
                    ui.end_row();
                }
            });
            if r.mode == Mode::Manual {
                ui.label(
                    RichText::new("The policy is Never offer: the apps find this version only with Help › Check for Updates.").color(DIM).small(),
                );
            }
            let blocked: Vec<&String> = rows.iter().filter(|v| r.get(v).status == Status::Blocked).collect();
            let deprecated: Vec<&String> = rows.iter().filter(|v| r.get(v).status == Status::Deprecated).collect();
            if !blocked.is_empty() || !deprecated.is_empty() {
                ui.add_space(6.0);
            }
            for v in blocked {
                match rollback_for(v, &on_server, r) {
                    Some(to) => ui.label(RichText::new(format!("{v} is blocked: PCs running it are told to roll back to {to}.")).color(BAD)),
                    None => ui.label(
                        RichText::new(format!(
                            "{v} is blocked, but no older version marked OK is on this server, so PCs running it have nothing to roll back to. Add an older installer on the App updates page."
                        ))
                        .color(BAD),
                    ),
                };
            }
            for v in deprecated {
                ui.label(RichText::new(format!("{v} is deprecated: no longer offered, and PCs running it are told to move on.")).color(WARN));
            }
        });

        card(ui, |ui| {
            ui.label(RichText::new("Versions").strong());
            if rows.is_empty() {
                ui.label(
                    RichText::new("No installers on this server yet: add one on the App updates page, or let the server fetch them from GitHub.")
                        .color(DIM),
                );
            } else {
                ui.label(RichText::new("A version with no settings of its own is stable and OK.").color(DIM).small());
                ui.add_space(4.0);
                let installers = self.installers.clone();
                ui.add_enabled_ui(admin, |ui| {
                    egui::Grid::new("releases").num_columns(5).striped(true).spacing(vec2(12.0, 8.0)).show(ui, |ui| {
                        for h in ["Version", "Installer", "Channel", "Status", "Note shown to people running it"] {
                            ui.label(RichText::new(h).strong());
                        }
                        ui.end_row();
                        for v in &rows {
                            let mut rel = self.dash.releases.get(v);
                            let before = rel.clone();
                            ui.label(RichText::new(v).strong());
                            let files: Vec<&system::Installer> =
                                installers.iter().filter(|i| installer_version(&i.name).as_deref() == Some(v)).collect();
                            if files.is_empty() {
                                ui.label(RichText::new("not on this server").color(DIM));
                            } else {
                                let size: u64 = files.iter().map(|i| i.size).sum();
                                let names: Vec<&str> = files.iter().map(|i| i.name.as_str()).collect();
                                ui.label(system::human_size(size)).on_hover_text(names.join("\n"));
                            }
                            ui.horizontal(|ui| {
                                for c in Channel::ALL {
                                    ui.selectable_value(&mut rel.channel, c, c.label());
                                }
                            });
                            ui.horizontal(|ui| {
                                for s in Status::ALL {
                                    let label = if rel.status == s { RichText::new(s.label()).strong() } else { RichText::new(s.label()) };
                                    ui.selectable_value(&mut rel.status, s, label).on_hover_text(s.hint());
                                }
                            });
                            ui.add(egui::TextEdit::singleline(&mut rel.note).desired_width(185.0).hint_text(if rel.status == Status::Ok {
                                "optional"
                            } else {
                                "what to tell them"
                            }));
                            if rel != before {
                                rel.note = capped(&rel.note, 4000);
                                self.dash.releases.releases.insert(v.clone(), rel);
                            }
                            ui.end_row();
                        }
                    });
                })
                .response
                .on_disabled_hover_text("Needs administrator rights.");
                let unnamed: Vec<&str> = installers.iter().filter(|i| installer_version(&i.name).is_none()).map(|i| i.name.as_str()).collect();
                for n in unnamed {
                    ui.label(RichText::new(format!("{n}: its file name carries no version, so the server never offers it.")).color(WARN));
                }
            }
        });

        if save {
            let r = save_releases(&self.dir, &self.dash.releases);
            if r.is_ok() {
                self.dash.releases = self.dash.releases.normalized();
                self.dash.saved = self.dash.releases.clone();
            }
            self.say(r.map(|()| "Saved. The server hands out the new settings within 30 seconds.".to_string()));
        }
        if discard {
            self.dash.releases = self.dash.saved.clone();
        }
    }

    pub(crate) fn feedback_page(&mut self, ui: &mut Ui) {
        self.dashboard_refresh(ui);
        ui.heading("Feedback");
        ui.label(
            RichText::new("Problems people reported from Dexterity Sheets, by version and feature. Select a group to read its reports.").color(DIM),
        );
        ui.add_space(8.0);
        problem(ui, &self.dash.feedback_problem);
        skipped(ui, self.dash.feedback_skipped);
        if self.dash.groups.is_empty() {
            empty(ui, "No reports yet", "Apps send them from Help › Report a Problem.");
            return;
        }
        if !self.dash.selected.as_ref().is_some_and(|(v, f)| self.dash.groups.iter().any(|g| &g.version == v && g.feature == *f)) {
            self.dash.selected = self.dash.groups.first().map(|g| (g.version.clone(), g.feature));
        }
        let mut open: Option<usize> = None;
        let d = &mut self.dash;
        card(ui, |ui| {
            ui.label(RichText::new(format!("{} report(s) in {} group(s)", d.reports.len(), d.groups.len())).strong());
            ui.add_space(4.0);
            egui::Grid::new("feedback").num_columns(7).striped(true).spacing(vec2(18.0, 7.0)).show(ui, |ui| {
                for h in ["Version", "Feature", "Reports", "Most severe", "After an update", "Rolled back", ""] {
                    ui.label(RichText::new(h).strong());
                }
                ui.end_row();
                for (i, g) in d.groups.iter().enumerate() {
                    let sel = d.selected.as_ref().is_some_and(|(v, f)| *v == g.version && *f == g.feature);
                    if ui.selectable_label(sel, RichText::new(&g.version).strong()).clicked() {
                        d.selected = Some((g.version.clone(), g.feature));
                    }
                    if ui.selectable_label(sel, feature_label(g.feature)).clicked() {
                        d.selected = Some((g.version.clone(), g.feature));
                    }
                    ui.label(g.reports.len().to_string());
                    ui.label(RichText::new(g.highest.label()).color(g.highest.color()));
                    ui.label(g.after_update.to_string());
                    ui.label(g.rolled_back.to_string());
                    if ui
                        .small_button("Open GitHub issue")
                        .on_hover_text("Opens a new issue on GitHub in your browser with these reports, without the names of people or PCs. Nothing is sent until you submit it there.")
                        .clicked()
                    {
                        open = Some(i);
                    }
                    ui.end_row();
                }
            });
        });
        if let Some(g) = open.and_then(|i| self.dash.groups.get(i)) {
            let names: Vec<String> = self.users.iter().flat_map(|u| [u.user.clone(), u.display.clone()]).collect();
            let url = issue_url(&self.repo, g, &self.dash.reports, &names);
            let r = system::open_url(&url).map(|()| {
                format!("A new issue for {} in {} opened in your browser: check it, then submit it there.", feature_label(g.feature), g.version)
            });
            self.say(r);
        }
        let d = &self.dash;
        let Some(g) = d.selected.as_ref().and_then(|(v, f)| d.groups.iter().find(|g| &g.version == v && g.feature == *f)) else { return };
        ui.label(RichText::new(format!("{} in {}: {} report(s)", feature_label(g.feature), g.version, g.reports.len())).size(16.0).strong());
        ui.add_space(4.0);
        for i in &g.reports {
            let Some(r) = d.reports.get(*i) else { continue };
            card(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(r.severity.label()).color(r.severity.color()).strong());
                    ui.label(RichText::new(format!("{} on {}", r.user, r.machine)).strong());
                    ui.label(RichText::new(when(r.time)).color(DIM));
                    if r.after_update {
                        tag(ui, "after an update", WARN);
                    }
                    if r.rolled_back {
                        tag(ui, "rolled back", BAD);
                    }
                });
                if !r.os.is_empty() {
                    ui.label(RichText::new(&r.os).color(DIM).small());
                }
                for (label, t) in [("What happened", &r.description), ("Expected", &r.expected), ("What happened instead", &r.actual)] {
                    ui.add_space(2.0);
                    ui.label(RichText::new(label).size(12.5).strong());
                    if t.is_empty() {
                        ui.label(RichText::new("(left empty)").color(DIM));
                    } else {
                        ui.add(egui::Label::new(t).wrap());
                    }
                }
                ui.add_space(2.0);
                match &r.logs {
                    Some(logs) => {
                        egui::CollapsingHeader::new(format!("Diagnostic logs ({})", system::human_size(logs.len() as u64)))
                            .id_salt(("logs", *i))
                            .show(ui, |ui| {
                                egui::ScrollArea::vertical().id_salt(("logs-scroll", *i)).max_height(240.0).show(ui, |ui| {
                                    ui.add(egui::Label::new(RichText::new(logs).monospace().size(11.5)).wrap_mode(egui::TextWrapMode::Extend));
                                });
                            });
                    }
                    None => {
                        ui.label(RichText::new("No diagnostic logs: the person did not include them.").color(DIM).small());
                    }
                }
            });
        }
    }

    pub(crate) fn health_page(&mut self, ui: &mut Ui) {
        self.dashboard_refresh(ui);
        ui.heading("System health");
        ui.label(
            RichText::new("Failed updates, starts after a crash or forced shutdown (unsaved work was offered back), and errors the apps report.")
                .color(DIM),
        );
        ui.add_space(8.0);
        let d = &self.dash;
        problem(ui, &d.health_problem);
        skipped(ui, d.health_skipped);
        if d.events.is_empty() {
            empty(ui, "No events yet", "Apps report failed updates, recoveries after a crash and errors here as they happen.");
            return;
        }
        let rows = health_rows(&d.events, d.now);
        card(ui, |ui| {
            ui.label(RichText::new("Last 7 and 30 days").strong());
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                for (kind, label) in KINDS {
                    let week: usize = rows.iter().filter(|r| r.kind == kind).map(|r| r.week).sum();
                    let month: usize = rows.iter().filter(|r| r.kind == kind).map(|r| r.month).sum();
                    egui::Frame::new()
                        .fill(ui.visuals().extreme_bg_color)
                        .stroke(egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color))
                        .corner_radius(6.0)
                        .inner_margin(egui::Margin::symmetric(12, 8))
                        .show(ui, |ui| {
                            ui.vertical(|ui| {
                                ui.label(RichText::new(label).strong());
                                ui.label(RichText::new(format!("{week} in 7 days")).size(17.0).color(if week > 0 { BAD } else { OK }));
                                ui.label(RichText::new(format!("{month} in 30 days")).color(DIM));
                            });
                        });
                }
            });
            ui.add_space(6.0);
            if rows.is_empty() {
                ui.label(RichText::new("Nothing in the last 30 days.").color(DIM));
            } else {
                egui::Grid::new("health-counts").num_columns(4).striped(true).spacing(vec2(22.0, 6.0)).show(ui, |ui| {
                    for h in ["What", "Version", "Last 7 days", "Last 30 days"] {
                        ui.label(RichText::new(h).strong());
                    }
                    ui.end_row();
                    for r in &rows {
                        ui.label(kind_label(&r.kind));
                        ui.label(&r.version);
                        ui.label(r.week.to_string());
                        ui.label(r.month.to_string());
                        ui.end_row();
                    }
                });
            }
        });
        card(ui, |ui| {
            let n = d.events.len().min(50);
            ui.label(RichText::new(format!("Latest {n} event(s)")).strong());
            ui.add_space(4.0);
            egui::Grid::new("health-events").num_columns(5).striped(true).spacing(vec2(16.0, 6.0)).max_col_width(420.0).show(ui, |ui| {
                for h in ["When", "What", "Version", "Person and PC", "Detail"] {
                    ui.label(RichText::new(h).strong());
                }
                ui.end_row();
                for e in d.events.iter().take(50) {
                    ui.label(ago(d.now, e.time)).on_hover_text(when(e.time));
                    let color = match e.kind.as_str() {
                        "updateFailed" | "error" => BAD,
                        "recovered" => WARN,
                        _ => DIM,
                    };
                    ui.label(RichText::new(kind_label(&e.kind)).color(color));
                    ui.label(&e.version);
                    ui.label(format!("{} on {}", e.user, e.machine));
                    ui.add(egui::Label::new(RichText::new(&e.detail).color(DIM)).wrap());
                    ui.end_row();
                }
            });
        });
    }

    pub(crate) fn adoption_page(&mut self, ui: &mut Ui) {
        self.dashboard_refresh(ui);
        ui.heading("Feature adoption");
        ui.label(RichText::new("Which commands people use, counted by the apps (command names only, never what they were used on).").color(DIM));
        ui.add_space(8.0);
        problem(ui, &self.dash.adoption_problem);
        if self.dash.adoption.days.is_empty() {
            empty(ui, "No use counted yet", "Apps send the commands they used about once an hour; the counts appear here.");
            return;
        }
        let versions = adoption_versions(&self.dash.adoption);
        if self.dash.adoption_version.as_ref().is_some_and(|v| !versions.contains(v)) {
            self.dash.adoption_version = None;
        }
        let d = &mut self.dash;
        card(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Period").color(DIM));
                for n in [7, 30, 90] {
                    ui.selectable_value(&mut d.adoption_days, n, format!("Last {n} days"));
                }
                ui.add_space(16.0);
                ui.label(RichText::new("Version").color(DIM));
                ui.selectable_value(&mut d.adoption_version, None, "All versions");
                for v in &versions {
                    ui.selectable_value(&mut d.adoption_version, Some(v.clone()), v.as_str());
                }
            });
        });
        let used = command_use(&d.adoption, d.today, d.adoption_days, d.adoption_version.as_deref());
        if used.is_empty() {
            empty(ui, "Nothing counted in this period", "Pick a longer period or another version.");
            return;
        }
        let total: u64 = used.iter().map(|(_, n)| *n).fold(0, u64::saturating_add);
        let top = used.first().map_or(1, |(_, n)| (*n).max(1));
        card(ui, |ui| {
            ui.label(RichText::new(format!("Most used: {} of {} command(s), {total} use(s) in all", used.len().min(TOP), used.len())).strong());
            ui.add_space(4.0);
            egui::Grid::new("adoption-top").num_columns(4).striped(true).spacing(vec2(16.0, 5.0)).show(ui, |ui| {
                for h in ["", "Command", "Uses", ""] {
                    ui.label(RichText::new(h).strong());
                }
                ui.end_row();
                for (i, (cmd, n)) in used.iter().take(TOP).enumerate() {
                    ui.label(RichText::new(format!("{}", i + 1)).color(DIM));
                    ui.label(RichText::new(cmd).monospace());
                    ui.label(n.to_string());
                    let (rect, _) = ui.allocate_exact_size(vec2(220.0, 12.0), egui::Sense::hover());
                    let w = (rect.width() * (*n as f32 / top as f32)).max(2.0);
                    ui.painter().rect_filled(egui::Rect::from_min_size(rect.min, vec2(w, rect.height())), 3.0, crate::ACCENT);
                    ui.end_row();
                }
            });
        });
        card(ui, |ui| {
            ui.label(RichText::new("Rarely used").strong());
            let rare = rarely_used(&used);
            if rare.is_empty() {
                ui.label(RichText::new(format!("Every command used in this period ({}) is in the list above.", used.len())).color(DIM));
            } else {
                ui.label(RichText::new("The least used of the commands used at all in this period.").color(DIM).small());
                ui.horizontal_wrapped(|ui| {
                    for (cmd, n) in rare {
                        ui.label(RichText::new(cmd).monospace());
                        ui.label(RichText::new(format!("{n}   ")).color(DIM));
                    }
                });
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("dexsheets-admin-{name}-{}-{}", std::process::id(), gridcraft_collab::now_ms()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn decode(s: &str) -> String {
        let b = s.as_bytes();
        let mut out = Vec::new();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'%' {
                out.push(u8::from_str_radix(&s[i + 1..i + 3], 16).unwrap());
                i += 3;
            } else {
                out.push(b[i]);
                i += 1;
            }
        }
        String::from_utf8(out).unwrap()
    }

    fn query(url: &str, key: &str) -> String {
        let q = url.split_once('?').unwrap().1;
        q.split('&').find_map(|kv| kv.strip_prefix(&format!("{key}="))).unwrap().to_string()
    }

    fn report(version: &str, features: &[&'static str], sev: Severity, time: i64) -> Report {
        Report {
            id: format!("fb-{time}"),
            time,
            user: "anita".into(),
            machine: "PC-ENG-07".into(),
            version: version.into(),
            features: features.to_vec(),
            severity: sev,
            ..Report::default()
        }
    }

    #[test]
    fn versions_order_as_the_server_does() {
        let pairs = [("1.10.0", "1.9.9"), ("1.2.0", "1.2.0-beta"), ("2", "1.99"), ("1.2.0.1", "1.2"), ("v1.3", "1.2.9")];
        for (newer, older_v) in pairs {
            assert_eq!(cmp_versions(newer, older_v), Ordering::Greater, "{newer} > {older_v}");
            assert!(gridcraft_collab::proto::version_newer(newer, older_v), "the server agrees: {newer} > {older_v}");
        }
        assert_eq!(installer_version("dexterity-sheets-1.2.0-windows-x64.msi").as_deref(), Some("1.2.0"));
        assert_eq!(installer_version("DexteritySheets-0.13.0-x64.msi").as_deref(), Some("0.13.0"));
        assert_eq!(installer_version("dexterity-sheets.msi"), None);
    }

    #[test]
    fn missing_files_are_empty() {
        let dir = temp("missing");
        assert!(load_clients(&dir).unwrap().is_empty());
        assert_eq!(load_releases(&dir).unwrap(), Releases::default());
        assert_eq!(load_feedback(&dir).unwrap(), (Vec::new(), 0));
        assert_eq!(load_health(&dir).unwrap(), (Vec::new(), 0));
        assert_eq!(load_adoption(&dir).unwrap(), Adoption::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn junk_lines_are_skipped_and_counted() {
        let dir = temp("junk");
        std::fs::create_dir_all(dir.join("feedback")).unwrap();
        let lines = [
            r#"{"id":"fb-1","time":1000,"user":"anita","machine":"PC-1","version":"1.2.2","os":"Windows 10","features":["sharing","saving","sharing"],"description":"Blank","expected":"Data","actual":"Nothing","severity":"high","afterUpdate":true,"rolledBack":false,"logs":null}"#,
            "not json at all",
            "",
            "42",
            r#"{"id":"fb-2","time":2000,"version":"1.2.2","features":["bogus"],"severity":"weird"}"#,
            r#"{"id":"fb-3","time":3000,"vers"#,
        ];
        std::fs::write(dir.join("feedback").join("feedback.jsonl"), lines.join("\n")).unwrap();
        let (reports, skipped) = load_feedback(&dir).unwrap();
        assert_eq!(skipped, 3, "plain text, a number and a cut line");
        assert_eq!(reports.len(), 2);
        assert_eq!(reports[0].features, vec!["sharing", "saving"], "a repeated feature counts once");
        assert_eq!(reports[0].severity, Severity::High);
        assert!(reports[0].after_update && reports[0].logs.is_none());
        assert_eq!(reports[1].features, vec!["other"], "an unknown feature counts as other");
        assert_eq!(reports[1].severity, Severity::Low);

        std::fs::write(
            dir.join("health.jsonl"),
            "{\"time\":5,\"user\":\"a\",\"machine\":\"m\",\"version\":\"1.2.2\",\"kind\":\"error\",\"detail\":\"x\"}\n{oops\n{\"time\":9,\"kind\":\"updateFailed\"}\n{\"time\":7}\n",
        )
        .unwrap();
        let (events, skipped) = load_health(&dir).unwrap();
        assert_eq!(skipped, 2, "the broken line and the one without a kind");
        assert_eq!(events.iter().map(|e| e.time).collect::<Vec<_>>(), vec![9, 5], "newest first");
        assert_eq!(events[0].version, "unknown");

        std::fs::write(
            dir.join("clients.json"),
            r#"{"clients":{"anita|PC-ENG-07":{"user":"anita","machine":"PC-ENG-07","version":"1.2.2","channel":"stable","lastSeen":10},
                "rahul|PC-2":{"version":"1.10.0","lastSeen":5}, "junk": 3,
                "priya|PC-3":{"user":"priya","machine":"PC-3","version":"1.2.2","lastSeen":20}}}"#,
        )
        .unwrap();
        let clients = load_clients(&dir).unwrap();
        assert_eq!(clients.len(), 3);
        assert_eq!(clients[0].version, "1.10.0", "newest version first");
        assert_eq!((clients[0].user.as_str(), clients[0].machine.as_str()), ("rahul", "PC-2"), "names from the key when missing");
        assert_eq!(clients[1].user, "priya", "then the most recently seen");
        assert_eq!(version_counts(&clients), vec![("1.10.0".to_string(), 1, 1), ("1.2.2".to_string(), 2, 2)]);

        std::fs::write(dir.join("clients.json"), "{ not json").unwrap();
        assert!(load_clients(&dir).is_err(), "a broken whole file is reported, not shown as empty");

        std::fs::write(
            dir.join("adoption.json"),
            r#"{"days":{"2026-10-28":{"1.2.2":{"home.bold":5,"collab.share":2,"bad":-3,"x":"y"}},"not-a-date":{"1.2.2":{"home.bold":99}},
                "2026-10-20":{"1.3.0":{"home.bold":1,"data.sort":4}},"2026-08-01":{"1.2.2":{"home.italic":7}}}}"#,
        )
        .unwrap();
        let a = load_adoption(&dir).unwrap();
        assert_eq!(a.days.len(), 3, "the bad date is skipped");
        let today = chrono::NaiveDate::from_ymd_opt(2026, 10, 28).unwrap();
        assert_eq!(command_use(&a, today, 7, None), vec![("home.bold".to_string(), 5), ("collab.share".to_string(), 2)]);
        assert_eq!(
            command_use(&a, today, 30, None),
            vec![("home.bold".to_string(), 6), ("data.sort".to_string(), 4), ("collab.share".to_string(), 2)]
        );
        assert_eq!(
            command_use(&a, today, 90, Some("1.2.2")).iter().map(|(c, _)| c.as_str()).collect::<Vec<_>>(),
            vec!["home.italic", "home.bold", "collab.share"]
        );
        assert_eq!(adoption_versions(&a), vec!["1.3.0".to_string(), "1.2.2".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rarely_used_lists_the_tail_outside_the_top() {
        let used: Vec<(String, u64)> = (0..45).map(|i| (format!("cmd.{i:02}"), 100 - i)).collect();
        let rare = rarely_used(&used);
        assert_eq!(rare.len(), RARE);
        assert_eq!(rare[0], ("cmd.44".to_string(), 56), "least used first");
        assert!(rarely_used(&used[..20]).is_empty(), "all already in the top list");
    }

    #[test]
    fn health_counts_the_last_7_and_30_days() {
        let now = 100 * DAY_MS;
        let ev = |days_ago: i64, kind: &str, version: &str| Event {
            time: now - days_ago * DAY_MS - 1,
            user: String::new(),
            machine: String::new(),
            version: version.into(),
            kind: kind.into(),
            detail: String::new(),
        };
        let events = vec![
            ev(1, "error", "1.2.2"),
            ev(10, "error", "1.2.2"),
            ev(40, "error", "1.2.2"),
            ev(2, "updateFailed", "1.3.0"),
            ev(3, "error", "1.3.0"),
        ];
        let rows = health_rows(&events, now);
        let find = |k: &str, v: &str| rows.iter().find(|r| r.kind == k && r.version == v).map(|r| (r.week, r.month));
        assert_eq!(find("error", "1.2.2"), Some((1, 2)), "the 40-day-old one is left out");
        assert_eq!(find("updateFailed", "1.3.0"), Some((1, 1)));
        assert_eq!(rows[0].kind, "updateFailed", "failed updates first");
    }

    #[test]
    fn feedback_groups_by_version_and_feature() {
        let mut a = report("1.2.2", &["sharing", "saving"], Severity::High, 10);
        a.after_update = true;
        let mut b = report("1.2.2", &["sharing"], Severity::Blocking, 20);
        b.rolled_back = true;
        let c = report("1.10.0", &["formulas"], Severity::Low, 30);
        let d = report("1.2.2", &["saving"], Severity::Low, 40);
        let reports = vec![a, b, c, d];
        let groups = group_feedback(&reports);
        let keys: Vec<(&str, &str, usize)> = groups.iter().map(|g| (g.version.as_str(), g.feature, g.reports.len())).collect();
        assert_eq!(keys, vec![("1.10.0", "formulas", 1), ("1.2.2", "sharing", 2), ("1.2.2", "saving", 2)], "newest version, then most severe");
        let sharing = &groups[1];
        assert_eq!(sharing.highest, Severity::Blocking);
        assert_eq!((sharing.after_update, sharing.rolled_back), (1, 1));
        assert_eq!(sharing.reports, vec![1, 0], "newest report first");
        assert_eq!(groups[2].highest, Severity::High);
    }

    fn releases(entries: &[(&str, Channel, Status)]) -> Releases {
        Releases {
            releases: entries.iter().map(|(v, c, s)| (v.to_string(), Release { channel: *c, status: *s, note: String::new() })).collect(),
            ..Releases::default()
        }
    }

    #[test]
    fn each_channel_is_offered_its_newest_usable_version() {
        let on: Vec<String> = ["1.3.0", "1.2.3", "1.2.2", "1.2.1", "1.2.0"].iter().map(|s| s.to_string()).collect();
        let r = releases(&[
            ("1.3.0", Channel::Testing, Status::Ok),
            ("1.2.3", Channel::Stable, Status::Blocked),
            ("1.2.2", Channel::Stable, Status::KnownIssue),
        ]);
        assert_eq!(offered(&on, &r, Channel::Stable).as_deref(), Some("1.2.2"), "blocked is skipped, a known issue is still offered");
        assert_eq!(offered(&on, &r, Channel::Testing).as_deref(), Some("1.3.0"));
        let r = releases(&[("1.3.0", Channel::Testing, Status::Deprecated)]);
        assert_eq!(offered(&on, &r, Channel::Testing).as_deref(), Some("1.2.3"), "deprecated is no longer offered");
        assert_eq!(offered(&on, &r, Channel::Stable).as_deref(), Some("1.2.3"), "no entry is stable and OK");
        assert_eq!(offered(&[], &r, Channel::Stable), None);
    }

    #[test]
    fn a_blocked_version_rolls_back_to_the_newest_older_ok_one() {
        let on: Vec<String> = ["1.10.0", "1.9.0", "1.3.0", "1.2.2", "1.2.1", "1.2.0"].iter().map(|s| s.to_string()).collect();
        let r = releases(&[
            ("1.10.0", Channel::Stable, Status::Blocked),
            ("1.9.0", Channel::Stable, Status::KnownIssue),
            ("1.3.0", Channel::Testing, Status::Deprecated),
            ("1.2.1", Channel::Stable, Status::Blocked),
        ]);
        assert_eq!(
            rollback_for("1.10.0", &on, &r).as_deref(),
            Some("1.2.2"),
            "known issue, deprecated and blocked are passed over; 1.10 is newer than 1.9"
        );
        assert_eq!(rollback_for("1.2.1", &on, &r).as_deref(), Some("1.2.0"));
        assert_eq!(rollback_for("1.2.0", &on, &r), None, "nothing older");
    }

    #[test]
    fn releases_round_trip_through_the_file() {
        let dir = temp("releases");
        let mut r = Releases { mode: Mode::Auto, default_channel: Channel::Testing, releases: BTreeMap::new() };
        r.releases.insert("1.3.0".into(), Release { channel: Channel::Testing, status: Status::Ok, note: String::new() });
        r.releases.insert(
            "1.2.2".into(),
            Release { channel: Channel::Stable, status: Status::KnownIssue, note: "  Sharing could send a blank workbook.  ".into() },
        );
        r.releases.insert("1.2.0".into(), Release::default());
        save_releases(&dir, &r).unwrap();
        assert!(!dir.join("updates").join("releases.tmp").exists(), "the temporary file is renamed");
        let back = load_releases(&dir).unwrap();
        assert!(back.same_as(&r));
        assert_eq!(back.releases.len(), 2, "an entry saying only the defaults is not written");
        assert_eq!(back.get("1.2.2").note, "Sharing could send a blank workbook.");
        let raw: Value = serde_json::from_slice(&std::fs::read(dir.join("updates").join("releases.json")).unwrap()).unwrap();
        assert_eq!(raw["policy"], json!({"mode": "auto", "defaultChannel": "testing"}));
        assert_eq!(raw["releases"]["1.2.2"]["status"], "knownIssue");
        assert_eq!(raw["releases"]["1.3.0"]["channel"], "testing");
        // Saved again over the existing file.
        r.mode = Mode::Manual;
        save_releases(&dir, &r).unwrap();
        assert_eq!(load_releases(&dir).unwrap().mode, Mode::Manual);
        // Unknown words fall back to the defaults.
        let odd = parse_releases(&json!({"policy": {"mode": "sometimes"}, "releases": {"9.9": {"status": "maybe", "channel": 3}}}));
        assert_eq!((odd.mode, odd.get("9.9")), (Mode::Prompt, Release::default()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_issue_address_is_encoded_capped_and_names_nobody() {
        let mut a = report("1.2.2", &["sharing"], Severity::High, 1_793_194_586_383);
        a.user = "anita.rao".into();
        a.machine = "PC-ENG-07".into();
        a.description = "Anita Rao shared Q3 & it went blank on pc-eng-07 for anita.rao; tell Rahul?".into();
        a.expected = "100% of the data".into();
        a.logs = Some("C:\\Users\\anita.rao\\log".into());
        a.after_update = true;
        let mut b = report("1.2.2", &["sharing"], Severity::Blocking, 1_793_194_586_999);
        b.user = "rahul".into();
        b.machine = "PC-2".into();
        b.actual = "naïve — unicode stays intact".into();
        let reports = vec![a, b];
        let groups = group_feedback(&reports);
        let url = issue_url("", &groups[0], &reports, &["Anita Rao".to_string()]);
        assert!(url.starts_with("https://github.com/vikramdex-ops/dexterity-sheets/issues/new?title="), "{url}");
        assert_eq!(decode(&query(&url, "labels")), "feedback,area:sharing");
        assert_eq!(decode(&query(&url, "title")), "Feedback: Sharing in 1.2.2 (2 reports)");
        assert!(!url.contains(' ') && !url.contains('\n'), "encoded");
        let body = decode(&query(&url, "body"));
        assert!(body.contains("- **Version:** 1.2.2"));
        assert!(body.contains("- **Severity:** blocking 1, high 1"));
        assert!(body.contains("1 after an update; 0 rolled back"));
        assert!(body.contains("Q3 & it went blank") && body.contains("100% of the data") && body.contains("naïve — unicode"));
        let lower = body.to_ascii_lowercase();
        for name in ["anita", "rahul", "pc-eng-07", "pc-2"] {
            assert!(!lower.contains(name), "{name} leaked into: {body}");
        }
        assert!(body.contains("[person] shared") && body.contains("on [PC] for [person]"));
        assert!(!body.contains("C:\\Users"), "logs stay on the server");

        // A custom repository is used; a bad one falls back.
        let url = issue_url("acme/sheets", &groups[0], &reports, &[]);
        assert!(url.starts_with("https://github.com/acme/sheets/issues/new?"));
        assert!(issue_url("evil.com/x?y", &groups[0], &reports, &[]).starts_with("https://github.com/vikramdex-ops/dexterity-sheets/"));

        // Many long reports: the body is cut to fit.
        let many: Vec<Report> = (0..40)
            .map(|i| {
                let mut r = report("1.2.2", &["saving"], Severity::Medium, i);
                r.description = format!("Report {i}: {}", "saving took very long, é ".repeat(20));
                r
            })
            .collect();
        let g = group_feedback(&many);
        let url = issue_url("", &g[0], &many, &[]);
        let enc = query(&url, "body");
        assert!(enc.len() <= BODY_CAP, "{}", enc.len());
        assert!(enc.len() > BODY_CAP - 100, "uses the room it has");
        assert!(decode(&enc).ends_with("the rest are in the Server Manager's Feedback page)"));
    }

    #[test]
    fn scrubbing_takes_whole_words_only() {
        assert_eq!(scrub_one("Ravi met ravindra; RAVI left", "ravi", "[person]"), "[person] met ravindra; [person] left");
        assert_eq!(scrub_one("ünïcode ravi", "ravi", "X"), "ünïcode X");
        assert_eq!(scrub_one("abc", "", "X"), "abc");
    }

    #[test]
    fn times_read_plainly() {
        let now = 1_000 * DAY_MS;
        assert_eq!(ago(now, now - 5_000), "just now");
        assert_eq!(ago(now, now - 5 * 60_000), "5 min ago");
        assert_eq!(ago(now, now - 3 * 3_600_000), "3 h ago");
        assert_eq!(ago(now, now - 4 * DAY_MS), "4 days ago");
        assert_eq!(ago(now, 0), "never");
    }
}
