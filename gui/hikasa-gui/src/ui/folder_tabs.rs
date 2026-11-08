//! Draws the folder rule tabs of the Session Settings dialog (M2 W9c, D-249): the models of `folder_tabs.rs` decide,
//! this only edits their fields. Core's messages show under the tab (`ui/session_dialog.rs`).

use std::any::Any;

use egui::{ComboBox, RichText, TextEdit, Ui};
use hikasa_core::Side;
use hikasa_folder::sync::{OrphanReading, Situation, SyncAction};

use crate::folder_tabs::{
    ARCHIVES, CASES, ComparisonTab, HandlingTab, METHODS, MiscTab, NameFiltersTab, PRESETS,
    SyncTab, archives_caption, case_caption, method_caption, orphans_caption,
};
use crate::ui::theme::Palette;

/// Draws the rule tab behind `tab` (nothing for a tab of another kind).
pub fn show(ui: &mut Ui, tab: &mut dyn Any, pal: &Palette) {
    if let Some(t) = tab.downcast_mut::<ComparisonTab>() {
        comparison(ui, t);
    } else if let Some(t) = tab.downcast_mut::<NameFiltersTab>() {
        name_filters(ui, t, pal);
    } else if let Some(t) = tab.downcast_mut::<HandlingTab>() {
        handling(ui, t);
    } else if let Some(t) = tab.downcast_mut::<MiscTab>() {
        misc(ui, t, pal);
    } else if let Some(t) = tab.downcast_mut::<SyncTab>() {
        sync(ui, t);
    }
}

fn comparison(ui: &mut Ui, t: &mut ComparisonTab) {
    let f = &mut t.fields;
    ui.checkbox(&mut f.size, "Compare file sizes");
    ui.checkbox(&mut f.timestamps, "Compare timestamps");
    ui.add_enabled_ui(f.timestamps, |ui| {
        ui.indent("hk-ts", |ui| {
            ui.horizontal(|ui| {
                ui.label("Tolerance (seconds)");
                ui.add(TextEdit::singleline(&mut f.tolerance).desired_width(60.0));
            });
            ui.checkbox(&mut f.ignore_dst, "Ignore daylight saving difference");
            ui.checkbox(&mut f.ignore_timezone, "Ignore timezone differences");
            ui.horizontal(|ui| {
                ui.label("Shift the times of");
                let shown = match f.offset_side {
                    None => "no side",
                    Some(Side::Right) => "the right side",
                    Some(_) => "the left side",
                };
                ComboBox::from_id_salt("hk-offset-side")
                    .selected_text(shown)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut f.offset_side, None, "no side");
                        ui.selectable_value(&mut f.offset_side, Some(Side::Left), "the left side");
                        ui.selectable_value(
                            &mut f.offset_side,
                            Some(Side::Right),
                            "the right side",
                        );
                    });
                ui.add_enabled(
                    f.offset_side.is_some(),
                    TextEdit::singleline(&mut f.offset_hours).desired_width(40.0),
                );
                ui.label("hours");
            });
        });
    });
    ui.checkbox(&mut f.filename_case, "Compare file name case");
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label("Content:");
        ComboBox::from_id_salt("hk-content-method")
            .selected_text(method_caption(f.method))
            .show_ui(ui, |ui| {
                for method in METHODS {
                    ui.selectable_value(&mut f.method, method, method_caption(method));
                }
            });
    });
    ui.checkbox(
        &mut f.skip_if_quick_same,
        "Skip if quick tests indicate files are the same",
    );
    ui.checkbox(&mut f.override_quick, "Override quick test results");
}

fn list_field(ui: &mut Ui, caption: &str, value: &mut String) {
    ui.label(caption);
    ui.add(
        TextEdit::multiline(value)
            .desired_rows(3)
            .desired_width(250.0)
            .font(egui::FontId::monospace(12.0)),
    );
}

