//! Releases, feedback and rollback in the app (Dexterity Sheets): What's New after an update,
//! Report a Problem (per feature, to the office server's administrator), Roll Back to an
//! earlier version, the update channel, the About details, the logo and the start-up splash.
//!
//! This is the application update system only. Rolling the app back never touches a workbook,
//! a shared workbook's versions or the crash-recovery copies: those are separate systems
//! (`docs/dexterity-roadmap.md`). File formats on the server: `docs/dexterity-admin-data.md`.

use std::collections::BTreeMap;

use egui::{Color32, Painter, Pos2, Rect, Stroke, vec2};
use gridcraft_collab::proto::{self, ClientMsg, Feedback, UpdateInfo};
use serde::{Deserialize, Serialize};
use serde_json::{Value as Json, json};

use crate::SheetApp;

/// The repository releases come from (shown in About).
pub const REPOSITORY: &str = "vikramdex-ops/dexterity-sheets";
/// Build identity, set by CI (`DEXSHEETS_BUILD`, e.g. "1234 (c6d43346)").
pub const BUILD: Option<&str> = option_env!("DEXSHEETS_BUILD");
/// Release notes compiled in, so What's New works without the server.
const CHANGES: &str = include_str!("../../../../packaging/dexterity/CHANGES.md");

/// The feature areas a problem can be reported against: (id, label).
pub const FEATURES: &[(&str, &str)] = &[
    ("opening", "Opening spreadsheets"),
    ("saving", "Saving spreadsheets"),
    ("formulas", "Formulas and calculations"),
    ("formatting", "Formatting and conditional formatting"),
    ("importExport", "Importing or exporting files"),
    ("sharing", "Sharing and permissions"),
    ("notifications", "Notifications"),
    ("startup", "Application startup"),
    ("performance", "Performance"),
    ("other", "Other"),
];
const SEVERITIES: &[(&str, &str)] = &[("low", "Low"), ("medium", "Medium"), ("high", "High"), ("blocking", "Blocks my work")];
/// Most of the app's log sent with a report (when the person agrees).
const LOG_TAIL: usize = 64 * 1024;
const USAGE_EVERY_MS: f64 = 3600.0 * 1000.0;

/// Kept in `%APPDATA%\Dexterity Sheets\updates.json`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Prefs {
    /// `stable` or `testing`; empty = the server's default.
    pub channel: String,
    /// Send command counts (never content) for the administrator's feature-adoption report.
    pub share_usage: bool,
    /// The version that ran last time: a different one means an update (or rollback) happened.
    pub last_version: String,
    /// Set when this PC rolled back from that version (pre-ticks "I rolled back").
    pub rolled_back_from: Option<String>,
}

impl Default for Prefs {
    fn default() -> Prefs {
        Prefs { channel: String::new(), share_usage: true, last_version: String::new(), rolled_back_from: None }
    }
}

#[derive(Default)]
pub struct Form {
    pub features: Vec<String>,
    pub description: String,
    pub expected: String,
    pub actual: String,
    pub severity: String,
    pub after_update: bool,
    pub rolled_back: bool,
    pub logs: bool,
}

#[derive(Default)]
pub struct Releases {
    loaded: bool,
    pub prefs: Prefs,
    /// This version as the office server knows it (status and note), and the older ones it holds.
    pub current: Option<UpdateInfo>,
    pub older: Vec<UpdateInfo>,
    pub policy: String,
    pub whats_new: bool,
    /// The version changed since the last run (an update or a rollback).
    pub updated: bool,
    pub report: bool,
    pub rollback: bool,
    pub form: Form,
    /// The status already told about (so a note shows once per status).
    told: Option<(String, String)>,
    usage_at: f64,
    auto_tried: Option<String>,
    pub splash_until: f64,
    health_sent: bool,
}

fn version() -> &'static str {
    gridcraft_collab::APP_VERSION.trim()
}

fn prefs_path() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("APPDATA")?;
    Some(std::path::PathBuf::from(base).join("Dexterity Sheets").join("updates.json"))
}

fn load_prefs() -> Prefs {
    if !crate::dex_ui::desktop() {
        return Prefs::default();
    }
    prefs_path().and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save_prefs(p: &Prefs) {
    if !crate::dex_ui::desktop() {
        return;
    }
    let Some(path) = prefs_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(b) = serde_json::to_vec_pretty(p) {
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, b).is_ok() {
            let _ = std::fs::rename(&tmp, &path);
        }
    }
}

