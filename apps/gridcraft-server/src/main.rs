//! Dexterity Sheets server.
//!
//! ```text
//! DexSheetsServer                      run
//! DexSheetsServer service              run as the Windows service the installer registers
//! DexSheetsServer profile list         everyone seen, their role and devices (codes, approval)
//! DexSheetsServer profile role DOMAIN\user admin|editor|viewer
//! DexSheetsServer profile approve CODE | profile revoke CODE    a device, by its code ABCD-EFGH
//! DexSheetsServer profile remove DOMAIN\user
//! People sign in with their Windows account: the first PC each person uses is trusted, any
//! other PC waits for an administrator to approve it. The first person on a new server becomes
//! its administrator. No accounts or passwords to create.
//! Updates: drop DexteritySheets-<version>-x64.msi into <data>\updates, or configure
//! <data>\update-source.json {"repo": "owner/name", "token": "…"} to fetch GitHub releases.
//! Dexterity Connect: the read API answers on TCP 47812 (DEXSHEETS_API_PORT to change it) with a
//! token made in the Server Manager; notices go to the hub set there (<data>\connect.json).
//! options: --data DIR (default %ProgramData%\Dexterity Sheets\Server), --port N, --name "Server name"
//! (the name otherwise comes from <data>\server.json {"name": "…"}, set in the Server Manager)
//! Most offices never need these commands: the Server Manager app does all of it.
//! ```

use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use gridcraft_collab::proto::{DEFAULT_PORT, DeviceStatus, Role};
use gridcraft_collab::server;

mod connect;
mod github;
#[cfg(windows)]
mod service;

/// `server.log` is kept under this size: past it, it becomes `server.log.1` (replacing the one
/// before) and a new log starts, so a server that runs for years never fills its disk.
const LOG_MAX_BYTES: u64 = 16 * 1024 * 1024;

struct LogFile {
    path: PathBuf,
    file: Option<std::fs::File>,
    size: u64,
}

impl LogFile {
    fn open(path: PathBuf) -> LogFile {
        let file = std::fs::OpenOptions::new().create(true).append(true).open(&path).ok();
        let size = file.as_ref().and_then(|f| f.metadata().ok()).map_or(0, |m| m.len());
        LogFile { path, file, size }
    }

    fn write(&mut self, line: &str) {
        if self.size + line.len() as u64 > LOG_MAX_BYTES {
            // Let go of the file first: Windows can't rename a file that is open.
            self.file = None;
            let old = self.path.with_extension("log.1");
            let _ = std::fs::remove_file(&old);
            let _ = std::fs::rename(&self.path, &old);
            *self = LogFile::open(self.path.clone());
        }
        if let Some(f) = &mut self.file
            && f.write_all(line.as_bytes()).is_ok()
        {
            self.size += line.len() as u64;
        }
    }
}

struct Logger {
    file: Mutex<LogFile>,
}

impl log::Log for Logger {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::Level::Info && ours(m.target())
    }
    fn log(&self, r: &log::Record) {
        if !self.enabled(r.metadata()) {
            return;
        }
        let line = format!("{} {:<5} {}\n", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"), r.level(), r.args());
        eprint!("{line}");
        if let Ok(mut f) = self.file.lock() {
            f.write(&line);
        }
    }
    fn flush(&self) {}
}

/// Lines from this program and the GridCraft crates go to the log; chatter from libraries doesn't.
/// This program's own lines carry its binary's crate name (`DexSheetsServer`), not `gridcraft`.
fn ours(target: &str) -> bool {
    target.starts_with("gridcraft") || target.starts_with(env!("CARGO_CRATE_NAME"))
}

fn default_dir() -> PathBuf {
    let base = std::env::var_os("ProgramData").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    base.join("Dexterity Sheets").join("Server")
}

fn arg_value(args: &mut Vec<String>, flag: &str) -> Option<String> {
    let i = args.iter().position(|a| a == flag)?;
    if i + 1 >= args.len() {
        args.remove(i);
        return None;
    }
    let v = args.remove(i + 1);
    args.remove(i);
    Some(v)
}

fn profile_command(dir: &std::path::Path, args: &[String]) -> Result<(), String> {
    let sub = args.first().map(String::as_str).unwrap_or("list");
    let arg = |i: usize| args.get(i).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let mut all = server::load_profiles(dir).map_err(|e| e.to_string())?;
    let said = match sub {
        "list" => {
            println!("{:<28} {:<26} {:<8} devices", "account", "name", "role");
            for p in all.profiles.values() {
                let devices: Vec<String> =
                    p.devices.iter().map(|d| format!("{} {} ({})", d.code, d.machine, format!("{:?}", d.status).to_lowercase())).collect();
                println!(
                    "{:<28} {:<26} {:<8} {}",
                    if p.account.is_empty() { &p.user } else { &p.account },
                    p.display,
                    format!("{:?}", p.role).to_lowercase(),
                    if devices.is_empty() { "none yet".to_string() } else { devices.join(", ") }
                );
            }
            for c in &all.carry_over {
                println!(
                    "{:<28} {:<26} {:<8} (from users.json: waits for a Windows account named {})",
                    c.user,
                    c.display,
                    format!("{:?}", c.role).to_lowercase(),
                    c.user
                );
            }
            return Ok(());
        }
        "role" => {
            let user = arg(1).ok_or("give a Windows account, DOMAIN\\user")?;
            let role = arg(2).and_then(|r| Role::parse(&r)).ok_or("role is admin, editor or viewer")?;
            all.set_role(&user, role).map_err(|e| e.to_string())?;
            format!("{user} is now {}.", format!("{role:?}").to_lowercase())
        }
        "approve" | "revoke" => {
            let code = arg(1).ok_or("give the device code, e.g. ABCD-EFGH")?;
            let status = if sub == "approve" { DeviceStatus::Approved } else { DeviceStatus::Revoked };
            let n = all.set_device(&code, status, None).map_err(|e| e.to_string())?;
            format!("{} device(s) {}.", n, if sub == "approve" { "approved" } else { "revoked" })
        }
        "remove" => {
            let user = arg(1).ok_or("give a Windows account, DOMAIN\\user")?;
            all.remove(&user).map_err(|e| e.to_string())?;
            format!("{user} and their devices removed.")
        }
        other => return Err(format!("unknown profile command {other}: use list, role, approve, revoke or remove")),
    };
    server::save_profiles(dir, &all).map_err(|e| e.to_string())?;
    println!("{said}");
    println!("A running server applies this within half a minute (a revoked PC is disconnected).");
    Ok(())
}

