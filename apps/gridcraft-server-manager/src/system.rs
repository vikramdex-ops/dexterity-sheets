//! What the manager does to the PC: the Windows service, administrator rights, the firewall,
//! network addresses, the data folder (size, log, update installers) and backups.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

pub const SERVICE: &str = "DexSheetsServer";
pub const PORT: u16 = gridcraft_collab::proto::DEFAULT_PORT;
pub const DISCOVERY_PORT: u16 = gridcraft_collab::proto::DISCOVERY_PORT;
pub const FIREWALL_TCP: &str = "Dexterity Sheets Server (TCP)";
pub const FIREWALL_UDP: &str = "Dexterity Sheets Discovery (UDP)";

pub fn default_data_dir() -> PathBuf {
    let base = std::env::var_os("ProgramData").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    base.join("Dexterity Sheets").join("Server")
}

fn system32(exe: &str) -> PathBuf {
    std::env::var_os("SystemRoot").map_or_else(|| PathBuf::from(exe), |root| PathBuf::from(root).join("System32").join(exe))
}

/// A console tool run without flashing a console window.
fn quiet(exe: &str) -> Command {
    #[allow(unused_mut)]
    let mut c = Command::new(system32(exe));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    c
}

/// Whether this process runs with administrator rights (`net session` only works elevated).
pub fn is_admin() -> bool {
    cfg!(windows) && quiet("net.exe").arg("session").output().is_ok_and(|o| o.status.success())
}

/// Starts this program again with administrator rights (Windows asks the person). `false` when
/// they said no or it could not be asked.
pub fn relaunch_elevated(args: &[String]) -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let quote = |s: &str| format!("'{}'", s.replace('\'', "''"));
    let list = if args.is_empty() {
        String::new()
    } else {
        format!(" -ArgumentList {}", args.iter().map(|a| quote(&format!("\"{a}\""))).collect::<Vec<_>>().join(","))
    };
    let script = format!("try {{ Start-Process -FilePath {}{list} -Verb RunAs; exit 0 }} catch {{ exit 1 }}", quote(&exe.display().to_string()));
    quiet(r"WindowsPowerShell\v1.0\powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", &script])
        .status()
        .is_ok_and(|s| s.success())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServiceState {
    NotInstalled,
    Stopped,
    Starting,
    Running,
    Stopping,
    Unknown(String),
}

#[cfg(windows)]
mod svc {
    use super::{SERVICE, ServiceState};
    use windows_service::service::{ServiceAccess, ServiceState as S};
    use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

