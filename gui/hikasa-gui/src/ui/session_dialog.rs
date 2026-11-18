//! Draws the session dialogs (M2 W4b, D-122): Save Session As, the name prompts and Session Settings. The models
//! decide; this only edits their fields.

use egui::{ComboBox, RichText, TextEdit, Ui};

use crate::session_dialog::{
    NameDialog, SCOPES, SaveSessionDialog, SessionSettingsDialog, SpecsTab,
};
use crate::ui::theme::Palette;

fn error_line(ui: &mut Ui, error: Option<&String>, pal: &Palette) {
    if let Some(error) = error {
        ui.add_space(4.0);
        ui.label(RichText::new(error).color(pal.error));
    }
}

/// Save Session As.
pub fn save_as(ui: &mut Ui, dialog: &mut SaveSessionDialog, pal: &Palette) {
    egui::Grid::new("hk-save-session")
        .num_columns(2)
        .show(ui, |ui| {
            ui.label("Name");
            ui.add(TextEdit::singleline(&mut dialog.name).desired_width(300.0));
            ui.end_row();
            ui.label("Folder");
            let shown = if dialog.folder.is_empty() {
                "(top level)".to_owned()
            } else {
                dialog.folder.clone()
            };
            ComboBox::from_id_salt("hk-save-folder")
                .selected_text(shown)
                .width(300.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut dialog.folder, String::new(), "(top level)");
                    for folder in &dialog.folders {
                        ui.selectable_value(&mut dialog.folder, folder.clone(), folder);
                    }
                });
            ui.end_row();
            ui.label("New folder");
            ui.add(TextEdit::singleline(&mut dialog.new_folder).desired_width(300.0));
            ui.end_row();
        });
    if !dialog.names.is_empty() {
        ui.add_space(4.0);
        ComboBox::from_id_salt("hk-save-existing")
            .selected_text("Existing sessions")
            .width(300.0)
            .show_ui(ui, |ui| {
                let names = dialog.names.clone();
                for name in names {
                    if ui.selectable_label(false, &name).clicked() {
                        match name.rsplit_once('/') {
                            Some((folder, leaf)) => {
                                dialog.folder = folder.to_owned();
                                dialog.name = leaf.to_owned();
                            }
                            None => {
                                dialog.folder.clear();
                                dialog.name = name;
                            }
                        }
                    }
                }
            });
    }
    error_line(ui, dialog.error.as_ref(), pal);
}

/// A name prompt.
pub fn name(ui: &mut Ui, dialog: &mut NameDialog, pal: &Palette) {
    ui.add(TextEdit::singleline(&mut dialog.name).desired_width(320.0));
    error_line(ui, dialog.error.as_ref(), pal);
}

fn path_row(
    ui: &mut Ui,
    caption: &str,
    id: &str,
    value: &mut String,
    mru: &[String],
    on: bool,
) -> bool {
    ui.label(caption);
    let mut browse = false;
    ui.horizontal(|ui| {
        ui.add_enabled(
            on,
            TextEdit::singleline(value)
                .desired_width(320.0)
                .font(egui::FontId::monospace(13.0)),
        );
        ComboBox::from_id_salt(id)
            .selected_text("Recent")
            .width(70.0)
            .show_ui(ui, |ui| {
                for item in mru {
                    if ui.selectable_label(false, item).clicked() {
                        value.clone_from(item);
                    }
                }
            });
        browse = ui.add_enabled(on, egui::Button::new("Browse…")).clicked();
    });
    ui.end_row();
    browse
}

