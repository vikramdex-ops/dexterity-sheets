//! Templates (Dexterity Sheets): the office's approved templates from the office server and the
//! person's own, on File › New and in the Templates window. A spreadsheet made from a template is
//! a new, unsaved copy: publishing a new version of the template never changes it (people who
//! used the template are told a new version exists). Only administrators publish office
//! templates; anyone keeps personal ones in `%APPDATA%\Dexterity Sheets\Templates`.

use std::path::PathBuf;
use std::sync::Arc;

use egui::{RichText, Ui, vec2};
use gridcraft_collab::proto::{self, ClientMsg, TemplateInfo};
use gridcraft_engine::model::Workbook;
use serde_json::{Value as Json, json};

use crate::SheetApp;

/// How long the Templates window waits for a template asked of the office server before it
/// says the server did not answer.
pub const WAIT_MS: f64 = 20_000.0;

#[derive(Default)]
pub struct Templates {
    pub window: bool,
    pub list: Vec<TemplateInfo>,
    asked: bool,
    search: String,
    selected: Option<String>,
    /// Downloaded template: (id, workbook), for the preview and for creating.
    got: Option<(String, Arc<Workbook>)>,
    create_when_got: Option<String>,
    /// When the template being created from was asked for (monotonic ms): the window keeps a
    /// spinner until it arrives, and gives up after [`WAIT_MS`].
    waiting_since: Option<f64>,
    /// Why the last create or save did not happen, shown in the window.
    error: Option<String>,
    name: String,
    category: String,
    description: String,
    mandatory: String,
    office: bool,
    /// Replace a personal template of the same name.
    overwrite: bool,
    /// Where personal templates are kept when not the default folder (tests).
    dir: Option<PathBuf>,
    /// The personal templates as last read from disk. Read again when the Templates window or
    /// File › New opens and after a save, not every frame.
    mine: Vec<(String, PathBuf)>,
    mine_fresh: bool,
    in_new_page: bool,
}

impl Templates {
    /// Keeps personal templates in `dir` instead of `%APPDATA%\Dexterity Sheets\Templates`.
    pub fn set_personal_dir(&mut self, dir: PathBuf) {
        self.dir = Some(dir);
        self.mine_fresh = false;
    }

    /// The template asked of the office server whose arrival creates a spreadsheet, and when it
    /// was asked for (the window shows it as waiting from then on).
    pub fn wait_for(&mut self, id: String, since_ms: f64) {
        self.create_when_got = Some(id);
        self.waiting_since = Some(since_ms);
        self.error = None;
    }

    /// The template the window is waiting for, if any.
    pub fn waiting(&self) -> Option<&str> {
        self.create_when_got.as_deref()
    }

    /// The window's current create or save problem, if any.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    fn personal_dir(&self) -> Option<PathBuf> {
        if let Some(d) = &self.dir {
            return Some(d.clone());
        }
        Some(PathBuf::from(std::env::var_os("APPDATA")?).join("Dexterity Sheets").join("Templates"))
    }

    /// The personal templates, read from disk when stale.
    fn mine(&mut self) -> &[(String, PathBuf)] {
        if !self.mine_fresh {
            self.mine = personal(self.personal_dir());
            self.mine_fresh = true;
        }
        &self.mine
    }
}

fn personal(dir: Option<PathBuf>) -> Vec<(String, PathBuf)> {
    let Some(dir) = dir else { return Vec::new() };
    let mut v: Vec<(String, PathBuf)> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("xlsx")))
                .filter_map(|p| Some((p.file_stem()?.to_string_lossy().into_owned(), p)))
                .take(500)
                .collect()
        })
        .unwrap_or_default();
    v.sort_by_key(|(n, _)| n.to_lowercase());
    v
}

/// A new, unsaved spreadsheet with the template's content (named like Excel's: "Name1").
fn create(app: &mut SheetApp, name: &str, wb: Workbook) {
    let mut doc = gridcraft_engine::DocState::new(wb, None, format!("{name}1"), gridcraft_engine::Trust::FromFile);
    doc.saved = Arc::new(Workbook::new());
    app.session.add_document(doc);
    app.backstage = None;
}

fn ask_list(app: &mut SheetApp) {
    if app.collab.signed_in() && !app.collab.templates.asked {
        app.collab.templates.asked = true;
        app.collab.send(ClientMsg::ListTemplates);
    }
}

