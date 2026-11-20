//! Server Manager › Dexterity Connect: the read API's tokens (Nib in Connect asks the server
//! what is assigned to whom and who was mentioned) and the hub the server posts mention and
//! assignment notices to.

use egui::{RichText, Ui, vec2};
use gridcraft_collab::{api, connect};

use crate::{App, DIM, OK, WARN, card, primary, system, when};

#[derive(Default)]
pub struct ConnectPage {
    tokens: Vec<api::TokenInfo>,
    /// What the next token is called.
    token_name: String,
    /// A token just made: shown once, then gone.
    fresh: Option<String>,
    /// A token waiting for "Revoke" to be confirmed: (id, name).
    revoke: Option<(String, String)>,
    hub: String,
    /// A new integration token typed in (empty keeps the saved one).
    bot_token: String,
    has_bot_token: bool,
    fingerprint: String,
    firewall: Option<bool>,
}

impl ConnectPage {
    pub fn load(&mut self, dir: &std::path::Path) {
        self.tokens = api::load_tokens(dir);
        let c = connect::load_config(dir);
        self.hub = c.hub;
        self.has_bot_token = !c.token.is_empty();
        self.fingerprint = c.fingerprint;
        self.bot_token.clear();
    }
}

impl App {
    pub(crate) fn connect_page(&mut self, ui: &mut Ui) {
        ui.heading("Dexterity Connect");
        ui.label(
            RichText::new("Dexterity Connect, the office chat, can show people what is assigned to them and who mentioned them in shared spreadsheets, and the server can send those notices to Connect as they happen.")
                .color(DIM),
        );
        ui.add_space(8.0);
        if self.connect.firewall.is_none() {
            self.connect.firewall = Some(system::firewall_rule_exists(system::FIREWALL_API));
        }
        self.read_api_card(ui);
        self.hub_card(ui);
    }

