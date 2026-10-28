//! Dexterity Sheets Server Manager: the office server's control panel for IT, with no commands to
//! type. Start and stop the server, see how teammates reach it, add people one by one or in bulk
//! (typed, pasted from Excel, or from the template file), reset passwords and roles, hand new app
//! versions to every PC, and back up or restore everything.
//!
//! ```text
//! DexSheetsServerManager [--data DIR] [--no-elevate] [--import PEOPLE.xlsx|.csv]
//!                        [--shot overview|people|add-many|updates|backup OUT.png]
//! ```
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod roster;
mod system;

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};

use egui::{Color32, RichText, Ui, vec2};
use gridcraft_collab::proto::Role;
use gridcraft_collab::server::{self, UserRecord};
use roster::{Checked, Row};
use system::ServiceState;

const ACCENT: Color32 = Color32::from_rgb(0x1f, 0x4e, 0x9a);
const OK: Color32 = Color32::from_rgb(0x1e, 0x8a, 0x3c);
const WARN: Color32 = Color32::from_rgb(0xb0, 0x6a, 0x00);
const BAD: Color32 = Color32::from_rgb(0xc0, 0x2b, 0x2b);
const DIM: Color32 = Color32::from_rgb(0x60, 0x66, 0x70);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Page {
    Overview,
    People,
    AddMany,
    Updates,
    Backup,
}

impl Page {
    const ALL: [Page; 5] = [Page::Overview, Page::People, Page::AddMany, Page::Updates, Page::Backup];
    fn label(self) -> &'static str {
        match self {
            Page::Overview => "Server",
            Page::People => "People",
            Page::AddMany => "Add many people",
            Page::Updates => "App updates",
            Page::Backup => "Backup and restore",
        }
    }
    fn id(self) -> &'static str {
        match self {
            Page::Overview => "overview",
            Page::People => "people",
            Page::AddMany => "add-many",
            Page::Updates => "updates",
            Page::Backup => "backup",
        }
    }
}

/// Work that takes a while (creating accounts hashes each password, backups, the service), run
/// off the window's thread so it never freezes.
enum Done {
    Message(Result<String, String>),
    Created { ok: Vec<Checked>, failed: Vec<(String, String)> },
}

struct Job {
    label: String,
    progress: Arc<Mutex<(usize, usize)>>,
    rx: Receiver<Done>,
}

#[derive(Default)]
struct PersonForm {
    user: String,
    display: String,
    role: Option<Role>,
    password: String,
    show: bool,
}

/// Facts about the PC that take a moment to find (firewall, addresses): looked up in the
/// background.
#[derive(Clone, Default)]
struct Network {
    addresses: Vec<String>,
    firewall_tcp: bool,
    firewall_udp: bool,
    checked: bool,
}

struct App {
    dir: PathBuf,
    page: Page,
    admin: bool,
    users: Vec<UserRecord>,
    users_error: Option<String>,
    service: ServiceState,
    port_answers: bool,
    polled_at: f64,
    network: Arc<Mutex<Network>>,
    size: u64,
    books: usize,
    log: String,
    name: String,
    message: Option<(String, bool)>,
    first: PersonForm,
    first_repeat: String,
    // People
    search: String,
    add: PersonForm,
    confirm: Option<(String, String)>,
    /// A password just set, shown once: (user, password).
    shown: Option<(String, String)>,
    // Add many
    rows: Vec<Row>,
    pasted: String,
    created: Vec<Checked>,
    failed: Vec<(String, String)>,
    // Updates
    installers: Vec<system::Installer>,
    notes: String,
    repo: String,
    token: String,
    // Backup
    with_installers: bool,
    job: Option<Job>,
    shot: Option<(Page, PathBuf, u32)>,
}

fn role_label(r: Role) -> &'static str {
    roster::role_name(r)
}

impl App {
    fn new(dir: PathBuf, admin: bool, shot: Option<(Page, PathBuf)>) -> App {
        let (repo, token) = system::update_source(&dir);
        let name = system::server_name(&dir);
        let mut app = App {
            page: shot.as_ref().map_or(Page::Overview, |s| s.0),
            dir,
            admin,
            users: Vec::new(),
            users_error: None,
            service: ServiceState::Unknown("checking".into()),
            port_answers: false,
            polled_at: f64::MIN,
            network: Arc::new(Mutex::new(Network::default())),
            size: 0,
            books: 0,
            log: String::new(),
            name,
            message: None,
            first: PersonForm { role: Some(Role::Admin), ..PersonForm::default() },
            first_repeat: String::new(),
            search: String::new(),
            add: PersonForm { role: Some(Role::Editor), ..PersonForm::default() },
            confirm: None,
            shown: None,
            rows: Vec::new(),
            pasted: String::new(),
            created: Vec::new(),
            failed: Vec::new(),
            installers: Vec::new(),
            notes: String::new(),
            repo,
            token,
            with_installers: false,
            job: None,
            shot: shot.map(|(p, path)| (p, path, 0)),
        };
        app.reload();
        app.check_network();
        app
    }

    fn reload(&mut self) {
        match server::load_users(&self.dir) {
            Ok(u) => {
                self.users = u;
                self.users_error = None;
            }
            Err(e) => self.users_error = Some(format!("The accounts file can't be read: {e}")),
        }
        self.installers = system::installers(&self.dir);
        self.log = system::log_tail(&self.dir, 200);
        self.size = system::folder_size(&self.dir);
        self.books = system::workbook_count(&self.dir);
    }