pub fn on_list(app: &mut SheetApp, list: Vec<TemplateInfo>) {
    app.collab.templates.list = list;
}

pub fn on_template(app: &mut SheetApp, info: TemplateInfo, xlsx: &str, sha256: &str) {
    let wb = proto::unb64_checked(xlsx, sha256)
        .map_err(|e| e.to_string())
        .and_then(|b| gridcraft_engine::io::open_bytes("template.xlsx", &b).map(|(w, _)| w).map_err(|e| e.to_string()));
    let awaited = app.collab.templates.create_when_got.as_deref() == Some(info.id.as_str());
    match wb {
        Ok(wb) => {
            if awaited {
                app.collab.templates.create_when_got = None;
                app.collab.templates.waiting_since = None;
                app.collab.templates.window = false;
                create(app, &info.name, wb.clone());
            }
            app.collab.templates.got = Some((info.id, Arc::new(wb)));
        }
        Err(e) => {
            let msg = format!("Could not open the template \"{}\": {e}", info.name);
            if awaited {
                app.collab.templates.create_when_got = None;
                app.collab.templates.waiting_since = None;
                app.collab.templates.error = Some(msg.clone());
            }
            app.message = Some(("Templates".into(), msg));
        }
    }
}

/// `file.newFromTemplate {id? | personal? | show?}` (with neither template named: the office
/// and personal lists; `show: true`, a menu-style call, opens the Templates window too) and
/// `file.saveAsTemplate {name, office?, category?, description?, mandatory?, overwrite?}`.
pub fn run_command(app: &mut SheetApp, id: &str, p: &Json) -> Option<Result<Json, String>> {
    let s = |k: &str| p.get(k).and_then(Json::as_str).map(str::to_string);
    Some(match id {
        "file.newFromTemplate" => {
            if let Some(name) = s("personal") {
                let path = personal(app.collab.templates.personal_dir()).into_iter().find(|(n, _)| n.eq_ignore_ascii_case(&name)).map(|(_, p)| p);
                return Some(
                    match path.ok_or_else(|| format!("no personal template \"{name}\"")).and_then(|p| {
                        let b = std::fs::read(&p).map_err(|e| e.to_string())?;
                        gridcraft_engine::io::open_bytes("template.xlsx", &b).map(|(w, _)| w).map_err(|e| e.to_string())
                    }) {
                        Ok(wb) => {
                            create(app, &name, wb);
                            Ok(Json::Null)
                        }
                        Err(e) => Err(e),
                    },
                );
            }
            match s("id") {
                Some(tid) => {
                    if !app.collab.signed_in() {
                        return Some(Err("Sign in to the office server first (File › Shared): office templates come from it.".into()));
                    }
                    app.collab.templates.create_when_got = Some(tid.clone());
                    app.collab.send(ClientMsg::GetTemplate { id: tid.clone() });
                    Ok(json!({"template": tid}))
                }
                None => {
                    if p.get("show").and_then(Json::as_bool) == Some(true) {
                        app.collab.templates.window = true;
                        app.collab.templates.error = None;
                    }
                    app.collab.templates.asked = false;
                    app.collab.templates.mine_fresh = false;
                    ask_list(app);
                    let t = &mut app.collab.templates;
                    let personal: Vec<String> = t.mine().iter().map(|(n, _)| n.clone()).collect();
                    Ok(
                        json!({"office": t.list.iter().map(|t| json!({"id": t.id, "name": t.name, "category": t.category, "version": t.version})).collect::<Vec<_>>(),
                              "personal": personal}),
                    )
                }
            }
        }
        "file.saveAsTemplate" => {
            let Some(d) = app.session.active() else { return Some(Err("no workbook open".into())) };
            let name = s("name").unwrap_or_else(|| d.title.trim_end_matches(".xlsx").to_string());
            let name: String = name.chars().filter(|c| !"\\/:*?\"<>|".contains(*c)).collect::<String>().trim().to_string();
            if name.is_empty() {
                return Some(Err("give the template a name".into()));
            }
            let bytes = match gridcraft_engine::io::save_bytes(&d.wb, "template.xlsx") {
                Ok(b) => b,
                Err(e) => return Some(Err(e.to_string())),
            };
            if p.get("office").and_then(Json::as_bool) == Some(true) {
                if !app.collab.signed_in() {
                    return Some(Err("Sign in to the office server first (File › Shared).".into()));
                }
                let existing = app.collab.templates.list.iter().find(|t| t.name.eq_ignore_ascii_case(&name)).map(|t| t.id.clone());
                app.collab.send(ClientMsg::PublishTemplate {
                    id: existing,
                    name: name.clone(),
                    category: s("category").unwrap_or_default(),
                    description: s("description").unwrap_or_default(),
                    mandatory: s("mandatory").unwrap_or_default(),
                    sha256: Some(proto::sha256_hex(&bytes)),
                    xlsx: proto::b64(&bytes),
                });
                Ok(json!({"published": name}))
            } else {
                let Some(dir) = app.collab.templates.personal_dir() else { return Some(Err("no folder for personal templates".into())) };
                let path = dir.join(format!("{name}.xlsx"));
                if path.exists() && p.get("overwrite").and_then(Json::as_bool) != Some(true) {
                    return Some(Err(format!(
                        "You already have a template named \"{name}\": give this one another name, or pass `overwrite: true` to replace it."
                    )));
                }
                app.collab.templates.mine_fresh = false;
                match std::fs::create_dir_all(&dir)
                    .map_err(|e| e.to_string())
                    .and_then(|()| gridcraft_engine::io::write_file(&path.to_string_lossy(), &bytes).map_err(|e| e.to_string()))
                {
                    Ok(()) => Ok(json!({"saved": path.to_string_lossy()})),
                    Err(e) => Err(e),
                }
            }
        }
        _ => return None,
    })
}

