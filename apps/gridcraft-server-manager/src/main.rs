//! Dexterity Sheets Server Manager: the office server's control panel for IT, with no commands to
//! type. Start and stop the server, see how teammates reach it, see who has connected and from
//! which PCs (approve or revoke a PC by its device code, set roles), hand new app versions to
//! every PC, and back up or restore everything. People sign in with their Windows account: there
//! are no accounts or passwords to create.
//!
//! ```text
//! DexSheetsServerManager [--data DIR] [--no-elevate]
//!                        [--shot overview|people|updates|backup|connect|versions|releases|feedback|health|adoption OUT.png]
//! ```
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod admin;
mod connect_page;
mod system;

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};

use egui::{Color32, RichText, Ui, vec2};
use gridcraft_collab::proto::{DeviceInfo, DeviceStatus, ProfileInfo, Role};
use gridcraft_collab::server::{self, UserRecord};
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
    Updates,
    Backup,
    Connect,
    // The dashboard (admin.rs).
    Versions,
    Releases,
    Feedback,
    Health,
    Adoption,
}

impl Page {
    const ALL: [Page; 10] = [
        Page::Overview,
        Page::People,
        Page::Updates,
        Page::Backup,
        Page::Connect,
        Page::Versions,
        Page::Releases,
        Page::Feedback,
        Page::Health,
        Page::Adoption,
    ];
    /// The first page of the dashboard group in the page list.
    const DASHBOARD: Page = Page::Versions;
    fn label(self) -> &'static str {
        match self {
            Page::Overview => "Server",
            Page::People => "People and devices",
            Page::Updates => "App updates",
            Page::Backup => "Backup and restore",
            Page::Connect => "Dexterity Connect",
            Page::Versions => "Installed versions",
            Page::Releases => "Releases",
            Page::Feedback => "Feedback",
            Page::Health => "System health",
            Page::Adoption => "Feature adoption",
        }
    }
    fn id(self) -> &'static str {
        match self {
            Page::Overview => "overview",
            Page::People => "people",
            Page::Updates => "updates",
            Page::Backup => "backup",
            Page::Connect => "connect",
            Page::Versions => "versions",
            Page::Releases => "releases",
            Page::Feedback => "feedback",
            Page::Health => "health",
            Page::Adoption => "adoption",
        }
    }
}

/// Work that takes a while (backups, the service), run off the window's thread so it never
/// freezes.
enum Done {
    Message(Result<String, String>),
}

struct Job {
    label: String,
    progress: Arc<Mutex<(usize, usize)>>,
    rx: Receiver<Done>,
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
    /// `profiles.json`: everyone seen and their devices.
    profiles: server::Profiles,
    /// The same people as the dashboard looks them up.
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
    // People
    search: String,
    /// What waits for a yes: ("remove", user) or ("restore", backup file).
    confirm: Option<(String, String)>,
    // Updates
    installers: Vec<system::Installer>,
    notes: String,
    repo: String,
    token: String,
    // Backup
    with_installers: bool,
    job: Option<Job>,
    shot: Option<(Page, PathBuf, u32)>,
    // Dexterity Connect
    connect: connect_page::ConnectPage,
    // Installed versions, Releases, Feedback, System health, Feature adoption
    dash: admin::Dashboard,
}

fn role_label(r: Role) -> &'static str {
    match r {
        Role::Admin => "Administrator",
        Role::Editor => "Editor",
        Role::Viewer => "Viewer",
    }
}

/// The Windows account as Windows spells it.
fn shown_account(p: &ProfileInfo) -> String {
    if p.account.is_empty() { p.user.clone() } else { p.account.clone() }
}

/// A date and time from ms since 1970, in this PC's time zone.
fn when(ms: i64) -> String {
    use chrono::TimeZone;
    chrono::Local.timestamp_millis_opt(ms).single().map(|t| t.format("%d %b %Y %H:%M").to_string()).unwrap_or_else(|| "—".into())
}

impl App {
    fn new(dir: PathBuf, admin: bool, shot: Option<(Page, PathBuf)>) -> App {
        let (repo, token) = system::update_source(&dir);
        let name = system::server_name(&dir);
        let mut app = App {
            page: shot.as_ref().map_or(Page::Overview, |s| s.0),
            dir,
            admin,
            profiles: server::Profiles::default(),
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
            search: String::new(),
            confirm: None,
            installers: Vec::new(),
            notes: String::new(),
            repo,
            token,
            with_installers: false,
            job: None,
            shot: shot.map(|(p, path)| (p, path, 0)),
            dash: admin::Dashboard::default(),
            connect: connect_page::ConnectPage::default(),
        };
        app.connect.load(&app.dir);
        app.reload();
        app.check_network();
        app
    }

