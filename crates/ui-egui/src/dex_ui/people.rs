//! People, assignments and the Connect quick panel (Dexterity Sheets).
//!
//! - **People** (`collab.people`): everyone on the office server with the name others see, their
//!   Windows account, colour, whether they are connected and which shared spreadsheet they are in
//!   (only spreadsheets you may open yourself). The server keeps the list current.
//! - **Assign To** (`collab.assign`): gives the selected cell or range of a shared spreadsheet to
//!   someone, with an optional due date and a status. They get a notification with Go to cell.
//! - **Assigned to Me** (`collab.assigned`): your open items in every shared spreadsheet, with Go
//!   to cell and Mark done.
//! - **Connect** (the button at the left of the status bar): recent mentions and assignments,
//!   and Discuss in Connect (`collab.discussInConnect`), which opens Dexterity Connect's share
//!   dialog with a link to the cell (`dexconnect://share?…&link=dexterity://open/…?at=…`).

use egui::{Color32, RichText, Ui, vec2};
use gridcraft_collab::proto::{self, Assignment, ClientMsg, PersonInfo, Role, UserInfo};
use serde_json::{Value as Json, json};

use crate::SheetApp;

/// Recent notices the Connect panel shows.
const RECENT: usize = 6;

#[derive(Default)]
pub struct People {
    /// Everyone on the server, as it last said.
    pub people: Vec<PersonInfo>,
    pub panel: bool,
    search: String,
    /// The signed-in person's assignments, open ones first.
    pub assigned: Vec<Assignment>,
    pub assigned_panel: bool,
    show_done: bool,
    /// The Assign To dialog.
    pub assign: Option<AssignForm>,
    pub connect_panel: bool,
    /// The last `dexconnect:` link handed to Windows (also for tests and the control channel).
    pub last_launch: Option<String>,
}

/// What the Assign To dialog holds.
#[derive(Clone, Debug, Default)]
pub struct AssignForm {
    book: String,
    book_name: String,
    sheet: String,
    cell: String,
    /// The person picked (account, lower case) and the search typed to find them.
    user: String,
    query: String,
    title: String,
    due: String,
    done: bool,
}

/// The server's list of people: kept, and it is also who can be @mentioned or picked.
pub fn on_people(app: &mut SheetApp, people: Vec<PersonInfo>) {
    app.collab.users = people
        .iter()
        .map(|p| UserInfo { user: p.user.clone(), display: p.display.clone(), role: Role::parse(&p.role).unwrap_or(Role::Editor), color: p.color })
        .collect();
    app.collab.people.people = people;
}

pub fn on_assigned(app: &mut SheetApp, items: Vec<Assignment>) {
    app.collab.people.assigned = items;
}

/// A notice arrived: a new assignment refreshes "Assigned to me".
pub fn on_notice(app: &mut SheetApp, n: &proto::Notice) {
    if n.kind == "assigned" && app.collab.signed_in() {
        app.collab.send(ClientMsg::ListAssigned);
    }
}

/// Signed in: the lists this module shows.
pub fn on_connected(app: &mut SheetApp) {
    app.collab.send(ClientMsg::ListPeople);
    app.collab.send(ClientMsg::ListAssigned);
}

/// Percent-encodes a query value (everything but letters, digits and `-._~`).
pub fn encode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { char::from(b).to_string() } else { format!("%{b:02X}") }).collect()
}

/// The first cell of `B4:D9`.
fn first_cell(range: &str) -> String {
    range.split(':').next().unwrap_or(range).to_string()
}