/// The release notes of `v` from the compiled-in changelog (the Unreleased section for a
/// development build).
pub fn notes_for(v: &str) -> (String, String) {
    let mut title = None;
    let mut body = Vec::new();
    for line in CHANGES.lines() {
        if let Some(h) = line.strip_prefix("## ") {
            if title.is_some() {
                break;
            }
            if h.split_whitespace().next() == Some(v) {
                title = Some(h.to_string());
            }
            continue;
        }
        if title.is_some() {
            body.push(line);
        }
    }
    if title.is_none() {
        // Not released under this number yet: what is in the works.
        let mut on = false;
        for line in CHANGES.lines() {
            if let Some(h) = line.strip_prefix("## ") {
                if on {
                    break;
                }
                on = h.trim() == "Unreleased";
                continue;
            }
            if on {
                body.push(line);
            }
        }
        title = Some(format!("{v} (in development)"));
    }
    let text = body.iter().map(|l| l.trim_end()).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n");
    (title.unwrap_or_default(), text.replace("**", ""))
}

/// The update check, with this PC's channel.
pub fn check_msg(app: &SheetApp) -> ClientMsg {
    let c = &app.collab.releases.prefs.channel;
    ClientMsg::CheckUpdate { version: version().into(), channel: (!c.is_empty()).then(|| c.clone()) }
}

/// Every frame: notices a new version, sends the adoption counts about once an hour, and the
/// crash-recovery health note once.
pub fn tick(app: &mut SheetApp) {
    let r = &mut app.collab.releases;
    if !r.loaded {
        r.loaded = true;
        r.prefs = load_prefs();
        r.splash_until = crate::clock::monotonic_ms() + 900.0;
        if !r.prefs.last_version.is_empty() && r.prefs.last_version != version() {
            r.updated = true;
            r.whats_new = true;
            r.form.after_update = true;
        }
        r.form.rolled_back = r.prefs.rolled_back_from.is_some();
        if r.prefs.last_version != version() {
            r.prefs.last_version = version().into();
            save_prefs(&r.prefs);
        }
    }
    if !app.collab.signed_in() {
        return;
    }
    if !app.collab.releases.health_sent && !app.collab.recovery.found.is_empty() {
        app.collab.releases.health_sent = true;
        let n = app.collab.recovery.found.len();
        app.collab.send(ClientMsg::Health {
            version: version().into(),
            kind: "recovered".into(),
            detail: format!("{n} workbook(s) with unsaved work offered for recovery"),
        });
    }
    let now = crate::clock::monotonic_ms();
    if app.collab.releases.prefs.share_usage && now - app.collab.releases.usage_at >= USAGE_EVERY_MS {
        app.collab.releases.usage_at = now;
        let counts: BTreeMap<String, u64> = std::mem::take(&mut app.session.dex.usage);
        if !counts.is_empty() {
            app.collab.send(ClientMsg::Usage { version: version().into(), counts });
        }
    }
}

/// The server's answer to the update check: this version's status, what it can roll back to,
/// and the policy (`auto` installs a newer version when nothing is unsaved).
pub fn on_update(app: &mut SheetApp, update: Option<&UpdateInfo>, current: Option<UpdateInfo>, older: Vec<UpdateInfo>, policy: String) {
    app.collab.releases.older = older;
    app.collab.releases.policy = policy;
    if let Some(c) = &current
        && c.status != "ok"
        && app.collab.releases.told.as_ref() != Some(&(c.version.clone(), c.status.clone()))
    {
        app.collab.releases.told = Some((c.version.clone(), c.status.clone()));
        let rollback_to = app.collab.releases.older.first().map(|u| u.version.clone());
        let note = if c.note.is_empty() { String::new() } else { format!("\n\n{}", c.note) };
        let text = match (c.status.as_str(), rollback_to) {
            ("blocked", Some(to)) => format!(
                "The administrator has marked Dexterity Sheets {} as not safe to use.{note}\n\nRoll back to {to} with Help › Roll Back to an Earlier Version (your workbooks are not touched).",
                c.version
            ),
            ("blocked", None) => format!("The administrator has marked Dexterity Sheets {} as not safe to use.{note}", c.version),
            ("deprecated", _) => format!("Dexterity Sheets {} is no longer supported here.{note}", c.version),
            _ => format!("Dexterity Sheets {} has a known issue.{note}", c.version),
        };
        app.message = Some(("Dexterity Sheets".into(), text));
    }
    app.collab.releases.current = current;
    // Policy "auto": install a newer version on its own, once, when no workbook is unsaved.
    if app.collab.releases.policy == "auto"
        && let Some(u) = update
        && app.collab.releases.auto_tried.as_deref() != Some(u.version.as_str())
        && !app.session.documents().iter().any(|d| d.is_dirty())
    {
        app.collab.releases.auto_tried = Some(u.version.clone());
        app.collab.update_state = Some(format!("Downloading {}...", u.version));
        app.collab.send(ClientMsg::GetUpdate { version: u.version.clone() });
    }
}

