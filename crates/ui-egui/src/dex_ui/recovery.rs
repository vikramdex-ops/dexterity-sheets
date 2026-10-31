//! Crash recovery for unsaved work (Dexterity Sheets).
//!
//! While a workbook has unsaved changes, a copy of it is kept in the recovery folder
//! (`%LOCALAPPDATA%\Dexterity Sheets\Recovery`), never beside or over the workbook's own file:
//! at most every [`EVERY_MS`] and only when it changed since the last copy. A crash, a forced
//! restart or a power cut then loses at most those last seconds. Saving or closing a workbook
//! removes its copy; a normal exit removes the run's folder.
//!
//! Each run of the app has its own folder (`run-<pid>-<started>`) with a `run.lock` it holds
//! locked while it runs. At start, a folder whose lock can be taken belongs to a run that ended
//! without cleaning up: its workbooks are offered in the Recover Unsaved Work dialog (recover,
//! open the last saved version, compare the two, or discard). A folder still locked belongs to
//! another window of the app that is running, and is left alone.
//!
//! Shared workbooks are not copied: every edit is already on the office server.
//! The recovery copy and the workbook's file are separate things, and so is the app itself:
//! updating or rolling back Dexterity Sheets never touches either.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gridcraft_engine::model::Workbook;
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};

use crate::SheetApp;

/// Shortest time between two copies of the same workbook.
pub const EVERY_MS: f64 = 10_000.0;

/// What a copy is: shown in the dialog.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ItemMeta {
    pub title: String,
    /// The workbook's own file, when it has one.
    #[serde(default)]
    pub path: Option<String>,
    /// When the copy was written (ms since 1970).
    pub saved_at: i64,
    #[serde(default)]
    pub cells: usize,
}

/// A workbook copy left by a run that did not end normally.
#[derive(Clone, Debug)]
pub struct Found {
    pub run: PathBuf,
    pub file: PathBuf,
    pub meta: ItemMeta,
}

struct Run {
    dir: PathBuf,
    /// Held locked for as long as this run lives.
    _lock: std::fs::File,
}

type Written = (u64, Arc<Workbook>, Result<(), String>);

#[derive(Default)]
pub struct Recovery {
    base: Option<PathBuf>,
    run: Option<Run>,
    checked: bool,
    /// Doc uid → the workbook as last copied, and when.
    written: HashMap<u64, (Arc<Workbook>, f64)>,
    writing: Option<std::sync::mpsc::Receiver<Written>>,
    pub found: Vec<Found>,
    pub show: bool,
    pub error: Option<String>,
}

/// The recovery folder: `DEXSHEETS_RECOVERY_DIR` when set (tests), else the local app data.
pub fn default_base() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("DEXSHEETS_RECOVERY_DIR") {
        return Some(PathBuf::from(d));
    }
    let local = std::env::var_os("LOCALAPPDATA").or_else(|| std::env::var_os("APPDATA"));
    if let Some(l) = local {
        return Some(PathBuf::from(l).join("Dexterity Sheets").join("Recovery"));
    }
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("state").join("dexterity-sheets").join("recovery"))
}

impl Recovery {
    pub fn with_base(base: PathBuf) -> Recovery {
        Recovery { base: Some(base), ..Recovery::default() }
    }

    fn base(&mut self) -> Option<PathBuf> {
        if self.base.is_none() {
            self.base = default_base();
        }
        self.base.clone()
    }