    fn check_network(&self) {
        let net = self.network.clone();
        let _ = std::thread::Builder::new().name("network-check".into()).spawn(move || {
            let n = Network {
                addresses: system::addresses(),
                firewall_tcp: system::firewall_rule_exists(system::FIREWALL_TCP),
                firewall_udp: system::firewall_rule_exists(system::FIREWALL_UDP),
                checked: true,
            };
            if let Ok(mut g) = net.lock() {
                *g = n;
            }
        });
    }

    fn say(&mut self, r: Result<String, String>) {
        self.message = Some(match r {
            Ok(m) => (m, false),
            Err(e) => (e, true),
        });
    }

    fn active_admins(&self) -> usize {
        self.users.iter().filter(|u| u.role == Role::Admin && !u.disabled).count()
    }

    fn run(&mut self, label: &str, ctx: &egui::Context, work: impl FnOnce(&Arc<Mutex<(usize, usize)>>) -> Done + Send + 'static) {
        if self.job.is_some() {
            return;
        }
        let (tx, rx) = channel();
        let progress = Arc::new(Mutex::new((0, 0)));
        let p = progress.clone();
        let ctx = ctx.clone();
        let started = std::thread::Builder::new().name("manager-job".into()).spawn(move || {
            let _ = tx.send(work(&p));
            ctx.request_repaint();
        });
        match started {
            Ok(_) => self.job = Some(Job { label: label.to_string(), progress, rx }),
            Err(e) => self.say(Err(e.to_string())),
        }
    }

    fn poll(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        if now - self.polled_at > 2.0 {
            self.polled_at = now;
            self.service = system::service_state();
            self.port_answers = system::port_answers(system::PORT);
            if self.page == Page::Overview || self.page == Page::People {
                self.reload();
            }
        }
        if let Some(job) = &self.job
            && let Ok(done) = job.rx.try_recv()
        {
            self.job = None;
            match done {
                Done::Message(r) => self.say(r),
                Done::Created { ok, failed } => {
                    let n = ok.len();
                    self.created = ok;
                    self.failed = failed;
                    self.rows.clear();
                    self.say(if self.failed.is_empty() {
                        Ok(format!("{n} account(s) created. Save the sign-in sheet to hand out the passwords."))
                    } else {
                        Err(format!("{n} created, {} failed (see the list).", self.failed.len()))
                    });
                }
            }
            self.polled_at = f64::MIN;
            self.reload();
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(if self.job.is_some() { 150 } else { 1000 }));
    }
}

fn card(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .stroke(egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color))
        .corner_radius(8.0)
        .inner_margin(14.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
    ui.add_space(12.0);
}

fn primary(text: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.to_string()).color(Color32::WHITE).strong()).fill(ACCENT).min_size(vec2(120.0, 30.0))
}

// A fixed list of three roles with no search box, so egui's combo box can't get stuck small
// (the reason the workspace prefers craft_ui::ComboBox); the Manager doesn't depend on craft-ui.
#[allow(clippy::disallowed_types)]
fn role_combo(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, role: &mut Role) -> bool {
    let before = *role;
    egui::ComboBox::from_id_salt(id).selected_text(role_label(*role)).width(170.0).show_ui(ui, |ui| {
        for r in [Role::Editor, Role::Viewer, Role::Admin] {
            ui.selectable_value(role, r, role_label(r));
        }
    });
    *role != before
}

fn password_field(ui: &mut Ui, text: &mut String, show: &mut bool, hint: &str) {
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(text).password(!*show).hint_text(hint.to_string()).desired_width(170.0));
        if ui.small_button(if *show { "Hide" } else { "Show" }).clicked() {
            *show = !*show;
        }
        if ui.small_button("Make one").on_hover_text("A readable 10-character password").clicked() {
            *text = roster::new_password();
            *show = true;
        }
    });
}

