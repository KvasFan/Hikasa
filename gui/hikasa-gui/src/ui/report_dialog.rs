//! Draws the report dialog (M2 W13c, D-366): the model of `report_dialog.rs` decides, this only edits its fields
//! with the vocabulary's words and captions (D-367). A *Browse…* request is returned to the dialog layer.

use egui::{ComboBox, RichText, TextEdit, Ui};
use hikasa_report::vocabulary::OptionEntry;

use crate::report_dialog::{ReportDialog, ReportTo};
use crate::ui::theme::Palette;

/// A choice of the words of `entry` (a combo box), with *none* when the entry has no default.
fn choice(ui: &mut Ui, id: &str, d: &mut ReportDialog, entry: &'static OptionEntry) {
    let current = entry.words.iter().copied().find(|w| d.options.contains(w));
    let caption = |w: Option<&str>| w.unwrap_or("(the view's)").to_owned();
    ComboBox::from_id_salt(id)
        .selected_text(caption(current))
        .width(200.0)
        .show_ui(ui, |ui| {
            if entry.default.is_none()
                && ui
                    .selectable_label(current.is_none(), "(the view's)")
                    .clicked()
            {
                d.set_option(entry, None);
            }
            for w in entry.words {
                if ui.selectable_label(current == Some(*w), *w).clicked() {
                    d.set_option(entry, Some(w));
                }
            }
        });
}

