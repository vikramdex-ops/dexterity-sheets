//! Messages between Dexterity Sheets clients and the server, and how they are framed on TCP.
//!
//! Frame: 4-byte big-endian length, 1 flag byte (1 = deflate), then the JSON body.

use std::io::{Read, Write};

use serde::{Deserialize, Serialize};

use crate::patch::WbPatch;
use crate::{Error, Result};

pub const PROTOCOL: u32 = 1;
pub const DEFAULT_PORT: u16 = 47810;
pub const DISCOVERY_PORT: u16 = 47811;
/// Frames past this size are refused (a corrupt length must not allocate gigabytes).
pub const MAX_FRAME: usize = 256 * 1024 * 1024;
/// Bodies past this size are compressed.
const COMPRESS_OVER: usize = 2048;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Admin,
    Editor,
    Viewer,
}

impl Role {
    pub fn can_edit(self) -> bool {
        !matches!(self, Role::Viewer)
    }
    pub fn parse(s: &str) -> Option<Role> {
        match s.to_ascii_lowercase().as_str() {
            "admin" => Some(Role::Admin),
            "editor" => Some(Role::Editor),
            "viewer" => Some(Role::Viewer),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "t")]
pub enum ClientMsg {
    Hello {
        protocol: u32,
        user: String,
        app: String,
        machine: String,
        instance: String,
    },
    Login {
        proof: String,
    },
    ChangePassword {
        old_proof: String,
        new_password: String,
    },
    ListBooks,
    /// A new shared workbook, from an .xlsx (base64) or empty. `source`: the file on someone's
    /// PC or network drive it was shared from, which that person's app keeps saving to.
    CreateBook {
        name: String,
        #[serde(default)]
        xlsx: Option<String>,
        #[serde(default)]
        source: Option<FileSource>,
        /// [`sha256_hex`] of the .xlsx: the server refuses an upload that arrived damaged.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sha256: Option<String>,
    },
    OpenBook {
        book: String,
    },
    CloseBook {
        book: String,
    },
    RenameBook {
        book: String,
        name: String,
    },
    /// Deletes a shared workbook (kept in the server's trash folder). `stopped`: its file was
    /// shared and the person stops sharing it; the file stays where it is.
    DeleteBook {
        book: String,
        #[serde(default)]
        stopped: bool,
    },
    Patch {
        book: String,
        cseq: u64,
        patch: WbPatch,
        /// The last edit the client had when it made this one: a cell someone else changed after
        /// it is a conflict, reported to both people (the later edit still wins).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        base: Option<u64>,
    },
    Presence {
        book: String,
        sheet: String,
        range: String,
        #[serde(default)]
        editing: bool,
    },
    /// Edits of one cell (`cell` given) or of the workbook, newest first.
    History {
        book: String,
        #[serde(default)]
        sheet: Option<String>,
        #[serde(default)]
        cell: Option<String>,
        #[serde(default)]
        user: Option<String>,
        limit: usize,
    },
    Versions {
        book: String,
    },
    SaveVersion {
        book: String,
        label: String,
    },
    GetVersion {
        book: String,
        version: String,
    },
    RestoreVersion {
        book: String,
        version: String,
    },
    /// Is there a newer Dexterity Sheets than `version`?
    CheckUpdate {
        version: String,
    },
    /// The installer for `version`.
    GetUpdate {
        version: String,
    },
    Ping,
}

/// A newer app version the server can hand out.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UpdateInfo {
    pub version: String,
    /// What changed (Markdown or plain text).
    pub notes: String,
    pub file: String,
    pub size: u64,
    pub sha256: String,
}

/// Where a shared workbook lives as a file: shared from a folder on someone's PC or a network
/// drive, it stays that file. The person who shared it keeps it saved there.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileSource {
    /// The path as the person who shared it sees it (`D:\Projects\Line list.xlsx`).
    pub path: String,
    /// Identifies the file across PCs: the network (UNC) path for a file on a network drive,
    /// otherwise the PC's name and the path. Lower case. See [`file_key`].
    pub key: String,
    /// The account that shared it; that person's app writes the file.
    pub user: String,
    pub machine: String,
}