impl App {
    fn overview(&mut self, ui: &mut Ui) {
        ui.heading("Server");
        ui.add_space(8.0);
        if self.users_error.is_none() && self.active_admins() == 0 {
            self.first_admin(ui);
        }
        let ctx = ui.ctx().clone();
        card(ui, |ui| {
            let (color, word) = match self.service {
                ServiceState::Running => (OK, "Running"),
                ServiceState::Starting | ServiceState::Stopping => (WARN, "Changing…"),
                ServiceState::Stopped => (BAD, "Stopped"),
                ServiceState::NotInstalled => (BAD, "Not installed"),
                ServiceState::Unknown(_) => (WARN, "Unknown"),
            };
            ui.horizontal(|ui| {
                let (dot, _) = ui.allocate_exact_size(vec2(16.0, 16.0), egui::Sense::hover());
                ui.painter().circle_filled(dot.center(), 7.0, color);
                ui.label(RichText::new(format!("The server is {}", word.to_lowercase())).size(18.0).strong());
            });
            if let ServiceState::Unknown(e) = &self.service {
                ui.label(RichText::new(e).color(DIM));
            }
            match self.service {
                ServiceState::Running if !self.port_answers => {
                    ui.label(
                        RichText::new(format!("It is running but not answering on port {} yet. If this lasts, read the log below.", system::PORT))
                            .color(WARN),
                    );
                }
                ServiceState::Running => {
                    ui.label(RichText::new("Teammates can sign in from Dexterity Sheets: File › Shared.").color(DIM));
                }
                ServiceState::NotInstalled => {
                    ui.label(RichText::new("Install the server with DexSheetsServer-Setup.exe; this manager then controls it.").color(DIM));
                }
                _ => {
                    ui.label(RichText::new("Nobody can open shared workbooks until it runs.").color(DIM));
                }
            }
            ui.add_space(8.0);
            let installed = self.service != ServiceState::NotInstalled;
            let busy = self.job.is_some();
            ui.horizontal(|ui| {
                let running = self.service == ServiceState::Running;
                if ui.add_enabled(installed && !running && !busy && self.admin, primary("Start")).clicked() {
                    self.run("Starting the server…", &ctx, |_| Done::Message(system::start_service().map(|()| "The server is running.".into())));
                }
                if ui.add_enabled(installed && running && !busy && self.admin, egui::Button::new("Stop")).clicked() {
                    self.run("Stopping the server…", &ctx, |_| Done::Message(system::stop_service().map(|()| "The server is stopped.".into())));
                }
                if ui.add_enabled(installed && running && !busy && self.admin, egui::Button::new("Restart")).clicked() {
                    self.run("Restarting the server…", &ctx, |_| Done::Message(system::restart_service().map(|()| "The server restarted.".into())));
                }
            });
        });

        card(ui, |ui| {
            ui.label(RichText::new("How teammates reach it").strong());
            ui.add_space(4.0);
            let host = std::env::var("COMPUTERNAME").unwrap_or_default();
            let net = self.network.lock().map(|n| n.clone()).unwrap_or_default();
            egui::Grid::new("reach").num_columns(2).spacing(vec2(16.0, 6.0)).show(ui, |ui| {
                ui.label("Shown to teammates as");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.name).hint_text(format!("Dexterity Sheets on {host}")).desired_width(260.0));
                    if ui.add_enabled(self.admin, egui::Button::new("Save")).clicked() {
                        let r = system::set_server_name(&self.dir, &self.name)
                            .map(|()| "Saved. Restart the server for teammates to see the new name.".to_string());
                        self.say(r);
                    }
                });
                ui.end_row();
                ui.label("Computer name");
                ui.label(RichText::new(&host).strong());
                ui.end_row();
                ui.label("Addresses");
                ui.label(if net.addresses.is_empty() { "—".to_string() } else { net.addresses.join(", ") });
                ui.end_row();
                ui.label("Port");
                ui.label(format!("TCP {} (workbooks), UDP {} (found on the network automatically)", system::PORT, system::DISCOVERY_PORT));
                ui.end_row();
                ui.label("Firewall");
                if !net.checked {
                    ui.label(RichText::new("checking…").color(DIM));
                } else if net.firewall_tcp && net.firewall_udp {
                    ui.label(RichText::new("Open for the office network").color(OK));
                } else {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Not open: other PCs can't reach the server").color(BAD));
                        if ui.add_enabled(self.admin, egui::Button::new("Open it")).clicked() {
                            let r = system::open_firewall(&system::server_exe())
                                .map(|()| "The firewall is open for the office network (Domain and Private).".to_string());
                            self.say(r);
                            self.check_network();
                        }
                    });
                }
                ui.end_row();
            });
            ui.label(
                RichText::new("If the server isn't found on its own, teammates type the computer name or an address in File › Shared.")
                    .color(DIM)
                    .small(),
            );
        });

        card(ui, |ui| {
            ui.label(RichText::new("What it holds").strong());
            ui.add_space(4.0);
            egui::Grid::new("holds").num_columns(2).spacing(vec2(16.0, 6.0)).show(ui, |ui| {
                ui.label("People");
                ui.label(format!("{} ({} active)", self.users.len(), self.users.iter().filter(|u| !u.disabled).count()));
                ui.end_row();
                ui.label("Shared workbooks");
                ui.label(self.books.to_string());
                ui.end_row();
                ui.label("Data folder");
                ui.horizontal(|ui| {
                    ui.label(format!("{}  ({})", self.dir.display(), system::human_size(self.size)));
                    if ui.small_button("Open").clicked() {
                        system::open_in_explorer(&self.dir);
                    }
                });
                ui.end_row();
                ui.label("Version");
                ui.label(format!("Server Manager {}", gridcraft_collab::APP_VERSION.trim()));
                ui.end_row();
            });
        });

        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Server log").strong());
                if ui.small_button("Refresh").clicked() {
                    self.log = system::log_tail(&self.dir, 200);
                }
                if ui.small_button("Open the log file").clicked() {
                    system::open_in_explorer(&self.dir.join("server.log"));
                }
            });
            egui::ScrollArea::vertical().max_height(220.0).stick_to_bottom(true).show(ui, |ui| {
                let text = if self.log.is_empty() { "The log is empty: the server has not run yet." } else { &self.log };
                ui.add(egui::Label::new(RichText::new(text).monospace().size(11.5)).wrap_mode(egui::TextWrapMode::Extend));
            });
        });
    }

    fn first_admin(&mut self, ui: &mut Ui) {
        card(ui, |ui| {
            ui.label(RichText::new("First, create the administrator account").size(17.0).strong().color(ACCENT));
            ui.label("This account signs in from Dexterity Sheets like everyone else, and can manage the shared workbooks.");
            ui.add_space(6.0);
            egui::Grid::new("first").num_columns(2).spacing(vec2(14.0, 8.0)).show(ui, |ui| {
                ui.label("Full name");
                ui.add(egui::TextEdit::singleline(&mut self.first.display).hint_text("Vikram Kumar").desired_width(240.0));
                ui.end_row();
                ui.label("User name");
                let hint = roster::user_from_name(&self.first.display);
                ui.add(
                    egui::TextEdit::singleline(&mut self.first.user)
                        .hint_text(if hint.is_empty() { "vikram".to_string() } else { hint })
                        .desired_width(240.0),
                );
                ui.end_row();
                ui.label("Password");
                password_field(ui, &mut self.first.password, &mut self.first.show, "6 characters or more");
                ui.end_row();
                if !self.first.show {
                    ui.label("Repeat password");
                    ui.add(egui::TextEdit::singleline(&mut self.first_repeat).password(true).desired_width(170.0));
                    ui.end_row();
                }
            });
            ui.add_space(6.0);
            if ui.add_enabled(self.admin, primary("Create administrator")).clicked() {
                let user = if self.first.user.trim().is_empty() {
                    roster::user_from_name(&self.first.display)
                } else {
                    self.first.user.trim().to_ascii_lowercase()
                };
                let r = if !self.first.show && self.first.password != self.first_repeat {
                    Err("The two passwords differ.".to_string())
                } else {
                    server::set_user(&self.dir, &user, &self.first.display, &self.first.password, Role::Admin)
                        .map(|()| format!("Administrator {user} created. Add your team on the People page."))
                        .map_err(|e| e.to_string())
                };
                if r.is_ok() {
                    self.first = PersonForm { role: Some(Role::Admin), ..PersonForm::default() };
                    self.first_repeat.clear();
                }
                self.say(r);
                self.reload();
            }
        });
    }

    fn people(&mut self, ui: &mut Ui) {
        ui.heading("People");
        ui.label(RichText::new("Everyone who can sign in to shared workbooks. Changes apply the next time the person signs in.").color(DIM));
        ui.add_space(8.0);
        card(ui, |ui| {
            ui.label(RichText::new("Add a person").strong());
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.add.display).hint_text("Full name").desired_width(180.0));
                let hint = roster::user_from_name(&self.add.display);
                ui.add(
                    egui::TextEdit::singleline(&mut self.add.user)
                        .hint_text(if hint.is_empty() { "user name".to_string() } else { hint })
                        .desired_width(140.0),
                );
                let mut role = self.add.role.unwrap_or(Role::Editor);
                role_combo(ui, "add-role", &mut role);
                self.add.role = Some(role);
                password_field(ui, &mut self.add.password, &mut self.add.show, "password (or Make one)");
                if ui.add_enabled(self.admin, primary("Add")).clicked() {
                    let row = Row {
                        line: 1,
                        user: self.add.user.clone(),
                        display: self.add.display.clone(),
                        role: role_label(role).into(),
                        password: self.add.password.clone(),
                    };
                    let c = roster::check(&[row], &self.users).remove(0);
                    let r = if let Some(p) = c.problems.first() {
                        Err(p.clone())
                    } else {
                        server::set_user(&self.dir, &c.user, &c.display, &c.password, c.role).map_err(|e| e.to_string()).map(|()| {
                            self.shown = Some((c.user.clone(), c.password.clone()));
                            format!("{} can now sign in as {}.", c.display, c.user)
                        })
                    };
                    if r.is_ok() {
                        self.add = PersonForm { role: Some(Role::Editor), ..PersonForm::default() };
                    }
                    self.say(r);
                    self.reload();
                }
            });
            ui.label(
                RichText::new("Many at once? Use \"Add many people\": type them in a list, paste from Excel, or import the template.")
                    .color(DIM)
                    .small(),
            );
        });
        self.shown_password(ui);
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("Search by name, user name or role").desired_width(300.0));
            ui.label(RichText::new(format!("{} people", self.users.len())).color(DIM));
        });
        ui.add_space(6.0);
        let admins = self.active_admins();
        let q = self.search.trim().to_ascii_lowercase();
        let mut changed: Option<UserRecord> = None;
        let mut reset: Option<String> = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("people").num_columns(5).striped(true).spacing(vec2(18.0, 8.0)).show(ui, |ui| {
                for h in ["Full name", "User name", "Role", "Status", ""] {
                    ui.label(RichText::new(h).strong());
                }
                ui.end_row();
                for u in &self.users {
                    if !q.is_empty()
                        && ![&u.display, &u.user, &role_label(u.role).to_ascii_lowercase()].iter().any(|s| s.to_ascii_lowercase().contains(&q))
                    {
                        continue;
                    }
                    let last_admin = u.role == Role::Admin && !u.disabled && admins <= 1;
                    ui.label(RichText::new(&u.display).strong());
                    ui.label(&u.user);
                    let mut role = u.role;
                    ui.add_enabled_ui(self.admin && !last_admin, |ui| {
                        if role_combo(ui, ("role", &u.user), &mut role) {
                            changed = Some(UserRecord { role, ..u.clone() });
                        }
                    })
                    .response
                    .on_disabled_hover_text(if last_admin {
                        "The last administrator stays an administrator."
                    } else {
                        "Needs administrator rights."
                    });
                    ui.label(if u.disabled { RichText::new("Disabled").color(BAD) } else { RichText::new("Active").color(OK) });
                    ui.horizontal(|ui| {
                        ui.add_enabled_ui(self.admin, |ui| {
                            if ui.small_button("Reset password…").clicked() {
                                reset = Some(u.user.clone());
                            }
                            let toggle = if u.disabled { "Enable" } else { "Disable" };
                            if ui.add_enabled(!last_admin, egui::Button::new(toggle).small()).clicked() {
                                changed = Some(UserRecord { disabled: !u.disabled, ..u.clone() });
                            }
                            if ui.add_enabled(!last_admin, egui::Button::new("Remove…").small()).clicked() {
                                self.confirm = Some(("remove".into(), u.user.clone()));
                            }
                        });
                    });
                    ui.end_row();
                }
            });
        });
        if let Some(u) = changed {
            let mut all = self.users.clone();
            if let Some(slot) = all.iter_mut().find(|x| x.user == u.user) {
                *slot = u.clone();
            }
            let r = server::save_users(&self.dir, &all).map(|()| format!("{} updated.", u.display)).map_err(|e| e.to_string());
            self.say(r);
            self.reload();
        }
        if let Some(user) = reset {
            self.confirm = Some(("reset".into(), user));
        }
        self.confirm_dialog(ui);
    }

    fn shown_password(&mut self, ui: &mut Ui) {
        let Some((user, pw)) = self.shown.clone() else { return };
        card(ui, |ui| {
            ui.label(RichText::new(format!("Password for {user}")).strong());
            ui.horizontal(|ui| {
                ui.label(RichText::new(&pw).monospace().size(18.0).color(ACCENT));
                if ui.button("Copy").clicked() {
                    ui.ctx().copy_text(format!("User name: {user}\nPassword: {pw}"));
                }
                if ui.button("Done").clicked() {
                    self.shown = None;
                }
            });
            ui.label(
                RichText::new("Shown only now. Give it to them in person; they can change it in File › Shared › Change my password.")
                    .color(DIM)
                    .small(),
            );
        });
    }

    fn confirm_dialog(&mut self, ui: &mut Ui) {
        let Some((what, user)) = self.confirm.clone() else { return };
        let display = self.users.iter().find(|u| u.user == user).map(|u| u.display.clone()).unwrap_or_else(|| user.clone());
        let mut open = true;
        let mut close = false;
        egui::Window::new(if what == "remove" { "Remove this person?" } else { "Reset the password?" })
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(ui.ctx(), |ui| {
                if what == "remove" {
                    ui.label(format!("{display} ({user}) will no longer be able to sign in. Their past edits keep their name in Edit History."));
                    ui.label(RichText::new("To stop someone only for a while, use Disable instead.").color(DIM));
                } else {
                    ui.label(format!("{display} ({user}) gets a new password. The old one stops working at their next sign-in."));
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let go = if what == "remove" { "Remove" } else { "Make a new password" };
                    if ui.add(primary(go)).clicked() {
                        let r = if what == "remove" {
                            let all: Vec<UserRecord> = self.users.iter().filter(|u| u.user != user).cloned().collect();
                            server::save_users(&self.dir, &all).map(|()| format!("{display} removed.")).map_err(|e| e.to_string())
                        } else {
                            let pw = roster::new_password();
                            let rec = self.users.iter().find(|u| u.user == user).cloned();
                            match rec {
                                Some(u) => server::set_user(&self.dir, &u.user, &u.display, &pw, u.role).map_err(|e| e.to_string()).map(|()| {
                                    self.shown = Some((u.user.clone(), pw));
                                    format!("New password made for {display}.")
                                }),
                                None => Err(format!("{user} no longer exists.")),
                            }
                        };
                        self.say(r);
                        self.reload();
                        close = true;
                    }
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                });
            });
        if close || !open {
            self.confirm = None;
        }
    }

    fn add_many(&mut self, ui: &mut Ui) {
        ui.heading("Add many people");
        ui.label(
            RichText::new(
                "Type them in, paste rows copied from Excel, or fill in the template and import it. Nothing is created until every row is right.",
            )
            .color(DIM),
        );
        ui.add_space(8.0);
        card(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.button("Download template…").on_hover_text("An Excel file with the columns to fill in").clicked() {
                    let r = roster::template_xlsx().and_then(|bytes| {
                        match rfd::FileDialog::new()
                            .add_filter("Excel workbook", &["xlsx"])
                            .set_file_name("Dexterity Sheets people template.xlsx")
                            .save_file()
                        {
                            Some(p) => std::fs::write(&p, bytes)
                                .map(|()| format!("Template saved to {}. Fill in one person per row, then import it.", p.display()))
                                .map_err(|e| e.to_string()),
                            None => Ok(String::new()),
                        }
                    });
                    if !matches!(&r, Ok(m) if m.is_empty()) {
                        self.say(r);
                    }
                }
                if ui.button("Import from file…").on_hover_text(".xlsx, .xls, .ods, .csv or .tsv").clicked()
                    && let Some(p) =
                        rfd::FileDialog::new().add_filter("People list", &["xlsx", "xlsm", "xls", "ods", "csv", "tsv", "txt"]).pick_file()
                {
                    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let r = std::fs::read(&p).map_err(|e| e.to_string()).and_then(|b| roster::from_file(&name, &b));
                    match r {
                        Ok(rows) => {
                            let n = rows.len();
                            self.rows.extend(rows);
                            self.say(Ok(format!("{n} row(s) read from {name}. Check them below, then click Create.")));
                        }
                        Err(e) => self.say(Err(e)),
                    }
                }
                if ui.button("Add a row").clicked() {
                    self.rows.push(Row { line: self.rows.len() + 1, ..Row::default() });
                }
                if !self.rows.is_empty() && ui.button("Clear the list").clicked() {
                    self.rows.clear();
                }
            });
            ui.add_space(6.0);
            ui.label(RichText::new("Or paste rows copied from Excel (User name, Full name, Role, Password):").color(DIM).small());
            let r = ui.add(egui::TextEdit::multiline(&mut self.pasted).desired_rows(2).desired_width(f32::INFINITY).hint_text("paste here"));
            if r.changed() && !self.pasted.trim().is_empty() {
                match roster::from_pasted(&self.pasted) {
                    Ok(rows) => {
                        let n = rows.len();
                        self.rows.extend(rows);
                        self.pasted.clear();
                        self.say(Ok(format!("{n} pasted row(s) added.")));
                    }
                    Err(e) => self.say(Err(e)),
                }
            }
        });
        let checked = roster::check(&self.rows, &self.users);
        let bad = checked.iter().filter(|c| !c.problems.is_empty()).count();
        if !self.rows.is_empty() {
            let mut remove = None;
            egui::ScrollArea::vertical().max_height(ui.available_height() - 120.0).show(ui, |ui| {
                egui::Grid::new("many").num_columns(6).striped(true).spacing(vec2(12.0, 6.0)).max_col_width(420.0).show(ui, |ui| {
                    for h in ["User name", "Full name", "Role", "Password", "", "Check"] {
                        ui.label(RichText::new(h).strong());
                    }
                    ui.end_row();
                    for (i, (row, c)) in self.rows.iter_mut().zip(&checked).enumerate() {
                        ui.add_sized([140.0, 24.0], egui::TextEdit::singleline(&mut row.user).hint_text(c.user.clone()));
                        ui.add_sized([180.0, 24.0], egui::TextEdit::singleline(&mut row.display));
                        let mut role = roster::role_of(&row.role).unwrap_or(Role::Editor);
                        if role_combo(ui, ("many-role", i), &mut role) || roster::role_of(&row.role).is_none() && row.role.is_empty() {
                            row.role = role_label(role).to_string();
                        }
                        ui.add_sized([130.0, 24.0], egui::TextEdit::singleline(&mut row.password).hint_text("made for them"));
                        if ui.small_button("Remove").on_hover_text("Take this row out of the list").clicked() {
                            remove = Some(i);
                        }
                        if c.problems.is_empty() {
                            ui.label(RichText::new("Ready").color(OK));
                        } else {
                            ui.add(egui::Label::new(RichText::new(c.problems.join("; ")).color(BAD)).wrap_mode(egui::TextWrapMode::Wrap));
                        }
                        ui.end_row();
                    }
                });
            });
            if let Some(i) = remove {
                self.rows.remove(i);
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let ready = checked.len() - bad;
                let ctx = ui.ctx().clone();
                if ui.add_enabled(bad == 0 && ready > 0 && self.admin && self.job.is_none(), primary(&format!("Create {ready} account(s)"))).clicked()
                {
                    let dir = self.dir.clone();
                    let list = checked.clone();
                    self.run("Creating accounts…", &ctx, move |progress| {
                        let mut ok = Vec::new();
                        let mut failed = Vec::new();
                        let total = list.len();
                        for (i, c) in list.into_iter().enumerate() {
                            if let Ok(mut p) = progress.lock() {
                                *p = (i, total);
                            }
                            match server::set_user(&dir, &c.user, &c.display, &c.password, c.role) {
                                Ok(()) => ok.push(c),
                                Err(e) => failed.push((c.user.clone(), e.to_string())),
                            }
                        }
                        Done::Created { ok, failed }
                    });
                }
                if bad > 0 {
                    ui.label(RichText::new(format!("{bad} row(s) need fixing first.")).color(BAD));
                }
            });
        }
        if !self.created.is_empty() || !self.failed.is_empty() {
            card(ui, |ui| {
                ui.label(RichText::new(format!("{} account(s) created", self.created.len())).strong().color(OK));
                for (u, e) in &self.failed {
                    ui.label(RichText::new(format!("{u}: {e}")).color(BAD));
                }
                ui.horizontal(|ui| {
                    if !self.created.is_empty() && ui.add(primary("Save the sign-in sheet…")).clicked() {
                        let server = std::env::var("COMPUTERNAME").unwrap_or_default();
                        let r = roster::sign_in_sheet_xlsx(&self.created, &server).and_then(|bytes| {
                            match rfd::FileDialog::new()
                                .add_filter("Excel workbook", &["xlsx"])
                                .set_file_name("Dexterity Sheets sign-in details.xlsx")
                                .save_file()
                            {
                                Some(p) => std::fs::write(&p, bytes)
                                    .map(|()| format!("Saved to {}. Delete it once everyone has signed in.", p.display()))
                                    .map_err(|e| e.to_string()),
                                None => Ok(String::new()),
                            }
                        });
                        if !matches!(&r, Ok(m) if m.is_empty()) {
                            self.say(r);
                        }
                    }
                    if ui.button("Copy as text").clicked() {
                        let text: String =
                            self.created.iter().map(|c| format!("{}\t{}\t{}\t{}\n", c.display, c.user, role_label(c.role), c.password)).collect();
                        ui.ctx().copy_text(text);
                        self.say(Ok("Copied: name, user name, role and password per line.".into()));
                    }
                    if ui.button("Done").clicked() {
                        self.created.clear();
                        self.failed.clear();
                    }
                });
                ui.label(RichText::new("Passwords are shown only now: save or copy them before you leave this page.").color(DIM).small());
            });
        }
    }

    fn updates(&mut self, ui: &mut Ui) {
        ui.heading("App updates");
        ui.label(RichText::new("Every Dexterity Sheets on the network asks this server for new versions and installs them with one click. Nothing on the office PCs needs internet access.").color(DIM));
        ui.add_space(8.0);
        card(ui, |ui| {
            ui.label(RichText::new("Fetch new versions automatically").strong());
            ui.label(RichText::new("The server checks the GitHub releases every 30 minutes, checks each download against the release's checksums, and offers it to every PC.").color(DIM));
            egui::Grid::new("src").num_columns(2).spacing(vec2(14.0, 6.0)).show(ui, |ui| {
                ui.label("Repository");
                ui.add(egui::TextEdit::singleline(&mut self.repo).hint_text("vikramdex-ops/dexterity-sheets").desired_width(300.0));
                ui.end_row();
                ui.label("Access token");
                ui.add(egui::TextEdit::singleline(&mut self.token).password(true).hint_text("only for a private repository").desired_width(300.0));
                ui.end_row();
            });
            ui.horizontal(|ui| {
                if ui.add_enabled(self.admin, egui::Button::new("Save")).clicked() {
                    let r = system::set_update_source(&self.dir, &self.repo, &self.token).map(|()| {
                        if self.repo.trim().is_empty() {
                            "Automatic fetching is off.".to_string()
                        } else {
                            "Saved. The server checks within 5 minutes.".to_string()
                        }
                    });
                    self.say(r);
                }
                if ui.add_enabled(self.admin && !self.repo.is_empty(), egui::Button::new("Turn off")).clicked() {
                    self.repo.clear();
                    self.token.clear();
                    let r = system::set_update_source(&self.dir, "", "").map(|()| "Automatic fetching is off.".to_string());
                    self.say(r);
                }
            });
        });
        card(ui, |ui| {
            ui.label(RichText::new("Installers offered to the office PCs").strong());
            ui.label(
                RichText::new("The newest version here is offered. Without internet on this server, add the .msi from the release page yourself.")
                    .color(DIM),
            );
            ui.add_space(4.0);
            let mut remove = None;
            if self.installers.is_empty() {
                ui.label(RichText::new("None yet.").color(DIM));
            }
            egui::Grid::new("inst").num_columns(3).striped(true).spacing(vec2(16.0, 6.0)).show(ui, |ui| {
                for (i, inst) in self.installers.iter().enumerate() {
                    ui.label(&inst.name).on_hover_text(if inst.notes.is_empty() { "no release notes" } else { &inst.notes });
                    ui.label(system::human_size(inst.size));
                    if ui.add_enabled(self.admin, egui::Button::new("Remove").small()).clicked() {
                        remove = Some(i);
                    }
                    ui.end_row();
                }
            });
            if let Some(i) = remove
                && let Some(inst) = self.installers.get(i).cloned()
            {
                let r = system::remove_installer(&inst).map(|()| format!("{} removed.", inst.name));
                self.say(r);
                self.reload();
            }
            ui.add_space(6.0);
            ui.label(RichText::new("What's new in it (optional, shown to everyone with the update):").small().color(DIM));
            ui.add(egui::TextEdit::multiline(&mut self.notes).desired_rows(2).desired_width(f32::INFINITY));
            if ui.add_enabled(self.admin, egui::Button::new("Add an installer…")).clicked()
                && let Some(p) = rfd::FileDialog::new().add_filter("Dexterity Sheets installer", &["msi"]).pick_file()
            {
                let r = system::add_installer(&self.dir, &p, &self.notes).map(|n| format!("{n} added: every PC is offered it within a minute."));
                if r.is_ok() {
                    self.notes.clear();
                }
                self.say(r);
                self.reload();
            }
        });
    }

    fn backup(&mut self, ui: &mut Ui) {
        ui.heading("Backup and restore");
        ui.label(RichText::new("A backup is one .zip with every account, shared workbook, edit history, version and setting.").color(DIM));
        ui.add_space(8.0);
        let ctx = ui.ctx().clone();
        card(ui, |ui| {
            ui.label(RichText::new("Back up now").strong());
            ui.checkbox(&mut self.with_installers, "Include the app installers (larger; they can be downloaded again)");
            if ui.add_enabled(self.job.is_none(), primary("Back up…")).clicked() {
                let file = format!("Dexterity Sheets Server backup {}.zip", chrono::Local::now().format("%Y-%m-%d %H%M"));
                if let Some(to) = rfd::FileDialog::new().add_filter("Backup", &["zip"]).set_file_name(&file).save_file() {
                    let dir = self.dir.clone();
                    let with = self.with_installers;
                    self.run("Backing up…", &ctx, move |_| {
                        Done::Message(system::backup(&dir, &to, with).map(|n| format!("Backed up {n} files to {}.", to.display())))
                    });
                }
            }
            ui.label(RichText::new("Keep backups on another PC or drive. The server keeps working while it is backed up.").color(DIM).small());
        });
        card(ui, |ui| {
            ui.label(RichText::new("Restore a backup").strong());
            ui.label(RichText::new("Replaces everything on this server with the backup. The server stops for a moment; the current data is kept beside it, so nothing is deleted.").color(DIM));
            if ui.add_enabled(self.admin && self.job.is_none(), egui::Button::new("Restore from a backup…")).clicked()
                && let Some(from) = rfd::FileDialog::new().add_filter("Backup", &["zip"]).pick_file()
            {
                self.confirm = Some(("restore".into(), from.display().to_string()));
            }
        });
        if let Some((what, path)) = self.confirm.clone()
            && what == "restore"
        {
            let mut open = true;
            let mut close = false;
            egui::Window::new("Restore this backup?")
                .collapsible(false)
                .resizable(false)
                .open(&mut open)
                .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0))
                .show(&ctx, |ui| {
                    ui.label(format!("{path}\n\nEveryone is disconnected for a moment, then sees the shared workbooks as they were in the backup."));
                    ui.horizontal(|ui| {
                        if ui.add(primary("Restore")).clicked() {
                            let dir = self.dir.clone();
                            let from = PathBuf::from(&path);
                            let was_running = self.service == ServiceState::Running;
                            self.run("Restoring…", &ctx, move |_| Done::Message(restore_flow(&dir, &from, was_running)));
                            close = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                });
            if close || !open {
                self.confirm = None;
            }
        }
    }
}