/// The report dialog's fields; true when *Browse…* was pressed.
pub fn report(ui: &mut Ui, d: &mut ReportDialog, pal: &Palette) -> bool {
    let mut browse = false;
    let vocab = d.vocabulary();
    let busy = d.running() || d.waiting();
    ui.add_enabled_ui(!busy, |ui| {
        egui::Grid::new("hk-report").num_columns(2).show(ui, |ui| {
            ui.label("Layout");
            let shown = d.layout_entry().caption;
            ComboBox::from_id_salt("hk-report-layout")
                .selected_text(shown)
                .width(260.0)
                .show_ui(ui, |ui| {
                    for l in vocab.layouts {
                        if ui.selectable_label(d.layout == l.word, l.caption).clicked() {
                            d.layout = l.word;
                        }
                    }
                });
            ui.end_row();
            let layout = d.layout_entry();
            if layout.display && !vocab.displays.is_empty() {
                ui.label("Show");
                ui.horizontal(|ui| {
                    let shown = vocab
                        .displays
                        .iter()
                        .find(|e| Some(e.word) == d.display)
                        .map_or("", |e| e.caption);
                    ComboBox::from_id_salt("hk-report-display")
                        .selected_text(shown)
                        .width(260.0)
                        .show_ui(ui, |ui| {
                            for e in vocab.displays {
                                if ui
                                    .selectable_label(d.display == Some(e.word), e.caption)
                                    .clicked()
                                {
                                    d.display = Some(e.word);
                                }
                            }
                        });
                    let number = vocab
                        .displays
                        .iter()
                        .any(|e| e.number && Some(e.word) == d.display);
                    if number || (layout.context_lines && !layout.display) {
                        ui.add(
                            egui::DragValue::new(&mut d.context)
                                .range(0..=999)
                                .suffix(" lines"),
                        );
                    }
                });
                ui.end_row();
            } else if layout.context_lines {
                ui.label("Context");
                ui.add(
                    egui::DragValue::new(&mut d.context)
                        .range(0..=999)
                        .suffix(" lines"),
                );
                ui.end_row();
            }
            if let (Some(entry), true) = (vocab.folders, layout.display) {
                ui.label(entry.caption);
                let mut current = d.folders;
                ComboBox::from_id_salt("hk-report-folders")
                    .selected_text(current.unwrap_or(""))
                    .show_ui(ui, |ui| {
                        for w in entry.words {
                            ui.selectable_value(&mut current, Some(*w), *w);
                        }
                    });
                d.folders = current;
                ui.end_row();
            }
            let checks: Vec<&'static OptionEntry> =
                layout.options.iter().filter(|o| o.is_check()).collect();
            for (k, entry) in layout.options.iter().filter(|o| !o.is_check()).enumerate() {
                ui.label(entry.caption);
                choice(ui, &format!("hk-report-choice-{k}"), d, entry);
                ui.end_row();
            }
            if layout.wrap
                && let Some(entry) = vocab.wrap
            {
                ui.label(entry.caption);
                let mut current = d.wrap;
                ComboBox::from_id_salt("hk-report-wrap")
                    .selected_text(current.unwrap_or(""))
                    .show_ui(ui, |ui| {
                        for w in entry.words {
                            ui.selectable_value(&mut current, Some(*w), *w);
                        }
                    });
                d.wrap = current;
                ui.end_row();
            }
            if !checks.is_empty() || vocab.ignore_unimportant || d.can_scope() {
                ui.label("Options");
                ui.horizontal_wrapped(|ui| {
                    for entry in checks {
                        let word = entry.words[0];
                        let mut on = d.options.contains(word);
                        if ui.checkbox(&mut on, entry.caption).changed() {
                            d.set_option(entry, on.then_some(word));
                        }
                    }
                    if vocab.ignore_unimportant {
                        ui.checkbox(&mut d.ignore_unimportant, "Ignore unimportant differences");
                    }
                    ui.add_enabled_ui(d.can_scope(), |ui| {
                        ui.checkbox(&mut d.just_selection, "Just selection");
                    });
                });
                ui.end_row();
            }
            let outputs = d.outputs();
            ui.label("Output");
            ui.add_enabled_ui(!outputs.is_empty(), |ui| {
                ui.horizontal_wrapped(|ui| {
                    for o in &outputs {
                        ui.radio_value(&mut d.output, o.word, o.caption);
                    }
                });
            });
            ui.end_row();
            if outputs.iter().any(|o| o.word == d.output)
                && d.output == hikasa_report::vocabulary::HTML_CUSTOM_WORD
            {
                ui.label("Stylesheet");
                ui.add(
                    TextEdit::singleline(&mut d.css)
                        .desired_width(320.0)
                        .hint_text("style.css"),
                );
                ui.end_row();
            }
            if layout.forms.html {
                ui.label("Print Preview");
                ui.horizontal(|ui| {
                    for (k, entry) in vocab.print.iter().enumerate() {
                        let value = if k == 0 {
                            &mut d.print.0
                        } else {
                            &mut d.print.1
                        };
                        for w in entry.words {
                            ui.radio_value(value, *w, *w);
                        }
                        ui.separator();
                    }
                });
                ui.end_row();
            }
            ui.label("Title");
            ui.add(
                TextEdit::singleline(&mut d.title)
                    .desired_width(320.0)
                    .hint_text("the default"),
            );
            ui.end_row();
            ui.label("Write to");
            ui.horizontal(|ui| {
                ui.radio_value(&mut d.to, ReportTo::File, "File");
                ui.radio_value(&mut d.to, ReportTo::Clipboard, "Clipboard");
            });
            ui.end_row();
            ui.label("File");
            ui.horizontal(|ui| {
                ui.add_enabled(
                    d.to == ReportTo::File,
                    TextEdit::singleline(&mut d.path)
                        .desired_width(320.0)
                        .hint_text("report.html"),
                );
                if ui
                    .add_enabled(d.to == ReportTo::File, egui::Button::new("Browse…"))
                    .clicked()
                {
                    browse = true;
                }
            });
            ui.end_row();
            if d.to == ReportTo::File {
                ui.label("");
                let shown = match d.file() {
                    Ok(path) => format!("Writes {}", path.display()),
                    Err(why) => why,
                };
                ui.label(RichText::new(shown).small().color(pal.muted));
                ui.end_row();
            }
        });
    });
    ui.add_space(6.0);
    if let Some(why) = d.refusal() {
        ui.label(RichText::new(why).color(pal.error));
    }
    if d.running() {
        let text = match d.progress() {
            Some(f) => format!("Writing the report… {:.0} %", f * 100.0),
            None => "Writing the report…".to_owned(),
        };
        ui.label(RichText::new(text).color(pal.muted));
    } else if let Some((ok, text)) = &d.message {
        ui.label(RichText::new(text).color(if *ok { pal.fg } else { pal.error }));
    }
    browse
}
