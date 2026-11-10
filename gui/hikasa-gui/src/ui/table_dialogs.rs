//! Draws the Table Compare dialogs (Go To, the report) and the table rule tabs of the Session Settings dialog: the
//! models of `table_dialogs.rs` and `table_tabs.rs` decide, this only edits their fields.

use std::any::Any;

use egui::{ComboBox, RichText, TextEdit, Ui};

use crate::table_dialogs::{DISPLAYS, GoToDialog, LAYOUTS, ReportTo, TableReportDialog};
use crate::table_tabs::{
    ALGORITHMS, COLUMN_MODES, ColumnsTab, ParsingTab, RowsTab, SHEET_MODES, SheetsTab,
};
use crate::ui::theme::Palette;

/// Go To: row (or `row:column`), column, sheet.
pub fn go_to(ui: &mut Ui, d: &mut GoToDialog, pal: &Palette) {
    egui::Grid::new("hk-goto").num_columns(2).show(ui, |ui| {
        ui.label("Row");
        let r = ui.add(
            TextEdit::singleline(&mut d.row)
                .desired_width(120.0)
                .hint_text("12 or 12:B"),
        );
        if !r.has_focus() && d.row.is_empty() {
            r.request_focus();
        }
        ui.end_row();
        ui.label("Column");
        ui.add(
            TextEdit::singleline(&mut d.column)
                .desired_width(200.0)
                .hint_text("a name or letters"),
        );
        ui.end_row();
        if d.sheets.len() > 1 {
            ui.label("Sheet");
            let shown = d.sheets.get(d.sheet).cloned().unwrap_or_default();
            ComboBox::from_id_salt("hk-goto-sheet")
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    for (k, name) in d.sheets.iter().enumerate() {
                        ui.selectable_value(&mut d.sheet, k, name);
                    }
                });
            ui.end_row();
        }
    });
    if let Some(e) = &d.error {
        ui.label(RichText::new(e).color(pal.error));
    }
}

fn pick(ui: &mut Ui, id: &str, value: &mut &'static str, list: &[(&'static str, &'static str)]) {
    let shown = list
        .iter()
        .find(|(w, _)| w == value)
        .map_or(*value, |(_, c)| *c);
    ComboBox::from_id_salt(id)
        .selected_text(shown)
        .show_ui(ui, |ui| {
            for (w, caption) in list {
                ui.selectable_value(value, *w, *caption);
            }
        });
}

/// The report dialog: layout, form, rows, title, target.
pub fn report(ui: &mut Ui, d: &mut TableReportDialog, pal: &Palette) {
    ui.add_enabled_ui(!d.running(), |ui| {
        egui::Grid::new("hk-table-report")
            .num_columns(2)
            .show(ui, |ui| {
                ui.label("Layout");
                pick(ui, "hk-tr-layout", &mut d.layout, &LAYOUTS);
                ui.end_row();
                ui.label("Rows");
                pick(ui, "hk-tr-display", &mut d.display, &DISPLAYS);
                ui.end_row();
                ui.label("Form");
                ui.horizontal(|ui| {
                    let structured = matches!(d.layout, "xml" | "json");
                    ui.add_enabled_ui(!structured, |ui| {
                        ui.radio_value(&mut d.html, true, "HTML");
                        ui.radio_value(&mut d.html, false, "Plain text");
                    });
                    ui.checkbox(&mut d.row_numbers, "Row numbers");
                });
                ui.end_row();
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
                ui.add_enabled(
                    d.to == ReportTo::File,
                    TextEdit::singleline(&mut d.path)
                        .desired_width(320.0)
                        .hint_text("/path/to/report.html"),
                );
                ui.end_row();
            });
    });
    ui.add_space(6.0);
    if d.running() {
        ui.label(RichText::new("Writing the report…").color(pal.muted));
    } else if let Some((ok, text)) = &d.message {
        ui.label(RichText::new(text).color(if *ok { pal.fg } else { pal.error }));
    }
}

/// Draws the table rule tab behind `tab` (nothing for a tab of another kind).
pub fn session_tab(ui: &mut Ui, tab: &mut dyn Any) {
    if let Some(t) = tab.downcast_mut::<SheetsTab>() {
        sheets(ui, t);
    } else if let Some(t) = tab.downcast_mut::<ColumnsTab>() {
        columns(ui, t);
    } else if let Some(t) = tab.downcast_mut::<RowsTab>() {
        rows(ui, t);
    } else if let Some(t) = tab.downcast_mut::<ParsingTab>() {
        parsing(ui, t);
    }
}