fn restore_flow(dir: &Path, from: &Path, was_running: bool) -> Result<String, String> {
    if was_running {
        system::stop_service()?;
    }
    let r = system::restore(dir, from);
    let started = if was_running { system::start_service() } else { Ok(()) };
    let old = r?;
    started?;
    Ok(format!("Restored. The data it replaced is kept in {}.", old.display()))
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll(&ctx);
        egui::Panel::top("title").exact_size(54.0).show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.add_space(8.0);
                ui.label(RichText::new("Dexterity Sheets Server Manager").size(19.0).strong().color(ACCENT));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(8.0);
                    if !self.admin {
                        ui.label(RichText::new("View only: run as administrator to make changes").color(WARN));
                    }
                });
            });
        });
        egui::Panel::bottom("status").exact_size(30.0).show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                if let Some(job) = &self.job {
                    ui.spinner();
                    let (i, n) = self.job.as_ref().and_then(|j| j.progress.lock().ok().map(|p| *p)).unwrap_or((0, 0));
                    ui.label(if n > 0 { format!("{} {i} of {n}", job.label) } else { job.label.clone() });
                } else if let Some((m, err)) = &self.message {
                    ui.label(RichText::new(m).color(if *err { BAD } else { OK }));
                }
            });
        });
        egui::Panel::left("nav").exact_size(190.0).resizable(false).show(ui, |ui| {
            ui.add_space(10.0);
            for p in Page::ALL {
                let r = ui.add_sized(vec2(174.0, 34.0), egui::Button::selectable(self.page == p, RichText::new(p.label()).size(14.5)));
                if r.clicked() && self.page != p {
                    self.page = p;
                    self.message = None;
                    self.polled_at = f64::MIN;
                }
            }
        });
        egui::CentralPanel::default().show(ui, |ui| {
            if let Some(e) = &self.users_error {
                ui.label(RichText::new(e).color(BAD));
            }
            egui::ScrollArea::vertical().id_salt(self.page.id()).show(ui, |ui| match self.page {
                Page::Overview => self.overview(ui),
                Page::People => self.people(ui),
                Page::AddMany => self.add_many(ui),
                Page::Updates => self.updates(ui),
                Page::Backup => self.backup(ui),
            });
        });
        self.take_shot(&ctx);
    }
}