fn name_filters(ui: &mut Ui, t: &mut NameFiltersTab, pal: &Palette) {
    egui::Grid::new("hk-name-filters")
        .num_columns(2)
        .show(ui, |ui| {
            list_field(ui, "Include files", &mut t.fields.include_files);
            list_field(ui, "Exclude files", &mut t.fields.exclude_files);
            ui.end_row();
            list_field(ui, "Include folders", &mut t.fields.include_folders);
            list_field(ui, "Exclude folders", &mut t.fields.exclude_folders);
            ui.end_row();
        });
    ui.horizontal(|ui| {
        if ui.button("Clear").clicked() {
            t.clear();
        }
        if ui.button("Add To Presets…").clicked() {
            t.ask_add_to_presets();
        }
        let presets = t.presets.clone();
        ComboBox::from_id_salt("hk-filter-presets")
            .selected_text("Presets")
            .show_ui(ui, |ui| {
                for (name, mask) in &presets {
                    if ui
                        .selectable_label(false, name)
                        .on_hover_text(mask)
                        .clicked()
                    {
                        let _ = t.take_preset(mask);
                    }
                }
            });
    });
    match t.text() {
        Ok(text) => {
            ui.label(RichText::new(format!("Filters: {text}")).color(pal.muted));
        }
        Err(e) => {
            ui.label(RichText::new(e).color(pal.error));
        }
    }
}

fn handling(ui: &mut Ui, t: &mut HandlingTab) {
    let f = &mut t.fields;
    ui.horizontal(|ui| {
        ui.label("Name case:");
        ComboBox::from_id_salt("hk-case")
            .selected_text(case_caption(f.case))
            .show_ui(ui, |ui| {
                for case in CASES {
                    ui.selectable_value(&mut f.case, case, case_caption(case));
                }
            });
    });
    ui.checkbox(
        &mut f.align_extensions,
        "Align filenames with different extensions",
    );
    ui.checkbox(
        &mut f.align_normalization,
        "Align filenames with different Unicode normalization forms",
    );
    ui.add_space(4.0);
    ui.checkbox(
        &mut f.auto_scan_subfolders,
        "Scan subfolders as they are found",
    );
    ui.checkbox(&mut f.scan_orphan_folders, "Scan folders on one side only");
    ui.checkbox(
        &mut f.expand_on_load,
        "Open the folders when the comparison loads",
    );
    ui.add_enabled_ui(f.expand_on_load, |ui| {
        ui.checkbox(
            &mut f.expand_differences_only,
            "Only folders with differences",
        );
    });
    ui.checkbox(&mut f.follow_symlinks, "Follow symbolic links");
    if !t.sync {
        ui.checkbox(&mut f.flatten, "Ignore folder structure");
    }
    ui.horizontal(|ui| {
        ui.label("Archives:");
        ComboBox::from_id_salt("hk-archives")
            .selected_text(archives_caption(f.archives))
            .show_ui(ui, |ui| {
                for mode in ARCHIVES {
                    ui.selectable_value(&mut f.archives, mode, archives_caption(mode));
                }
            });
    });
}

fn misc(ui: &mut Ui, t: &mut MiscTab, pal: &Palette) {
    ui.label("Enabled file formats (bold: this session's own state)");
    let rows = t.rows();
    let mut set = None;
    let mut reset = None;
    egui::ScrollArea::vertical()
        .id_salt("hk-misc-formats")
        .max_height(180.0)
        .show(ui, |ui| {
            for row in &rows {
                ui.horizontal(|ui| {
                    let mut on = row.effective();
                    let text = if row.session.is_some() {
                        RichText::new(&row.name).strong()
                    } else {
                        RichText::new(&row.name)
                    };
                    if ui.checkbox(&mut on, text).changed() {
                        set = Some((row.id.clone(), on));
                    }
                    if row.session.is_some() && ui.small_button("Reset").clicked() {
                        reset = Some(row.id.clone());
                    }
                });
            }
        });
    if let Some((id, on)) = set {
        t.set_enabled(&id, on);
    }
    if let Some(id) = reset {
        t.reset(&id);
    }
    ui.add_space(6.0);
    ui.label("Ignored items");
    if t.fields.ignored.is_empty() {
        ui.label(RichText::new("(none)").color(pal.muted));
    }
    let mut remove = None;
    for (k, path) in t.fields.ignored.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.monospace(path);
            if ui.small_button("Remove").clicked() {
                remove = Some(k);
            }
        });
    }
    if let Some(k) = remove {
        t.remove_ignored(k);
    }
}

