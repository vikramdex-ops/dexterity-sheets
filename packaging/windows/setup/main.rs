//! DexteritySheets-Setup.exe: writes the MSI it carries to a temporary folder, runs Windows
//! Installer on it (the MSI's own dialogs: desktop shortcut choice, progress, finish) and passes
//! its exit code back. One product in "Installed apps", the same as installing the .msi directly,
//! so the in-app updater (which installs .msi files) keeps working for either.
//!
//! Options for IT: /quiet (no UI), /passive (progress bar only), /log <file>, and any
//! PROPERTY=value pair for the MSI (for example DESKTOPSHORTCUT=1).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::process::Command;

const MSI: &[u8] = include_bytes!(env!("DEXSHEETS_SETUP_MSI"));
const VERSION: &str = env!("DEXSHEETS_SETUP_VERSION");

fn main() {
    let dir = std::env::temp_dir().join("Dexterity Sheets Setup");
    let msi = dir.join(format!("dexterity-sheets-{VERSION}-windows-x64.msi"));
    if let Err(e) = std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&msi, MSI)) {
        fail(&format!("Dexterity Sheets Setup could not unpack the installer to {}:\n{e}\n\nFree some disk space and run Setup again.", dir.display()));
        std::process::exit(1);
    }
    let mut args: Vec<String> = vec!["/i".into(), msi.display().to_string()];
    let mut quiet = false;
    let mut rest = std::env::args().skip(1);
    while let Some(a) = rest.next() {
        match a.to_ascii_lowercase().as_str() {
            "/quiet" | "/silent" | "/s" | "/qn" | "-q" => {
                quiet = true;
                args.push("/qn".into());
            }
            "/passive" | "/qb" => {
                quiet = true;
                args.push("/passive".into());
            }
            "/norestart" => args.push("/norestart".into()),
            "/log" | "/l" => {
                if let Some(file) = rest.next() {
                    args.push("/l*v".into());
                    args.push(file);
                }
            }
            _ if a.contains('=') => args.push(a),
            _ => {}
        }
    }
    let code = match Command::new(system32("msiexec.exe")).args(&args).status() {
        Ok(s) => s.code().unwrap_or(1),
        Err(e) => {
            if !quiet {
                fail(&format!("Dexterity Sheets Setup could not start Windows Installer:\n{e}"));
            }
            1
        }
    };
    let _ = std::fs::remove_file(&msi);
    let _ = std::fs::remove_dir(&dir);
    // Windows Installer's own code: 0 installed, 3010 installed (restart pending), 1602 cancelled.
    std::process::exit(code);
}

fn system32(exe: &str) -> PathBuf {
    std::env::var_os("SystemRoot").map_or_else(|| PathBuf::from(exe), |root| PathBuf::from(root).join("System32").join(exe))
}

/// A message box without any unsafe code: PowerShell's WPF MessageBox.
fn fail(message: &str) {
    let text = message.replace('\'', "''");
    let _ = Command::new(system32(r"WindowsPowerShell\v1.0\powershell.exe"))
        .args(["-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command"])
        .arg(format!("Add-Type -AssemblyName PresentationFramework; [void][System.Windows.MessageBox]::Show('{text}', 'Dexterity Sheets Setup', 'OK', 'Error')"))
        .status();
}