impl App {
    /// `--import FILE`: opens on Add many people with that list read in and checked.
    fn with_import(mut self, file: Option<&str>) -> App {
        let Some(file) = file else { return self };
        let name = Path::new(file).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        match std::fs::read(file).map_err(|e| format!("{file}: {e}")).and_then(|b| roster::from_file(&name, &b)) {
            Ok(rows) => {
                let n = rows.len();
                self.rows = rows;
                self.say(Ok(format!("{n} row(s) read from {name}. Check them below, then click Create.")));
            }
            Err(e) => self.say(Err(e)),
        }
        self.page = Page::AddMany;
        self
    }

    /// `--shot`: once the page has settled, saves the window as a PNG and closes.
    fn take_shot(&mut self, ctx: &egui::Context) {
        let Some((_, path, frames)) = &mut self.shot else { return };
        *frames += 1;
        ctx.request_repaint();
        if *frames != 20 {
            return;
        }
        let path = path.clone();
        let c = ctx.clone();
        ctx.request_screenshot(move |img| {
            let size = [img.width() as u32, img.height() as u32];
            let bytes: Vec<u8> = img.pixels.iter().flat_map(|p| p.to_array()).collect();
            if let Some(buf) = image::RgbaImage::from_raw(size[0], size[1], bytes) {
                let _ = buf.save(&path);
            }
            c.send_viewport_cmd(egui::ViewportCommand::Close);
        });
    }
}