/// Rolling back: the installer of an older version is about to run.
pub fn rolling_back(app: &mut SheetApp, to: &str) {
    app.collab.releases.prefs.rolled_back_from = Some(version().into());
    save_prefs(&app.collab.releases.prefs);
    log::info!("rolling back from {} to {to}", version());
}

fn log_tail() -> Option<String> {
    let base = std::env::var_os("APPDATA")?;
    let path = std::path::PathBuf::from(base).join("Dexterity Sheets").join("logs").join("gridcraft.log");
    let bytes = std::fs::read(path).ok()?;
    let start = bytes.len().saturating_sub(LOG_TAIL);
    Some(String::from_utf8_lossy(bytes.get(start..).unwrap_or_default()).into_owned())
}

fn os_name() -> String {
    format!("{} {}", std::env::consts::OS, std::env::var("OS").unwrap_or_default()).trim().to_string()
}

fn send_report(app: &mut SheetApp) -> Result<Json, String> {
    let f = &app.collab.releases.form;
    if f.features.is_empty() {
        return Err("Tick at least one area that has the problem.".into());
    }
    if f.description.trim().is_empty() {
        return Err("Describe the problem in a sentence or two.".into());
    }
    if !app.collab.signed_in() {
        return Err("Sign in to the office server first (File › Shared): reports go to its administrator.".into());
    }
    let severity = if SEVERITIES.iter().any(|(k, _)| *k == f.severity) { f.severity.clone() } else { "medium".into() };
    let report = Feedback {
        version: version().into(),
        os: os_name(),
        features: f.features.clone(),
        description: f.description.trim().into(),
        expected: f.expected.trim().into(),
        actual: f.actual.trim().into(),
        severity,
        after_update: f.after_update,
        rolled_back: f.rolled_back,
        logs: if f.logs { log_tail() } else { None },
    };
    app.collab.send(ClientMsg::Feedback { report });
    app.collab.releases.form =
        Form { after_update: app.collab.releases.updated, rolled_back: app.collab.releases.prefs.rolled_back_from.is_some(), ..Form::default() };
    app.collab.releases.report = false;
    Ok(json!({"sent": true}))
}

/// The Help commands (the engine's entries are the catalogue; the window does the work).
pub fn run_command(app: &mut SheetApp, id: &str, p: &Json) -> Option<Result<Json, String>> {
    let s = |k: &str| p.get(k).and_then(Json::as_str).map(str::to_string);
    Some(match id {
        "help.whatsNew" => {
            app.collab.releases.whats_new = true;
            let (title, notes) = notes_for(version());
            Ok(json!({"version": version(), "title": title, "notes": notes}))
        }
        "help.reportProblem" => {
            let f = &mut app.collab.releases.form;
            if let Some(a) = p.get("features").and_then(Json::as_array) {
                f.features = a.iter().filter_map(Json::as_str).filter(|x| FEATURES.iter().any(|(k, _)| k == x)).map(str::to_string).collect();
            }
            for (k, field) in
                [("description", &mut f.description), ("expected", &mut f.expected), ("actual", &mut f.actual), ("severity", &mut f.severity)]
            {
                if let Some(v) = p.get(k).and_then(Json::as_str) {
                    *field = v.to_string();
                }
            }
            for (k, field) in [("afterUpdate", &mut f.after_update), ("rolledBack", &mut f.rolled_back), ("logs", &mut f.logs)] {
                if let Some(v) = p.get(k).and_then(Json::as_bool) {
                    *field = v;
                }
            }
            if p.get("send").and_then(Json::as_bool) == Some(true) {
                send_report(app)
            } else {
                app.collab.releases.report = true;
                Ok(Json::Null)
            }
        }
        "help.rollBack" => match s("version") {
            None => {
                app.collab.releases.rollback = true;
                if app.collab.signed_in() {
                    app.collab.send(check_msg(app));
                }
                Ok(
                    json!({"current": version(), "older": app.collab.releases.older.iter().map(|u| json!({"version": u.version, "status": u.status, "note": u.note})).collect::<Vec<_>>()}),
                )
            }
            Some(v) => {
                if !app.collab.signed_in() {
                    return Some(Err("Sign in to the office server first (File › Shared): earlier versions come from it.".into()));
                }
                if !app.collab.releases.older.iter().any(|u| u.version == v) {
                    return Some(Err(format!("The office server has no {v} to roll back to.")));
                }
                app.collab.update_state = Some(format!("Downloading {v}..."));
                app.collab.send(ClientMsg::GetUpdate { version: v.clone() });
                Ok(json!({"rollingBackTo": v}))
            }
        },
        "help.updateChannel" => {
            if let Some(c) = s("channel") {
                if c != "stable" && c != "testing" {
                    return Some(Err("channel: stable or testing".into()));
                }
                app.collab.releases.prefs.channel = c;
                save_prefs(&app.collab.releases.prefs);
                if app.collab.signed_in() {
                    app.collab.send(check_msg(app));
                }
            }
            let c = app.collab.releases.prefs.channel.clone();
            Ok(json!({"channel": if c.is_empty() { "stable".to_string() } else { c }}))
        }
        _ => return None,
    })
}