    fn open(access: ServiceAccess) -> Result<Option<windows_service::service::Service>, String> {
        let m = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT).map_err(|e| e.to_string())?;
        match m.open_service(SERVICE, access) {
            Ok(s) => Ok(Some(s)),
            // ERROR_SERVICE_DOES_NOT_EXIST
            Err(windows_service::Error::Winapi(e)) if e.raw_os_error() == Some(1060) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    pub fn state() -> ServiceState {
        match open(ServiceAccess::QUERY_STATUS) {
            Ok(None) => ServiceState::NotInstalled,
            Ok(Some(s)) => match s.query_status() {
                Ok(st) => match st.current_state {
                    S::Running => ServiceState::Running,
                    S::Stopped => ServiceState::Stopped,
                    S::StartPending | S::ContinuePending => ServiceState::Starting,
                    S::StopPending | S::PausePending | S::Paused => ServiceState::Stopping,
                },
                Err(e) => ServiceState::Unknown(e.to_string()),
            },
            Err(e) => ServiceState::Unknown(e),
        }
    }

    pub fn start() -> Result<(), String> {
        let s = open(ServiceAccess::START)?.ok_or("the server service is not installed: run the server installer")?;
        s.start(&[] as &[&std::ffi::OsStr]).map_err(|e| e.to_string())
    }

    pub fn stop() -> Result<(), String> {
        let s = open(ServiceAccess::STOP)?.ok_or("the server service is not installed")?;
        s.stop().map(|_| ()).map_err(|e| e.to_string())
    }
}

#[cfg(not(windows))]
mod svc {
    use super::ServiceState;
    pub fn state() -> ServiceState {
        ServiceState::NotInstalled
    }
    pub fn start() -> Result<(), String> {
        Err("the server service runs on Windows".into())
    }
    pub fn stop() -> Result<(), String> {
        Err("the server service runs on Windows".into())
    }
}

pub fn service_state() -> ServiceState {
    svc::state()
}

fn wait_for(want: &ServiceState, secs: u64) -> bool {
    for _ in 0..secs * 4 {
        if &service_state() == want {
            return true;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    false
}

pub fn start_service() -> Result<(), String> {
    svc::start()?;
    if wait_for(&ServiceState::Running, 30) { Ok(()) } else { Err("the service did not start within 30 seconds: see the server log".into()) }
}

pub fn stop_service() -> Result<(), String> {
    if service_state() == ServiceState::Stopped {
        return Ok(());
    }
    svc::stop()?;
    if wait_for(&ServiceState::Stopped, 30) { Ok(()) } else { Err("the service did not stop within 30 seconds".into()) }
}

pub fn restart_service() -> Result<(), String> {
    stop_service()?;
    start_service()
}

/// Whether something answers on the server's port on this PC.
pub fn port_answers(port: u16) -> bool {
    std::net::TcpStream::connect_timeout(&std::net::SocketAddr::from(([127, 0, 0, 1], port)), Duration::from_millis(400)).is_ok()
}

/// This PC's IPv4 addresses on the network (what teammates can type if the server isn't found).
pub fn addresses() -> Vec<String> {
    use std::net::ToSocketAddrs;
    let host = std::env::var("COMPUTERNAME").unwrap_or_default();
    let mut out: Vec<String> = (host.as_str(), 0)
        .to_socket_addrs()
        .map(|a| a.filter(|a| a.is_ipv4() && !a.ip().is_loopback()).map(|a| a.ip().to_string()).collect())
        .unwrap_or_default();
    out.sort();
    out.dedup();
    out
}

pub fn firewall_rule_exists(name: &str) -> bool {
    quiet("netsh.exe").args(["advfirewall", "firewall", "show", "rule"]).arg(format!("name={name}")).output().is_ok_and(|o| o.status.success())
}

/// Opens the server's ports for the office network (Domain and Private profiles), as the installer
/// does; replaces rules of the same name.
pub fn open_firewall(server_exe: &Path) -> Result<(), String> {
    for (name, proto, port) in [(FIREWALL_TCP, "TCP", PORT), (FIREWALL_UDP, "UDP", DISCOVERY_PORT)] {
        let _ = quiet("netsh.exe").args(["advfirewall", "firewall", "delete", "rule"]).arg(format!("name={name}")).output();
        let o = quiet("netsh.exe")
            .args(["advfirewall", "firewall", "add", "rule"])
            .arg(format!("name={name}"))
            .args(["dir=in", "action=allow", "profile=domain,private"])
            .arg(format!("protocol={proto}"))
            .arg(format!("localport={port}"))
            .arg(format!("program={}", server_exe.display()))
            .output()
            .map_err(|e| e.to_string())?;
        if !o.status.success() {
            return Err(format!("netsh: {}", String::from_utf8_lossy(&o.stdout).trim()));
        }
    }
    Ok(())
}

/// `DexSheetsServer.exe` next to this program (both are installed in the same folder).
pub fn server_exe() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("DexSheetsServer.exe")))
        .unwrap_or_else(|| PathBuf::from("DexSheetsServer.exe"))
}

pub fn open_in_explorer(path: &Path) {
    let explorer = std::env::var_os("SystemRoot").map_or_else(|| PathBuf::from("explorer.exe"), |r| PathBuf::from(r).join("explorer.exe"));
    // A file is shown selected in its folder; a folder opens.
    let _ = if path.is_file() {
        Command::new(explorer).arg(format!("/select,{}", path.display())).spawn()
    } else {
        Command::new(explorer).arg(path).spawn()
    };
}

