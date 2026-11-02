//! Draws the dialogs of folder operations (M2 W6d, §1.6.20): the question of a plan with its counts and "Do not ask
//! again", and the Delete dialog's sides and check boxes. The models decide; this only edits their fields.

use egui::{RichText, Ui};

use crate::op_dialogs::{DeleteDialog, PlanConfirmDialog, SideChoice};
use crate::ui::theme::Palette;
use hikasa_core::Side;

/// The confirmation dialog's body.
pub fn confirm(ui: &mut Ui, dialog: &mut PlanConfirmDialog, pal: &Palette) {
    ui.label(RichText::new(dialog.subject()).color(pal.fg).strong());
    ui.add_space(4.0);
    if let Some(question) = dialog.question() {
        ui.label(RichText::new(question).color(pal.fg));
    }
    let remaining = dialog.remaining();
    if remaining > 1 {
        ui.add_space(2.0);
        ui.label(RichText::new(format!("{remaining} questions left")).color(pal.muted));
    }
    if !dialog.counts().is_empty() {
        ui.add_space(6.0);
        for line in dialog.counts() {
            ui.label(RichText::new(line).color(pal.muted));
        }
    }
    if dialog.do_not_ask_key().is_some() {
        ui.add_space(6.0);
        ui.checkbox(&mut dialog.ticked, "Do not ask again");
    }
}

/// The Delete dialog's body.
pub fn delete(ui: &mut Ui, dialog: &mut DeleteDialog, pal: &Palette) {
    ui.label(RichText::new(dialog.path()).color(pal.fg).strong());
    for (side, name) in [(Side::Left, "Left"), (Side::Right, "Right")] {
        let text = dialog.holds(side).unwrap_or("nothing");
        ui.label(RichText::new(format!("{name}: {text}")).color(pal.muted));
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label("Delete on");
        for choice in SideChoice::ALL {
            let on = dialog.choice() == Some(choice);
            let enabled = dialog.enabled(choice);
            if ui
                .add_enabled(enabled, egui::RadioButton::new(on, choice.label()))
                .clicked()
            {
                dialog.set_choice(choice);
            }
        }
    });
    ui.add_space(4.0);
    let mut bin = dialog.use_bin();
    if ui
        .add_enabled(
            !dialog.permanent(),
            egui::Checkbox::new(&mut bin, "Use recycle bin if possible"),
        )
        .changed()
    {
        dialog.set_use_bin(bin);
    }
    if dialog.shows_fallback() {
        let mut fallback = dialog.fallback();
        if ui
            .add_enabled(
                dialog.use_bin(),
                egui::Checkbox::new(
                    &mut fallback,
                    "If the recycle bin fails, delete permanently",
                ),
            )
            .changed()
        {
            dialog.set_fallback(fallback);
        }
    }
}