    /// This run's folder, made (and locked) on first use.
    fn run_dir(&mut self) -> Result<PathBuf, String> {
        if let Some(r) = &self.run {
            return Ok(r.dir.clone());
        }
        let base = self.base().ok_or("no folder for recovery copies")?;
        let dir = base.join(format!("run-{}-{}", std::process::id(), crate::now_ms() as i64));
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join("run.lock"))
            .map_err(|e| format!("{}: {e}", dir.display()))?;
        lock.try_lock().map_err(|e| format!("{}: {e}", dir.display()))?;
        self.run = Some(Run { dir: dir.clone(), _lock: lock });
        Ok(dir)
    }

    /// Workbook copies of runs that ended without cleaning up. Empty folders of such runs are
    /// removed; folders of runs still going (their lock is held) are left alone.
    pub fn scan(&mut self) -> Vec<Found> {
        let mut out = Vec::new();
        let Some(base) = self.base() else { return out };
        let mine = self.run.as_ref().map(|r| r.dir.clone());
        let Ok(rd) = std::fs::read_dir(&base) else { return out };
        for e in rd.flatten() {
            let dir = e.path();
            if Some(&dir) == mine.as_ref() || !e.file_name().to_string_lossy().starts_with("run-") {
                continue;
            }
            let Ok(lock) = std::fs::OpenOptions::new().write(true).open(dir.join("run.lock")) else {
                // No lock file: an unfinished folder of a run that died at once.
                let _ = std::fs::remove_dir_all(&dir);
                continue;
            };
            if lock.try_lock().is_err() {
                continue; // another window of the app, running
            }
            let mut items = items_in(&dir);
            drop(lock);
            if items.is_empty() {
                let _ = std::fs::remove_dir_all(&dir);
            }
            out.append(&mut items);
        }
        out.sort_by_key(|f| std::cmp::Reverse(f.meta.saved_at));
        out
    }

    /// Copies a workbook (synchronously): `<uid>.xlsx` plus `<uid>.json`, each written to a
    /// temporary name first so a copy is never half written.
    pub fn write_copy(&mut self, uid: u64, wb: &Workbook, meta: &ItemMeta) -> Result<(), String> {
        let dir = self.run_dir()?;
        write_item(&dir, uid, wb, meta)
    }

    /// Removes a workbook's copy (saved, or closed).
    pub fn forget(&mut self, uid: u64) {
        self.written.remove(&uid);
        if let Some(r) = &self.run {
            let _ = std::fs::remove_file(r.dir.join(format!("{uid}.xlsx")));
            let _ = std::fs::remove_file(r.dir.join(format!("{uid}.json")));
        }
    }

    /// A normal exit: copies of workbooks with no unsaved changes go; this run's folder goes
    /// with them unless a workbook still has unsaved changes (Windows shutting the app down),
    /// whose final copy is written now and offered next time.
    pub fn clean_exit(&mut self, dirty: &[(u64, Arc<Workbook>, ItemMeta)]) {
        if dirty.is_empty() {
            if let Some(r) = self.run.take() {
                let dir = r.dir.clone();
                drop(r);
                let _ = std::fs::remove_dir_all(dir);
            }
            return;
        }
        for (uid, wb, meta) in dirty {
            let current = self.written.get(uid).is_some_and(|(w, _)| Arc::ptr_eq(w, wb));
            if !current && let Err(e) = self.write_copy(*uid, wb, meta) {
                log::error!("recovery copy of {}: {e}", meta.title);
            }
        }
    }
}

fn items_in(dir: &Path) -> Vec<Found> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out = Vec::new();
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        let file = p.with_extension("xlsx");
        let meta = std::fs::read(&p).ok().and_then(|b| serde_json::from_slice::<ItemMeta>(&b).ok());
        match meta {
            Some(meta) if file.is_file() => out.push(Found { run: dir.to_path_buf(), file, meta }),
            _ => {}
        }
    }
    out
}