/// Opens a web address in the default browser, as Dexterity Sheets opens links: only `http` and
/// `https` addresses that pass the app's link check are handed to the system.
pub fn open_url(url: &str) -> Result<(), String> {
    let gridcraft_engine::links::LinkKind::External(url) = gridcraft_engine::links::classify(url) else {
        return Err("That address can't be opened.".into());
    };
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("https://") || lower.starts_with("http://")) {
        return Err("Only web addresses are opened.".into());
    }
    let cmd = if cfg!(windows) {
        std::env::var_os("SystemRoot").map_or_else(|| PathBuf::from("explorer.exe"), |r| PathBuf::from(r).join("explorer.exe"))
    } else if cfg!(target_os = "macos") {
        PathBuf::from("open")
    } else {
        PathBuf::from("xdg-open")
    };
    Command::new(&cmd).arg(&url).spawn().map(|_| ()).map_err(|e| format!("The browser could not be opened: {e}"))
}

/// The last `lines` lines of the server log.
pub fn log_tail(dir: &Path, lines: usize) -> String {
    let Ok(mut f) = std::fs::File::open(dir.join("server.log")) else { return String::new() };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    // Only the end of a long log is read.
    let from = len.saturating_sub(256 * 1024);
    let mut buf = Vec::new();
    use std::io::Seek;
    let _ = f.seek(std::io::SeekFrom::Start(from));
    let _ = f.take(256 * 1024).read_to_end(&mut buf);
    let text = String::from_utf8_lossy(&buf);
    let all: Vec<&str> = text.lines().collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

/// Total size of a folder, in bytes.
pub fn folder_size(dir: &Path) -> u64 {
    let mut total = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            match e.file_type() {
                Ok(t) if t.is_dir() => stack.push(e.path()),
                Ok(_) => total += e.metadata().map(|m| m.len()).unwrap_or(0),
                Err(_) => {}
            }
        }
    }
    total
}

pub fn human_size(bytes: u64) -> String {
    let b = bytes as f64;
    if b >= 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1} GB", b / (1024.0 * 1024.0 * 1024.0))
    } else if b >= 1024.0 * 1024.0 {
        format!("{:.1} MB", b / (1024.0 * 1024.0))
    } else {
        format!("{:.0} KB", (b / 1024.0).ceil())
    }
}

/// How many shared workbooks the server holds.
pub fn workbook_count(dir: &Path) -> usize {
    std::fs::read_dir(dir.join("books")).map(|d| d.flatten().filter(|e| e.path().join("book.json").exists()).count()).unwrap_or(0)
}

/// An installer in the updates folder.
#[derive(Clone, Debug)]
pub struct Installer {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub notes: String,
}