/// File › New: the office templates and the person's own, below the blank workbook.
pub fn new_page(app: &mut SheetApp, ui: &mut Ui, close: &mut bool) {
    ask_list(app);
    ui.add_space(18.0);
    ui.heading("Templates");
    let t = &mut app.collab.templates;
    if !t.in_new_page {
        // The page just opened: read the personal templates once, not every frame.
        t.in_new_page = true;
        t.mine_fresh = false;
    }
    let mine = t.mine().to_vec();
    if app.collab.templates.list.is_empty() && mine.is_empty() {
        ui.label(
            RichText::new(if app.collab.signed_in() {
                "No office templates yet: an administrator publishes them. Save one of your own with Save as template."
            } else {
                "Sign in to the office server (File › Shared) to see the office's templates."
            })
            .weak(),
        );
    }
    ui.horizontal_wrapped(|ui| {
        let office: Vec<TemplateInfo> = app.collab.templates.list.iter().take(8).cloned().collect();
        for t in office {
            let label = if t.mandatory.is_empty() { t.name.clone() } else { format!("{} (required)", t.name) };
            if ui.button(label).on_hover_text(format!("{}\n{} · v{} · updated by {}", t.description, t.category, t.version, t.updated_by)).clicked()
            {
                let _ = run_command(app, "file.newFromTemplate", &json!({"id": t.id}));
                *close = true;
            }
        }
        for (n, _) in mine.iter().take(8) {
            if ui.button(format!("{n} (mine)")).clicked() {
                if let Some(Err(e)) = run_command(app, "file.newFromTemplate", &json!({"personal": n})) {
                    app.message = Some(("Templates".into(), e));
                }
                *close = true;
            }
        }
    });
    ui.horizontal(|ui| {
        if ui.button("All templates…").clicked() {
            let _ = run_command(app, "file.newFromTemplate", &json!({"show": true}));
            *close = true;
        }
        if app.session.active().is_some() && ui.button("Save this workbook as a template…").clicked() {
            app.collab.templates.window = true;
            app.collab.templates.mine_fresh = false;
            *close = true;
        }
    });
}

