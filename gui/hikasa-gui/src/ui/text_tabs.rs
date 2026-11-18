//! Draws the text rule tabs of Session Settings (M2 W11c; contract §1.11.25): Alignment, Importance (the unimportant
//! text list with the regex helper, the element checklist with its origins and *Reset to format*, *Edit Grammar…*) and
//! Format. The models decide; this only edits their fields.

use std::any::Any;

use egui::{ComboBox, RichText, TextEdit, Ui};

use crate::text_tabs::{AlignmentTab, ENGINES, FormatTab, ImportanceTab, Origin};
use crate::ui::theme::Palette;

/// Draws `tab` when it is one of the text tabs.
pub fn show(ui: &mut Ui, tab: &mut dyn Any, pal: &Palette) {
    if let Some(t) = tab.downcast_mut::<AlignmentTab>() {
        alignment(ui, t);
    } else if let Some(t) = tab.downcast_mut::<ImportanceTab>() {
        importance(ui, t, pal);
    } else if let Some(t) = tab.downcast_mut::<FormatTab>() {
        format(ui, t);
    }
}

fn alignment(ui: &mut Ui, t: &mut AlignmentTab) {
    let f = &mut t.fields;
    ui.label(RichText::new("Engine").strong());
    for (engine, caption) in ENGINES {
        let caption = match (engine, &t.auto_engine) {
            (hikasa_text::AlgorithmChoice::Auto, Some(chosen)) => {
                format!("{caption} (chose {chosen} last)")
            }
            _ => caption.to_owned(),
        };
        ui.radio_value(&mut f.algorithm, engine, caption);
    }
    ui.add_space(6.0);
    egui::Grid::new("hk-text-alignment")
        .num_columns(2)
        .show(ui, |ui| {
            ui.label("Skew tolerance (lines)");
            ui.add_enabled(
                f.algorithm == hikasa_text::AlgorithmChoice::Standard,
                TextEdit::singleline(&mut f.skew).desired_width(80.0),
            );
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut f.closeness, "Use closeness matching");
            ui.end_row();
            ui.label("Threshold (‰, 0–1,000)");
            ui.add_enabled(
                f.closeness,
                TextEdit::singleline(&mut f.threshold).desired_width(80.0),
            );
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut f.never_align, "Never align differences");
            ui.end_row();
        });
}