/// Help › About: build, update channel and repository, beside the version line.
pub fn about_details(ui: &mut egui::Ui) {
    let channel = load_prefs().channel;
    let (rect, _) = ui.allocate_exact_size(vec2(56.0, 56.0), egui::Sense::hover());
    paint_logo(ui.painter(), rect);
    egui::Grid::new("dex_about").num_columns(2).spacing(vec2(12.0, 4.0)).show(ui, |ui| {
        ui.label("Build");
        ui.label(BUILD.unwrap_or("local build"));
        ui.end_row();
        ui.label("Update channel");
        ui.label(if channel == "testing" { "Testing" } else { "Stable" });
        ui.end_row();
        ui.label("Repository");
        ui.hyperlink_to(REPOSITORY, format!("https://github.com/{REPOSITORY}"));
        ui.end_row();
    });
}

/// The app icon ("linked cells", `packaging/dexterity/make_icon.py`) drawn with the painter, so
/// About and the splash need no image loader. Same geometry, in a 512-unit square.
pub fn paint_logo(p: &Painter, rect: Rect) {
    let s = rect.width() / 512.0;
    let at = |x: f32, y: f32| Pos2::new(rect.left() + x * s, rect.top() + y * s);
    let indigo = Color32::from_rgb(0x1e, 0x2a, 0x78);
    let teal = Color32::from_rgb(0x0e, 0x8c, 0x8c);
    // The rounded tile as a fan of triangles, each corner point coloured by where it lies on the
    // diagonal: indigo top left, teal bottom right.
    let r = 112.0 * s;
    let colour = |q: Pos2| lerp(indigo, teal, ((q.x - rect.left()) + (q.y - rect.top())) / (rect.width() + rect.height()).max(1.0));
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.center(), colour(rect.center()));
    let corners = [
        (Pos2::new(rect.right() - r, rect.bottom() - r), 0.0_f32),
        (Pos2::new(rect.left() + r, rect.bottom() - r), 90.0),
        (Pos2::new(rect.left() + r, rect.top() + r), 180.0),
        (Pos2::new(rect.right() - r, rect.top() + r), 270.0),
    ];
    for (c, start) in corners {
        for k in 0..=8 {
            let a = (start + k as f32 * 90.0 / 8.0).to_radians();
            let q = c + vec2(a.cos(), a.sin()) * r;
            mesh.colored_vertex(q, colour(q));
        }
    }
    let n = 36u32;
    for i in 1..=n {
        mesh.add_triangle(0, i, if i == n { 1 } else { i + 1 });
    }
    p.add(egui::Shape::mesh(mesh));
    let lit = [(0usize, 1usize), (1, 2), (2, 0)];
    for row in 0..3 {
        for col in 0..3 {
            let x = 88.0 + col as f32 * 120.0;
            let y = 88.0 + row as f32 * 120.0;
            let fill = if lit.contains(&(col, row)) { Color32::WHITE } else { Color32::from_white_alpha(56) };
            p.rect_filled(Rect::from_min_max(at(x, y), at(x + 96.0, y + 96.0)), 20.0 * s, fill);
        }
    }
    let amber = Color32::from_rgb(0xff, 0xb0, 0x20);
    let pts: Vec<Pos2> = lit.iter().map(|&(c, r)| at(136.0 + c as f32 * 120.0, 136.0 + r as f32 * 120.0)).collect();
    p.add(egui::Shape::line(pts.clone(), Stroke::new(30.0 * s, amber)));
    for q in pts {
        p.circle_filled(q, 25.0 * s, amber);
        p.circle_filled(q, 9.0 * s, Color32::WHITE);
    }
}

fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// The windows this module owns: the start-up splash, What's New, Report a Problem, Roll Back.
pub fn windows(app: &mut SheetApp, ctx: &egui::Context) {
    splash(app, ctx);
    whats_new(app, ctx);
    report(app, ctx);
    rollback(app, ctx);
}

/// The logo over the window while the app finishes starting (fonts, the first workbook, files
/// opened from Explorer), fading out.
fn splash(app: &mut SheetApp, ctx: &egui::Context) {
    let now = crate::clock::monotonic_ms();
    let left = app.collab.releases.splash_until - now;
    if left <= 0.0 {
        return;
    }
    let alpha = (left / 300.0).clamp(0.0, 1.0) as f32;
    let screen = ctx.content_rect();
    egui::Area::new(egui::Id::unique("dex_splash")).order(egui::Order::Foreground).fixed_pos(screen.min).interactable(false).show(ctx, |ui| {
        let p = ui.painter();
        p.rect_filled(screen, 0.0, ctx.global_style().visuals.panel_fill.gamma_multiply(alpha));
        let side = 128.0;
        let logo = Rect::from_center_size(screen.center() - vec2(0.0, 24.0), vec2(side, side));
        if alpha > 0.05 {
            paint_logo(p, logo);
            p.text(
                logo.center_bottom() + vec2(0.0, 18.0),
                egui::Align2::CENTER_TOP,
                "Dexterity Sheets",
                egui::FontId::proportional(20.0),
                ctx.global_style().visuals.text_color().gamma_multiply(alpha),
            );
        }
    });
    ctx.request_repaint();
}

fn whats_new(app: &mut SheetApp, ctx: &egui::Context) {
    if !app.collab.releases.whats_new {
        return;
    }
    let (title, notes) = notes_for(version());
    let mut open = true;
    let mut report = false;
    egui::Window::new(format!("Dexterity Sheets {title} — What's New"))
        .collapsible(false)
        .open(&mut open)
        .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.set_max_width(560.0);
            egui::ScrollArea::vertical().max_height(340.0).show(ui, |ui| {
                if notes.is_empty() {
                    ui.label("No changes are listed for this version.");
                }
                for line in notes.lines() {
                    ui.label(line.trim_start_matches("- ").to_string());
                }
            });
            if let Some(c) = &app.collab.releases.current
                && !c.note.is_empty()
            {
                ui.add_space(6.0);
                ui.colored_label(Color32::from_rgb(0xd0, 0x70, 0x00), &c.note);
            }
            ui.separator();
            ui.label("Something not working since this version?");
            ui.horizontal(|ui| {
                if ui.button("Report a problem with this version").clicked() {
                    report = true;
                }
                if ui
                    .add_enabled(!app.collab.releases.older.is_empty(), egui::Button::new("Roll back to an earlier version"))
                    .on_disabled_hover_text("Sign in to the office server: earlier versions come from it")
                    .clicked()
                {
                    app.collab.releases.rollback = true;
                }
            });
        });
    if report {
        app.collab.releases.report = true;
        open = false;
    }
    if !open {
        app.collab.releases.whats_new = false;
    }
}