    fn reload(&mut self) {
        match server::load_profiles(&self.dir) {
            Ok(p) => {
                self.users = p.users();
                self.profiles = p;
                self.users_error = None;
            }
            Err(e) => self.users_error = Some(format!("The people file (profiles.json) can't be read: {e}")),
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

    /// Administrators, counting old accounts still waiting to be taken over.
    fn admins(&self) -> usize {
        self.profiles.profiles.values().filter(|p| p.role == Role::Admin).count()
            + self.profiles.carry_over.iter().filter(|c| c.role == Role::Admin && !c.disabled).count()
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

impl App {
    fn overview(&mut self, ui: &mut Ui) {
        ui.heading("Server");
        ui.add_space(8.0);
        if self.users_error.is_none() && self.admins() == 0 {
            card(ui, |ui| {
                ui.label(RichText::new("No administrator yet").size(17.0).strong().color(ACCENT));
                ui.label("The first person to connect from Dexterity Sheets becomes the administrator: open it on your own PC first. Nobody needs an account or a password; everyone signs in with their Windows account.");
            });
        }
        // An administrator carried over from the old accounts list waits for approval here: with
        // no other administrator signed in, this window is the only place to approve them.
        let admin_in =
            |p: &&ProfileInfo| p.role == Role::Admin && (p.devices.is_empty() || p.devices.iter().any(|d| d.status == DeviceStatus::Approved));
        let waiting: Vec<String> = self
            .profiles
            .profiles
            .values()
            .filter(|p| p.role == Role::Admin && p.devices.iter().any(|d| d.status == DeviceStatus::Pending))
            .map(|p| format!("{} ({})", p.display, shown_account(p)))
            .collect();
        if self.users_error.is_none() && !waiting.is_empty() && !self.profiles.profiles.values().any(|p| admin_in(&p)) {
            card(ui, |ui| {
                ui.label(RichText::new("An administrator waits for approval").size(17.0).strong().color(WARN));
                ui.label(format!(
                    "{} took over an administrator account from the old accounts list. Check the device code with them, then approve their PC on People and devices: until then nobody is the administrator.",
                    waiting.join(", ")
                ));
                if ui.button("Open People and devices").clicked() {
                    self.page = Page::People;
                }
            });
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
                    ui.label(
                        RichText::new("Dexterity Sheets on each office PC finds this server and signs in with the person's Windows account.")
                            .color(DIM),
                    );
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
                let waiting = self.profiles.profiles.values().flat_map(|p| &p.devices).filter(|d| d.status == DeviceStatus::Pending).count();
                ui.label(if waiting > 0 {
                    RichText::new(format!("{} ({waiting} PC(s) waiting for approval on People and devices)", self.profiles.profiles.len()))
                        .color(WARN)
                } else {
                    RichText::new(self.profiles.profiles.len().to_string())
                });
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

    /// Changes `profiles.json` (read fresh, so nothing the server wrote meanwhile is lost) and
    /// says how it went. The running server applies it within half a minute.
    fn change(&mut self, f: impl FnOnce(&mut server::Profiles) -> Result<String, String>) {
        let r = server::load_profiles(&self.dir)
            .map_err(|e| e.to_string())
            .and_then(|mut all| f(&mut all).and_then(|said| server::save_profiles(&self.dir, &all).map(|()| said).map_err(|e| e.to_string())));
        self.say(r);
        self.reload();
    }

    fn people(&mut self, ui: &mut Ui) {
        ui.heading("People and devices");
        ui.label(
            RichText::new(
                "Everyone who has connected from Dexterity Sheets, with the PCs they use. People sign in with their Windows account: nobody has a password to create or reset. A person's first PC is trusted automatically; another PC waits here until you approve it. Changes reach the running server within half a minute.",
            )
            .color(DIM),
        );
        ui.add_space(8.0);
        let mut todo: Option<(String, String, DeviceStatus)> = None;
        let pending: Vec<(ProfileInfo, DeviceInfo)> = self
            .profiles
            .profiles
            .values()
            .flat_map(|p| p.devices.iter().filter(|d| d.status == DeviceStatus::Pending).map(move |d| (p.clone(), d.clone())))
            .collect();
        if !pending.is_empty() {
            egui::Frame::new()
                .fill(Color32::from_rgb(0xff, 0xf4, 0xdc))
                .stroke(egui::Stroke::new(1.0, WARN))
                .corner_radius(8.0)
                .inner_margin(14.0)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(format!("{} PC(s) waiting for approval", pending.len())).strong().color(WARN));
                    ui.label(
                        RichText::new("Approve a PC only if the person tells you the same device code: Dexterity Sheets shows it in File › Shared.")
                            .color(DIM)
                            .small(),
                    );
                    ui.add_space(4.0);
                    egui::Grid::new("pending").num_columns(5).spacing(vec2(16.0, 6.0)).show(ui, |ui| {
                        for (p, d) in &pending {
                            ui.label(RichText::new(&p.display).strong());
                            ui.label(shown_account(p));
                            ui.label(RichText::new(&d.code).monospace().strong());
                            ui.label(format!("{} · first seen {}", if d.machine.is_empty() { "unknown PC" } else { &d.machine }, when(d.first_seen)));
                            ui.horizontal(|ui| {
                                ui.add_enabled_ui(self.admin, |ui| {
                                    if ui.add(primary("Approve")).clicked() {
                                        todo = Some((p.user.clone(), d.code.clone(), DeviceStatus::Approved));
                                    }
                                    if ui.button("Refuse").on_hover_text("Revokes this PC: it can't sign in until approved").clicked() {
                                        todo = Some((p.user.clone(), d.code.clone(), DeviceStatus::Revoked));
                                    }
                                })
                                .response
                                .on_disabled_hover_text("Needs administrator rights.");
                            });
                            ui.end_row();
                        }
                    });
                });
            ui.add_space(12.0);
        }
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("Search by name, account, PC or device code").desired_width(320.0));
            ui.label(RichText::new(format!("{} people", self.profiles.profiles.len())).color(DIM));
        });
        ui.add_space(6.0);
        let admins = self.profiles.profiles.values().filter(|p| p.role == Role::Admin).count();
        let q = self.search.trim().to_lowercase();
        let mut role_change: Option<(String, Role)> = None;
        if self.profiles.profiles.is_empty() {
            card(ui, |ui| {
                ui.label("Nobody has connected yet.");
                ui.label(
                    RichText::new("Each person opens Dexterity Sheets on their PC; it finds this server and signs them in with their Windows account. The first person becomes the administrator.")
                        .color(DIM),
                );
            });
        }
        egui::Grid::new("people").num_columns(5).striped(true).spacing(vec2(18.0, 8.0)).show(ui, |ui| {
            if !self.profiles.profiles.is_empty() {
                for h in ["Name", "Windows account", "Role", "Devices", ""] {
                    ui.label(RichText::new(h).strong());
                }
                ui.end_row();
            }
            for p in self.profiles.profiles.values() {
                let hay = format!(
                    "{} {} {} {}",
                    p.display,
                    p.account,
                    role_label(p.role),
                    p.devices.iter().map(|d| format!("{} {}", d.code, d.machine)).collect::<Vec<_>>().join(" ")
                )
                .to_lowercase();
                if !q.is_empty() && !hay.contains(&q) {
                    continue;
                }
                let last_admin = p.role == Role::Admin && admins <= 1;
                ui.label(RichText::new(&p.display).strong());
                ui.label(shown_account(p));
                let mut role = p.role;
                ui.add_enabled_ui(self.admin && !last_admin, |ui| {
                    if role_combo(ui, ("role", &p.user), &mut role) {
                        role_change = Some((p.user.clone(), role));
                    }
                })
                .response
                .on_disabled_hover_text(if last_admin {
                    "The last administrator stays an administrator."
                } else {
                    "Needs administrator rights."
                });
                ui.vertical(|ui| {
                    if p.devices.is_empty() {
                        ui.label(RichText::new("No PC yet: their first one is approved when they connect").color(DIM));
                    }
                    for d in &p.devices {
                        ui.horizontal(|ui| {
                            let (color, word) = match d.status {
                                DeviceStatus::Approved => (OK, "Approved"),
                                DeviceStatus::Pending => (WARN, "Waiting for approval"),
                                DeviceStatus::Revoked => (BAD, "Revoked"),
                            };
                            ui.label(RichText::new(&d.code).monospace());
                            ui.label(if d.machine.is_empty() { "unknown PC".to_string() } else { d.machine.clone() });
                            ui.label(RichText::new(word).color(color));
                            ui.label(RichText::new(format!("last seen {}", when(d.last_seen))).color(DIM).small());
                            ui.add_enabled_ui(self.admin, |ui| {
                                if d.status != DeviceStatus::Approved && ui.small_button("Approve").clicked() {
                                    todo = Some((p.user.clone(), d.code.clone(), DeviceStatus::Approved));
                                }
                                if d.status != DeviceStatus::Revoked
                                    && ui
                                        .small_button("Revoke")
                                        .on_hover_text("This PC can't sign in any more; if it is connected, it is disconnected")
                                        .clicked()
                                {
                                    todo = Some((p.user.clone(), d.code.clone(), DeviceStatus::Revoked));
                                }
                            });
                        });
                    }
                });
                ui.add_enabled_ui(self.admin && !last_admin, |ui| {
                    if ui.small_button("Remove…").clicked() {
                        self.confirm = Some(("remove".into(), p.user.clone()));
                    }
                })
                .response
                .on_disabled_hover_text(if last_admin { "The last administrator stays." } else { "Needs administrator rights." });
                ui.end_row();
            }
        });
        if !self.profiles.carry_over.is_empty() {
            ui.add_space(10.0);
            card(ui, |ui| {
                ui.label(RichText::new("Carried over from the old accounts list").strong());
                ui.label(
                    RichText::new("These accounts were made when people signed in with passwords. Each one passes, with its name, role and workbooks, to the first Windows account with the same user name that connects. An administrator's account passes only once you approve that person's PC here.")
                        .color(DIM)
                        .small(),
                );
                for c in &self.profiles.carry_over {
                    ui.label(format!("{} ({}, {}){}", c.display, c.user, role_label(c.role), if c.disabled { " · was disabled" } else { "" }));
                }
            });
        }
        if let Some((user, code, status)) = todo {
            self.change(move |all| {
                all.set_device(&code, status, Some(&user)).map_err(|e| e.to_string())?;
                Ok(format!("{code} {}.", if status == DeviceStatus::Approved { "approved: it signs in at its next try" } else { "revoked" }))
            });
        }
        if let Some((user, role)) = role_change {
            self.change(move |all| {
                all.set_role(&user, role).map_err(|e| e.to_string())?;
                Ok(format!("{user} is now {}.", role_label(role).to_lowercase()))
            });
        }
        self.confirm_dialog(ui);
    }