/// The server's name from `<data>\server.json` (the Server Manager writes it), if set.
fn configured_name(dir: &std::path::Path) -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("server.json")).ok()?).ok()?;
    v.get("name").and_then(serde_json::Value::as_str).map(str::trim).filter(|n| !n.is_empty()).map(str::to_string)
}

fn start_logging(dir: &std::path::Path) {
    let file = Mutex::new(LogFile::open(dir.join("server.log")));
    if log::set_boxed_logger(Box::new(Logger { file })).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
}

/// Starts serving: workbooks, discovery, app updates. Returns once listening.
fn serve(dir: PathBuf, name: String, port: u16) -> Result<std::sync::Arc<server::Server>, String> {
    let srv = server::start(dir.clone(), name.clone(), port).map_err(|e| format!("{e}. Is another server already using port {port}?"))?;
    log::info!("{name} listening on TCP port {port}; data in {}", dir.display());
    match github::load(&dir) {
        Some(src) => log::info!("app updates: checking GitHub releases of {} (and the updates folder)", src.repo),
        None => log::info!("app updates: put new installers (.msi) in {}", srv.updates_dir().display()),
    }
    let watch_dir = dir.clone();
    let watched = srv.clone();
    let _ = std::thread::Builder::new().name("github-updates".into()).spawn(move || github::watch(watch_dir, watched));
    // The read API for Dexterity Connect: refuses every request until a token is made in the
    // Server Manager. A busy port leaves the server itself running.
    let api_port = std::env::var("DEXSHEETS_API_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(gridcraft_collab::api::DEFAULT_PORT);
    match gridcraft_collab::api::start(srv.clone(), api_port) {
        Ok(p) => log::info!("read API for Dexterity Connect on TCP port {p} ({} token(s))", gridcraft_collab::api::load_tokens(&dir).len()),
        Err(e) => log::error!("read API not started on TCP port {api_port}: {e}"),
    }
    let posting = srv.clone();
    let _ = std::thread::Builder::new().name("connect-push".into()).spawn(move || connect::watch(posting));
    log::info!("Teammates: open Dexterity Sheets, choose File > Shared, and pick this server.");
    Ok(srv)
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let dir = arg_value(&mut args, "--data").map(PathBuf::from).unwrap_or_else(default_dir);
    let port = arg_value(&mut args, "--port").and_then(|p| p.parse().ok()).unwrap_or(DEFAULT_PORT);
    let host = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "server".into());
    let name = arg_value(&mut args, "--name").or_else(|| configured_name(&dir)).unwrap_or_else(|| format!("Dexterity Sheets on {host}"));
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("Cannot create the data folder {}: {e}", dir.display());
        std::process::exit(1);
    }
    if args.first().map(String::as_str) == Some("user") {
        // Scripts written for user names and passwords must fail loudly, not start a server.
        eprintln!("There are no user accounts any more: people sign in with their Windows account. Use `profile list|role|approve|revoke|remove`.");
        std::process::exit(2);
    }
    if args.first().map(String::as_str) == Some("profile") {
        if let Err(e) = profile_command(&dir, &args[1..]) {
            eprintln!("{e}");
            std::process::exit(1);
        }
        return;
    }
    if matches!(args.first().map(String::as_str), Some("-h" | "--help" | "help")) {
        println!(
            "{}",
            include_str!("main.rs")
                .lines()
                .take_while(|l| l.starts_with("//!"))
                .map(|l| l.trim_start_matches("//!").trim_start_matches(' '))
                .collect::<Vec<_>>()
                .join("\n")
        );
        return;
    }
    if args.first().map(String::as_str) == Some("service") {
        start_logging(&dir);
        #[cfg(windows)]
        let r = service::run(dir, name, port);
        #[cfg(not(windows))]
        let r: Result<(), String> = Err("the service mode is for Windows".into());
        if let Err(e) = r {
            log::error!("{e}");
            eprintln!("{e}");
            std::process::exit(1);
        }
        return;
    }
    start_logging(&dir);
    // Carries over an older server's users.json now, so the log says so at start.
    if let Err(e) = server::load_profiles(&dir) {
        eprintln!("profiles.json is unreadable: {e}");
        std::process::exit(1);
    }
    match serve(dir, name, port) {
        Ok(_srv) => loop {
            std::thread::park();
        },
        Err(e) => {
            eprintln!("Cannot start: {e}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_servers_own_lines_reach_its_log() {
        // "listening on TCP port", the update source and service errors are logged from here.
        assert!(super::ours(module_path!()));
        assert!(super::ours("gridcraft_collab::server"));
        assert!(!super::ours("rustls::client"));
    }
}
