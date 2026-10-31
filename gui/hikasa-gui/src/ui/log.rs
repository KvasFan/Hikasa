//! Draws the Log panel of the folder views (D-116; FC-100): the shared log book, newest last, following the end
//! unless scrolled up, with a Clear button; colours `[folder] log_panel` and the scheme's warning and error
//! colours.

use egui::{Align, Frame, Layout, Margin, RichText, ScrollArea, Ui};
use hikasa_session::log::{LogBook, LogKind};

use crate::ui::hint::Hint as _;
use crate::ui::theme::Palette;

/// What the panel asked for.
pub enum LogAction {
    /// Clear the book.
    Clear,
    /// Hide the panel (`ShowLog`).
    Hide,
}

/// Draws the panel's content.
pub fn show(ui: &mut Ui, log: &LogBook, pal: &Palette) -> Option<LogAction> {
    let mut out = None;
    Frame::NONE
        .fill(pal.log_bg)
        .inner_margin(Margin::symmetric(8, 4))
        .show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            ui.horizontal(|ui| {
                ui.label(RichText::new("Log").strong().size(12.0).color(pal.fg));
                ui.label(
                    RichText::new(format!("{} entries", log.len()))
                        .size(11.0)
                        .color(pal.muted),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.small_button("×").hint("Hide the log").clicked() {
                        out = Some(LogAction::Hide);
                    }
                    if ui.small_button("Clear").clicked() {
                        out = Some(LogAction::Clear);
                    }
                });
            });
            // Only the rows on screen are formatted and laid out (the book holds up to `display_up_to`).
            let font = egui::FontId::monospace(11.5);
            let row_height = ui.ctx().fonts_mut(|f| f.row_height(&font));
            ScrollArea::both()
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .show_rows(ui, row_height, log.len(), |ui, rows| {
                    for e in log.entries().skip(rows.start).take(rows.len()) {
                        let colour = match e.kind {
                            LogKind::Error => pal.error,
                            LogKind::Warning => pal.warning,
                            LogKind::Debug => pal.muted,
                            LogKind::Summary | LogKind::Detail => pal.f_text,
                        };
                        let line =
                            format!("{}  {}: {}", e.time.strftime("%H:%M:%S"), e.source, e.text);
                        ui.add(
                            egui::Label::new(RichText::new(line).font(font.clone()).color(colour))
                                .extend(),
                        );
                    }
                });
        });
    out
}
