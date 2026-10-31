//! Updates straight from the GitHub releases of Dexterity Sheets.
//!
//! The office server is the first source of updates (`collab_ui`, `dex_ui::releases`): it hands
//! out the installers IT approved, with channels, statuses and rollbacks. A PC without a server,
//! or whose server holds nothing newer, asks the releases page of
//! `vikramdex-ops/dexterity-sheets` itself: at start, once a day, and on Check for Updates. The
//! HTTP, release parsing and checksum verification come from upstream GridCraft's `craft-update`;
//! the install is Dexterity's MSI path (`collab_ui::install_update`), the same as for an installer
//! from the server, so both routes behave alike. The web build never checks. Tests never reach the
//! network: the check runs in the desktop app only (`dex_ui::desktop()`).

use std::sync::mpsc::{Receiver, channel};

use gridcraft_collab::proto::{self, UpdateInfo};

use crate::SheetApp;

/// Dexterity Sheets as the update check knows it: releases are only ever read from this
/// repository (never GridCraft's), and its installers are named `dexterity-sheets-<version>-…`.
#[cfg(not(target_arch = "wasm32"))]
pub const APP: craft_update::App = craft_update::App {
    former_ids: &[],
    former_names: &[],
    icon_png: Some(include_bytes!("../../../assets/app-icon/hicolor/128x128/apps/ai.storyteller.gridcraft.png")),
    accent: Some([0x1f, 0x4e, 0x9c]),
    developer: "Dexterity Design Services",
    ..craft_update::App::new("dexterity-sheets", "Dexterity Sheets", "Spreadsheets and data", REPO)
};

/// The repository the app reads its releases from.
pub const REPO: &str = "vikramdex-ops/dexterity-sheets";

/// Automatic checks: a few seconds after the start, then once a day while the app stays open.
const START_DELAY_MS: f64 = 6_000.0;
const EVERY_MS: f64 = 24.0 * 3600.0 * 1000.0;
/// At most this many releases are read (the newest is the update; the ones behind this version
/// can be rolled back to).
const RELEASES_READ: usize = 12;

/// What a check found: the newest release above this version, and the ones below it.
#[derive(Default, Debug, Clone, PartialEq)]
pub(crate) struct Found {
    pub update: Option<UpdateInfo>,
    pub older: Vec<UpdateInfo>,
}

pub(crate) enum Outcome {
    Checked(Result<Found, String>),
    Downloaded(Result<(UpdateInfo, Vec<u8>), String>),
}

/// The check or download in flight, and what the last check said (for the update dialog).
#[derive(Default)]
pub(crate) struct Runtime {
    rx: Option<Receiver<Outcome>>,
    /// The person asked (Check for Updates): the dialog says "latest" or shows the error.
    manual: bool,
    first_tick: Option<f64>,
    last_check: Option<f64>,
    /// When the last check finished, as text for the dialog ("14:05").
    pub(crate) checked_at: Option<String>,
    pub(crate) error: Option<String>,
    /// A check is running.
    pub(crate) checking: bool,
    /// A download is running: the version.
    pub(crate) downloading: Option<String>,
}

/// Starts a check against GitHub (`manual`: the person asked, so the outcome is told either way).
/// Nothing happens on the web, outside the desktop app, or when the person turned GitHub checks
/// off.
pub(crate) fn start(app: &mut SheetApp, manual: bool) {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (app, manual);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let rt = &mut app.updates_rt;
        if !crate::dex_ui::desktop() {
            if manual {
                rt.error = Some("This build does not check the GitHub releases.".into());
            }
            return;
        }
        if !app.collab.releases.prefs.github_updates {
            if manual {
                rt.checked_at = None;
                rt.error = Some("Checking the GitHub releases page is turned off for this PC (turn it on below).".into());
            }
            return;
        }
        if rt.rx.is_some() {
            rt.manual |= manual;
            return;
        }
        rt.manual = manual;
        rt.checking = true;
        rt.error = None;
        rt.last_check = Some(crate::clock::monotonic_ms());
        let (tx, rx) = channel();
        rt.rx = Some(rx);
        let version = gridcraft_collab::APP_VERSION.trim().to_string();
        let ctx = app.collab_ctx.clone();
        let spawned = std::thread::Builder::new().name("dexsheets-github-updates".into()).spawn(move || {
            let _ = tx.send(Outcome::Checked(check(&version)));
            if let Some(ctx) = ctx {
                ctx.request_repaint();
            }
        });
        if spawned.is_err() {
            rt.rx = None;
            rt.checking = false;
            rt.error = Some("The update check could not start.".into());
        }
    }
}

