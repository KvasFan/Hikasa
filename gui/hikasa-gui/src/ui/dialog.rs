//! Draws a window's top dialog (D-112): an egui modal inside its native window (a dimmed backdrop takes the
//! window's input), never an extra viewport. The model decides; this only draws and reports clicks.

use egui::{Align, Button, Layout, RichText, Ui};

use crate::dialog::{AboutDialog, ButtonRole, DialogModel, Prompt};
use crate::session_dialog::{NameDialog, SaveSessionDialog, SessionSettingsDialog};
use crate::ui::hint::Hint as _;
use crate::ui::theme::Palette;
use crate::ui::{session_dialog, workspace};
use crate::workspace::{ManageWorkspacesDialog, SaveWorkspaceDialog};

/// What the user did in the dialog.
pub enum DialogInput {
    /// A button was pressed.
    Button(ButtonRole),
    /// The prompt's "Do not ask again" check box changed.
    DoNotAsk(bool),
    /// The dialog's help (F1 button).
    Help(&'static str),
}

/// Draws `dialog` (the top of a window's stack) as a modal.
pub fn show(
    ui: &mut Ui,
    dialog: &mut dyn DialogModel,
    depth: usize,
    pal: &Palette,
) -> Vec<DialogInput> {
    let mut out = Vec::new();
    let ctx = ui.ctx().clone();
    let id = egui::Id::unique(("hk-dialog", depth));
    egui::Modal::new(id).show(&ctx, |ui| {
        ui.set_min_width(360.0);
        ui.set_max_width(560.0);
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(dialog.title())
                    .strong()
                    .size(15.0)
                    .color(pal.fg),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.small_button("?").hint("Help (F1)").clicked() {
                    out.push(DialogInput::Help(dialog.help_topic()));
                }
            });
        });
        ui.add_space(8.0);
        if let Some(prompt) = dialog.as_any().downcast_ref::<Prompt>() {
            ui.label(RichText::new(&prompt.message).color(pal.fg));
            if prompt.do_not_ask.is_some() {
                ui.add_space(6.0);
                let mut ticked = prompt.ticked;
                if ui.checkbox(&mut ticked, "Do not ask again").changed() {
                    out.push(DialogInput::DoNotAsk(ticked));
                }
            }
        } else if let Some(about) = dialog.as_any().downcast_ref::<AboutDialog>() {
            ui.label(RichText::new(about.text()).color(pal.fg));
        } else if let Some(d) = dialog.as_any_mut().downcast_mut::<SaveSessionDialog>() {
            session_dialog::save_as(ui, d, pal);
        } else if let Some(d) = dialog.as_any_mut().downcast_mut::<NameDialog>() {
            session_dialog::name(ui, d, pal);
        } else if let Some(d) = dialog.as_any_mut().downcast_mut::<SessionSettingsDialog>() {
            session_dialog::settings(ui, d, pal);
        } else if let Some(d) = dialog.as_any_mut().downcast_mut::<SaveWorkspaceDialog>() {
            workspace::save_as(ui, d, pal);
        } else if let Some(d) = dialog.as_any_mut().downcast_mut::<ManageWorkspacesDialog>() {
            workspace::manage(ui, d, pal);
        }
        ui.add_space(12.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            for b in dialog.buttons().iter().rev() {
                let text = if b.default {
                    RichText::new(&b.label).strong()
                } else {
                    RichText::new(&b.label)
                };
                if ui
                    .add_enabled(
                        b.enabled,
                        Button::new(text).min_size(egui::vec2(80.0, 26.0)),
                    )
                    .clicked()
                {
                    out.push(DialogInput::Button(b.role));
                }
            }
        });
    });
    out
}