    fn confirm_dialog(&mut self, ui: &mut Ui) {
        let Some((what, user)) = self.confirm.clone() else { return };
        if what != "remove" {
            return;
        }
        let display = self.profiles.profiles.get(&user).map(|p| p.display.clone()).unwrap_or_else(|| user.clone());
        let mut open = true;
        let mut close = false;
        egui::Window::new("Remove this person?")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .show(ui.ctx(), |ui| {
                ui.label(format!("{display} ({user}) and their PCs are forgotten. Their past edits keep their name in Edit History."));
                ui.label(
                    RichText::new("If they connect again they start as someone new, with the Editor role. To keep a PC out, revoke it instead.")
                        .color(DIM),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.add(primary("Remove")).clicked() {
                        let who = user.clone();
                        let shown = display.clone();
                        self.change(move |all| all.remove(&who).map(|()| format!("{shown} removed.")).map_err(|e| e.to_string()));
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

    fn updates(&mut self, ui: &mut Ui) {
        ui.heading("App updates");
        ui.label(RichText::new("Every Dexterity Sheets on the network asks this server for new versions and installs them with one click. Office PCs need no internet access for that; a PC without a server reads the GitHub releases page itself.").color(DIM));
        ui.add_space(8.0);
        card(ui, |ui| {
            ui.label(RichText::new("Fetch new versions automatically").strong());
            ui.label(RichText::new(format!("The server checks the GitHub releases of {} every 30 minutes (from the start, until turned off here), checks each download against the release's checksums, and offers it to every PC.", system::DEFAULT_REPO)).color(DIM));
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
                if self.repo.trim().is_empty() {
                    ui.label(RichText::new("Off: new versions reach the PCs only as installers you add below.").color(DIM));
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
        ui.label(
            RichText::new("A backup is one .zip with everyone's profile and devices, every shared workbook, edit history, version and setting.")
                .color(DIM),
        );
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
                if p == Page::DASHBOARD {
                    ui.add_space(10.0);
                    ui.label(RichText::new("Dashboard").color(DIM).small().strong());
                }
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
                Page::Updates => self.updates(ui),
                Page::Backup => self.backup(ui),
                Page::Connect => self.connect_page(ui),
                Page::Versions => self.versions_page(ui),
                Page::Releases => self.releases_page(ui),
                Page::Feedback => self.feedback_page(ui),
                Page::Health => self.health_page(ui),
                Page::Adoption => self.adoption_page(ui),
            });
        });
        self.take_shot(&ctx);
    }
}

impl App {
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
            Ok(Box::new(App::new(dir, admin, shot)))
        }),
    )
}
