//! The Hex Compare view's dialogs (M2 W13a, contract §1.13.4, §1.13.6): Go To (the address, its default radix, the
//! side; the core's message under the field) and Layout (bytes per line with presets, group, characters, the
//! addresses and their radix, the panes' layout, the context rows, *Use for new Hex Compare views*). The models are
//! [`crate::hex::nav::HexGoToDialog`] and [`crate::hex::layout::LayoutDialog`]; numbers clamp in the widgets.

use egui::{ComboBox, RichText, Ui};
use hikasa_core::Side;
use hikasa_hex::{CharSet, Radix};

use crate::hex::PaneLayout;
use crate::hex::layout::{BYTES_PRESETS, GROUPS, LayoutDialog};
use crate::hex::nav::HexGoToDialog;
use crate::ui::theme::Palette;

/// Go To: the address (with its recent list), the radix of bare digits, the side.
pub fn go_to(ui: &mut Ui, d: &mut HexGoToDialog, pal: &Palette) {
    egui::Grid::new("hk-hex-goto")
        .num_columns(2)
        .show(ui, |ui| {
            ui.label("Address");
            ui.horizontal(|ui| {
                let r = ui.add(
                    egui::TextEdit::singleline(&mut d.text)
                        .desired_width(180.0)
                        .hint_text("0x1F40, #8000, +16, end"),
                );
                if !r.has_focus() && d.text.is_empty() {
                    r.request_focus();
                }
                if !d.recent.is_empty() {
                    ComboBox::from_id_salt("hk-hex-goto-recent")
                        .selected_text("")
                        .width(24.0)
                        .show_ui(ui, |ui| {
                            for item in d.recent.clone() {
                                if ui.selectable_label(false, &item).clicked() {
                                    d.text = item;
                                }
                            }
                        });
                }
            });
            ui.end_row();
            ui.label("Digits are");
            ui.horizontal(|ui| {
                ui.radio_value(&mut d.radix, Radix::Hex, "Hexadecimal");
                ui.radio_value(&mut d.radix, Radix::Decimal, "Decimal");
            });
            ui.end_row();
            ui.label("Side");
            ui.horizontal(|ui| {
                ui.radio_value(&mut d.side, Side::Left, "Left");
                ui.radio_value(&mut d.side, Side::Right, "Right");
            });
            ui.end_row();
        });
    if let Some(e) = &d.error {
        ui.label(RichText::new(e).color(pal.error));
    }
}

/// Layout: the rows' layout and the panes.
pub fn layout(ui: &mut Ui, d: &mut LayoutDialog) {
    egui::Grid::new("hk-hex-layout")
        .num_columns(2)
        .show(ui, |ui| {
            ui.label("Bytes per line");
            ui.horizontal(|ui| {
                let mut n = i64::from(d.choice.layout.bytes_per_line);
                if ui
                    .add(egui::DragValue::new(&mut n).range(1..=256))
                    .changed()
                {
                    d.set_bytes_per_line(n);
                }
                for preset in BYTES_PRESETS {
                    if ui
                        .selectable_label(
                            d.choice.layout.bytes_per_line == preset,
                            preset.to_string(),
                        )
                        .clicked()
                    {
                        d.set_bytes_per_line(i64::from(preset));
                    }
                }
            });
            ui.end_row();
            ui.label("Group bytes by");
            ui.horizontal(|ui| {
                for g in GROUPS {
                    if ui
                        .radio(d.choice.layout.group == g, g.to_string())
                        .clicked()
                    {
                        d.set_group(g);
                    }
                }
            });
            ui.end_row();
            ui.label("Characters");
            ui.horizontal(|ui| {
                for (set, caption) in [
                    (CharSet::Ascii, "ASCII"),
                    (CharSet::Latin1, "Latin-1"),
                    (CharSet::None, "None"),
                ] {
                    if ui.radio(d.choice.layout.chars == set, caption).clicked() {
                        d.set_chars(set);
                    }
                }
            });
            ui.end_row();
            ui.label("Addresses");
            ui.horizontal(|ui| {
                ui.checkbox(&mut d.choice.addresses, "Show");
                ui.add_enabled_ui(d.choice.addresses, |ui| {
                    for (radix, caption) in
                        [(Radix::Hex, "Hexadecimal"), (Radix::Decimal, "Decimal")]
                    {
                        if ui
                            .radio(d.choice.layout.address == radix, caption)
                            .clicked()
                        {
                            d.set_radix(radix);
                        }
                    }
                });
            });
            ui.end_row();
            ui.label("Panes");
            ui.horizontal(|ui| {
                ui.radio_value(&mut d.choice.panes, PaneLayout::SideBySide, "Side by side");
                ui.radio_value(&mut d.choice.panes, PaneLayout::OverUnder, "Over-under");
            });
            ui.end_row();
            ui.label("Context rows of Differences");
            let mut c = i64::from(d.choice.context);
            if ui.add(egui::DragValue::new(&mut c).range(0..=64)).changed() {
                d.set_context(c);
            }
            ui.end_row();
        });
    ui.checkbox(&mut d.defaults, "Use for new Hex Compare views");
}