/// The active shared spreadsheet's (book id, name, sheet, selected range).
fn target(app: &SheetApp) -> Result<(String, String, String, String), String> {
    let d = app.session.active().ok_or("Open a spreadsheet first.")?;
    let sd = app.collab.shared(d.uid).ok_or("Share the spreadsheet first (File › Share): assignments live on the office server.")?;
    let sheet = d.wb.active().map(|s| s.name.clone()).unwrap_or_default();
    let r = d.selection.current();
    // Whole rows or columns are too big to point at: the active cell stands for them.
    let cell = if r.is_full_rows() || r.is_full_cols() { d.selection.active.a1() } else { r.a1() };
    Ok((sd.book.id.clone(), sd.book.name.clone(), sheet, cell))
}

/// `dexconnect://share?…` for the active cell of the shared spreadsheet.
pub fn discuss_link(app: &SheetApp) -> Result<String, String> {
    let (book, name, sheet, cell) = target(app)?;
    let link = proto::link_at(&app.collab.server_address(), &book, &sheet, &first_cell(&cell));
    let text = format!("{sheet}!{cell} in {name}");
    Ok(format!("dexconnect://share?from={}&title={}&text={}&link={}", encode("Dexterity Sheets"), encode(&name), encode(&text), encode(&link)))
}

/// Hands a `dexconnect:` link to Windows (Dexterity Connect takes it from there).
fn launch(app: &mut SheetApp, url: &str) {
    app.collab.people.last_launch = Some(url.to_string());
    #[cfg(windows)]
    if crate::dex_ui::desktop() {
        // Only this scheme, with every value encoded above, ever reaches the shell here.
        if let Err(e) = std::process::Command::new("explorer").arg(url).spawn() {
            log::warn!("dexconnect: {e}");
        }
    }
}

/// The people, assignment and Connect commands. `None` = not one of them.
pub fn run_command(app: &mut SheetApp, id: &str, p: &Json) -> Option<Result<Json, String>> {
    Some(match id {
        "collab.people" => {
            app.collab.people.panel = p.get("show").and_then(Json::as_bool).unwrap_or(true);
            if app.collab.signed_in() {
                app.collab.send(ClientMsg::ListPeople);
            }
            Ok(json!(
                app.collab
                    .people
                    .people
                    .iter()
                    .map(|x| json!({"user": x.user, "account": x.account, "display": x.display, "online": x.online, "books": x.books.iter().map(|b| b.1.clone()).collect::<Vec<_>>()}))
                    .collect::<Vec<_>>()
            ))
        }
        "collab.assigned" => {
            app.collab.people.assigned_panel = p.get("show").and_then(Json::as_bool).unwrap_or(true);
            if app.collab.signed_in() {
                app.collab.send(ClientMsg::ListAssigned);
            }
            Ok(serde_json::to_value(&app.collab.people.assigned).unwrap_or(Json::Null))
        }
        // {user?, title?, due?: "YYYY-MM-DD", status?: open|done, sheet?, cell?}: without a user the
        // Assign To dialog opens for the selection.
        "collab.assign" => assign(app, p),
        // {book, id, status: open|done}
        "collab.setAssignment" => {
            let g = |k: &str| p.get(k).and_then(Json::as_str).unwrap_or("").to_string();
            let (book, id, status) = (g("book"), g("id"), g("status"));
            if book.is_empty() || id.is_empty() {
                Err("Give the assignment's book and id (collab.assigned lists them).".into())
            } else if !app.collab.signed_in() {
                Err("Connect to the office server first (File › Shared).".into())
            } else {
                let status = if status.is_empty() { "done".to_string() } else { status };
                app.collab.send(ClientMsg::SetAssignment { book, id, status });
                Ok(Json::Null)
            }
        }
        "collab.connectPanel" => {
            let on = p.get("on").and_then(Json::as_bool).unwrap_or(!app.collab.people.connect_panel);
            app.collab.people.connect_panel = on;
            if on && app.collab.signed_in() {
                app.collab.send(ClientMsg::ListAssigned);
            }
            Ok(json!({"on": on}))
        }
        "collab.discussInConnect" => discuss_link(app).map(|url| {
            launch(app, &url);
            json!({"url": url})
        }),
        _ => return None,
    })
}