fn specs_tab(ui: &mut Ui, specs: &mut SpecsTab, _folder: bool) {
    let on = !specs.specs_disabled;
    if specs.read_only_sides > 0 && on {
        // *Disable editing* per side (FC-003, M2 W9c; text sessions M2 W11c, Text Edit its one file).
        ui.horizontal(|ui| {
            ui.label("Disable editing:");
            if specs.read_only_sides == 1 {
                ui.checkbox(&mut specs.read_only[0], "The file");
            } else {
                ui.checkbox(&mut specs.read_only[0], "Left");
                ui.checkbox(&mut specs.read_only[1], "Right");
            }
        });
        ui.add_space(4.0);
    }
    egui::Grid::new("hk-session-specs")
        .num_columns(2)
        .show(ui, |ui| {
            let (mru_left, mru_right) = (specs.mru_left.clone(), specs.mru_right.clone());
            if path_row(ui, "Left", "hk-mru-left", &mut specs.left, &mru_left, on) {
                specs.browse = Some(0);
            }
            if path_row(
                ui,
                "Right",
                "hk-mru-right",
                &mut specs.right,
                &mru_right,
                on,
            ) {
                specs.browse = Some(1);
            }
            ui.label("Description");
            ui.add(TextEdit::singleline(&mut specs.description).desired_width(320.0));
            ui.end_row();
            if specs.file_type {
                ui.label("Format");
                let shown = specs
                    .formats
                    .iter()
                    .find(|(id, _)| *id == specs.format)
                    .map_or("Detect", |(_, name)| name.as_str())
                    .to_owned();
                ComboBox::from_id_salt("hk-session-format")
                    .selected_text(shown)
                    .width(320.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut specs.format, String::new(), "Detect");
                        for (id, name) in &specs.formats {
                            ui.selectable_value(&mut specs.format, id.clone(), name);
                        }
                    });
                ui.end_row();
                for (caption, value) in [
                    ("Left encoding", &mut specs.left_encoding),
                    ("Right encoding", &mut specs.right_encoding),
                ] {
                    ui.label(caption);
                    ui.add_enabled(
                        on,
                        TextEdit::singleline(value)
                            .hint_text("detect")
                            .desired_width(320.0),
                    );
                    ui.end_row();
                }
            }
        });
}

/// The Session Settings dialog.
pub fn settings(ui: &mut Ui, dialog: &mut SessionSettingsDialog, pal: &Palette) {
    if dialog.tabs.len() > 1 {
        ui.horizontal(|ui| {
            for i in 0..dialog.tabs.len() {
                // A tab that holds an error is marked: OK is off for any of them (review F11).
                let caption = dialog.tab_caption(i);
                let text = if dialog.tabs[i].error().is_some() {
                    RichText::new(caption).color(pal.error)
                } else {
                    RichText::new(caption)
                };
                if ui.selectable_label(dialog.active == i, text).clicked() {
                    dialog.active = i;
                }
            }
        });
        ui.separator();
    }
    let folder = dialog.session.kind.is_folder();
    let active = dialog.active;
    if let Some(tab) = dialog.tabs.get_mut(active) {
        let any = tab.as_any_mut();
        if let Some(specs) = any.downcast_mut::<SpecsTab>() {
            specs_tab(ui, specs, folder);
        } else {
            crate::ui::folder_tabs::show(ui, any, pal);
            crate::ui::table_dialogs::session_tab(ui, any);
            crate::ui::text_tabs::show(ui, any, pal);
            crate::ui::hex_tabs::show(ui, any);
        }
        if let Some(error) = tab.error() {
            ui.add_space(4.0);
            ui.label(RichText::new(error).color(pal.error));
        }
    }
    if !matches!(
        dialog.target,
        crate::session_dialog::EditTarget::Defaults(_)
    ) {
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            for (scope, caption) in SCOPES {
                let available = dialog.scope_available(scope);
                ui.add_enabled_ui(available, |ui| {
                    ui.radio_value(&mut dialog.scope, scope, caption);
                });
            }
        });
    }
    error_line(ui, dialog.error.as_ref(), pal);
}

/// The name prompt of *Add To Presets* (M2 W9c, D-250).
pub fn preset_name(ui: &mut Ui, dialog: &mut crate::filters_edit::PresetNameDialog, pal: &Palette) {
    ui.label(format!("A name for the preset `{}`:", dialog.mask));
    ui.add(TextEdit::singleline(&mut dialog.name).desired_width(320.0));
    error_line(ui, dialog.error.as_ref(), pal);
}