/// Downloads `info.url` (checked against `info.sha256`), then installs it like an installer from
/// the office server.
pub(crate) fn download(app: &mut SheetApp, info: UpdateInfo) {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (app, info);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        if info.sha256.len() != 64 {
            app.collab.update_state = None;
            app.message = Some(("Dexterity Sheets".into(), format!("The release publishes no checksum for {}, so it was not installed.", info.file)));
            return;
        }
        let rt = &mut app.updates_rt;
        if rt.rx.is_some() {
            return;
        }
        rt.downloading = Some(info.version.clone());
        app.collab.update_state = Some(format!("Downloading {} from GitHub...", info.version));
        let (tx, rx) = channel();
        rt.rx = Some(rx);
        let ctx = app.collab_ctx.clone();
        let version = gridcraft_collab::APP_VERSION.trim().to_string();
        let spawned = std::thread::Builder::new().name("dexsheets-github-download".into()).spawn(move || {
            let r = fetch(&version, &info).map(|bytes| (info, bytes));
            let _ = tx.send(Outcome::Downloaded(r));
            if let Some(ctx) = ctx {
                ctx.request_repaint();
            }
        });
        if spawned.is_err() {
            rt.rx = None;
            rt.downloading = None;
            app.collab.update_state = None;
            app.message = Some(("Dexterity Sheets".into(), "The download could not start.".into()));
        }
    }
}

/// Every frame: the automatic checks, and the outcome of a check or download.
pub(crate) fn tick(app: &mut SheetApp, _ctx: &egui::Context) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let now = crate::clock::monotonic_ms();
        let rt = &mut app.updates_rt;
        let first = *rt.first_tick.get_or_insert(now);
        let due = match rt.last_check {
            None => now - first >= START_DELAY_MS,
            Some(at) => now - at >= EVERY_MS,
        };
        if due && rt.rx.is_none() && crate::dex_ui::desktop() {
            // `start` records the time even when checks are off, so this is asked once a day.
            rt.last_check = Some(now);
            start(app, false);
        }
        let outcome = match &app.updates_rt.rx {
            Some(rx) => rx.try_recv().ok(),
            None => None,
        };
        let Some(outcome) = outcome else { return };
        app.updates_rt.rx = None;
        match outcome {
            Outcome::Checked(r) => {
                let manual = std::mem::take(&mut app.updates_rt.manual);
                app.updates_rt.checking = false;
                app.updates_rt.checked_at = Some(chrono::Local::now().format("%H:%M").to_string());
                match r {
                    Ok(found) => {
                        app.updates_rt.error = None;
                        crate::dex_ui::releases::on_github(app, found, manual);
                    }
                    Err(e) => {
                        log::warn!("GitHub update check: {e}");
                        app.updates_rt.error = Some(e);
                    }
                }
            }
            Outcome::Downloaded(r) => {
                app.updates_rt.downloading = None;
                match r {
                    Ok((info, bytes)) => crate::collab_ui::install_update(app, &info, bytes),
                    Err(e) => {
                        app.collab.update_state = None;
                        app.message = Some(("Dexterity Sheets".into(), format!("The update could not be downloaded: {e}")));
                    }
                }
            }
        }
    }
}

/// Nothing of its own to draw: the update dialog lives in `collab_ui::update_badge`.
pub(crate) fn show(_app: &mut SheetApp, _ctx: &egui::Context) {}

#[cfg(not(target_arch = "wasm32"))]
fn http(version: &str) -> Result<craft_update::Http, String> {
    craft_update::Http::new(&format!("DexteritySheets/{version}"))
}

/// Reads the newest releases and picks, for each, the Windows x64 `.msi` with its checksum.
#[cfg(not(target_arch = "wasm32"))]
fn check(version: &str) -> Result<Found, String> {
    let http = http(version)?;
    let url = format!("https://api.github.com/repos/{}/releases?per_page={RELEASES_READ}", APP.repo);
    let text = match http.get_api(&url, None)? {
        craft_update::http::Fetched::Body { text, .. } => text,
        craft_update::http::Fetched::RateLimited { .. } => return Err("GitHub's request limit was reached; try again in an hour.".into()),
        craft_update::http::Fetched::NotModified | craft_update::http::Fetched::NotFound => return Ok(Found::default()),
    };
    let list: Vec<serde_json::Value> = serde_json::from_str(&text).map_err(|e| format!("GitHub's answer could not be read: {e}"))?;
    let infos: Vec<UpdateInfo> = list.iter().filter_map(|v| release_info(&http, v)).collect();
    Ok(found_in(&infos, version))
}