fn word_combo(ui: &mut Ui, id: &str, value: &mut String, list: &[(&str, &str)]) {
    let shown = list
        .iter()
        .find(|(w, _)| *w == value.as_str())
        .map_or(value.as_str(), |(_, c)| *c)
        .to_owned();
    ComboBox::from_id_salt(id)
        .selected_text(shown)
        .width(320.0)
        .show_ui(ui, |ui| {
            for (w, caption) in list {
                if ui.selectable_label(value == w, *caption).clicked() {
                    *value = (*w).to_owned();
                }
            }
        });
}

fn sheets(ui: &mut Ui, t: &mut SheetsTab) {
    ui.horizontal(|ui| {
        ui.label("Pair the sheets");
        word_combo(ui, "hk-tt-sheets", &mut t.fields.mode, &SHEET_MODES);
    });
    ui.add_space(4.0);
    ui.label("Custom pairs (left=right, left=, =right; a name or a 1-based number); when given, only these pairs:");
    egui::ScrollArea::vertical()
        .id_salt("hk-tt-pairs")
        .max_height(120.0)
        .show(ui, |ui| {
            for (k, pair) in t.fields.pairs.iter().enumerate() {
                if ui.selectable_label(t.selected == Some(k), pair).clicked() {
                    t.selected = Some(k);
                }
            }
        });
    ui.horizontal(|ui| {
        ui.add(
            TextEdit::singleline(&mut t.new_pair)
                .desired_width(200.0)
                .hint_text("Data=Data 2"),
        );
        if ui.button("Add").clicked() {
            t.add_pair();
        }
        let selected = t.selected.is_some();
        if ui
            .add_enabled(selected, egui::Button::new("Remove"))
            .clicked()
        {
            t.remove_pair();
        }
        if ui
            .add_enabled(selected, egui::Button::new("Move Up"))
            .clicked()
        {
            t.move_pair(-1);
        }
        if ui
            .add_enabled(selected, egui::Button::new("Move Down"))
            .clicked()
        {
            t.move_pair(1);
        }
        if ui.button("Tidy").clicked() {
            t.tidy();
        }
    });
    ui.add_space(4.0);
    ui.checkbox(
        &mut t.fields.formulas,
        "Compare formulas (a differing formula is an important difference)",
    );
}

fn columns(ui: &mut Ui, t: &mut ColumnsTab) {
    ui.horizontal(|ui| {
        ui.label("Pair the columns");
        word_combo(ui, "hk-tt-columns", &mut t.fields.mode, &COLUMN_MODES);
    });
    ui.label("Custom column pairs, one left=right a line:");
    ui.add(
        TextEdit::multiline(&mut t.fields.pairs)
            .desired_rows(2)
            .desired_width(f32::INFINITY),
    );
    ui.add_space(4.0);
    ui.label("Column rules (the first line is every column's default):");
    let header = [
        "Columns",
        "Unimp.",
        "Text",
        "Number",
        "Case",
        "Spaces",
        "Exact",
        "Hide",
        "Tolerance",
        "Date tol.",
    ];
    egui::ScrollArea::vertical()
        .id_salt("hk-tt-rules")
        .max_height(170.0)
        .show(ui, |ui| {
            egui::Grid::new("hk-tt-rules-grid")
                .striped(true)
                .show(ui, |ui| {
                    for h in header {
                        ui.label(RichText::new(h).strong());
                    }
                    ui.end_row();
                    let d = &mut t.fields.default;
                    ui.label("(every column)");
                    for _ in 0..3 {
                        ui.label("");
                    }
                    ui.checkbox(&mut d.ignore_case, "");
                    ui.checkbox(&mut d.ignore_whitespace, "");
                    ui.checkbox(&mut d.exact_numbers, "");
                    ui.label("");
                    ui.add(TextEdit::singleline(&mut d.tolerance).desired_width(60.0));
                    ui.add(TextEdit::singleline(&mut d.date_tolerance).desired_width(60.0));
                    ui.end_row();
                    for (k, r) in t.fields.rows.iter_mut().enumerate() {
                        let resp =
                            ui.add(TextEdit::singleline(&mut r.columns).desired_width(120.0));
                        if resp.gained_focus() || resp.clicked() {
                            t.selected = Some(k);
                        }
                        ui.checkbox(&mut r.unimportant, "");
                        ui.checkbox(&mut r.text, "");
                        ui.checkbox(&mut r.number, "");
                        ui.checkbox(&mut r.ignore_case, "");
                        ui.checkbox(&mut r.ignore_whitespace, "");
                        ui.checkbox(&mut r.exact_numbers, "");
                        ui.checkbox(&mut r.hide, "");
                        ui.add(TextEdit::singleline(&mut r.tolerance).desired_width(60.0));
                        ui.add(TextEdit::singleline(&mut r.date_tolerance).desired_width(60.0));
                        ui.end_row();
                    }
                });
        });
    ui.horizontal(|ui| {
        ui.add(
            TextEdit::singleline(&mut t.new_columns)
                .desired_width(160.0)
                .hint_text("a column, B, or a,b"),
        );
        if ui.button("Add Rule").clicked() {
            t.add_row();
        }
        if ui
            .add_enabled(t.selected.is_some(), egui::Button::new("Remove Rule"))
            .clicked()
        {
            t.remove_row();
        }
    });
}

