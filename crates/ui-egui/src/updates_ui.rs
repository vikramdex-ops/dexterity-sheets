//! Update checking is disabled in Dexterity Sheets.
//!
//! Dexterity Sheets is an internal LAN tool of Dexterity Design Services Pvt. Ltd. and must never
//! contact the internet. Upstream GridCraft checked GitHub for releases here (through
//! `craft-update`) and offered a self-update; that code is gone. New builds are distributed by the
//! company's IT. The functions below keep the shape the rest of the UI calls into, and do nothing
//! (a manual check only says how updates arrive).

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
    ..craft_update::App::new("dexterity-sheets", "Dexterity Sheets", "Spreadsheets and data", "vikramdex-ops/dexterity-sheets")
};

/// What a manual Check for Updates (the `app.checkForUpdates` command) reports.
pub(crate) const UPDATES_DISABLED: &str =
    "Dexterity Sheets does not check the internet for updates. New versions are distributed by Dexterity Design Services IT.";

/// No state: there is no updater.
#[derive(Default, Clone, Copy)]
pub(crate) struct Runtime;

/// Never starts a check. A manual request is answered with a message; nothing is fetched.
pub(crate) fn start(app: &mut SheetApp, manual: bool) {
    if manual {
        let message = app.ui.language.tr(UPDATES_DISABLED).to_string();
        app.message = Some(("Dexterity Sheets".into(), message));
    }
}

/// Nothing to poll.
pub(crate) fn tick(_app: &mut SheetApp, _ctx: &egui::Context) {}

/// Nothing to show. (`SheetApp::updates_rt` stays in place; reading it here keeps the field used
/// without touching `lib.rs`.)
pub(crate) fn show(app: &mut SheetApp, _ctx: &egui::Context) {
    let Runtime = app.updates_rt;
}