/// The release in GitHub's JSON as an `UpdateInfo` with the installer's download URL, or `None`
/// for drafts, pre-releases and releases without a Windows installer.
#[cfg(not(target_arch = "wasm32"))]
fn release_info(http: &craft_update::Http, v: &serde_json::Value) -> Option<UpdateInfo> {
    use craft_update::{PackageKind, Platform, pick_asset, release};
    let flag = |k: &str| v.get(k).and_then(serde_json::Value::as_bool).unwrap_or(false);
    if flag("draft") || flag("prerelease") {
        return None;
    }
    let mut rel = release::parse_github_release(&v.to_string(), APP.repo).ok()?;
    let platform = Platform::current()?;
    let stems = [APP.id];
    let picked = pick_asset(&rel, &stems, platform, &[PackageKind::Msi]).map(|(a, _)| a.clone())?;
    if picked.sha256.is_none() {
        // The release's checksum list: `SHA256SUMS.txt` (craft-update's name) or `SHA256SUMS`.
        let sums = rel
            .sums_asset()
            .map(|a| a.url.clone())
            .or_else(|| rel.assets.iter().find(|a| a.name.eq_ignore_ascii_case("SHA256SUMS")).map(|a| a.url.clone()));
        if let Some(url) = sums
            && let Ok(Some(text)) = http.get_text(&url)
        {
            rel.apply_sums(&release::parse_sha256sums(&text));
        }
    }
    let (asset, _) = pick_asset(&rel, &stems, platform, &[PackageKind::Msi])?;
    Some(UpdateInfo {
        version: rel.version.as_ref().map(ToString::to_string)?,
        notes: rel.notes.clone(),
        file: asset.name.clone(),
        size: asset.size,
        sha256: asset.sha256.clone().unwrap_or_default(),
        channel: "stable".into(),
        status: "ok".into(),
        note: String::new(),
        url: asset.url.clone(),
    })
}

/// From releases newest first: the newest one above `version`, and up to five below it.
fn found_in(infos: &[UpdateInfo], version: &str) -> Found {
    let mut sorted: Vec<&UpdateInfo> = infos.iter().collect();
    sorted.sort_by(|a, b| {
        if proto::version_newer(&a.version, &b.version) {
            std::cmp::Ordering::Less
        } else if proto::version_newer(&b.version, &a.version) {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    });
    Found {
        update: sorted.iter().find(|u| proto::version_newer(&u.version, version)).map(|u| (*u).clone()),
        older: sorted.iter().filter(|u| proto::version_newer(version, &u.version)).take(5).map(|u| (*u).clone()).collect(),
    }
}

/// Downloads the installer into the update folder and checks it against the release's checksum.
#[cfg(not(target_arch = "wasm32"))]
fn fetch(version: &str, info: &UpdateInfo) -> Result<Vec<u8>, String> {
    let http = http(version)?;
    let dir = std::env::temp_dir().join("DexteritySheets-update");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let name: String = info.file.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')).collect();
    let dest = dir.join(if name.to_ascii_lowercase().ends_with(".msi") { name } else { format!("DexteritySheets-{}.msi", info.version) });
    let stop = craft_update::Stop::default();
    http.download(&info.url, &dest, info.size, &info.sha256, &stop, &mut |_| {})?;
    let bytes = std::fs::read(&dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    let _ = std::fs::remove_file(&dest);
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(v: &str) -> UpdateInfo {
        UpdateInfo { version: v.into(), url: format!("https://example.invalid/{v}.msi"), ..UpdateInfo::default() }
    }

    #[test]
    fn the_newest_release_above_this_version_is_the_update_and_the_ones_below_roll_back() {
        let infos = [info("1.3.0"), info("1.4.1"), info("1.2.2"), info("1.4.0"), info("1.3.2")];
        let f = found_in(&infos, "1.3.2");
        assert_eq!(f.update.as_ref().map(|u| u.version.as_str()), Some("1.4.1"));
        assert_eq!(f.older.iter().map(|u| u.version.as_str()).collect::<Vec<_>>(), ["1.3.0", "1.2.2"]);
        let latest = found_in(&infos, "1.4.1");
        assert_eq!(latest.update, None, "nothing newer than the newest release");
        assert_eq!(latest.older.len(), 4);
        assert_eq!(found_in(&[], "1.0.0"), Found::default());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_app_descriptor_names_our_repository_and_installer_stem() {
        assert_eq!(APP.repo, REPO);
        assert_eq!(APP.id, "dexterity-sheets");
        assert_eq!(APP.name, "Dexterity Sheets");
        assert!(!APP.repo.contains("storytold"), "releases must never be read from the upstream repository");
    }

    /// The release workflow names the installer `dexterity-sheets-<version>-windows-x64.msi`;
    /// craft-update must pick it for this PC.
    #[cfg(all(not(target_arch = "wasm32"), windows))]
    #[test]
    fn the_release_installer_name_is_picked_for_windows() {
        use craft_update::{Asset, PackageKind, Platform, Release, pick_asset};
        let release = Release {
            assets: [
                "dexterity-sheets-1.3.2-windows-x64.msi",
                "dexterity-sheets-1.3.2-windows-x64-portable.zip",
                "DexteritySheets-Setup.exe",
                "DexSheetsServer-1.3.2-x64.msi",
                "SHA256SUMS",
            ]
            .iter()
            .map(|n| Asset { name: (*n).into(), size: 1, url: format!("https://github.com/{REPO}/releases/download/v1.3.2/{n}"), sha256: None })
            .collect(),
            ..Release::default()
        };
        let platform = Platform::current().expect("windows");
        let (asset, _) = pick_asset(&release, &[APP.id], platform, &[PackageKind::Msi]).expect("the app installer");
        assert_eq!(asset.name, "dexterity-sheets-1.3.2-windows-x64.msi", "never the server's installer or the zip");
    }
}
