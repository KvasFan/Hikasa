//! Draws a Folder Sync view's header (M2 W9b, FS-001): the *Operations* preset drop-down, the plan's state ("Planning…",
//! the core message of a refused plan, the counts) and *Run Sync*. The tree below is the folder view's, with the
//! planned action in its centre column (`ui/folder.rs`). The model decides; this only reports what was picked.

use egui::{RichText, Ui};
use hikasa_folder::sync::SyncPreset;

use crate::folder_view::FolderViewModel;
use crate::ui::theme::Palette;

/// What the header asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncHeaderAction {
    /// A preset was picked.
    Preset(SyncPreset),
    /// *Run Sync*.
    Run,
}

/// The caption of a preset in the drop-down.
pub fn preset_caption(preset: SyncPreset) -> &'static str {
    match preset {
        SyncPreset::UpdateLeft => "Update Left",
        SyncPreset::UpdateRight => "Update Right",
        SyncPreset::UpdateBoth => "Update Both",
        SyncPreset::MirrorLeft => "Mirror to Left",
        SyncPreset::MirrorRight => "Mirror to Right",
        SyncPreset::Custom => "Custom",
    }
}

/// The header of a Folder Sync view; `run_enabled` is `Run`'s predicate.
pub fn header(
    ui: &mut Ui,
    m: &FolderViewModel,
    run_enabled: bool,
    pal: &Palette,
) -> Option<SyncHeaderAction> {
    let sync = m.sync()?;
    let mut action = None;
    ui.horizontal(|ui| {
        ui.label("Operations");
        let current = sync.options.preset;
        egui::ComboBox::from_id_salt("hk-sync-preset")
            .selected_text(preset_caption(current))
            .show_ui(ui, |ui| {
                for preset in SyncPreset::ALL {
                    if ui
                        .selectable_label(preset == current, preset_caption(preset))
                        .clicked()
                        && preset != current
                    {
                        action = Some(SyncHeaderAction::Preset(preset));
                    }
                }
            });
        ui.separator();
        if let Some(error) = sync.error() {
            ui.label(RichText::new(error).color(pal.error));
        } else if sync.planning() || m.scanning() {
            ui.label(RichText::new("Planning…").color(pal.muted));
        } else if let Some(preview) = sync.preview() {
            ui.label(
                RichText::new(preview.counts_line().trim_start_matches("# ")).color(pal.muted),
            );
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add_enabled(run_enabled, egui::Button::new("Run Sync"))
                .clicked()
            {
                action = Some(SyncHeaderAction::Run);
            }
        });
    });
    action
}