fn write_item(dir: &Path, uid: u64, wb: &Workbook, meta: &ItemMeta) -> Result<(), String> {
    let bytes = gridcraft_engine::io::save_bytes(wb, "recovery.xlsx").map_err(|e| e.to_string())?;
    let put = |name: String, data: &[u8]| -> Result<(), String> {
        let tmp = dir.join(format!("{name}.tmp"));
        std::fs::write(&tmp, data).map_err(|e| format!("{}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, dir.join(&name)).map_err(|e| format!("{name}: {e}"))
    };
    put(format!("{uid}.xlsx"), &bytes)?;
    let m = serde_json::to_vec_pretty(meta).map_err(|e| e.to_string())?;
    put(format!("{uid}.json"), &m)
}

fn meta_of(d: &gridcraft_engine::DocState) -> ItemMeta {
    ItemMeta {
        title: d.display_title(),
        path: d.path.clone(),
        saved_at: crate::now_ms() as i64,
        cells: d.wb.sheets.iter().map(|s| s.cells.len()).sum(),
    }
}

/// Every frame: copies changed workbooks (one at a time, off the UI thread), drops copies of
/// saved and closed ones, and on the first frame looks for work left by an earlier run.
pub fn tick(app: &mut SheetApp) {
    if !crate::dex_ui::desktop() {
        return;
    }
    if !app.collab.recovery.checked {
        app.collab.recovery.checked = true;
        app.collab.recovery.found = app.collab.recovery.scan();
        app.collab.recovery.show = !app.collab.recovery.found.is_empty();
        if app.collab.recovery.show {
            let n = app.collab.recovery.found.len();
            crate::dex_ui::share::local(
                app,
                "recovery",
                &format!("The last session ended unexpectedly: {n} workbook(s) with unsaved work can be recovered"),
            );
        }
    }
    if let Some(rx) = &app.collab.recovery.writing
        && let Ok((uid, wb, r)) = rx.try_recv()
    {
        app.collab.recovery.writing = None;
        match r {
            Ok(()) => {
                app.collab.recovery.written.insert(uid, (wb, crate::clock::monotonic_ms()));
                app.collab.recovery.error = None;
            }
            Err(e) => {
                log::error!("recovery copy: {e}");
                app.collab.recovery.error = Some(e);
            }
        }
    }
    let now = crate::clock::monotonic_ms();
    let mut gone: Vec<u64> = app.collab.recovery.written.keys().copied().collect();
    let mut next = None;
    // The soonest a changed workbook is due a copy: an idle window draws no frames, so it asks
    // for one then (or the last edits before walking away would never be copied).
    let mut wake: Option<f64> = None;
    for d in app.session.documents() {
        if app.collab.shared(d.uid).is_some() || !d.is_dirty() {
            continue;
        }
        gone.retain(|u| *u != d.uid);
        let last = app.collab.recovery.written.get(&d.uid);
        let changed = last.is_none_or(|(w, _)| !Arc::ptr_eq(w, &d.wb));
        let due = last.is_none_or(|(_, t)| now - t >= EVERY_MS);
        if next.is_none() && changed && due && app.editor.is_none() {
            next = Some((d.uid, d.wb.clone(), meta_of(d)));
        } else if changed {
            let left = last.map_or(0.0, |(_, t)| (EVERY_MS - (now - t)).max(0.0)) + 50.0;
            wake = Some(wake.map_or(left, |w: f64| w.min(left)));
        }
    }
    if let (Some(ms), Some(ctx)) = (wake, &app.collab_ctx) {
        // While a cell is being edited the copy waits for it; check again shortly.
        let ms = if app.editor.is_some() { ms.max(1000.0) } else { ms };
        ctx.request_repaint_after(std::time::Duration::from_millis(ms as u64));
    }
    for uid in gone {
        app.collab.recovery.forget(uid);
    }
    if app.collab.recovery.writing.is_none()
        && let Some((uid, wb, meta)) = next
    {
        match app.collab.recovery.run_dir() {
            Ok(dir) => {
                let (tx, rx) = std::sync::mpsc::channel();
                let w = wb.clone();
                let ctx = app.collab_ctx.clone();
                let spawned = std::thread::Builder::new().name("dexsheets-recovery".into()).spawn(move || {
                    let r = write_item(&dir, uid, &w, &meta);
                    let _ = tx.send((uid, w, r));
                    if let Some(c) = ctx {
                        c.request_repaint();
                    }
                });
                match spawned {
                    Ok(_) => app.collab.recovery.writing = Some(rx),
                    Err(e) => app.collab.recovery.error = Some(e.to_string()),
                }
            }
            Err(e) => {
                // Retried after the usual wait, not every frame.
                app.collab.recovery.written.insert(uid, (Arc::new(Workbook::new()), now));
                app.collab.recovery.error = Some(e);
            }
        }
    }
}

/// The desktop app is exiting normally.
pub fn clean_exit(app: &mut SheetApp) {
    if !crate::dex_ui::desktop() {
        return;
    }
    // An update or roll-back that waited for this exit starts its installer now.
    crate::collab_ui::run_installer(app);
    let dirty: Vec<(u64, Arc<Workbook>, ItemMeta)> = app
        .session
        .documents()
        .iter()
        .filter(|d| d.is_dirty() && app.collab.shared(d.uid).is_none())
        .map(|d| (d.uid, d.wb.clone(), meta_of(d)))
        .collect();
    app.collab.recovery.clean_exit(&dirty);
}

fn when(ms: i64) -> String {
    use chrono::TimeZone;
    match chrono::Local.timestamp_millis_opt(ms).single() {
        Some(t) => t.format("%d %b %Y, %H:%M").to_string(),
        None => "earlier".into(),
    }
}

/// Opens a copy as a new workbook with unsaved changes (Save asks where to keep it), and
/// removes the copy: from now on this run protects it.
fn recover(app: &mut SheetApp, f: &Found) -> Result<(), String> {
    let bytes = std::fs::read(&f.file).map_err(|e| format!("{}: {e}", f.file.display()))?;
    let (wb, _) = gridcraft_engine::io::open_bytes("recovered.xlsx", &bytes).map_err(|e| e.to_string())?;
    let stem = f.meta.title.trim_end_matches(".xlsx").to_string();
    let mut doc = gridcraft_engine::DocState::new(wb, None, format!("{stem} (Recovered)"), gridcraft_engine::Trust::FromFile);
    doc.saved = Arc::new(Workbook::new());
    app.session.add_document(doc);
    discard(f);
    Ok(())
}

fn discard(f: &Found) {
    let _ = std::fs::remove_file(&f.file);
    let _ = std::fs::remove_file(f.file.with_extension("json"));
    if items_in(&f.run).is_empty() {
        let _ = std::fs::remove_dir_all(&f.run);
    }
}

/// `file.recoverUnsaved` and the dialog's buttons: {action?: list|recover|openSaved|compare|discard|discardAll, index?}.
pub fn run_command(app: &mut SheetApp, p: &Json) -> Result<Json, String> {
    let action = p.get("action").and_then(Json::as_str).unwrap_or("list");
    let index = p.get("index").and_then(Json::as_u64).map(|i| i as usize);
    if action == "list" {
        app.collab.recovery.found = app.collab.recovery.scan();
        // The window opens for a menu-style call (`show: true`); a programmatic call gets the list.
        app.collab.recovery.show |= p.get("show").and_then(Json::as_bool) == Some(true);
        return Ok(json!({"items": app.collab.recovery.found.iter().map(|f| json!({
            "title": f.meta.title, "path": f.meta.path, "savedAt": f.meta.saved_at, "cells": f.meta.cells,
        })).collect::<Vec<_>>()}));
    }
    if action == "discardAll" {
        for f in std::mem::take(&mut app.collab.recovery.found) {
            discard(&f);
        }
        app.collab.recovery.show = false;
        return Ok(Json::Null);
    }
    let i = index.ok_or("give the item's `index`")?;
    let f = app.collab.recovery.found.get(i).cloned().ok_or("no such recovered workbook")?;
    match action {
        "recover" => {
            recover(app, &f)?;
            app.collab.recovery.found.remove(i);
        }
        "openSaved" => {
            let path = f.meta.path.clone().filter(|p| Path::new(p).is_file()).ok_or("this workbook has no saved file")?;
            app.open_path(&path);
        }
        "compare" => {
            let path = f.meta.path.clone().filter(|p| Path::new(p).is_file()).ok_or("this workbook has no saved file to compare with")?;
            let r = app.session.run(
                "review.compareFiles",
                json!({"old": path, "new": f.file.to_string_lossy(), "oldLabel": "Last saved", "newLabel": "Recovered"}),
            )?;
            return Ok(r);
        }
        "discard" => {
            discard(&f);
            app.collab.recovery.found.remove(i);
        }
        other => return Err(format!("unknown action \"{other}\"")),
    }
    app.collab.recovery.show = !app.collab.recovery.found.is_empty();
    Ok(Json::Null)
}

/// The Recover Unsaved Work dialog.
pub fn dialog(app: &mut SheetApp, ctx: &egui::Context) {
    if !app.collab.recovery.show {
        return;
    }
    let mut open = true;
    let mut act: Option<(&'static str, Option<usize>)> = None;
    egui::Window::new("Recover unsaved work")
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.set_max_width(520.0);
            ui.label("Dexterity Sheets did not close normally last time. These workbooks had changes that were not saved; a copy of each was kept:");
            ui.add_space(8.0);
            for (i, f) in app.collab.recovery.found.iter().enumerate() {
                ui.group(|ui| {
                    ui.strong(&f.meta.title);
                    let place = f.meta.path.clone().unwrap_or_else(|| "never saved".into());
                    ui.label(format!("{place} · last copy kept {}", when(f.meta.saved_at)));
                    ui.horizontal(|ui| {
                        if ui.button("Recover unsaved changes").clicked() {
                            act = Some(("recover", Some(i)));
                        }
                        let has_file = f.meta.path.as_ref().is_some_and(|p| Path::new(p).is_file());
                        if ui.add_enabled(has_file, egui::Button::new("Open the last saved version")).clicked() {
                            act = Some(("openSaved", Some(i)));
                        }
                        if ui
                            .add_enabled(has_file, egui::Button::new("Compare"))
                            .on_hover_text("What the recovered copy changes against the saved file")
                            .clicked()
                        {
                            act = Some(("compare", Some(i)));
                        }
                        if ui.button("Discard").clicked() {
                            act = Some(("discard", Some(i)));
                        }
                    });
                });
            }
            ui.add_space(8.0);
            ui.label(egui::RichText::new("Recovering opens the copy as a new workbook; your saved file is not changed until you save.").weak());
            ui.horizontal(|ui| {
                if ui
                    .button("Keep for later")
                    .on_hover_text("Search for Recover Unsaved Work (the search box in the title bar) to see them again")
                    .clicked()
                {
                    act = Some(("later", None));
                }
                if ui.button("Discard all").clicked() {
                    act = Some(("discardAll", None));
                }
            });
        });
    if !open {
        act = Some(("later", None));
    }
    match act {
        Some(("later", _)) => app.collab.recovery.show = false,
        Some((a, i)) => {
            let mut p = json!({"action": a});
            if let Some(i) = i {
                p["index"] = json!(i);
            }
            if let Err(e) = run_command(app, &p) {
                app.message = Some(("Recover unsaved work".into(), e));
            }
        }
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("dexsheets-recovery-{tag}-{}-{}", std::process::id(), crate::now_ms() as i64))
    }

    fn book(text: &str) -> Workbook {
        let mut s = gridcraft_engine::Session::new();
        s.new_workbook();
        s.execute("cell.set", json!({"cell": "A1", "input": text})).unwrap();
        (*s.active().unwrap().wb).clone()
    }

    #[test]
    fn a_crashed_run_is_found_and_a_live_or_clean_one_is_not() {
        let b = base("scan");
        // A run that crashed: copies left, lock released (its process is gone).
        let mut crashed = Recovery::with_base(b.clone());
        let meta = ItemMeta { title: "Project_Tracker.xlsx".into(), path: Some(r"D:\Jobs\Project_Tracker.xlsx".into()), saved_at: 1, cells: 1 };
        crashed.write_copy(7, &book("unsaved 7Q"), &meta).unwrap();
        let crashed_dir = crashed.run.as_ref().unwrap().dir.clone();
        // A run still going: its lock is held, so it is never offered.
        let mut live = Recovery::with_base(b.clone());
        live.write_copy(9, &book("still being edited"), &ItemMeta { title: "Live".into(), ..ItemMeta::default() }).unwrap();
        // The crashed run's process ends without cleaning up: its lock goes, its files stay.
        drop(crashed.run.take());

        let mut next = Recovery::with_base(b.clone());
        let found = next.scan();
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].meta.title, "Project_Tracker.xlsx");
        assert_eq!(found[0].run, crashed_dir);
        let (wb, _) = gridcraft_engine::io::open_bytes("x.xlsx", &std::fs::read(&found[0].file).unwrap()).unwrap();
        let a1 = wb.sheet(0).and_then(|s| s.cell(gridcraft_engine::core::CellRef::new(0, 0))).map(|c| c.value.clone());
        assert_eq!(a1, Some(gridcraft_engine::core::Value::Text("unsaved 7Q".into())), "the copy holds the unsaved content");

        // A clean exit leaves nothing to offer.
        live.clean_exit(&[]);
        discard(&found[0]);
        assert!(next.scan().is_empty());
        let _ = std::fs::remove_dir_all(&b);
    }

    #[test]
    fn exiting_with_unsaved_changes_keeps_a_final_copy() {
        let b = base("exit");
        let mut r = Recovery::with_base(b.clone());
        let wb = Arc::new(book("typed just before Windows restarted"));
        r.clean_exit(&[(3, wb, ItemMeta { title: "Book1".into(), ..ItemMeta::default() })]);
        let dir = r.run.take().unwrap().dir;
        assert!(dir.join("3.xlsx").is_file() && dir.join("3.json").is_file());
        assert_eq!(Recovery::with_base(b.clone()).scan().len(), 1);
        let _ = std::fs::remove_dir_all(&b);
    }
}