/// The Templates window: search, preview, create; save or publish the open workbook.
pub fn window(app: &mut SheetApp, ctx: &egui::Context) {
    if app.backstage.is_none() {
        app.collab.templates.in_new_page = false;
    }
    if !app.collab.templates.window {
        return;
    }
    let mut open = true;
    let mut act: Option<(&'static str, Json)> = None;
    let admin = app.collab.me.as_ref().is_some_and(|m| m.role == proto::Role::Admin);
    let signed_in = app.collab.signed_in();
    let now = crate::clock::monotonic_ms();
    {
        let t = &mut app.collab.templates;
        if t.create_when_got.is_some() && t.waiting_since.is_some_and(|since| now - since > WAIT_MS) {
            t.create_when_got = None;
            t.waiting_since = None;
            t.error = Some(format!(
                "The office server did not send the template within {} s. Check the connection (File › Shared) and try again.",
                (WAIT_MS / 1000.0) as u32
            ));
        }
        if t.create_when_got.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
    }
    egui::Window::new("Templates").collapsible(false).open(&mut open).default_width(620.0).show(ctx, |ui| {
        let t = &mut app.collab.templates;
        ui.horizontal(|ui| {
            ui.label("Search");
            ui.add(egui::TextEdit::singleline(&mut t.search).hint_text("tracker, register, minutes…").desired_width(260.0));
        });
        let q = t.search.to_lowercase();
        let matches = |name: &str, extra: &str| q.is_empty() || name.to_lowercase().contains(&q) || extra.to_lowercase().contains(&q);
        ui.columns(2, |cols| {
            egui::ScrollArea::vertical().id_salt("dex_tpl_list").max_height(300.0).show(&mut cols[0], |ui| {
                ui.strong("Office templates");
                if t.list.is_empty() {
                    ui.label(RichText::new(if signed_in { "None published yet." } else { "Sign in to the office server to see them." }).weak());
                }
                let mut last_cat = String::new();
                for tpl in &t.list {
                    if !matches(&tpl.name, &format!("{} {}", tpl.category, tpl.description)) {
                        continue;
                    }
                    if tpl.category != last_cat {
                        last_cat = tpl.category.clone();
                        ui.label(RichText::new(if last_cat.is_empty() { "Other" } else { &last_cat }).weak());
                    }
                    let label = if tpl.mandatory.is_empty() { tpl.name.clone() } else { format!("{} (required)", tpl.name) };
                    if ui.selectable_label(t.selected.as_deref() == Some(tpl.id.as_str()), label).clicked() {
                        t.selected = Some(tpl.id.clone());
                        act = Some(("preview", json!(tpl.id)));
                    }
                }
                ui.add_space(6.0);
                ui.strong("My templates");
                let mine: Vec<String> = t.mine().iter().map(|(n, _)| n.clone()).collect();
                if mine.is_empty() {
                    ui.label(RichText::new("None yet: save the open workbook as one below.").weak());
                }
                for n in mine {
                    if matches(&n, "") && ui.button(format!("New from {n}")).clicked() {
                        act = Some(("personal", json!(n)));
                    }
                }
            });
            let ui = &mut cols[1];
            match t.selected.as_ref().and_then(|id| t.list.iter().find(|x| &x.id == id)) {
                None => {
                    ui.label(RichText::new("Pick a template to see it.").weak());
                }
                Some(tpl) => {
                    ui.heading(&tpl.name);
                    if !tpl.mandatory.is_empty() {
                        ui.colored_label(egui::Color32::from_rgb(0xd0, 0x70, 0x00), format!("Required for: {}", tpl.mandatory));
                    }
                    ui.label(&tpl.description);
                    ui.label(RichText::new(format!("Version {} · updated {} by {}", tpl.version, when(tpl.updated), tpl.updated_by)).weak());
                    match &t.got {
                        Some((id, wb)) if *id == tpl.id => preview(ui, wb),
                        _ => {
                            ui.spinner();
                        }
                    }
                    let waiting = t.create_when_got.as_deref() == Some(tpl.id.as_str());
                    if waiting {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Getting the template from the office server…");
                        });
                    }
                    if ui.add_enabled(!waiting, egui::Button::new("Create a spreadsheet from it")).clicked() {
                        act = Some(("create", json!(tpl.id)));
                    }
                    if admin && ui.button("Remove from the office library").clicked() {
                        act = Some(("delete", json!(tpl.id)));
                    }
                }
            }
        });
        ui.separator();
        ui.strong("Save the open workbook as a template");
        egui::Grid::new("dex_tpl_save").num_columns(2).spacing(vec2(10.0, 4.0)).show(ui, |ui| {
            ui.label("Name");
            ui.text_edit_singleline(&mut t.name);
            ui.end_row();
            if !t.office {
                ui.label("");
                ui.checkbox(&mut t.overwrite, "Replace my template of this name if there is one");
                ui.end_row();
            }
            if admin {
                ui.label("Where");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut t.office, false, "My templates");
                    ui.radio_value(&mut t.office, true, "Office library (approved)");
                });
                ui.end_row();
                if t.office {
                    ui.label("Category");
                    ui.add(egui::TextEdit::singleline(&mut t.category).hint_text("Project trackers, Registers, Reports…"));
                    ui.end_row();
                    ui.label("Description");
                    ui.text_edit_singleline(&mut t.description);
                    ui.end_row();
                    ui.label("Required for");
                    ui.add(egui::TextEdit::singleline(&mut t.mandatory).hint_text("leave empty if optional"));
                    ui.end_row();
                }
            }
        });
        if ui.add_enabled(!t.name.trim().is_empty(), egui::Button::new("Save as template")).clicked() {
            act = Some(("save", Json::Null));
        }
        if let Some(e) = &t.error {
            ui.colored_label(egui::Color32::from_rgb(0xd0, 0x30, 0x30), e);
        }
        if !admin {
            ui.label(RichText::new("Office templates are published by an administrator; yours are kept on this PC.").weak());
        }
    });
    match act {
        Some(("preview", id)) => {
            if let Some(id) = id.as_str()
                && app.collab.templates.got.as_ref().is_none_or(|(g, _)| g != id)
            {
                app.collab.send(ClientMsg::GetTemplate { id: id.to_string() });
            }
        }
        Some(("create", id)) => {
            let id = id.as_str().unwrap_or_default().to_string();
            let got = app.collab.templates.got.clone().filter(|(g, _)| *g == id);
            let name = app.collab.templates.list.iter().find(|x| x.id == id).map(|x| x.name.clone()).unwrap_or_default();
            match got {
                Some((_, wb)) => {
                    create(app, &name, (*wb).clone());
                    open = false;
                }
                // Not downloaded yet: the window stays, with a spinner, until it arrives
                // (`on_template` creates the spreadsheet and closes the window) or `WAIT_MS` pass.
                None => match run_command(app, "file.newFromTemplate", &json!({"id": id})) {
                    Some(Ok(_)) => app.collab.templates.wait_for(id, now),
                    Some(Err(e)) => app.collab.templates.error = Some(e),
                    None => {}
                },
            };
        }
        Some(("personal", n)) => {
            if let Some(Err(e)) = run_command(app, "file.newFromTemplate", &json!({"personal": n})) {
                app.message = Some(("Templates".into(), e));
            }
            open = false;
        }
        Some(("delete", id)) => {
            app.collab.send(ClientMsg::DeleteTemplate { id: id.as_str().unwrap_or_default().to_string() });
        }
        Some(("save", _)) => {
            let t = &app.collab.templates;
            let p = json!({"name": t.name, "office": t.office, "category": t.category, "description": t.description, "mandatory": t.mandatory, "overwrite": t.overwrite});
            match run_command(app, "file.saveAsTemplate", &p) {
                Some(Ok(_)) => {
                    app.collab.templates.error = None;
                    app.collab.templates.overwrite = false;
                    app.toast = Some(("Template saved.".into(), crate::now_ms()));
                }
                Some(Err(e)) => app.collab.templates.error = Some(e),
                None => {}
            }
        }
        _ => {}
    }
    if !open {
        app.collab.templates.window = false;
    }
}

/// The first rows and columns of the template's first sheet.
fn preview(ui: &mut Ui, wb: &Workbook) {
    let Some(sh) = wb.sheets.first() else { return };
    egui::Grid::new("dex_tpl_preview").striped(true).spacing(vec2(8.0, 2.0)).show(ui, |ui| {
        for r in 0..8u32 {
            for c in 0..5u32 {
                let v =
                    sh.cell(gridcraft_engine::core::CellRef::new(r, c)).map(|x| gridcraft_collab::patch::value_text(&x.value)).unwrap_or_default();
                let short: String = v.chars().take(14).collect();
                ui.label(RichText::new(short).small());
            }
            ui.end_row();
        }
    });
}

fn when(ms: i64) -> String {
    use chrono::TimeZone;
    chrono::Local.timestamp_millis_opt(ms).single().map(|t| t.format("%d %b %Y").to_string()).unwrap_or_default()
}
