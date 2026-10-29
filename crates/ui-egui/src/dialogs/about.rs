use super::Dialog;

/// Help ▸ About GridCraft: the app, its contributors and the AI models that helped. The credits
/// are compiled in (`crate::credits`, docs/contributors.md).
pub(super) fn about(ui: &mut egui::Ui, d: &mut Dialog) {
    ui.horizontal(|ui| {
        for tab in ["About", "Contributors", "Models"] {
            if ui.selectable_label(d.tab == tab, tab).clicked() {
                d.tab = tab.to_string();
            }
        }
    });
    ui.separator();
    match d.tab.as_str() {
        "Contributors" => crate::credits::contributors_ui(ui),
        "Models" => crate::credits::models_ui(ui),
        _ => {
            ui.heading("Dexterity Sheets");
            ui.label(format!("Version {}", gridcraft_collab::APP_VERSION.trim()));
            crate::dex_ui::releases::about_details(ui);
            ui.add_space(4.0);
            ui.label("Internal spreadsheet of Dexterity Design Services Pvt. Ltd.");
            ui.add_space(4.0);
            // Attribution required by the upstream licence (MIT OR Apache-2.0): keep it.
            ui.label("Based on GridCraft by the ArtCraft team (MIT/Apache-2.0).");
        }
    }
}