/// [`FileSource::key`] of a file: `\\server\share\dir\file.xlsx` for a network file (a mapped
/// drive letter resolved to its network path, so every PC agrees), `machine|d:\dir\file.xlsx`
/// for a file on this PC's own disk. `canonical` is the path as `std::fs::canonicalize` gives it.
pub fn file_key(machine: &str, canonical: &str) -> String {
    let p = canonical.replace('/', "\\");
    let p = if let Some(rest) = p.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = p.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        p
    };
    if p.starts_with(r"\\") { p.to_lowercase() } else { format!("{}|{}", machine.to_lowercase(), p.to_lowercase()) }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BookInfo {
    pub id: String,
    pub name: String,
    pub created: i64,
    pub created_by: String,
    pub modified: i64,
    pub modified_by: String,
    pub seq: u64,
    /// Display names of the people who have it open.
    pub open_by: Vec<String>,
    /// The file it was shared from, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Box<FileSource>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Peer {
    pub user: String,
    pub display: String,
    pub color: [u8; 3],
    pub sheet: String,
    pub range: String,
    #[serde(default)]
    pub editing: bool,
    /// One entry per connection, so the same person on two machines shows twice.
    pub conn: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Logged {
    pub seq: u64,
    pub time: i64,
    pub user: String,
    /// `<client instance>:<client sequence>` of the edit, so a client that reconnects can tell
    /// which of its edits the server already ordered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    pub patch: WbPatch,
}

/// Who last changed a cell's content.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LastEdit {
    pub sheet: String,
    pub r: u32,
    pub c: u32,
    pub user: String,
    pub time: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Change {
    pub seq: u64,
    pub time: i64,
    pub user: String,
    pub display: String,
    pub sheet: String,
    /// A1 address, or "" for a change that is not one cell (a sheet added, a column width).
    pub cell: String,
    pub old: String,
    pub new: String,
}

/// A same-cell conflict: `user` changed the cell (to `theirs`) after the editor last saw it,
/// and the editor's edit replaced that. The replaced value stays in the cell's history.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Conflict {
    pub sheet: String,
    pub cell: String,
    pub user: String,
    pub display: String,
    pub theirs: String,
    pub yours: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VersionInfo {
    pub id: String,
    pub seq: u64,
    pub time: i64,
    pub user: String,
    pub label: String,
    pub size: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserInfo {
    pub user: String,
    pub display: String,
    pub role: Role,
    pub color: [u8; 3],
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "t")]
pub enum ServerMsg {
    Challenge {
        salt: String,
        iterations: u32,
        nonce: String,
        server: String,
    },
    Welcome {
        me: UserInfo,
        conn: u64,
        users: Vec<UserInfo>,
    },
    Books {
        books: Vec<BookInfo>,
    },
    /// A workbook as of `snapshot_seq` (an .xlsx, base64) plus the edits since, up to `seq`.
    Opened {
        book: BookInfo,
        snapshot_seq: u64,
        xlsx: String,
        log: Vec<Logged>,
        last_edits: Vec<LastEdit>,
        peers: Vec<Peer>,
        /// [`sha256_hex`] of the decoded snapshot, so a damaged download is never opened as the
        /// workbook. Empty from servers older than this field.
        #[serde(default, skip_serializing_if = "String::is_empty")]
        sha256: String,
    },
    Patched {
        book: String,
        seq: u64,
        time: i64,
        user: String,
        #[serde(default)]
        origin: Option<String>,
        patch: WbPatch,
        /// Cells this edit changed after someone else had changed them since the editor last
        /// saw the workbook. The edit is applied (the later one wins); both people are told.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        conflicts: Vec<Conflict>,
    },
    /// The server could not take a patch; the client reloads the workbook.
    Rejected {
        book: String,
        cseq: u64,
        reason: String,
    },
    PeerMoved {
        book: String,
        peer: Peer,
    },
    PeerLeft {
        book: String,
        conn: u64,
    },
    History {
        book: String,
        changes: Vec<Change>,
    },
    Versions {
        book: String,
        versions: Vec<VersionInfo>,
    },
    Version {
        book: String,
        version: String,
        xlsx: String,
    },
    /// The workbook was replaced (a version restored): reopen it.
    Reload {
        book: String,
        reason: String,
    },
    /// The workbook was deleted, or its sharing stopped (`by`: who, as a display name).
    BookGone {
        book: String,
        #[serde(default)]
        by: Option<String>,
        #[serde(default)]
        stopped: bool,
    },
    /// Sent after `CheckUpdate`, and to everyone when the server gets a newer version. `None`:
    /// the app is up to date.
    Update {
        update: Option<UpdateInfo>,
    },
    /// The installer, base64.
    UpdateFile {
        info: UpdateInfo,
        data: String,
    },
    Ok {
        what: String,
    },
    Error {
        message: String,
    },
    Pong,
}

pub fn write_frame<W: Write, T: Serialize>(w: &mut W, msg: &T) -> Result<()> {
    let body = serde_json::to_vec(msg)?;
    let (flag, body) = if body.len() > COMPRESS_OVER {
        let mut enc = flate2::write::DeflateEncoder::new(Vec::with_capacity(body.len() / 4), flate2::Compression::fast());
        enc.write_all(&body)?;
        (1u8, enc.finish()?)
    } else {
        (0u8, body)
    };
    if body.len() > MAX_FRAME {
        return Err(Error::Protocol("message too large".into()));
    }
    let len = u32::try_from(body.len()).map_err(|_| Error::Protocol("message too large".into()))?;
    let mut buf = Vec::with_capacity(body.len() + 5);
    buf.extend_from_slice(&len.to_be_bytes());
    buf.push(flag);
    buf.extend_from_slice(&body);
    w.write_all(&buf)?;
    w.flush()?;
    Ok(())
}

pub fn read_frame<R: Read, T: for<'de> Deserialize<'de>>(r: &mut R) -> Result<T> {
    let mut head = [0u8; 5];
    r.read_exact(&mut head)?;
    let len = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
    if len > MAX_FRAME {
        return Err(Error::Protocol(format!("frame of {len} bytes refused")));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    let body = if head[4] == 1 {
        let mut out = Vec::new();
        flate2::read::DeflateDecoder::new(body.as_slice()).take(MAX_FRAME as u64 * 4).read_to_end(&mut out)?;
        out
    } else {
        body
    };
    Ok(serde_json::from_slice(&body)?)
}

pub fn b64(bytes: &[u8]) -> String {
    gridcraft_engine::io::base64_encode(bytes)
}

pub fn unb64(s: &str) -> Result<Vec<u8>> {
    gridcraft_engine::io::base64_decode(s).ok_or_else(|| Error::Protocol("bad base64".into()))
}

/// SHA-256 of `bytes` in lower-case hex: the integrity check on workbook uploads and downloads.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    crate::auth::hex(&sha2::Sha256::digest(bytes))
}

/// Decodes a workbook sent as base64 and checks it against its SHA-256 (when one was sent).
pub fn unb64_checked(s: &str, sha256: &str) -> Result<Vec<u8>> {
    let bytes = unb64(s)?;
    if !sha256.is_empty() && !sha256_hex(&bytes).eq_ignore_ascii_case(sha256) {
        return Err(Error::Protocol("the workbook arrived damaged (its checksum does not match)".into()));
    }
    Ok(bytes)
}

/// Distinct, readable colours for people's cursors.
pub const PALETTE: [[u8; 3]; 12] = [
    [0x1f, 0x77, 0xb4],
    [0xd6, 0x27, 0x28],
    [0x2c, 0xa0, 0x2c],
    [0x94, 0x67, 0xbd],
    [0xff, 0x7f, 0x0e],
    [0x17, 0xbe, 0xcf],
    [0x8c, 0x56, 0x4b],
    [0xe3, 0x77, 0xc2],
    [0x7f, 0x7f, 0x00],
    [0x00, 0x6d, 0x77],
    [0xb0, 0x3a, 0x2e],
    [0x3b, 0x4c, 0xc0],
];

/// The client sequence number in `origin` when the edit came from `instance`.
pub fn own_cseq(origin: Option<&str>, instance: &str) -> Option<u64> {
    let (inst, c) = origin?.rsplit_once(':')?;
    (inst == instance).then(|| c.parse().ok()).flatten()
}

/// Compares dotted version numbers ("0.12.10" > "0.12.9"); a pre-release tag sorts before the release.
pub fn version_newer(candidate: &str, current: &str) -> bool {
    fn parts(v: &str) -> (Vec<u64>, bool) {
        let v = v.trim().trim_start_matches(['v', 'V']);
        let (core, pre) = match v.split_once('-') {
            Some((c, _)) => (c, true),
            None => (v, false),
        };
        (core.split('.').map(|p| p.parse().unwrap_or(0)).collect(), pre)
    }
    let (a, apre) = parts(candidate);
    let (b, bpre) = parts(current);
    let n = a.len().max(b.len());
    for i in 0..n {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    bpre && !apre
}