fn assign(app: &mut SheetApp, p: &Json) -> Result<Json, String> {
    let (book, book_name, sheet, cell) = target(app)?;
    let g = |k: &str| p.get(k).and_then(Json::as_str).map(str::to_string);
    let sheet = g("sheet").unwrap_or(sheet);
    let cell = g("cell").unwrap_or(cell);
    let Some(user) = g("user").filter(|u| !u.trim().is_empty()) else {
        app.collab.people.assign = Some(AssignForm { book, book_name, sheet, cell, ..AssignForm::default() });
        if app.collab.signed_in() {
            app.collab.send(ClientMsg::ListPeople);
        }
        return Ok(Json::Null);
    };
    if !app.collab.signed_in() {
        return Err("Connect to the office server first (File › Shared).".into());
    }
    let due = g("due").unwrap_or_default();
    let due = gridcraft_collab::tasks::clean_due(&due).ok_or("Type the due date as YYYY-MM-DD, or leave it empty.")?;
    let status = gridcraft_collab::tasks::clean_status(&g("status").unwrap_or_default()).ok_or("The status is open or done.")?;
    let cell = gridcraft_collab::tasks::clean_range(&cell).ok_or_else(|| format!("\"{cell}\" is not a cell or range (B4, B4:D9)."))?;
    app.collab.send(ClientMsg::Assign {
        book: book.clone(),
        sheet: sheet.clone(),
        cell: cell.clone(),
        user: user.clone(),
        title: g("title").unwrap_or_default(),
        due: (!due.is_empty()).then_some(due),
        status: Some(status.to_string()),
    });
    Ok(json!({"book": book, "sheet": sheet, "cell": cell, "user": user}))
}

fn dot(ui: &mut Ui, c: [u8; 3], online: bool) {
    let (r, _) = ui.allocate_exact_size(vec2(12.0, 12.0), egui::Sense::hover());
    let col = Color32::from_rgb(c[0], c[1], c[2]);
    if online {
        ui.painter().circle_filled(r.center(), 5.0, col);
    } else {
        ui.painter().circle_stroke(r.center(), 4.5, egui::Stroke::new(1.5, col));
    }
}

/// The People, Assigned to Me and Assign To windows and the Connect panel.
pub fn windows(app: &mut SheetApp, ctx: &egui::Context) {
    people_window(app, ctx);
    assigned_window(app, ctx);
    assign_window(app, ctx);
    connect_window(app, ctx);
}

fn people_window(app: &mut SheetApp, ctx: &egui::Context) {
    if !app.collab.people.panel {
        return;
    }
    let mut open = true;
    let mut go_book: Option<String> = None;
    let mut assign_to: Option<String> = None;
    let signed_in = app.collab.signed_in();
    let shared = crate::collab_ui::active_shared(app).is_some();
    let me = app.collab.me.as_ref().map(|m| m.user.clone()).unwrap_or_default();
    egui::Window::new("People").collapsible(false).open(&mut open).default_width(460.0).show(ctx, |ui| {
        if !signed_in {
            ui.label(RichText::new("Connect to the office server (File › Shared) to see your colleagues.").weak());
            return;
        }
        let st = &mut app.collab.people;
        ui.add(egui::TextEdit::singleline(&mut st.search).hint_text("Search by name or account").desired_width(f32::INFINITY));
        let q = st.search.trim().to_lowercase();
        let online = st.people.iter().filter(|p| p.online).count();
        ui.label(RichText::new(format!("{} people, {online} connected now", st.people.len())).weak());
        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
            for p in &st.people {
                if !q.is_empty() && !format!("{} {}", p.display, p.account).to_lowercase().contains(&q) {
                    continue;
                }
                ui.horizontal(|ui| {
                    dot(ui, p.color, p.online);
                    ui.label(RichText::new(&p.display).strong());
                    ui.label(RichText::new(if p.account.is_empty() { &p.user } else { &p.account }).weak());
                    ui.label(if p.online {
                        RichText::new("online").color(Color32::from_rgb(0x2c, 0xa0, 0x2c))
                    } else {
                        RichText::new("offline").weak()
                    });
                    if shared
                        && p.user != me
                        && ui.small_button("Assign…").on_hover_text("Give them the selected cells of this spreadsheet").clicked()
                    {
                        assign_to = Some(p.user.clone());
                    }
                });
                if !p.books.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.add_space(18.0);
                        ui.label(RichText::new("in").weak());
                        for (id, name) in &p.books {
                            if ui.link(name).on_hover_text("Open this spreadsheet").clicked() {
                                go_book = Some(id.clone());
                            }
                        }
                    });
                }
            }
        });
    });
    if let Some(b) = go_book {
        crate::dex_ui::share::go_to(app, &b, "", "");
    }
    if let Some(u) = assign_to {
        if let Err(e) = assign(app, &json!({})) {
            app.message = Some(("People".into(), e));
        } else if let Some(f) = app.collab.people.assign.as_mut() {
            f.user = u;
        }
    }
    if !open {
        app.collab.people.panel = false;
    }
}

