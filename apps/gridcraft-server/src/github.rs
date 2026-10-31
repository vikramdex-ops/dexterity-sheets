//! Fetches new Dexterity Sheets installers from the company's GitHub Releases into the server's
//! `updates/` folder, from where clients on the LAN are offered them. Clients never go to the
//! internet themselves.
//!
//! Configured by `<data>\update-source.json`:
//! `{"repo": "owner/name", "token": "github_pat_… (only for a private repository)"}`.

use std::path::Path;
use std::time::Duration;

use serde::Deserialize;
use sha2::Digest;

#[derive(Deserialize)]
pub struct Source {
    pub repo: String,
    #[serde(default)]
    pub token: Option<String>,
    /// Minutes between checks (default 30).
    #[serde(default)]
    pub every_minutes: Option<u64>,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    /// The API URL; with `Accept: application/octet-stream` it serves the file (private repos too).
    url: String,
}

/// The releases of Dexterity Sheets itself: what a server fetches from until IT says otherwise.
pub const DEFAULT_REPO: &str = "vikramdex-ops/dexterity-sheets";

/// The update source: `update-source.json` when it exists, else the official repository. A file
/// with an empty `repo` turns fetching off (the Server Manager's "Turn off").
pub fn load(dir: &Path) -> Option<Source> {
    let Ok(bytes) = std::fs::read(dir.join("update-source.json")) else {
        return Some(Source { repo: DEFAULT_REPO.into(), token: None, every_minutes: None });
    };
    match serde_json::from_slice::<Source>(&bytes) {
        Ok(s) if s.repo.contains('/') => Some(s),
        Ok(s) if s.repo.trim().is_empty() => None,
        Ok(_) => {
            log::warn!("update-source.json: \"repo\" must be owner/name");
            None
        }
        Err(e) => {
            log::warn!("update-source.json is unreadable: {e}");
            None
        }
    }
}

fn client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .user_agent(concat!("DexSheetsServer/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(600))
        .build()
        .map_err(|e| e.to_string())
}

fn get(c: &reqwest::blocking::Client, url: &str, token: Option<&str>, accept: &str) -> Result<Vec<u8>, String> {
    let mut rq = c.get(url).header("Accept", accept).header("X-GitHub-Api-Version", "2022-11-28");
    if let Some(t) = token.filter(|t| !t.is_empty()) {
        rq = rq.bearer_auth(t);
    }
    let resp = rq.send().map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("{url}: HTTP {}", resp.status()));
    }
    resp.bytes().map(|b| b.to_vec()).map_err(|e| e.to_string())
}

/// One check: downloads the newest release's installer when `updates/` lacks it, and the release
/// before it when that is missing too, so Roll Back always has somewhere to go. Returns the
/// newest version downloaded.
pub fn check_once(src: &Source, updates: &Path, newest_local: Option<&str>) -> Result<Option<String>, String> {
    let c = client()?;
    let token = src.token.as_deref();
    let body = get(&c, &format!("https://api.github.com/repos/{}/releases?per_page=10", src.repo), token, "application/vnd.github+json")?;
    let list: Vec<Release> = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
    let published: Vec<Release> = list.into_iter().filter(|r| !r.draft && !r.prerelease).collect();
    let local = local_versions(updates);
    let mut newest = None;
    for rel in published.iter().take(2) {
        let version = rel.tag_name.trim_start_matches(['v', 'V']).to_string();
        if local.iter().any(|v| v == &version) {
            continue;
        }
        if newest.is_none() && newest_local.is_some_and(|v| !gridcraft_collab::proto::version_newer(&version, v)) {
            // The newest release is already here (under another file name): nothing to fetch.
            continue;
        }
        download(&c, token, rel, &version, updates)?;
        if newest.is_none() {
            newest = Some(version);
        }
    }
    Ok(newest)
}

