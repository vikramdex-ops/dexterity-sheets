//! Who is using this PC, and this PC's device key.
//!
//! The person is the Windows account they are signed in to (`DOMAIN\user`; on a PC that is not in
//! a domain, `PCNAME\user`). Nobody creates an account or types a password: the office server
//! learns the person from the account name and trusts the device key it sees first for them.
//!
//! The device key lives in `%LOCALAPPDATA%\Dexterity Sheets\device-key`, protected with Windows
//! DPAPI for the current Windows user (through PowerShell's `ProtectedData`, since this code uses
//! no `unsafe`). If DPAPI is not available the key is kept as a plain file in the same per-user
//! folder, and that is logged.

use std::path::{Path, PathBuf};

use crate::auth::DeviceKey;
use crate::{Error, Result};

/// The Windows account: `account` as Windows spells it (`DDS\Vikram`), `display` the name shown
/// to others until the person changes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub account: String,
    pub display: String,
}

/// The signed-in Windows account, from the environment (`USERDOMAIN`, `USERNAME`). Off Windows,
/// `USER`. `None` when nothing names the person.
pub fn windows_account() -> Option<Account> {
    let var = |k: &str| std::env::var(k).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    let user = var("USERNAME").or_else(|| var("USER"))?;
    let account = match var("USERDOMAIN") {
        Some(domain) => format!("{domain}\\{user}"),
        None => user.clone(),
    };
    valid_account(&account).then_some(Account { account, display: user })
}

/// The key the server files a person under: the account in lower case.
pub fn account_key(account: &str) -> String {
    account.trim().to_lowercase()
}

/// The user-name part of an account (`DDS\Vikram` → `Vikram`).
pub fn user_part(account: &str) -> &str {
    account.rsplit_once('\\').map_or(account, |(_, u)| u)
}

/// A Windows account name the server accepts: `user` or `DOMAIN\user`, no control characters
/// and none of the characters Windows forbids in names.
pub fn valid_account(a: &str) -> bool {
    let parts: Vec<&str> = a.split('\\').collect();
    !a.is_empty()
        && a.chars().count() <= 128
        && a.trim() == a
        && parts.len() <= 2
        && parts.iter().all(|p| !p.trim().is_empty())
        && a.chars().all(|c| !c.is_control() && !"/:*?\"<>|[]=+;,@".contains(c))
}

/// Where this PC's device key is kept: `%LOCALAPPDATA%\Dexterity Sheets\device-key`.
pub fn default_key_path() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA").or_else(|| std::env::var_os("HOME"))?;
    Some(PathBuf::from(base).join("Dexterity Sheets").join("device-key"))
}

/// How the key file is protected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protection {
    /// Encrypted with DPAPI for this Windows user.
    Dpapi,
    /// A plain file in the user's own folder (DPAPI was not available).
    Plain,
}

const DPAPI_TAG: &str = "dpapi:";
const PLAIN_TAG: &str = "plain:";

/// Loads this PC's device key, making and saving one the first time. Blocking (it may start
/// PowerShell): call it off the UI thread.
pub fn load_or_create(path: &Path) -> Result<(DeviceKey, Protection)> {
    if let Ok(text) = std::fs::read_to_string(path) {
        let text = text.trim();
        if let Some(b64) = text.strip_prefix(DPAPI_TAG) {
            let sealed = crate::proto::unb64(b64)?;
            let pkcs8 = dpapi(&sealed, false)?;
            return Ok((DeviceKey::from_pkcs8(&pkcs8)?, Protection::Dpapi));
        }
        if let Some(b64) = text.strip_prefix(PLAIN_TAG) {
            return Ok((DeviceKey::from_pkcs8(&crate::proto::unb64(b64)?)?, Protection::Plain));
        }
        // Anything else is not ours: keep it aside rather than overwrite it.
        let _ = std::fs::rename(path, path.with_extension("damaged"));
        log::warn!("the device key at {} was unreadable; it was set aside and a new key made", path.display());
    }
    let key = DeviceKey::generate()?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let (text, protection) = match dpapi(key.pkcs8(), true) {
        Ok(sealed) => (format!("{DPAPI_TAG}{}", crate::proto::b64(&sealed)), Protection::Dpapi),
        Err(e) => {
            log::warn!("Windows data protection is not available ({e}); the device key is kept as a plain file in {}", path.display());
            (format!("{PLAIN_TAG}{}", crate::proto::b64(key.pkcs8())), Protection::Plain)
        }
    };
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)?;
    Ok((key, protection))
}