pub fn installers(dir: &Path) -> Vec<Installer> {
    let mut out: Vec<Installer> = std::fs::read_dir(dir.join("updates"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| gridcraft_collab::server::is_app_installer(&e.file_name().to_string_lossy()))
        .map(|e| {
            let path = e.path();
            let notes = ["md", "txt"].iter().find_map(|x| std::fs::read_to_string(path.with_extension(x)).ok()).unwrap_or_default();
            Installer { name: e.file_name().to_string_lossy().into_owned(), size: e.metadata().map(|m| m.len()).unwrap_or(0), path, notes }
        })
        .collect();
    out.sort_by(|a, b| b.name.cmp(&a.name));
    out
}

/// Copies an app installer into the updates folder, with optional release notes next to it.
pub fn add_installer(dir: &Path, from: &Path, notes: &str) -> Result<String, String> {
    let name = from.file_name().map(|n| n.to_string_lossy().into_owned()).ok_or("no file name")?;
    if !gridcraft_collab::server::is_app_installer(&name) {
        return Err(format!("{name} is not a Dexterity Sheets app installer (.msi). The server's own installer can't be offered to office PCs."));
    }
    let has_version =
        name.split(['-', '_']).any(|p| p.trim_end_matches(".msi").split('.').count() >= 2 && p.starts_with(|c: char| c.is_ascii_digit()));
    if !has_version {
        return Err(format!("{name}: the file name must carry the version, as released (dexterity-sheets-1.2.0-windows-x64.msi)."));
    }
    let updates = dir.join("updates");
    std::fs::create_dir_all(&updates).map_err(|e| e.to_string())?;
    let to = updates.join(&name);
    let part = updates.join(format!("{name}.part"));
    std::fs::copy(from, &part).map_err(|e| e.to_string())?;
    if !notes.trim().is_empty() {
        std::fs::write(to.with_extension("md"), notes.trim()).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&part, &to).map_err(|e| e.to_string())?;
    Ok(name)
}

pub fn remove_installer(i: &Installer) -> Result<(), String> {
    std::fs::remove_file(&i.path).map_err(|e| e.to_string())?;
    for x in ["md", "txt"] {
        let _ = std::fs::remove_file(i.path.with_extension(x));
    }
    Ok(())
}

/// `update-source.json`: fetch new app versions from GitHub releases.
pub fn update_source(dir: &Path) -> (String, String) {
    let v: serde_json::Value = std::fs::read(dir.join("update-source.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let s = |k: &str| v.get(k).and_then(serde_json::Value::as_str).unwrap_or("").to_string();
    (s("repo"), s("token"))
}

pub fn set_update_source(dir: &Path, repo: &str, token: &str) -> Result<(), String> {
    let path = dir.join("update-source.json");
    if repo.trim().is_empty() {
        return match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.to_string()),
        };
    }
    let repo = repo.trim().trim_start_matches("https://github.com/").trim_end_matches('/');
    if repo.split('/').count() != 2 || repo.split('/').any(str::is_empty) {
        return Err("Write the repository as owner/name (vikramdex-ops/dexterity-sheets).".into());
    }
    // Other settings in the file (a custom check interval) are kept.
    let mut v: serde_json::Value = std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .filter(serde_json::Value::is_object)
        .unwrap_or_else(|| serde_json::json!({}));
    v["repo"] = serde_json::Value::String(repo.to_string());
    if token.trim().is_empty() {
        if let Some(o) = v.as_object_mut() {
            o.remove("token");
        }
    } else {
        v["token"] = serde_json::Value::String(token.trim().to_string());
    }
    write_atomic(&path, &serde_json::to_vec_pretty(&v).map_err(|e| e.to_string())?)
}

/// The server's display name (`server.json`), as the server reads it.
pub fn server_name(dir: &Path) -> String {
    std::fs::read(dir.join("server.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|v| v.get("name").and_then(serde_json::Value::as_str).map(str::to_string))
        .unwrap_or_default()
}

pub fn set_server_name(dir: &Path, name: &str) -> Result<(), String> {
    write_atomic(&dir.join("server.json"), &serde_json::to_vec_pretty(&serde_json::json!({"name": name.trim()})).map_err(|e| e.to_string())?)
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Zips the data folder (accounts, workbooks, edit history, versions, settings) into `to`.
/// App installers are left out unless asked: they can be downloaded again.
pub fn backup(dir: &Path, to: &Path, with_installers: bool) -> Result<u64, String> {
    let file = std::fs::File::create(to).map_err(|e| format!("{}: {e}", to.display()))?;
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated).large_file(true);
    let mut count = 0;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).map_err(|e| e.to_string())?.flatten() {
            let path = e.path();
            let rel = path.strip_prefix(dir).map_err(|e| e.to_string())?.to_string_lossy().replace('\\', "/");
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                stack.push(path);
                continue;
            }
            let lower = rel.to_ascii_lowercase();
            if lower.ends_with(".part") || lower.ends_with(".tmp") || (!with_installers && lower.starts_with("updates/") && lower.ends_with(".msi")) {
                continue;
            }
            // The log may be in use by the running server: copy what can be read.
            let mut f = match std::fs::File::open(&path) {
                Ok(f) => f,
                Err(e) if lower == "server.log" => {
                    let _ = e;
                    continue;
                }
                Err(e) => return Err(format!("{rel}: {e}")),
            };
            zip.start_file(rel, opts).map_err(|e| e.to_string())?;
            std::io::copy(&mut f, &mut zip).map_err(|e| e.to_string())?;
            count += 1;
        }
    }
    zip.finish().map_err(|e| e.to_string())?.flush().map_err(|e| e.to_string())?;
    Ok(count)
}

