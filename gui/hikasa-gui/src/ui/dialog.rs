//! Draws a window's top dialog (D-112): an egui modal inside its native window (a dimmed backdrop takes the
//! window's input), never an extra viewport. Its title bar moves it, so that what lies under it (the theme preview,
//! D-130) can be seen; the place is remembered per title until the program ends. The model decides; this only draws
//! and reports clicks.

use egui::{Align, Align2, Button, CursorIcon, Label, Layout, RichText, Sense, Ui, Vec2, vec2};

use crate::dialog::{AboutDialog, ButtonRole, DialogModel, Prompt};
use crate::options::OptionsDialog;
use crate::session_dialog::{NameDialog, SaveSessionDialog, SessionSettingsDialog};
use crate::ui::hint::Hint as _;
use crate::ui::theme::Palette;
use crate::ui::{options, session_dialog, wizards, workspace};
use crate::wizards::{ExportWizard, ImportWizard, RestoreWizard};
use crate::workspace::{ManageWorkspacesDialog, SaveWorkspaceDialog};

/// The height of the title bar.
const TITLE_HEIGHT: f32 = 20.0;
/// The height of the buttons.
const BUTTON_HEIGHT: f32 = 26.0;
/// What a dialog takes around its body: the title, the buttons, the margins.
const DIALOG_CHROME: f32 = 110.0;

/// What the user did in the dialog.
pub enum DialogInput {
    /// A button was pressed.
    Button(ButtonRole),
    /// The prompt's "Do not ask again" check box changed.
    DoNotAsk(bool),
    /// The dialog's help (F1 button).
    Help(&'static str),
    /// A field of the dialog changed (the theme preview, D-130).
    Edited,
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
    let moved_id = egui::Id::unique(("hk-dialog-moved", dialog.title()));
    let moved: Vec2 = ctx.data(|d| d.get_temp(moved_id)).unwrap_or_default();
    let area = egui::Modal::default_area(id).anchor(Align2::CENTER_CENTER, moved);
    let shown = egui::Modal::new(id).area(area).show(&ctx, |ui| {
        // Text fields stand out from the dialog's background.
        ui.visuals_mut().extreme_bg_color = pal.chrome;
        let mut height = 0.0;
        match dialog.size() {
            Some((width, body)) => {
                ui.set_min_width(width);
                ui.set_max_width(width);
                // A short window keeps the buttons on screen.
                height = body
                    .min(ctx.content_rect().height() - DIALOG_CHROME)
                    .max(120.0);
            }
            None => {
                ui.set_min_width(360.0);
                ui.set_max_width(560.0);
            }
        }
        let dragged = ui
            .horizontal(|ui| {
                // The bar comes first: the help button, added after it, stays on top and keeps its clicks.
                let bar = egui::Rect::from_min_size(
                    ui.cursor().min,
                    vec2(ui.available_width(), TITLE_HEIGHT),
                );
                let handle = ui
                    .interact(bar, id.with("title"), Sense::drag())
                    .on_hover_cursor(CursorIcon::Grab);
                let title = RichText::new(dialog.title())
                    .strong()
                    .size(15.0)
                    .color(pal.fg);
                ui.add(Label::new(title).selectable(false));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.small_button("?").hint("Help (F1)").clicked() {
                        out.push(DialogInput::Help(dialog.help_topic()));
                    }
                });
                handle.drag_delta()
            })
            .inner;
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
        } else if let Some(d) = dialog.as_any_mut().downcast_mut::<OptionsDialog>() {
            options::show(ui, d, pal);
            out.push(DialogInput::Edited);
        } else if let Some(d) = dialog.as_any_mut().downcast_mut::<ExportWizard>() {
            fixed_body(ui, height, |ui| wizards::export(ui, d, pal));
        } else if let Some(d) = dialog.as_any_mut().downcast_mut::<ImportWizard>() {
            fixed_body(ui, height, |ui| wizards::import(ui, d, pal));
        } else if let Some(d) = dialog.as_any_mut().downcast_mut::<RestoreWizard>() {
            fixed_body(ui, height, |ui| wizards::restore(ui, d, pal));
        }
        ui.add_space(12.0);
        // A row of its own height: a layout that fills what is left would centre the buttons in it.
        let row = vec2(ui.available_width(), BUTTON_HEIGHT);
        ui.allocate_ui_with_layout(row, Layout::right_to_left(Align::Center), |ui| {
            for b in dialog.buttons().iter().rev() {
                let text = if b.default {
                    RichText::new(&b.label).strong()
                } else {
                    RichText::new(&b.label)
                };
                if ui
                    .add_enabled(
                        b.enabled,
                        Button::new(text).min_size(vec2(80.0, BUTTON_HEIGHT)),
                    )
                    .clicked()
                {
                    out.push(DialogInput::Button(b.role));
                }
            }
        });
        dragged
    });
    // The whole dialog stays on screen.
    let room = ((ctx.content_rect().size() - shown.response.rect.size()) / 2.0).max(Vec2::ZERO);
    let place = (moved + shown.inner).clamp(-room, room);
    if place != moved {
        ctx.data_mut(|d| d.insert_temp(moved_id, place));
    }
    out
}

/// A body of exactly `height`: a wizard keeps its size on every step and its buttons stay at the bottom.
fn fixed_body(ui: &mut Ui, height: f32, add: impl FnOnce(&mut Ui)) {
    let size = vec2(ui.available_width(), height);
    ui.allocate_ui_with_layout(size, Layout::top_down(Align::Min), |ui| {
        ui.set_min_height(height);
        add(ui);
    });
}
