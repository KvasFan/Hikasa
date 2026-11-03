//! Draws the script dialogs (M2 W8b, §1.8.14): the status window (path, command, progress, log, end line), the
//! stop-on-error question and the Run Script dialog. The models decide; this only shows them and edits their fields.

use egui::{ComboBox, RichText, ScrollArea, TextEdit, Ui};

use crate::script_dialog::{CONFIRM_MODES, ContinueDialog, RunScriptDialog, ScriptDialog};
use crate::ui::theme::Palette;
use hikasa_session::api::script::LineKind;

/// How long the title flashes after a beep (seconds).
const FLASH_SECONDS: f64 = 1.5;

/// The key of the flash's end time in egui's memory.
fn flash_id() -> egui::Id {
    egui::Id::unique("hk-script-flash")
}

/// A beep: the status window's title flashes for a moment (with the window's request for attention).
pub fn flash(ctx: &egui::Context) {
    let until = ctx.input(|i| i.time) + FLASH_SECONDS;
    ctx.data_mut(|d| d.insert_temp(flash_id(), until));
    ctx.request_repaint_after(std::time::Duration::from_secs_f64(FLASH_SECONDS));
}

/// True while the title flashes.
pub fn flashing(ctx: &egui::Context) -> bool {
    let until: f64 = ctx.data(|d| d.get_temp(flash_id())).unwrap_or(0.0);
    ctx.input(|i| i.time) < until
}

/// The status window's body.
pub fn status(ui: &mut Ui, dialog: &ScriptDialog, pal: &Palette) {
    ui.label(RichText::new(dialog.path()).color(pal.muted));
    ui.add_space(4.0);
    let command = dialog
        .command_line()
        .unwrap_or_else(|| "Starting…".to_owned());
    ui.label(RichText::new(command).color(pal.fg).strong());
    if let Some(progress) = dialog.progress() {
        ui.label(RichText::new(progress).color(pal.muted));
    }
    ui.add_space(6.0);
    let height = (ui.available_height() - 40.0).max(120.0);
    egui::Frame::new()
        .fill(pal.chrome)
        .stroke(egui::Stroke::new(1.0, pal.border))
        .inner_margin(6.0)
        .show(ui, |ui| {
            ScrollArea::vertical()
                .max_height(height)
                .min_scrolled_height(height)
                .stick_to_bottom(true)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for (kind, line) in dialog.lines() {
                        let color = match kind {
                            LineKind::Error => pal.error,
                            LineKind::Warning => pal.warning,
                            LineKind::Detail => pal.muted,
                            LineKind::Run | LineKind::Info => pal.fg,
                        };
                        ui.label(RichText::new(line).monospace().color(color));
                    }
                });
        });
    if let Some(end) = dialog.end_line() {
        ui.add_space(6.0);
        let failed = dialog.end().is_some_and(|e| e.exit_code != 0);
        let color = if failed { pal.error } else { pal.fg };
        ui.label(RichText::new(end).color(color).strong());
    }
}

/// The stop-on-error question.
pub fn ask_continue(ui: &mut Ui, dialog: &ContinueDialog, pal: &Palette) {
    ui.label(RichText::new(dialog.message()).color(pal.fg));
}

/// The Run Script dialog.
pub fn run_script(ui: &mut Ui, dialog: &mut RunScriptDialog, pal: &Palette) {
    egui::Grid::new("hk-run-script")
        .num_columns(2)
        .show(ui, |ui| {
            ui.label("Script");
            ui.add(TextEdit::singleline(&mut dialog.path).desired_width(360.0));
            ui.end_row();
            ui.label("Arguments");
            ui.add(TextEdit::singleline(&mut dialog.args).desired_width(360.0));
            ui.end_row();
            ui.label("Confirm");
            let shown = CONFIRM_MODES
                .iter()
                .find(|(mode, _)| *mode == dialog.confirm)
                .map_or("Ask", |(_, label)| label);
            ComboBox::from_id_salt("hk-run-script-confirm")
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    for (mode, label) in CONFIRM_MODES {
                        ui.selectable_value(&mut dialog.confirm, mode, label);
                    }
                });
            ui.end_row();
        });
    ui.checkbox(&mut dialog.stop_on_error, "Stop on error");
    if !dialog.errors().is_empty() {
        ui.add_space(6.0);
        ScrollArea::vertical().max_height(160.0).show(ui, |ui| {
            for line in dialog.errors() {
                ui.label(RichText::new(line).monospace().color(pal.error));
            }
        });
    }
}