fn arg_value(args: &mut Vec<String>, flag: &str) -> Option<String> {
    let i = args.iter().position(|a| a == flag)?;
    let v = args.get(i + 1).cloned();
    args.drain(i..(i + 2).min(args.len()));
    v
}

fn main() -> eframe::Result {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let original = args.clone();
    let dir = arg_value(&mut args, "--data").map(PathBuf::from).unwrap_or_else(system::default_data_dir);
    let import = arg_value(&mut args, "--import");
    let no_elevate = args.iter().any(|a| a == "--no-elevate");
    let shot = args.iter().position(|a| a == "--shot").and_then(|i| {
        let page = Page::ALL.into_iter().find(|p| Some(p.id()) == args.get(i + 1).map(String::as_str))?;
        Some((page, PathBuf::from(args.get(i + 2)?)))
    });
    let mut admin = system::is_admin();
    // Managing the service and the protected data folder needs administrator rights: ask once.
    if !admin && !no_elevate && shot.is_none() && system::relaunch_elevated(&original) {
        return Ok(());
    }
    if no_elevate && !admin {
        // Tests and screenshots work on a folder of their own; let them change it.
        admin = std::fs::create_dir_all(&dir).is_ok() && std::fs::write(dir.join(".write-test"), b"").is_ok();
        let _ = std::fs::remove_file(dir.join(".write-test"));
    }
    let _ = std::fs::create_dir_all(&dir);
    let icon = image::load_from_memory(include_bytes!("../../../assets/app-icon/hicolor/256x256/apps/ai.storyteller.gridcraft.png")).ok().map(|i| {
        let i = i.to_rgba8();
        egui::IconData { width: i.width(), height: i.height(), rgba: i.into_raw() }
    });
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Dexterity Sheets Server Manager")
        .with_inner_size([1100.0, 760.0])
        .with_min_inner_size([820.0, 560.0]);
    if let Some(icon) = icon {
        viewport = viewport.with_icon(Arc::new(icon));
    }
    let mut options = eframe::NativeOptions { viewport, ..Default::default() };
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut options.wgpu_options.wgpu_setup {
        // DirectX 12 only (with Windows' software renderer on a server without a graphics
        // driver); FXC, the shader compiler that ships with Windows.
        create.instance_descriptor.backends = eframe::wgpu::Backends::DX12;
        create.instance_descriptor.backend_options.dx12.shader_compiler = eframe::wgpu::Dx12Compiler::Fxc;
    }
    eframe::run_native(
        "Dexterity Sheets Server Manager",
        options,
        Box::new(move |cc| {
            cc.egui_ctx.set_theme(egui::Theme::Light);
            cc.egui_ctx.all_styles_mut(|s| {
                s.spacing.button_padding = vec2(10.0, 5.0);
                s.spacing.item_spacing = vec2(8.0, 7.0);
            });
            Ok(Box::new(App::new(dir, admin, shot).with_import(import.as_deref())))
        }),
    )
}
