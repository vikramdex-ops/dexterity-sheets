//! Automation rules of a shared workbook (Dexterity Sheets): the window that lists and edits
//! them. The office server keeps and runs them (`server::run_rule`), so they work with nobody
//! having the workbook open. Only fixed steps exist: notify people, Quality Check, overdue
//! items, a report workbook, a completion time; never macros or scripts.

use egui::{RichText, vec2};
use gridcraft_collab::proto::{Action, ClientMsg, Rule, Trigger};
use serde_json::{Value as Json, json};

use crate::SheetApp;

#[derive(Default)]
pub struct Automation {
    pub window: bool,
    book: String,
    rules: Vec<Rule>,
    draft: Draft,
}

struct Draft {
    name: String,
    schedule: bool,
    every: String,
    day: u8,
    time: String,
    column: String,
    equals: String,
    people: Vec<String>,
    quality: bool,
    overdue: bool,
    date_col: String,
    status_col: String,
    done: String,
    report: bool,
    stamp: String,
}

impl Default for Draft {
    fn default() -> Draft {
        Draft {
            name: String::new(),
            schedule: true,
            every: "weekly".into(),
            day: 0,
            time: "09:00".into(),
            column: "F".into(),
            equals: "Completed".into(),
            people: Vec::new(),
            quality: true,
            overdue: true,
            date_col: "D".into(),
            status_col: "F".into(),
            done: "Completed".into(),
            report: true,
            stamp: String::new(),
        }
    }
}

const DAYS: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];

fn active_book(app: &SheetApp) -> Option<String> {
    let d = app.session.active()?;
    app.collab.shared(d.uid).map(|s| s.book.id.clone())
}

pub fn on_rules(app: &mut SheetApp, book: String, rules: Vec<Rule>) {
    if app.collab.automation.book == book {
        app.collab.automation.rules = rules;
    }
}

/// `automate.rules {rules?: [...]}`: opens the window, or (with `rules`) replaces them.
pub fn run_command(app: &mut SheetApp, p: &Json) -> Result<Json, String> {
    let book = active_book(app).ok_or("Automation runs on the office server: share the workbook first (Share…).")?;
    if let Some(r) = p.get("rules") {
        let rules: Vec<Rule> = serde_json::from_value(r.clone()).map_err(|e| format!("rules: {e}"))?;
        app.collab.send(ClientMsg::SetRules { book: book.clone(), rules });
        return Ok(json!({"book": book}));
    }
    if let Some(id) = p.get("run").and_then(Json::as_str) {
        app.collab.send(ClientMsg::RunRule { book: book.clone(), id: id.to_string() });
        return Ok(json!({"book": book}));
    }
    app.collab.automation.window = true;
    app.collab.automation.book = book.clone();
    app.collab.send(ClientMsg::GetRules { book: book.clone() });
    Ok(json!({"book": book, "rules": app.collab.automation.rules}))
}

fn describe(r: &Rule) -> String {
    let when = match &r.trigger {
        Trigger::Schedule { every, day, time } if every == "weekly" => {
            format!("Every {} at {time}", DAYS.get(usize::from(*day)).unwrap_or(&"Monday"))
        }
        Trigger::Schedule { time, .. } => format!("Every day at {time}"),
        Trigger::Change { column, equals } => format!("When a cell in column {column} becomes \"{equals}\""),
    };
    let steps: Vec<String> = r
        .actions
        .iter()
        .map(|a| match a {
            Action::Notify { people } => format!("notify {}", people.join(", ")),
            Action::QualityCheck => "quality check".into(),
            Action::Overdue { date_column, status_column, done } => format!("overdue items ({date_column} past, {status_column} not \"{done}\")"),
            Action::Report => "report workbook".into(),
            Action::Stamp { column } => format!("completion time in {column}"),
        })
        .collect();
    format!("{when}: {}", steps.join(", "))
}