fn rows(ui: &mut Ui, t: &mut RowsTab) {
    let f = &mut t.fields;
    egui::Grid::new("hk-tt-rows").num_columns(2).show(ui, |ui| {
        ui.label("Key columns");
        ui.add(
            TextEdit::singleline(&mut f.keys)
                .desired_width(260.0)
                .hint_text("id, date (priority order)"),
        );
        ui.end_row();
        ui.label("Alignment");
        word_combo(ui, "hk-tt-algorithm", &mut f.algorithm, &ALGORITHMS);
        ui.end_row();
        ui.label("Skew tolerance");
        ui.add(
            TextEdit::singleline(&mut f.skew)
                .desired_width(80.0)
                .hint_text("300"),
        );
        ui.end_row();
        ui.label("Closeness matching");
        ui.horizontal(|ui| {
            ui.radio_value(&mut f.closeness, None, "Default");
            ui.radio_value(&mut f.closeness, Some(true), "On");
            ui.radio_value(&mut f.closeness, Some(false), "Off");
            ui.label("threshold ‰");
            ui.add(
                TextEdit::singleline(&mut f.threshold)
                    .desired_width(60.0)
                    .hint_text("500"),
            );
        });
        ui.end_row();
    });
    ui.checkbox(&mut f.never_align, "Never align differences");
    ui.checkbox(&mut f.sort_rows, "Sort rows before alignment");
    ui.checkbox(&mut f.ignore_hidden_rows, "Leave hidden rows out");
    ui.checkbox(&mut f.ignore_hidden_columns, "Leave hidden columns out");
}

fn parsing(ui: &mut Ui, t: &mut ParsingTab) {
    let f = &mut t.fields;
    ui.label("Empty fields keep the sides' formats' settings.");
    egui::Grid::new("hk-tt-parse")
        .num_columns(4)
        .show(ui, |ui| {
            let field = |ui: &mut Ui, label: &str, value: &mut String, hint: &str| {
                ui.label(label);
                ui.add(
                    TextEdit::singleline(value)
                        .desired_width(110.0)
                        .hint_text(hint),
                );
            };
            field(ui, "Type", &mut f.kind, "detect");
            field(ui, "First line", &mut f.first_line, "detect");
            ui.end_row();
            field(ui, "Qualifier", &mut f.qualifier, "\"");
            field(ui, "Widths", &mut f.widths, "10;20;8");
            ui.end_row();
            field(ui, "Decimal", &mut f.decimal, ".");
            field(ui, "Thousands", &mut f.thousands, ", or none");
            ui.end_row();
            field(ui, "Date order", &mut f.date_order, "ymd");
            field(ui, "Date separator", &mut f.date_sep, "any");
            ui.end_row();
            field(ui, "Regional", &mut f.regional, "default or system");
            field(ui, "Encoding", &mut f.encoding, "utf-8");
            ui.end_row();
        });
    ui.label("Delimiters, one a line (, ; tab space pipe …):");
    ui.add(
        TextEdit::multiline(&mut f.delimiters)
            .desired_rows(2)
            .desired_width(200.0),
    );
    ui.checkbox(&mut f.consecutive, "Treat consecutive delimiters as one");
    ui.checkbox(&mut f.trim, "Trim the fields");
    ui.checkbox(&mut f.ctrl_z, "Ctrl+Z ends the file");
}