fn assigned_window(app: &mut SheetApp, ctx: &egui::Context) {
    if !app.collab.people.assigned_panel {
        return;
    }
    let mut open = true;
    let mut go: Option<(String, String, String)> = None;
    let mut set: Option<(String, String, &'static str)> = None;
    let signed_in = app.collab.signed_in();
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    egui::Window::new("Assigned to Me").collapsible(false).open(&mut open).default_width(520.0).show(ctx, |ui| {
        if !signed_in {
            ui.label(RichText::new("Connect to the office server (File › Shared) to see what is assigned to you.").weak());
            return;
        }
        let st = &mut app.collab.people;
        ui.checkbox(&mut st.show_done, "Show done");
        let shown: Vec<&Assignment> = st.assigned.iter().filter(|a| st.show_done || a.status != "done").collect();
        if shown.is_empty() {
            ui.label(RichText::new("Nothing open is assigned to you. When someone gives you cells to look at, they are listed here.").weak());
        }
        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
            egui::Grid::new("dex_assigned").num_columns(5).striped(true).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
                for a in shown {
                    ui.label(RichText::new(&a.book_name).strong());
                    ui.label(format!("{}!{}", a.sheet, a.cell));
                    let what = if a.title.is_empty() { format!("from {}", a.by_display) } else { format!("{} · from {}", a.title, a.by_display) };
                    ui.label(what);
                    if a.due.is_empty() {
                        ui.label("");
                    } else if a.status != "done" && a.due < today {
                        ui.label(RichText::new(format!("overdue {}", a.due)).color(Color32::from_rgb(0xc0, 0x2b, 0x2b)));
                    } else {
                        ui.label(format!("due {}", a.due));
                    }
                    ui.horizontal(|ui| {
                        if ui.small_button("Go to cell").clicked() {
                            go = Some((a.book.clone(), a.sheet.clone(), first_cell(&a.cell)));
                        }
                        let done = a.status == "done";
                        if ui.small_button(if done { "Reopen" } else { "Mark done" }).clicked() {
                            set = Some((a.book.clone(), a.id.clone(), if done { "open" } else { "done" }));
                        }
                    });
                    ui.end_row();
                }
            });
        });
    });
    if let Some((b, s, c)) = go {
        crate::dex_ui::share::go_to(app, &b, &s, &c);
    }
    if let Some((book, id, status)) = set {
        app.collab.send(ClientMsg::SetAssignment { book, id, status: status.into() });
    }
    if !open {
        app.collab.people.assigned_panel = false;
    }
}