pub fn window(app: &mut SheetApp, ctx: &egui::Context) {
    if !app.collab.automation.window {
        return;
    }
    let users: Vec<(String, String)> = app.collab.users.iter().map(|u| (u.user.clone(), u.display.clone())).collect();
    let mut open = true;
    let mut save: Option<Vec<Rule>> = None;
    let mut run: Option<String> = None;
    egui::Window::new("Automation rules").collapsible(false).open(&mut open).default_width(560.0).show(ctx, |ui| {
        let a = &mut app.collab.automation;
        ui.label(
            RichText::new(
                "The office server runs these on its copy of the workbook, so nobody needs to have it open. Only these steps exist: no macros.",
            )
            .weak(),
        );
        ui.add_space(4.0);
        if a.rules.is_empty() {
            ui.label("No rules yet.");
        }
        let mut remove = None;
        let mut toggled = false;
        for (i, r) in a.rules.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                toggled |= ui.checkbox(&mut r.enabled, "").changed();
                ui.vertical(|ui| {
                    ui.strong(&r.name);
                    ui.label(describe(r));
                });
                if ui.small_button("Run now").clicked() {
                    run = Some(r.id.clone());
                }
                if ui.small_button("Delete").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            a.rules.remove(i);
            toggled = true;
        }
        if toggled {
            save = Some(a.rules.clone());
        }
        ui.separator();
        ui.strong("New rule");
        let d = &mut a.draft;
        egui::Grid::new("dex_rule").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
            ui.label("Name");
            ui.add(egui::TextEdit::singleline(&mut d.name).hint_text("Monday progress report"));
            ui.end_row();
            ui.label("When");
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.radio_value(&mut d.schedule, true, "On a schedule");
                    ui.radio_value(&mut d.schedule, false, "When a value changes");
                });
                if d.schedule {
                    ui.horizontal(|ui| {
                        ui.radio_value(&mut d.every, "daily".to_string(), "Every day");
                        ui.radio_value(&mut d.every, "weekly".to_string(), "Every week on");
                        if d.every == "weekly" {
                            for (n, name) in DAYS.iter().enumerate() {
                                ui.radio_value(&mut d.day, n as u8, &name[..3]);
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("at");
                        ui.add(egui::TextEdit::singleline(&mut d.time).desired_width(50.0));
                    });
                } else {
                    ui.horizontal(|ui| {
                        ui.label("a cell in column");
                        ui.add(egui::TextEdit::singleline(&mut d.column).desired_width(40.0));
                        ui.label("becomes");
                        ui.add(egui::TextEdit::singleline(&mut d.equals).desired_width(120.0));
                    });
                }
            });
            ui.end_row();
            ui.label("Then");
            ui.vertical(|ui| {
                ui.checkbox(&mut d.quality, "Run the Quality Check");
                ui.horizontal(|ui| {
                    ui.checkbox(&mut d.overdue, "Find overdue items: date in");
                    ui.add(egui::TextEdit::singleline(&mut d.date_col).desired_width(30.0));
                    ui.label("has passed and status in");
                    ui.add(egui::TextEdit::singleline(&mut d.status_col).desired_width(30.0));
                    ui.label("is not");
                    ui.add(egui::TextEdit::singleline(&mut d.done).desired_width(90.0));
                });
                ui.checkbox(&mut d.report, "Write a report workbook the people below can open");
                if !d.schedule {
                    ui.horizontal(|ui| {
                        ui.label("Write the completion time in column");
                        ui.add(egui::TextEdit::singleline(&mut d.stamp).hint_text("e.g. G").desired_width(40.0));
                    });
                }
            });
            ui.end_row();
            ui.label("Notify");
            egui::ScrollArea::vertical().max_height(110.0).show(ui, |ui| {
                for (u, name) in &users {
                    let mut on = d.people.contains(u);
                    if ui.checkbox(&mut on, name).changed() {
                        d.people.retain(|x| x != u);
                        if on {
                            d.people.push(u.clone());
                        }
                    }
                }
            });
            ui.end_row();
        });
        if ui.add_enabled(!d.name.trim().is_empty(), egui::Button::new("Add rule")).clicked() {
            let trigger = if d.schedule {
                Trigger::Schedule { every: d.every.clone(), day: d.day, time: d.time.trim().to_string() }
            } else {
                Trigger::Change { column: d.column.trim().to_string(), equals: d.equals.trim().to_string() }
            };
            let mut actions = Vec::new();
            if d.quality {
                actions.push(Action::QualityCheck);
            }
            if d.overdue {
                actions.push(Action::Overdue {
                    date_column: d.date_col.trim().into(),
                    status_column: d.status_col.trim().into(),
                    done: d.done.trim().into(),
                });
            }
            if d.report {
                actions.push(Action::Report);
            }
            if !d.schedule && !d.stamp.trim().is_empty() {
                actions.push(Action::Stamp { column: d.stamp.trim().into() });
            }
            if !d.people.is_empty() {
                actions.push(Action::Notify { people: d.people.clone() });
            }
            let mut rules = a.rules.clone();
            rules.push(Rule { name: d.name.trim().into(), enabled: true, trigger, actions, ..Rule::default() });
            *d = Draft::default();
            save = Some(rules);
        }
    });
    if let Some(rules) = save {
        let book = app.collab.automation.book.clone();
        app.collab.send(ClientMsg::SetRules { book, rules });
    }
    if let Some(id) = run {
        let book = app.collab.automation.book.clone();
        app.collab.send(ClientMsg::RunRule { book, id });
    }
    if !open {
        app.collab.automation.window = false;
    }
}
