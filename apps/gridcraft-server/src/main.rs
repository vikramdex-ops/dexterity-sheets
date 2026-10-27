//! Dexterity Sheets server.
//!
//! ```text
//! DexSheetsServer                      run (creates the first administrator if there is none)
//! DexSheetsServer service              run as the Windows service the installer registers
//! DexSheetsServer user add NAME [--display "Full Name"] [--role admin|editor|viewer]
//! DexSheetsServer user passwd NAME     set a new password
//! DexSheetsServer user role NAME ROLE
//! DexSheetsServer user disable NAME | user enable NAME
//! DexSheetsServer user list
//! Updates: drop DexteritySheets-<version>-x64.msi into <data>\updates, or configure
//! <data>\update-source.json {"repo": "owner/name", "token": "…"} to fetch GitHub releases.
//! options: --data DIR (default %ProgramData%\Dexterity Sheets\Server), --port N, --name "Server name"
//! (the name otherwise comes from <data>\server.json {"name": "…"}, set in the Server Manager)
//! Most offices never need these commands: the Server Manager app does all of it.
//! ```

use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use gridcraft_collab::proto::{DEFAULT_PORT, Role};
use gridcraft_collab::server;

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
        m.level() <= log::Level::Info && m.target().starts_with("gridcraft")
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

fn ask(prompt: &str) -> String {
    print!("{prompt}");
    let _ = std::io::stdout().flush();
    let mut s = String::new();
    let _ = std::io::stdin().read_line(&mut s);
    s.trim().to_string()
}

fn ask_password(user: &str) -> Result<String, String> {
    for _ in 0..3 {
        let p1 = rpassword::prompt_password(format!("Password for {user}: ")).map_err(|e| e.to_string())?;
        let p2 = rpassword::prompt_password("Repeat password: ").map_err(|e| e.to_string())?;
        if p1 != p2 {
            println!("The passwords differ; try again.");
            continue;
        }
        if p1.chars().count() < 6 {
            println!("Use at least 6 characters.");
            continue;
        }
        return Ok(p1);
    }
    Err("no password set".into())
}

fn user_command(dir: &std::path::Path, args: &[String]) -> Result<(), String> {
    let sub = args.first().map(String::as_str).unwrap_or("list");
    let mut rest: Vec<String> = args.iter().skip(1).cloned().collect();
    let display = arg_value(&mut rest, "--display");
    let role = arg_value(&mut rest, "--role");
    let password = arg_value(&mut rest, "--password");
    let mut users = server::load_users(dir).map_err(|e| e.to_string())?;
    let name = rest.first().map(|s| s.trim().to_ascii_lowercase());
    match sub {
        "list" => {
            println!("{:<20} {:<28} {:<8} status", "user", "name", "role");
            for u in &users {
                println!(
                    "{:<20} {:<28} {:<8} {}",
                    u.user,
                    u.display,
                    format!("{:?}", u.role).to_lowercase(),
                    if u.disabled { "disabled" } else { "active" }
                );
            }
            Ok(())
        }
        "add" | "passwd" => {
            let name = name.ok_or("give a user name")?;
            let existing = users.iter().find(|u| u.user == name).cloned();
            if sub == "add" && existing.is_some() {
                return Err(format!("{name} exists; use `user passwd {name}`"));
            }
            if sub == "passwd" && existing.is_none() {
                return Err(format!("no user {name}"));
            }
            let role = match role.as_deref() {
                Some(r) => Role::parse(r).ok_or("role is admin, editor or viewer")?,
                None => existing.as_ref().map_or(Role::Editor, |u| u.role),
            };
            let display = display.or_else(|| existing.as_ref().map(|u| u.display.clone())).unwrap_or_else(|| ask(&format!("Full name for {name}: ")));
            let password = match password {
                Some(p) => p,
                None => ask_password(&name)?,
            };
            server::set_user(dir, &name, &display, &password, role).map_err(|e| e.to_string())?;
            println!("{} {name} ({display}, {}).", if sub == "add" { "Added" } else { "Updated" }, format!("{role:?}").to_lowercase());
            println!("A running server picks this up at the next login.");
            Ok(())
        }
        "role" => {
            let name = name.ok_or("give a user name")?;
            let r = rest.get(1).and_then(|r| Role::parse(r)).ok_or("role is admin, editor or viewer")?;
            let u = users.iter_mut().find(|u| u.user == name).ok_or(format!("no user {name}"))?;
            u.role = r;
            server::save_users(dir, &users).map_err(|e| e.to_string())?;
            println!("{name} is now {}.", format!("{r:?}").to_lowercase());
            Ok(())
        }
        "disable" | "enable" => {
            let name = name.ok_or("give a user name")?;
            let u = users.iter_mut().find(|u| u.user == name).ok_or(format!("no user {name}"))?;
            u.disabled = sub == "disable";
            server::save_users(dir, &users).map_err(|e| e.to_string())?;
            println!("{name} {}.", if sub == "disable" { "disabled" } else { "enabled" });
            Ok(())
        }
        other => Err(format!("unknown user command {other}")),
    }
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
        if let Err(e) = user_command(&dir, &args[1..]) {
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
    match server::load_users(&dir) {
        Ok(u) if u.is_empty() => {
            println!("No accounts yet. Create the first administrator.");
            let name = ask("Administrator user name: ").to_ascii_lowercase();
            let display = ask("Full name: ");
            match ask_password(&name).and_then(|p| server::set_user(&dir, &name, &display, &p, Role::Admin).map_err(|e| e.to_string())) {
                Ok(()) => println!("Administrator {name} created."),
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
        }
        Ok(_) => {}
        Err(e) => {
            eprintln!("users.json is unreadable: {e}");
            std::process::exit(1);
        }
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