fn assign_window(app: &mut SheetApp, ctx: &egui::Context) {
    let Some(mut f) = app.collab.people.assign.clone() else { return };
    let mut open = true;
    let mut submit = false;
    let mut cancel = false;
    let me = app.collab.me.as_ref().map(|m| m.user.clone()).unwrap_or_default();
    let people: Vec<(String, String)> =
        app.collab.people.people.iter().filter(|p| p.user != me).map(|p| (p.user.clone(), p.display.clone())).collect();
    egui::Window::new("Assign To").collapsible(false).resizable(false).open(&mut open).anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0)).show(
        ctx,
        |ui| {
            ui.set_max_width(420.0);
            ui.label(RichText::new(format!("{} · {}!{}", f.book_name, f.sheet, f.cell)).strong());
            ui.add_space(4.0);
            egui::Grid::new("dex_assign").num_columns(2).spacing(vec2(12.0, 8.0)).show(ui, |ui| {
                ui.label("Person");
                ui.vertical(|ui| {
                    let chosen = people.iter().find(|p| p.0 == f.user).map(|p| p.1.clone());
                    if let Some(name) = &chosen {
                        ui.horizontal(|ui| {
                            ui.strong(name);
                            if ui.small_button("Change").clicked() {
                                f.user.clear();
                            }
                        });
                    } else {
                        ui.add(egui::TextEdit::singleline(&mut f.query).hint_text("Type a name").desired_width(240.0));
                        for (u, d) in gridcraft_collab::mention::suggestions(&f.query, &people, 6) {
                            if ui.selectable_label(false, format!("{d}  ·  {u}")).clicked() {
                                f.user = u;
                            }
                        }
                        if people.is_empty() {
                            ui.label(RichText::new("Nobody else has signed in to this server yet.").weak());
                        }
                    }
                });
                ui.end_row();
                ui.label("What to do");
                ui.add(egui::TextEdit::singleline(&mut f.title).hint_text("Check the rating (optional)").desired_width(240.0));
                ui.end_row();
                ui.label("Due");
                ui.add(egui::TextEdit::singleline(&mut f.due).hint_text("YYYY-MM-DD (optional)").desired_width(140.0));
                ui.end_row();
                ui.label("Status");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut f.done, false, "Open");
                    ui.radio_value(&mut f.done, true, "Done");
                });
                ui.end_row();
            });
            ui.add_space(4.0);
            ui.label(
                RichText::new("They get a notification that opens the spreadsheet at these cells, and see it under Assigned to Me.").weak().small(),
            );
            ui.horizontal(|ui| {
                if ui.add_enabled(!f.user.is_empty(), egui::Button::new("Assign")).on_disabled_hover_text("Pick a person first").clicked() {
                    submit = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        },
    );
    if submit && target(app).map(|t| t.0).as_deref() != Ok(f.book.as_str()) {
        // Another window came to the front meanwhile: the cells named are in the first one.
        app.message = Some(("Assign To".into(), format!("Go back to \"{}\" to assign these cells.", f.book_name)));
        app.collab.people.assign = Some(f);
    } else if submit {
        let p =
            json!({"user": f.user, "title": f.title, "due": f.due, "status": if f.done { "done" } else { "open" }, "sheet": f.sheet, "cell": f.cell});
        match assign(app, &p) {
            Ok(_) => app.collab.people.assign = None,
            Err(e) => {
                app.message = Some(("Assign To".into(), e));
                app.collab.people.assign = Some(f);
            }
        }
    } else if cancel || !open {
        app.collab.people.assign = None;
    } else {
        app.collab.people.assign = Some(f);
    }
}

/// The Connect button at the far left of the status bar.
pub fn status_button(app: &mut SheetApp, ui: &mut Ui) {
    let open = app.collab.people.assigned.iter().filter(|a| a.status != "done").count();
    let label = if open > 0 { format!("Connect · {open}") } else { "Connect".to_string() };
    let t = crate::theme::Tokens::get(ui.ctx());
    let r = ui
        .add(egui::Label::new(RichText::new(label).font(crate::theme::ui_bold(12.5)).color(t.accent)).selectable(false).sense(egui::Sense::click()))
        .on_hover_text("Your mentions and assignments, and Discuss in Connect");
    if r.clicked() {
        let _ = run_command(app, "collab.connectPanel", &json!({}));
    }
    ui.add_space(12.0);
}

