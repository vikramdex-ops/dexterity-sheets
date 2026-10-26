//! Dexterity Sheets LAN co-authoring.
//!
//! One server on the office network keeps each shared workbook, orders every edit and stores it
//! durably with who made it. Clients send what their edits changed as [`patch::WbPatch`]es and
//! apply everyone's patches in the server's order ([`sync`]), so all replicas converge. Every
//! connection is a named account: there is no anonymous access.
//!
//! - [`patch`]: diff and apply workbook changes.
//! - [`proto`]: messages and framing.
//! - [`auth`]: password hashing and the login challenge.
//! - [`client`]: the client connection (background threads, reconnects).
//! - [`sync`]: keeping an open workbook in step with the server.
//! - [`server`]: the server.
//! - [`discovery`]: finding servers on the LAN.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod auth;
pub mod client;
pub mod discovery;
pub mod patch;
pub mod proto;
pub mod server;
pub mod sync;

#[cfg(test)]
mod tests;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Json(serde_json::Error),
    Protocol(String),
    Engine(String),
    Denied(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "network or disk error: {e}"),
            Error::Json(e) => write!(f, "malformed data: {e}"),
            Error::Protocol(e) => write!(f, "protocol error: {e}"),
            Error::Engine(e) => write!(f, "{e}"),
            Error::Denied(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}
impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Json(e)
    }
}
impl From<gridcraft_engine::EngineError> for Error {
    fn from(e: gridcraft_engine::EngineError) -> Self {
        Error::Engine(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