/// Fetches one release's app installer and its notes into `updates/`, checked against the
/// release's `SHA256SUMS` when it publishes one.
fn download(c: &reqwest::blocking::Client, token: Option<&str>, rel: &Release, version: &str, updates: &Path) -> Result<(), String> {
    // The app's installer only: a release also carries the server's own installer, which must
    // never be offered to office PCs.
    let app_msi = |a: &&Asset| gridcraft_collab::server::is_app_installer(&a.name);
    let Some(msi) = rel.assets.iter().filter(app_msi).find(|a| a.name.contains("x64")).or_else(|| rel.assets.iter().find(app_msi)) else {
        return Err(format!("release {} has no .msi asset", rel.tag_name));
    };
    let data = get(c, &msi.url, token, "application/octet-stream")?;
    // The release's SHA256SUMS, when published, must match.
    if let Some(sums) = rel.assets.iter().find(|a| a.name.eq_ignore_ascii_case("SHA256SUMS") || a.name.eq_ignore_ascii_case("SHA256SUMS.txt")) {
        let text = String::from_utf8_lossy(&get(c, &sums.url, token, "application/octet-stream")?).to_string();
        let have = gridcraft_collab::auth::hex(&sha2::Sha256::digest(&data));
        let expect = text.lines().find(|l| l.contains(&msi.name)).and_then(|l| l.split_whitespace().next()).map(str::to_ascii_lowercase);
        if expect.as_deref() != Some(have.as_str()) {
            return Err(format!("{}: checksum mismatch (got {have}, SHA256SUMS says {expect:?}); not offered to clients", msi.name));
        }
    }
    // Name it so the version is a part the server recognises.
    let name = if msi.name.contains(version) { msi.name.clone() } else { format!("DexteritySheets-{version}-x64.msi") };
    let part = updates.join(format!("{name}.part"));
    std::fs::write(&part, &data).map_err(|e| e.to_string())?;
    let notes = updates.join(Path::new(&name).with_extension("md"));
    let _ = std::fs::write(notes, rel.body.clone().unwrap_or_default());
    std::fs::rename(&part, updates.join(&name)).map_err(|e| e.to_string())?;
    log::info!("downloaded Dexterity Sheets {version} ({name})");
    Ok(())
}

/// The versions of the app installers in `updates/`.
fn local_versions(updates: &Path) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(updates) else { return Vec::new() };
    rd.flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            if !gridcraft_collab::server::is_app_installer(&n) {
                return None;
            }
            n.trim_end_matches(".msi").split('-').find(|p| p.starts_with(|c: char| c.is_ascii_digit()) && p.contains('.')).map(str::to_string)
        })
        .collect()
}

/// Runs forever: checks GitHub every `every_minutes`.
pub fn watch(dir: std::path::PathBuf, server: std::sync::Arc<gridcraft_collab::server::Server>) {
    loop {
        if let Some(src) = load(&dir) {
            let updates = server.updates_dir();
            let newest = newest_local(&updates);
            match check_once(&src, &updates, newest.as_deref()) {
                Ok(Some(v)) => {
                    log::info!("downloaded Dexterity Sheets {v} from {}", src.repo);
                    server.poll_updates();
                }
                Ok(None) => {}
                Err(e) => log::warn!("update check against {} failed: {e}", src.repo),
            }
            std::thread::sleep(Duration::from_secs(60 * src.every_minutes.unwrap_or(30).max(5)));
        } else {
            std::thread::sleep(Duration::from_secs(300));
        }
    }
}

fn newest_local(updates: &Path) -> Option<String> {
    let mut best: Option<String> = None;
    for e in std::fs::read_dir(updates).ok()?.flatten() {
        let n = e.file_name().to_string_lossy().to_string();
        if !gridcraft_collab::server::is_app_installer(&n) {
            continue;
        }
        if let Some(v) = n.trim_end_matches(".msi").split('-').find(|p| p.starts_with(|c: char| c.is_ascii_digit()) && p.contains('.'))
            && best.as_deref().is_none_or(|b| gridcraft_collab::proto::version_newer(v, b))
        {
            best = Some(v.to_string());
        }
    }
    best
}