fn action_caption(action: SyncAction) -> &'static str {
    crate::folder_view::sync::action_text(action)
}

fn situation_caption(situation: Situation) -> &'static str {
    match situation {
        Situation::NewerLeft => "Newer on the left",
        Situation::NewerRight => "Newer on the right",
        Situation::AddedLeft => "Added on the left",
        Situation::AddedRight => "Added on the right",
        Situation::DeletedLeft => "Deleted on the left",
        Situation::DeletedRight => "Deleted on the right",
        Situation::Different => "Different, no time difference",
        Situation::Same => "The same",
    }
}

fn sync(ui: &mut Ui, t: &mut SyncTab) {
    ui.horizontal(|ui| {
        ui.label("Preset:");
        let mut preset = t.fields.options.preset;
        ComboBox::from_id_salt("hk-sync-preset")
            .selected_text(crate::ui::sync_view::preset_caption(preset))
            .show_ui(ui, |ui| {
                for p in PRESETS {
                    ui.selectable_value(&mut preset, p, crate::ui::sync_view::preset_caption(p));
                }
            });
        if preset != t.fields.options.preset {
            t.set_preset(preset);
        }
    });
    let matrix = t.matrix();
    let mut change = None;
    egui::Grid::new("hk-sync-matrix")
        .num_columns(2)
        .striped(true)
        .show(ui, |ui| {
            for situation in Situation::ALL {
                ui.label(situation_caption(situation));
                let current = matrix.get(situation);
                ComboBox::from_id_salt(("hk-cell", situation.as_str()))
                    .selected_text(action_caption(current))
                    .show_ui(ui, |ui| {
                        for action in SyncAction::ALL {
                            // A cell that names a side its item is not on is disabled (FS-002).
                            let allowed = SyncTab::allows(situation, action);
                            if ui
                                .add_enabled(
                                    allowed,
                                    egui::Button::selectable(
                                        current == action,
                                        action_caption(action),
                                    ),
                                )
                                .clicked()
                            {
                                change = Some((situation, action));
                            }
                        }
                    });
                ui.end_row();
            }
        });
    if let Some((situation, action)) = change {
        t.set_cell(situation, action);
    }
    let o = &mut t.fields.options;
    ComboBox::from_id_salt("hk-orphans")
        .selected_text(orphans_caption(o.orphans))
        .show_ui(ui, |ui| {
            for reading in [OrphanReading::Added, OrphanReading::Deleted] {
                ui.selectable_value(&mut o.orphans, reading, orphans_caption(reading));
            }
        });
    ui.checkbox(&mut o.create_empty, "Sync empty folders");
    ui.checkbox(&mut o.within_archives, "Sync within archives");
    ui.checkbox(&mut o.recycle, "Delete to recycle bin if possible");
    ui.checkbox(
        &mut o.recycle_fallback,
        "If the recycle bin fails, delete permanently",
    );
    ui.add_space(4.0);
    ui.label("Overrides");
    let overrides: Vec<(String, SyncAction)> =
        o.overrides.iter().map(|(p, a)| (p.clone(), *a)).collect();
    let mut remove = None;
    for (path, action) in &overrides {
        ui.horizontal(|ui| {
            ui.monospace(path);
            ui.label(action_caption(*action));
            if ui.small_button("Remove").clicked() {
                remove = Some(path.clone());
            }
        });
    }
    if let Some(path) = remove {
        t.remove_override(&path);
    }
}