    fn read_api_card(&mut self, ui: &mut Ui) {
        let dir = self.dir.clone();
        let mut said: Option<Result<String, String>> = None;
        card(ui, |ui| {
            ui.label(RichText::new("Read access for Connect (Nib)").strong());
            ui.label(
                RichText::new(format!(
                    "Connect asks this server on TCP port {} for a person's assignments and mentions and the list of shared spreadsheets, with a token made here. In Connect: Control room › Integrations › Dexterity Sheets, paste the token, then Test. Nothing can be changed through it.",
                    system::API_PORT
                ))
                .color(DIM),
            );
            ui.add_space(4.0);
            match self.connect.firewall {
                Some(true) => ui.label(RichText::new(format!("Firewall: TCP {} open for the office network", system::API_PORT)).color(OK)),
                _ => ui.label(
                    RichText::new(format!(
                        "Firewall: TCP {} is not open: Connect on another PC can't reach it (Server › Firewall › Open it)",
                        system::API_PORT
                    ))
                    .color(WARN),
                ),
            };
            ui.add_space(6.0);
            if let Some(t) = self.connect.fresh.clone() {
                egui::Frame::new().fill(egui::Color32::from_rgb(0xe8, 0xf4, 0xea)).corner_radius(6.0).inner_margin(10.0).show(ui, |ui| {
                    ui.label(RichText::new("The new token, shown only now: copy it into Connect.").strong());
                    ui.horizontal(|ui| {
                        let mut shown = t.clone();
                        ui.add(egui::TextEdit::singleline(&mut shown).desired_width(520.0).font(egui::TextStyle::Monospace));
                        if ui.button("Copy").clicked() {
                            ui.ctx().copy_text(t.clone());
                        }
                        if ui.button("Done").clicked() {
                            self.connect.fresh = None;
                        }
                    });
                    ui.label(
                        RichText::new("The server keeps only a fingerprint of it: if it is lost, make a new one and revoke this one.")
                            .color(DIM)
                            .small(),
                    );
                });
                ui.add_space(6.0);
            }
            if self.connect.tokens.is_empty() {
                ui.label(RichText::new("No tokens: every request is refused.").color(DIM));
            }
            let tokens = self.connect.tokens.clone();
            egui::Grid::new("api_tokens").num_columns(4).striped(true).spacing(vec2(16.0, 6.0)).show(ui, |ui| {
                for t in &tokens {
                    ui.label(RichText::new(&t.name).strong());
                    ui.label(format!("made {}", when(t.created)));
                    ui.label(if t.last_used > 0 { format!("last used {}", when(t.last_used)) } else { "not used yet".into() });
                    if ui.add_enabled(self.admin, egui::Button::new("Revoke…").small()).clicked() {
                        self.connect.revoke = Some((t.id.clone(), t.name.clone()));
                    }
                    ui.end_row();
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.connect.token_name).hint_text("What it is for (Dexterity Connect)").desired_width(260.0));
                if ui.add_enabled(self.admin, primary("Make a token")).on_disabled_hover_text("Needs administrator rights.").clicked() {
                    match api::create_token(&dir, &self.connect.token_name) {
                        Ok((token, _)) => {
                            self.connect.fresh = Some(token);
                            self.connect.token_name.clear();
                            said = Some(Ok("Token made: the server accepts it at once.".into()));
                        }
                        Err(e) => said = Some(Err(e.to_string())),
                    }
                }
            });
        });
        if let Some((id, name)) = self.connect.revoke.clone() {
            let mut open = true;
            let mut close = false;
            egui::Window::new("Revoke this token?")
                .collapsible(false)
                .resizable(false)
                .open(&mut open)
                .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0))
                .show(ui.ctx(), |ui| {
                    ui.label(format!("\"{name}\" stops working at once: Connect can no longer read this server with it."));
                    ui.horizontal(|ui| {
                        if ui.add(primary("Revoke")).clicked() {
                            said = Some(api::revoke_token(&dir, &id).map(|()| format!("\"{name}\" revoked.")).map_err(|e| e.to_string()));
                            close = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                });
            if close || !open {
                self.connect.revoke = None;
            }
        }
        if let Some(r) = said {
            self.say(r);
            self.connect.tokens = api::load_tokens(&dir);
        }
    }

    fn hub_card(&mut self, ui: &mut Ui) {
        let dir = self.dir.clone();
        let mut said: Option<Result<String, String>> = None;
        card(ui, |ui| {
            ui.label(RichText::new("Send notices to Connect").strong());
            ui.label(
                RichText::new("When someone is mentioned or given a cell to look at, they are also told in Connect, by Nib, with a link that opens the spreadsheet at that cell, and everyone's Pulse shows a Sheets card. Make an integration token in Connect (Control room › Integrations) and paste it here. Optional: without it, notices stay in Dexterity Sheets.")
                    .color(DIM),
            );
            ui.add_space(4.0);
            egui::Grid::new("hub").num_columns(2).spacing(vec2(14.0, 6.0)).show(ui, |ui| {
                ui.label("Hub address");
                ui.add(egui::TextEdit::singleline(&mut self.connect.hub).hint_text("https://hub-pc:47900").desired_width(320.0));
                ui.end_row();
                ui.label("Integration token");
                let hint = if self.connect.has_bot_token { "saved (leave empty to keep it)" } else { "paste the token from Connect" };
                ui.add(egui::TextEdit::singleline(&mut self.connect.bot_token).password(true).hint_text(hint).desired_width(320.0));
                ui.end_row();
                ui.label("Certificate fingerprint");
                ui.add(
                    egui::TextEdit::singleline(&mut self.connect.fingerprint)
                        .hint_text("optional: SHA-256 shown on the same Connect page")
                        .desired_width(320.0),
                );
                ui.end_row();
            });
            ui.label(
                RichText::new("The fingerprint is needed when the hub uses its own certificate rather than one Windows trusts: the server then talks only to the hub with that certificate.")
                    .color(DIM)
                    .small(),
            );
            ui.horizontal(|ui| {
                if ui.add_enabled(self.admin, egui::Button::new("Save")).on_disabled_hover_text("Needs administrator rights.").clicked() {
                    said = Some(connect::save_config(&dir, &self.connect.hub, &self.connect.bot_token, &self.connect.fingerprint).map(|()| {
                        if self.connect.hub.trim().is_empty() {
                            "Sending to Connect is off.".to_string()
                        } else {
                            "Saved: notices go to Connect from now on (the server tries again if the hub is away).".to_string()
                        }
                    }));
                }
                if ui.add_enabled(self.admin && !self.connect.hub.is_empty(), egui::Button::new("Turn off")).clicked() {
                    said = Some(connect::save_config(&dir, "", "", "").map(|()| "Sending to Connect is off; the token is forgotten.".to_string()));
                }
            });
        });
        if let Some(r) = said {
            self.say(r);
            self.connect.load(&dir);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn the_page_lists_tokens_and_never_shows_a_saved_secret() {
        let dir = std::env::temp_dir().join(format!("dexsheets-mgr-connect-{}-{}", std::process::id(), gridcraft_collab::now_ms()));
        std::fs::create_dir_all(&dir).expect("dir");
        let (token, id) = api::create_token(&dir, "Dexterity Connect").expect("token");
        connect::save_config(&dir, "https://hub:47900", "tok_secret", "").expect("hub");
        let mut page = ConnectPage::default();
        page.load(&dir);
        assert_eq!(page.tokens.len(), 1);
        assert_eq!(page.tokens[0].id, id);
        assert_ne!(page.tokens[0].hash, token, "only the hash is kept");
        assert_eq!(page.hub, "https://hub:47900");
        assert!(page.has_bot_token && page.bot_token.is_empty(), "the saved token is never put back in the field");
        api::revoke_token(&dir, &id).expect("revoke");
        page.load(&dir);
        assert!(page.tokens.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
