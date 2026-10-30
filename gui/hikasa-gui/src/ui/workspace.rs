//! Draws the workspace dialogs (M2 W4b, §1.4.12). The models decide; this only edits their fields.

use egui::{ComboBox, RichText, TextEdit, Ui};

use crate::ui::theme::Palette;
use crate::workspace::{ManageWorkspacesDialog, SaveWorkspaceDialog};

/// Save Workspace As.
pub fn save_as(ui: &mut Ui, dialog: &mut SaveWorkspaceDialog, pal: &Palette) {
    ui.horizontal(|ui| {
        ui.label("Name");
        ui.add(TextEdit::singleline(&mut dialog.name).desired_width(240.0));
        if !dialog.names.is_empty() {
            ComboBox::from_id_salt("hk-workspace-names")
                .selected_text("Existing")
                .width(80.0)
                .show_ui(ui, |ui| {
                    let names = dialog.names.clone();
                    for name in names {
                        if ui.selectable_label(false, &name).clicked() {
                            dialog.name = name;
                        }
                    }
                });
        }
    });
    if let Some(error) = &dialog.error {
        ui.label(RichText::new(error).color(pal.error));
    }
}

/// Manage Workspaces.
pub fn manage(ui: &mut Ui, dialog: &mut ManageWorkspacesDialog, pal: &Palette) {
    if dialog.rows.is_empty() {
        ui.label(RichText::new("No saved workspaces.").color(pal.muted));
        return;
    }
    egui::Grid::new("hk-workspaces")
        .num_columns(4)
        .striped(true)
        .show(ui, |ui| {
            for caption in ["Name", "Windows", "Tabs", "Shortcut"] {
                ui.label(RichText::new(caption).strong());
            }
            ui.end_row();
            let mut pick = None;
            for (i, item) in dialog.rows.iter().enumerate() {
                if ui
                    .selectable_label(dialog.selected == Some(i), &item.name)
                    .clicked()
                {
                    pick = Some(i);
                }
                ui.label(item.windows.to_string());
                ui.label(item.tabs.to_string());
                ui.label(&item.shortcut);
                ui.end_row();
            }
            if let Some(i) = pick {
                dialog.select(i);
            }
        });
    ui.add_space(8.0);
    ui.add_enabled_ui(dialog.selected.is_some(), |ui| {
        ui.horizontal(|ui| {
            ui.label("Name");
            ui.add(TextEdit::singleline(&mut dialog.rename).desired_width(180.0));
            ui.label("Shortcut");
            ui.add(
                TextEdit::singleline(&mut dialog.shortcut)
                    .hint_text("Ctrl+Alt+1")
                    .desired_width(120.0),
            );
        });
    });
    for conflict in dialog.conflicts() {
        ui.label(RichText::new(conflict).color(pal.warning));
    }
    if let Some(error) = &dialog.error {
        ui.label(RichText::new(error).color(pal.error));
    }
}
