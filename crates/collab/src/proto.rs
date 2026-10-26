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
    Hello { protocol: u32, user: String, app: String, machine: String, instance: String },
    Login { proof: String },
    ChangePassword { old_proof: String, new_password: String },
    ListBooks,
    /// A new shared workbook, from an .xlsx (base64) or empty.
    CreateBook { name: String, #[serde(default)] xlsx: Option<String> },
    OpenBook { book: String },
    CloseBook { book: String },
    RenameBook { book: String, name: String },
    DeleteBook { book: String },
    Patch { book: String, cseq: u64, patch: WbPatch },
    Presence { book: String, sheet: String, range: String, #[serde(default)] editing: bool },
    /// Edits of one cell (`cell` given) or of the workbook, newest first.
    History { book: String, #[serde(default)] sheet: Option<String>, #[serde(default)] cell: Option<String>, #[serde(default)] user: Option<String>, limit: usize },
    Versions { book: String },
    SaveVersion { book: String, label: String },
    GetVersion { book: String, version: String },
    RestoreVersion { book: String, version: String },
    Ping,
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
    Challenge { salt: String, iterations: u32, nonce: String, server: String },
    Welcome { me: UserInfo, conn: u64, users: Vec<UserInfo> },
    Books { books: Vec<BookInfo> },
    /// A workbook as of `snapshot_seq` (an .xlsx, base64) plus the edits since, up to `seq`.
    Opened { book: BookInfo, snapshot_seq: u64, xlsx: String, log: Vec<Logged>, last_edits: Vec<LastEdit>, peers: Vec<Peer> },
    Patched { book: String, seq: u64, time: i64, user: String, #[serde(default)] origin: Option<String>, patch: WbPatch },
    /// The server could not take a patch; the client reloads the workbook.
    Rejected { book: String, cseq: u64, reason: String },
    PeerMoved { book: String, peer: Peer },
    PeerLeft { book: String, conn: u64 },
    History { book: String, changes: Vec<Change> },
    Versions { book: String, versions: Vec<VersionInfo> },
    Version { book: String, version: String, xlsx: String },
    /// The workbook was replaced (a version restored): reopen it.
    Reload { book: String, reason: String },
    BookGone { book: String },
    Ok { what: String },
    Error { message: String },
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
