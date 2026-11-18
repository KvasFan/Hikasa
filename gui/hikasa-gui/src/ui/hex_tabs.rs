//! The rule tabs of a Hex Compare session drawn (M2 W13a, contract §1.13.18): **Alignment** (the mode, the block size,
//! skew, window, budget, the unlocked maximum) and **Format** (per side *Auto* or an enabled Hex format). The models
//! are [`crate::hex_tabs`]; the words are core's to parse (the tab shows its message).

use std::any::Any;

use egui::{ComboBox, RichText, Ui};

use crate::hex_tabs::{AlignmentTab, BLOCKS, HexFormatTab, mode_caption};

/// Draws a hex tab (no-op for another tab).
pub fn show(ui: &mut Ui, tab: &mut dyn Any) {
    if let Some(t) = tab.downcast_mut::<AlignmentTab>() {
        alignment(ui, t);
    } else if let Some(t) = tab.downcast_mut::<HexFormatTab>() {
        format(ui, t);
    }
}

fn alignment(ui: &mut Ui, t: &mut AlignmentTab) {
    let f = &mut t.fields;
    ui.label(RichText::new("Alignment").strong());
    for mode in hikasa_hex::AlignMode::ALL {
        let word = mode.as_str();
        if ui.radio(f.mode == word, mode_caption(*mode)).clicked() {
            f.mode = word.to_owned();
        }
    }
    ui.add_space(6.0);
    egui::Grid::new("hk-hex-align")
        .num_columns(2)
        .show(ui, |ui| {
            ui.label("Block size");
            let shown = BLOCKS
                .iter()
                .find(|(w, _)| *w == f.block)
                .map_or(f.block.clone(), |(_, c)| (*c).to_owned());
            ComboBox::from_id_salt("hk-hex-block")
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    for (word, caption) in BLOCKS {
                        if ui.selectable_label(f.block == word, caption).clicked() {
                            f.block = word.to_owned();
                        }
                    }
                });
            ui.end_row();
            for (label, value, hint) in [
                ("Skew", &mut f.skew, "16MiB"),
                ("Window", &mut f.window, "256k"),
                ("Budget (MiB)", &mut f.budget_mib, "1024"),
            ] {
                ui.label(label);
                ui.add(
                    egui::TextEdit::singleline(value)
                        .desired_width(120.0)
                        .hint_text(hint),
                );
                ui.end_row();
            }
            ui.label("Load files up to N MiB into memory without keeping them open");
            ui.add(
                egui::TextEdit::singleline(&mut f.unlocked_max_mib)
                    .desired_width(80.0)
                    .hint_text("16"),
            );
            ui.end_row();
        });
}

fn format(ui: &mut Ui, t: &mut HexFormatTab) {
    let formats = t.formats.clone().unwrap_or_default();
    egui::Grid::new("hk-hex-format")
        .num_columns(2)
        .show(ui, |ui| {
            for (label, value) in [("Left", &mut t.left), ("Right", &mut t.right)] {
                ui.label(label);
                let shown = if value.is_empty() {
                    "Auto".to_owned()
                } else {
                    formats
                        .iter()
                        .find(|(id, name)| id == value || name == value)
                        .map_or(value.clone(), |(_, n)| n.clone())
                };
                ComboBox::from_id_salt(("hk-hex-format", label))
                    .selected_text(shown)
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(value.is_empty(), "Auto").clicked() {
                            value.clear();
                        }
                        for (id, name) in &formats {
                            if ui.selectable_label(value == id, name).clicked() {
                                value.clone_from(id);
                            }
                        }
                    });
                ui.end_row();
            }
        });
}