/// Protects (`seal`) or unprotects bytes with DPAPI for the current Windows user, through
/// PowerShell. The bytes go through standard input, never the command line.
#[cfg(windows)]
fn dpapi(bytes: &[u8], seal: bool) -> Result<Vec<u8>> {
    use std::io::Write;
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let op = if seal { "Protect" } else { "Unprotect" };
    let script = format!(
        "$ErrorActionPreference='Stop'; Add-Type -AssemblyName System.Security; \
         $b=[Convert]::FromBase64String([Console]::In.ReadToEnd().Trim()); \
         $r=[Security.Cryptography.ProtectedData]::{op}($b,$null,[Security.Cryptography.DataProtectionScope]::CurrentUser); \
         [Console]::Out.Write([Convert]::ToBase64String($r))"
    );
    let root = std::env::var_os("SystemRoot").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
    let exe = root.join("System32").join("WindowsPowerShell").join("v1.0").join("powershell.exe");
    let mut child = Command::new(if exe.exists() { exe } else { PathBuf::from("powershell.exe") })
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", &script])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(crate::proto::b64(bytes).as_bytes())?;
    }
    let out = child.wait_with_output()?;
    if !out.status.success() {
        let why = String::from_utf8_lossy(&out.stderr).lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_string();
        return Err(Error::Protocol(format!("Windows data protection failed: {why}")));
    }
    crate::proto::unb64(String::from_utf8_lossy(&out.stdout).trim())
}

#[cfg(not(windows))]
fn dpapi(_bytes: &[u8], _seal: bool) -> Result<Vec<u8>> {
    Err(Error::Protocol("Windows data protection exists only on Windows".into()))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn account_names_windows_accepts_and_the_server_files_under() {
        assert!(valid_account(r"DDS\Vikram"));
        assert!(valid_account("vikram"));
        assert!(valid_account(r"PC-01\John Smith"));
        for bad in ["", r"\vikram", r"DDS\", r"a\b\c", "a/b", "x\u{7}", " lead", "who?"] {
            assert!(!valid_account(bad), "{bad:?}");
        }
        assert_eq!(account_key(r"DDS\Vikram"), r"dds\vikram");
        assert_eq!(user_part(r"DDS\Vikram"), "Vikram");
        assert_eq!(user_part("vikram"), "vikram");
    }

    #[test]
    fn the_device_key_is_made_once_and_read_back() {
        let dir = std::env::temp_dir().join(format!("dexsheets-key-{}-{}", std::process::id(), crate::now_ms()));
        let path = dir.join("device-key");
        let (k1, how) = load_or_create(&path).expect("first");
        let text = std::fs::read_to_string(&path).expect("saved");
        assert!(text.starts_with(DPAPI_TAG) || text.starts_with(PLAIN_TAG), "{text}");
        if cfg!(windows) && how == Protection::Dpapi {
            // DPAPI really sealed it: the key bytes are not in the file.
            assert!(!text.contains(&crate::proto::b64(k1.pkcs8())));
        }
        let (k2, _) = load_or_create(&path).expect("again");
        assert_eq!(k1.public_key(), k2.public_key(), "the same key every time");
        // A damaged file is set aside, never a crash, and a new key is made.
        std::fs::write(&path, "nonsense").expect("damage");
        let (k3, _) = load_or_create(&path).expect("new");
        assert_ne!(k3.public_key(), k1.public_key());
        assert!(path.with_extension("damaged").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