/// Replaces the data folder with a backup. The current folder is kept beside it
/// (`Server.before-restore-<time>`), so a restore can be undone by hand. The service must be
/// stopped first.
pub fn restore(dir: &Path, from: &Path) -> Result<PathBuf, String> {
    let file = std::fs::File::open(from).map_err(|e| format!("{}: {e}", from.display()))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("not a backup: {e}"))?;
    if zip.by_name("users.json").is_err() {
        return Err("This zip is not a Dexterity Sheets Server backup (it has no users.json).".into());
    }
    let parent = dir.parent().ok_or("the data folder has no parent folder")?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let fresh = parent.join(format!("Server.restoring-{stamp}"));
    std::fs::create_dir_all(&fresh).map_err(|e| e.to_string())?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| e.to_string())?;
        // Never outside the folder (a crafted "..\" name).
        let Some(rel) = entry.enclosed_name() else { continue };
        let out = fresh.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(p) = out.parent() {
            std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        let mut f = std::fs::File::create(&out).map_err(|e| e.to_string())?;
        std::io::copy(&mut entry, &mut f).map_err(|e| e.to_string())?;
    }
    let old = parent.join(format!("Server.before-restore-{stamp}"));
    if dir.exists() {
        std::fs::rename(dir, &old).map_err(|e| format!("the current data folder could not be moved aside: {e}"))?;
    }
    if let Err(e) = std::fs::rename(&fresh, dir) {
        let _ = std::fs::rename(&old, dir);
        return Err(format!("the backup could not be put in place: {e}"));
    }
    Ok(old)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_and_restore_round_trip() {
        let root = std::env::temp_dir().join(format!("dexsheets-mgr-{}", gridcraft_collab::now_ms()));
        let dir = root.join("Server");
        std::fs::create_dir_all(dir.join("books").join("b1")).expect("dirs");
        std::fs::create_dir_all(dir.join("updates")).expect("dirs");
        std::fs::write(dir.join("users.json"), b"[]").expect("w");
        std::fs::write(dir.join("books").join("b1").join("book.json"), b"{}").expect("w");
        std::fs::write(dir.join("updates").join("dexterity-sheets-9.9.9-windows-x64.msi"), b"msi").expect("w");
        let zip = root.join("backup.zip");
        assert_eq!(backup(&dir, &zip, false).expect("backup"), 2, "installers left out");
        std::fs::write(dir.join("users.json"), b"changed").expect("w");
        let old = restore(&dir, &zip).expect("restore");
        assert_eq!(std::fs::read(dir.join("users.json")).expect("r"), b"[]");
        assert!(dir.join("books").join("b1").join("book.json").exists());
        assert_eq!(std::fs::read(old.join("users.json")).expect("r"), b"changed", "the replaced folder is kept");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_zip_without_accounts_is_refused() {
        let root = std::env::temp_dir().join(format!("dexsheets-mgr-bad-{}", gridcraft_collab::now_ms()));
        std::fs::create_dir_all(root.join("Server")).expect("dirs");
        let zip = root.join("x.zip");
        let mut w = zip::ZipWriter::new(std::fs::File::create(&zip).expect("f"));
        w.start_file("readme.txt", zip::write::SimpleFileOptions::default()).expect("s");
        w.write_all(b"hi").expect("w");
        w.finish().expect("finish");
        assert!(restore(&root.join("Server"), &zip).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn only_app_installers_are_accepted() {
        let root = std::env::temp_dir().join(format!("dexsheets-mgr-inst-{}", gridcraft_collab::now_ms()));
        std::fs::create_dir_all(&root).expect("dirs");
        let server = root.join("DexSheetsServer-1.2.0-x64.msi");
        std::fs::write(&server, b"x").expect("w");
        assert!(add_installer(&root, &server, "").is_err());
        let app = root.join("dexterity-sheets-1.2.0-windows-x64.msi");
        std::fs::write(&app, b"x").expect("w");
        assert_eq!(add_installer(&root, &app, "- notes").expect("add"), "dexterity-sheets-1.2.0-windows-x64.msi");
        assert_eq!(installers(&root).len(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }
}