fn connect_window(app: &mut SheetApp, ctx: &egui::Context) {
    if !app.collab.people.connect_panel {
        return;
    }
    let mut open = true;
    let mut go: Option<(String, String, String)> = None;
    let mut run: Option<&'static str> = None;
    let signed_in = app.collab.signed_in();
    let shared = crate::collab_ui::active_shared(app).is_some();
    let recent: Vec<proto::Notice> = app
        .collab
        .share
        .prefs
        .history
        .iter()
        .filter(|n| matches!(n.kind.as_str(), "mention" | "assigned" | "assignmentDone"))
        .take(RECENT)
        .cloned()
        .collect();
    let open_items = app.collab.people.assigned.iter().filter(|a| a.status != "done").count();
    egui::Window::new("Connect")
        .collapsible(false)
        .resizable(false)
        .open(&mut open)
        .anchor(egui::Align2::LEFT_BOTTOM, vec2(8.0, -34.0))
        .default_width(360.0)
        .show(ctx, |ui| {
            ui.set_max_width(380.0);
            ui.label(RichText::new("Recent in Sheets").strong());
            if recent.is_empty() {
                ui.label(RichText::new("No mentions or assignments yet.").weak());
            }
            for n in &recent {
                ui.horizontal_wrapped(|ui| {
                    let who = if n.by.is_empty() { "Someone".to_string() } else { n.by.clone() };
                    let what = match n.kind.as_str() {
                        "mention" => format!("{who} mentioned you"),
                        "assigned" => format!("{who} assigned you"),
                        _ => format!("{who} finished"),
                    };
                    ui.label(RichText::new(what).strong());
                    ui.label(format!("{} · {}!{}", n.name, n.sheet, n.cell));
                    if !n.cell.is_empty() && ui.small_button("Go to cell").clicked() {
                        go = Some((n.book.clone(), n.sheet.clone(), n.cell.clone()));
                    }
                });
                if !n.text.is_empty() {
                    ui.label(RichText::new(&n.text).weak().small());
                }
            }
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(format!("Assigned to me: {open_items} open"));
                if ui.small_button("Open list").clicked() {
                    run = Some("collab.assigned");
                }
                if ui.small_button("People").clicked() {
                    run = Some("collab.people");
                }
            });
            ui.add_space(4.0);
            if ui
                .add_enabled(shared, egui::Button::new("Discuss in Connect"))
                .on_hover_text("Opens Dexterity Connect's share dialog with a link to this cell")
                .on_disabled_hover_text("Open a shared spreadsheet: the link points at its cell on the office server")
                .clicked()
            {
                run = Some("collab.discussInConnect");
            }
            if !signed_in {
                ui.label(RichText::new("Connect to the office server (File › Shared) for mentions and assignments.").weak().small());
            }
        });
    if let Some((b, s, c)) = go {
        crate::dex_ui::share::go_to(app, &b, &s, &c);
    }
    if let Some(id) = run
        && let Some(Err(e)) = run_command(app, id, &json!({}))
    {
        app.message = Some(("Dexterity Connect".into(), e));
    }
    if !open {
        app.collab.people.connect_panel = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn share_links_encode_every_value() {
        assert_eq!(encode("Line List!B4 & co"), "Line%20List%21B4%20%26%20co");
        let link = proto::link_at("PC-SERVER:47810", "line-100", "Line List", "B4");
        assert_eq!(encode(&link), "dexterity%3A%2F%2Fopen%2FPC-SERVER%3A47810%2Fline-100%3Fat%3DLine%2520List%21B4");
        assert_eq!(first_cell("B4:D9"), "B4");
    }
}