fn report(app: &mut SheetApp, ctx: &egui::Context) {
    if !app.collab.releases.report {
        return;
    }
    let mut open = true;
    let mut send = false;
    let signed_in = app.collab.signed_in();
    egui::Window::new(format!("Report a problem with Dexterity Sheets {}", version()))
        .collapsible(false)
        .open(&mut open)
        .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.set_max_width(560.0);
            let f = &mut app.collab.releases.form;
            ui.label("Which areas have the problem? Tick one or more:");
            egui::Grid::new("dex_feat").num_columns(2).show(ui, |ui| {
                for (i, (k, label)) in FEATURES.iter().enumerate() {
                    let mut on = f.features.iter().any(|x| x == k);
                    if ui.checkbox(&mut on, *label).changed() {
                        f.features.retain(|x| x != k);
                        if on {
                            f.features.push((*k).to_string());
                        }
                    }
                    if i % 2 == 1 {
                        ui.end_row();
                    }
                }
            });
            ui.add_space(6.0);
            ui.label("What happened?");
            ui.add(egui::TextEdit::multiline(&mut f.description).desired_rows(3).desired_width(f32::INFINITY));
            ui.label("What did you expect?");
            ui.add(egui::TextEdit::singleline(&mut f.expected).desired_width(f32::INFINITY));
            ui.label("What did you get instead?");
            ui.add(egui::TextEdit::singleline(&mut f.actual).desired_width(f32::INFINITY));
            ui.horizontal(|ui| {
                ui.label("How bad is it?");
                for (k, label) in SEVERITIES {
                    ui.radio_value(&mut f.severity, (*k).to_string(), *label);
                }
            });
            ui.checkbox(&mut f.after_update, "It started after an update");
            ui.checkbox(&mut f.rolled_back, "I rolled back to an earlier version");
            ui.checkbox(&mut f.logs, "Include diagnostic logs (what the app did, never your workbooks' contents)");
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(signed_in, egui::Button::new("Send to the administrator"))
                    .on_disabled_hover_text("Sign in to the office server first (File › Shared)")
                    .clicked()
                {
                    send = true;
                }
                if !signed_in {
                    ui.label(egui::RichText::new("Sign in to the office server to send it.").weak());
                }
            });
        });
    if send {
        match send_report(app) {
            Ok(_) => app.toast = Some(("Sending your report…".into(), crate::now_ms())),
            Err(e) => app.message = Some(("Report a problem".into(), e)),
        }
    }
    if !open {
        app.collab.releases.report = false;
    }
}

fn rollback(app: &mut SheetApp, ctx: &egui::Context) {
    if !app.collab.releases.rollback {
        return;
    }
    let mut open = true;
    let mut pick = None;
    egui::Window::new("Roll back to an earlier version").collapsible(false).open(&mut open).anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0)).show(
        ctx,
        |ui| {
            ui.set_max_width(520.0);
            ui.label(format!(
                "You have Dexterity Sheets {}. Rolling back installs an earlier version from the office server and restarts the app.",
                version()
            ));
            ui.label(
                egui::RichText::new("Only the app changes: your workbooks, shared workbooks and their versions stay exactly as they are.").strong(),
            );
            ui.add_space(6.0);
            if !app.collab.signed_in() {
                ui.label("Sign in to the office server first (File › Shared): earlier versions come from it.");
            } else if app.collab.releases.older.is_empty() {
                ui.label("The office server holds no earlier version to roll back to.");
            }
            for u in &app.collab.releases.older {
                ui.horizontal(|ui| {
                    ui.strong(&u.version);
                    if u.status == "knownIssue" {
                        ui.colored_label(Color32::from_rgb(0xd0, 0x70, 0x00), "known issue");
                    }
                    if !u.note.is_empty() {
                        ui.label(egui::RichText::new(&u.note).weak());
                    }
                    if ui.button("Roll back to this version").clicked() {
                        pick = Some(u.version.clone());
                    }
                });
            }
            ui.add_space(6.0);
            ui.label(egui::RichText::new("Please also tell the administrator what went wrong: Help › Report a Problem.").weak());
        },
    );
    if let Some(v) = pick {
        let r = run_command(app, "help.rollBack", &json!({"version": v}));
        if let Some(Err(e)) = r {
            app.message = Some(("Roll back".into(), e));
        } else {
            app.collab.releases.form.rolled_back = true;
            open = false;
        }
    }
    if !open {
        app.collab.releases.rollback = false;
    }
}

/// True when installing `to` moves this PC to an older version.
pub fn is_rollback(to: &str) -> bool {
    proto::version_newer(version(), to)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_come_from_the_changelog_section_of_the_version() {
        let (title, notes) = notes_for("1.2.2");
        assert!(title.starts_with("1.2.2"), "{title}");
        assert!(notes.contains("server's log file"), "{notes}");
        assert!(!notes.contains("**"));
        // A version not released yet shows what is in the works.
        let (title, notes) = notes_for("99.0.0");
        assert!(title.contains("in development"));
        assert!(!notes.is_empty());
    }

    #[test]
    fn rollback_means_an_older_version() {
        assert!(is_rollback("0.0.1"));
        assert!(!is_rollback("999.0.0"));
    }
}