fn importance(ui: &mut Ui, t: &mut ImportanceTab, pal: &Palette) {
    ui.label(RichText::new("Unimportant text").strong());
    egui::ScrollArea::vertical()
        .id_salt("hk-text-unimportant")
        .max_height(80.0)
        .show(ui, |ui| {
            for k in 0..t.fields.unimportant.len() {
                let p = &t.fields.unimportant[k];
                let text = format!(
                    "{}{}{}",
                    p.pattern,
                    if p.regex { "  (regex)" } else { "" },
                    if p.match_case { "" } else { "  (any case)" }
                );
                if ui.selectable_label(t.selected == Some(k), text).clicked() {
                    t.select(k);
                }
            }
        });
    ui.horizontal(|ui| {
        if t.pattern.regex {
            crate::ui::formats::regex_field(
                ui,
                &mut t.pattern.pattern,
                egui::Id::unique("hk-text-pattern"),
                220.0,
            );
        } else {
            ui.add(TextEdit::singleline(&mut t.pattern.pattern).desired_width(220.0));
        }
        ui.checkbox(&mut t.pattern.regex, "Regular expression");
        ui.checkbox(&mut t.pattern.match_case, "Match case");
    });
    let some = t.selected.is_some();
    ui.horizontal(|ui| {
        if ui.button("Add").clicked() {
            t.add_pattern();
        }
        if ui.add_enabled(some, egui::Button::new("Edit")).clicked() {
            t.edit_pattern();
        }
        if ui.add_enabled(some, egui::Button::new("Remove")).clicked() {
            t.remove_pattern();
        }
        if ui.add_enabled(some, egui::Button::new("Move Up")).clicked() {
            t.move_pattern(-1);
        }
        if ui
            .add_enabled(some, egui::Button::new("Move Down"))
            .clicked()
        {
            t.move_pattern(1);
        }
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("Elements").strong());
        if ui
            .add_enabled(t.focus_format.is_some(), egui::Button::new("Edit Grammar…"))
            .clicked()
        {
            t.press_edit_grammar();
        }
    });
    egui::ScrollArea::vertical()
        .id_salt("hk-text-elements")
        .max_height(120.0)
        .show(ui, |ui| {
            egui::Grid::new("hk-text-elements-grid")
                .num_columns(3)
                .show(ui, |ui| {
                    for row in t.rows() {
                        let mut important = row.important;
                        if ui.checkbox(&mut important, &row.name).changed() {
                            t.set_important(&row.name, important);
                        }
                        let colour = if row.origin == Origin::Session {
                            pal.accent
                        } else {
                            pal.muted
                        };
                        ui.label(RichText::new(row.origin.caption()).color(colour));
                        if ui
                            .add_enabled(
                                row.origin != Origin::Format,
                                egui::Button::new("Reset to format"),
                            )
                            .clicked()
                        {
                            t.reset_to_format(&row.name);
                        }
                        ui.end_row();
                    }
                });
        });
    ui.add_space(6.0);
    let f = &mut t.fields;
    ui.horizontal_wrapped(|ui| {
        ui.checkbox(&mut f.leading, "Leading whitespace");
        ui.checkbox(&mut f.embedded, "Embedded whitespace");
        ui.checkbox(&mut f.trailing, "Trailing whitespace");
    });
    ui.horizontal(|ui| {
        ui.checkbox(&mut f.everything_else, "Everything else");
        ui.add_enabled_ui(f.everything_else, |ui| {
            ui.checkbox(&mut f.everything_else_case, "Character case");
        });
    });
    ui.checkbox(&mut f.orphans, "Orphan lines are always important");
    ui.checkbox(&mut f.line_endings, "Compare line endings");
}

fn format(ui: &mut Ui, t: &mut FormatTab) {
    if t.disabled {
        ui.label("The defaults hold no format or encoding: they are each view's.");
        return;
    }
    let formats = t.formats.clone();
    let combo = |ui: &mut Ui, id: &str, value: &mut String| {
        let shown = if value.is_empty() {
            "Detect".to_owned()
        } else {
            value.clone()
        };
        ComboBox::from_id_salt(id)
            .selected_text(shown)
            .width(260.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(value, String::new(), "Detect");
                for name in &formats {
                    ui.selectable_value(value, name.clone(), name);
                }
            });
    };
    let f = &mut t.fields;
    if !t.one_side {
        ui.checkbox(&mut f.same, "Same format for both sides");
    }
    egui::Grid::new("hk-text-format")
        .num_columns(2)
        .show(ui, |ui| {
            if f.same || t.one_side {
                ui.label("Format");
                if t.one_side && !f.same {
                    combo(ui, "hk-text-format-left", &mut f.left);
                } else {
                    combo(ui, "hk-text-format-both", &mut f.format);
                }
                ui.end_row();
            } else {
                ui.label("Left format");
                combo(ui, "hk-text-format-left", &mut f.left);
                ui.end_row();
                ui.label("Right format");
                combo(ui, "hk-text-format-right", &mut f.right);
                ui.end_row();
            }
            let encodings: &mut [(&str, &mut String)] = if t.one_side {
                &mut [("Encoding", &mut f.left_encoding)]
            } else {
                &mut [
                    ("Left encoding", &mut f.left_encoding),
                    ("Right encoding", &mut f.right_encoding),
                ]
            };
            for (caption, value) in encodings.iter_mut() {
                ui.label(*caption);
                ui.add(
                    TextEdit::singleline(*value)
                        .hint_text("detect")
                        .desired_width(260.0),
                );
                ui.end_row();
            }
        });
}
