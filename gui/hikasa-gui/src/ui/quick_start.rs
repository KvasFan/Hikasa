//! Draws the Quick Compare dialog at startup (M2 W13c, D-368): the model of `quick_start.rs` decides, this only
//! edits its method and its check.

use egui::{RichText, Ui};
use hikasa_session::api::QuickMethod;

use crate::quick_start::QuickStartDialog;
use crate::ui::theme::Palette;

/// The dialog's body.
pub fn quick_start(ui: &mut Ui, d: &mut QuickStartDialog, pal: &Palette) {
    egui::Grid::new("hk-quick-start")
        .num_columns(2)
        .show(ui, |ui| {
            for (k, caption) in ["Left", "Right"].into_iter().enumerate() {
                ui.label(caption);
                ui.vertical(|ui| {
                    ui.label(d.specs[k].to_string_lossy());
                    let facts = match &d.facts[k] {
                        Some(Ok((size, time))) => format!(
                            "{size} bytes{}",
                            time.as_deref()
                                .map(|t| format!(", modified {t}"))
                                .unwrap_or_default()
                        ),
                        Some(Err(e)) => e.clone(),
                        None => String::new(),
                    };
                    ui.label(RichText::new(facts).small().color(pal.muted));
                });
                ui.end_row();
            }
            ui.label("Method");
            ui.horizontal(|ui| {
                let mut method = d.method;
                ui.radio_value(&mut method, QuickMethod::Binary, "Binary");
                ui.radio_value(&mut method, QuickMethod::Rules, "Rules-based");
                if method != d.method {
                    d.set_method(method);
                }
            });
            ui.end_row();
            ui.label("Result");
            if d.running() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Comparing…");
                });
            } else {
                ui.label(d.result_text().unwrap_or_default());
            }
            ui.end_row();
            ui.label("Open View opens");
            ui.label(d.kind_title());
            ui.end_row();
        });
    ui.add_space(6.0);
    let mut show = d.show_at_start;
    if ui
        .checkbox(&mut show, "Show this dialog when starting with two files")
        .changed()
    {
        d.set_show_at_start(show);
    }
}
