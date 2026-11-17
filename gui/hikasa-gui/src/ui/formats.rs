//! Draws the File Formats manager, the format editor, the Grammar Item dialog (M2 W11c; contract §1.11.26,
//! §1.11.27) and the regex helper's ▸ button beside a regex field (FF-028, D-320). The models decide; this only edits
//! their fields and reports the browse dialogs they ask for.

use egui::text::CCursor;
use egui::text_selection::CCursorRange;
use egui::{ComboBox, RichText, TextEdit, Ui};
use hikasa_formats::{ElementInfo, EncodingSetting, FilenameEncoding, FormatKind, TableType};
use hikasa_session::api::MoveTo;

use crate::browse::{BrowseMode, BrowseTarget};
use crate::formats::editor::{ConversionChoice, EditorTab, FormatEditor, summary};
use crate::formats::grammar_item::{Category, GrammarItemDialog};
use crate::formats::{FileFormatManager, NEW_KINDS, RowOrigin, helper_insert};
use crate::ui::theme::Palette;

/// A single-line regex field with the helper's ▸ menu beside it: an item replaces the field's selection and puts the
/// caret where the item says (inside the brackets of *Any of*, *None of*). True when the text changed.
pub fn regex_field(ui: &mut Ui, text: &mut String, id: egui::Id, width: f32) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        let out = TextEdit::singleline(text)
            .id(id)
            .desired_width(width)
            .show(ui);
        changed |= out.response.changed();
        ui.menu_button("▸", |ui| {
            for (k, item) in hikasa_formats::regex_helper::ITEMS.iter().enumerate() {
                if ui
                    .button(format!("{}    {}", item.caption, item.insert))
                    .clicked()
                {
                    let ctx = ui.ctx().clone();
                    let mut state =
                        egui::text_edit::TextEditState::load(&ctx, id).unwrap_or_default();
                    let (a, b) = state
                        .cursor
                        .char_range()
                        .map_or((text.chars().count(), text.chars().count()), |r| {
                            (r.primary.index.0, r.secondary.index.0)
                        });
                    let byte = |c: usize| text.char_indices().nth(c).map_or(text.len(), |(i, _)| i);
                    let (start, end) = (byte(a.min(b)), byte(a.max(b)));
                    let caret = helper_insert(text, start..end, k);
                    let chars = text[..caret].chars().count();
                    state
                        .cursor
                        .set_char_range(Some(CCursorRange::one(CCursor::new(chars))));
                    state.store(&ctx, id);
                    ctx.memory_mut(|m| m.request_focus(id));
                    changed = true;
                    ui.close();
                }
            }
        })
        .response
        .on_hover_text("Regular expression helper");
    });
    changed
}

/// The manager; the browse dialog it asks for.
pub fn manager(
    ui: &mut Ui,
    d: &mut FileFormatManager,
    pal: &Palette,
) -> Option<(BrowseMode, String, BrowseTarget)> {
    let mut browse = None;
    let busy = d.waiting();
    egui::ScrollArea::vertical()
        .id_salt("hk-formats-list")
        .max_height(300.0)
        .show(ui, |ui| {
            egui::Grid::new("hk-formats-grid")
                .num_columns(5)
                .striped(true)
                .show(ui, |ui| {
                    for head in ["On", "Name", "Kind", "Mask", "Origin"] {
                        ui.label(RichText::new(head).strong());
                    }
                    ui.end_row();
                    for k in 0..d.rows.len() {
                        let row = d.rows[k].clone();
                        let mut on = row.enabled;
                        if ui
                            .add_enabled(!row.fixed && !busy, egui::Checkbox::without_text(&mut on))
                            .changed()
                        {
                            d.set_enabled(k, on);
                        }
                        let colour = if !row.enabled {
                            pal.muted
                        } else if !row.shadowed_by.is_empty() {
                            pal.accent
                        } else {
                            pal.fg
                        };
                        let mut name = ui.selectable_label(
                            d.selected == Some(k),
                            RichText::new(&row.name).color(colour),
                        );
                        if let Some(tip) = row.tooltip() {
                            name = name.on_hover_text(tip);
                        }
                        if name.clicked() {
                            d.select(k);
                        }
                        if name.double_clicked() && !row.fixed {
                            d.edit();
                        }
                        ui.label(&row.kind);
                        ui.label(&row.mask);
                        ui.label(row.origin.caption());
                        ui.end_row();
                    }
                });
        });
    let selected = d.selected_row().cloned();
    let editable = selected.as_ref().is_some_and(|r| !r.fixed) && !busy;
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        ui.add_enabled_ui(!busy, |ui| {
            ui.menu_button("New ▸", |ui| {
                for (kind, caption) in NEW_KINDS {
                    if ui.button(caption).clicked() {
                        d.start_new(kind);
                        ui.close();
                    }
                }
            });
        });
        if ui
            .add_enabled(editable, egui::Button::new("Clone"))
            .clicked()
        {
            d.start_clone();
        }
        if ui
            .add_enabled(editable, egui::Button::new("Edit…"))
            .clicked()
        {
            d.edit();
        }
        let deletable = selected
            .as_ref()
            .is_some_and(|r| r.origin == RowOrigin::User)
            && !busy;
        if ui
            .add_enabled(deletable, egui::Button::new("Delete"))
            .clicked()
        {
            d.ask_delete();
        }
        for (caption, to) in [
            ("Move Up", MoveTo::Up),
            ("Move Down", MoveTo::Down),
            ("Move to Top", MoveTo::Top),
            ("Move to Bottom", MoveTo::Bottom),
        ] {
            if ui
                .add_enabled(editable, egui::Button::new(caption))
                .clicked()
            {
                d.move_selected(to);
            }
        }
    });
    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled(!busy, egui::Button::new("Import…"))
            .clicked()
        {
            browse = Some((BrowseMode::File, String::new(), BrowseTarget::FormatImport));
        }
        if ui
            .add_enabled(editable, egui::Button::new("Export…"))
            .clicked()
            && let Some(row) = &selected
        {
            browse = Some((
                BrowseMode::SaveFile {
                    default_name: format!("{}.toml", row.id),
                },
                String::new(),
                BrowseTarget::FormatExport(row.id.clone()),
            ));
        }
        if ui
            .add_enabled(
                d.can_restore() && !busy,
                egui::Button::new("Restore Built-in"),
            )
            .clicked()
        {
            d.ask_restore(false);
        }
        if ui
            .add_enabled(!busy, egui::Button::new("Restore All…"))
            .clicked()
        {
            d.ask_restore(true);
        }
    });
    if let Some(p) = d.prompt.as_mut() {
        ui.add_space(6.0);
        ui.separator();
        let what = match (p.kind, &p.from) {
            (_, Some(from)) => format!("Clone of {from}"),
            (Some(kind), None) => format!("New {kind} format"),
            (None, None) => "New format".to_owned(),
        };
        ui.label(RichText::new(what).strong());
        let mut create = false;
        egui::Grid::new("hk-formats-new").show(ui, |ui| {
            ui.label("Id");
            ui.add(TextEdit::singleline(&mut p.id).hint_text("my-format"));
            ui.end_row();
            ui.label("Name");
            ui.add(TextEdit::singleline(&mut p.name));
            ui.end_row();
        });
        ui.horizontal(|ui| {
            create = ui.button("Create").clicked();
            if ui.button("Cancel").clicked() {
                d.prompt = None;
            }
        });
        if create {
            d.create();
        }
    }
    if let Some(e) = &d.error {
        ui.add_space(4.0);
        ui.colored_label(pal.error, e);
    }
    browse
}

/// A text field of a number or a character.
fn small(ui: &mut Ui, value: &mut String) -> bool {
    ui.add(TextEdit::singleline(value).desired_width(80.0))
        .changed()
}

/// The format editor; the browse dialog it asks for (*Load sample…*).
pub fn editor(
    ui: &mut Ui,
    e: &mut FormatEditor,
    pal: &Palette,
) -> Option<(BrowseMode, String, BrowseTarget)> {
    if !e.ready() {
        ui.label("Reading the format…");
        if let Some(err) = &e.error {
            ui.colored_label(pal.error, err);
        }
        return None;
    }
    let mut changed = false;
    ui.horizontal(|ui| {
        for tab in e.tabs() {
            if ui.selectable_label(e.tab == *tab, tab.caption()).clicked() {
                e.tab = *tab;
            }
        }
    });
    ui.separator();
    let text_format = e.format.kind == FormatKind::Text;
    let height = if text_format { 270.0 } else { 400.0 };
    egui::ScrollArea::vertical()
        .id_salt("hk-format-editor")
        .max_height(height)
        .show(ui, |ui| match e.tab {
            EditorTab::General => changed |= general(ui, e),
            EditorTab::Conversion => changed |= conversion(ui, e),
            EditorTab::Grammar => changed |= grammar(ui, e),
            EditorTab::Misc => changed |= misc(ui, e),
            EditorTab::Type => changed |= table_type(ui, e),
            EditorTab::Regional => changed |= regional(ui, e),
        });
    if changed {
        e.touched();
    }
    let mut browse = None;
    if text_format {
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(RichText::new("Preview").strong());
            if ui.button("Load sample…").clicked() {
                browse = Some((
                    BrowseMode::File,
                    String::new(),
                    BrowseTarget::FormatSample(e.id.clone()),
                ));
            }
            if e.preview.running() {
                ui.spinner();
            }
        });
        ui.columns(2, |cols| {
            if cols[0]
                .add(
                    TextEdit::multiline(&mut e.preview.sample)
                        .font(egui::TextStyle::Monospace)
                        .desired_rows(8)
                        .desired_width(f32::INFINITY),
                )
                .changed()
            {
                e.preview.touch(std::time::Instant::now());
            }
            let names = e.element_names();
            egui::ScrollArea::vertical()
                .id_salt("hk-format-preview")
                .max_height(150.0)
                .show(&mut cols[1], |ui| {
                    if let Some(err) = &e.preview.error {
                        ui.colored_label(pal.error, err);
                    }
                    for line in &e.preview.lines {
                        let mut job = egui::text::LayoutJob::default();
                        for t in &line.tokens {
                            let slot = names
                                .iter()
                                .position(|n| *n == t.element)
                                .map_or(0, |k| k as u32 + 1);
                            job.append(
                                &t.text,
                                0.0,
                                egui::TextFormat {
                                    font_id: egui::FontId::monospace(12.0),
                                    color: pal.syntax_colour(slot),
                                    ..Default::default()
                                },
                            );
                        }
                        ui.label(job);
                    }
                });
        });
    }
    if let Some(err) = &e.error {
        ui.add_space(4.0);
        ui.colored_label(pal.error, err);
    }
    browse
}

fn general(ui: &mut Ui, e: &mut FormatEditor) -> bool {
    let mut changed = false;
    egui::Grid::new("hk-fe-general").num_columns(2).show(ui, |ui| {
        ui.label("Name");
        changed |= ui.add(TextEdit::singleline(&mut e.format.name).desired_width(360.0)).changed();
        ui.end_row();
        ui.label("Id");
        ui.label(e.format.id.as_str());
        ui.end_row();
        ui.label("Mask");
        changed |= ui
            .add(
                TextEdit::singleline(&mut e.format.mask)
                    .desired_width(360.0)
                    .hint_text("*.c;*.h — empty: picked by hand only"),
            )
            .on_hover_text("Name masks separated by `;` (D-10): `*` any characters, `?` one, `[abc]` a set.")
            .changed();
        ui.end_row();
        ui.label("Description");
        changed |= ui
            .add(TextEdit::multiline(&mut e.format.description).desired_width(360.0).desired_rows(3))
            .changed();
        ui.end_row();
    });
    changed
}

fn conversion(ui: &mut Ui, e: &mut FormatEditor) -> bool {
    let mut changed = false;
    let mut choice = e.conversion();
    ui.horizontal(|ui| {
        for (c, caption) in [
            (ConversionChoice::None, "None"),
            (ConversionChoice::Sorted, "Sorted"),
            (ConversionChoice::External, "External"),
            (ConversionChoice::Spreadsheet, "Spreadsheet text"),
        ] {
            ui.radio_value(&mut choice, c, caption);
        }
    });
    if choice != e.conversion() {
        e.set_conversion(choice);
        changed = true;
    }
    let mut timeout = e.typed.timeout.clone();
    let mut disable = None;
    if let Some(x) = e.external_mut() {
        egui::Grid::new("hk-fe-conv").num_columns(2).show(ui, |ui| {
            ui.label("Loading");
            changed |= ui
                .add(
                    TextEdit::singleline(&mut x.load)
                        .desired_width(380.0)
                        .hint_text("convert %s %t"),
                )
                .changed();
            ui.end_row();
            let mut disabled = x.save.is_none();
            ui.label("Saving");
            ui.vertical(|ui| {
                if ui.checkbox(&mut disabled, "Disable editing").changed() {
                    disable = Some(disabled);
                }
                if let Some(save) = x.save.as_mut() {
                    changed |= ui
                        .add(
                            TextEdit::singleline(save)
                                .desired_width(380.0)
                                .hint_text("convert %s %t"),
                        )
                        .changed();
                }
            });
            ui.end_row();
            ui.label("Timeout (s)");
            changed |= small(ui, &mut timeout);
            ui.end_row();
            ui.label("File names");
            ui.horizontal(|ui| {
                changed |= ui
                    .radio_value(
                        &mut x.filename_encoding,
                        FilenameEncoding::Unicode,
                        "Unicode",
                    )
                    .changed();
                changed |= ui
                    .radio_value(&mut x.filename_encoding, FilenameEncoding::Ansi, "ANSI")
                    .changed();
            });
            ui.end_row();
        });
        ui.label("%s the input file, %t the output file, %o the original file name.");
    }
    e.typed.timeout = timeout;
    if let Some(disabled) = disable {
        e.set_disable_editing(disabled);
        changed = true;
    }
    changed
}

fn grammar(ui: &mut Ui, e: &mut FormatEditor) -> bool {
    let mut changed = false;
    ui.label(RichText::new("Grammar items (the first that matches wins)").strong());
    egui::ScrollArea::vertical()
        .id_salt("hk-fe-items")
        .max_height(120.0)
        .show(ui, |ui| {
            for k in 0..e.format.grammar.len() {
                let item = &e.format.grammar[k];
                let text = format!("{}  {}  {}", k + 1, item.element.as_str(), summary(item));
                let r = ui.selectable_label(e.item_selected == Some(k), text);
                if r.clicked() {
                    e.item_selected = Some(k);
                }
                if r.double_clicked() {
                    e.item_dialog(Some(k));
                }
            }
        });
    let some = e.item_selected.is_some();
    ui.horizontal(|ui| {
        if ui.button("Add…").clicked() {
            e.item_dialog(None);
        }
        if ui.add_enabled(some, egui::Button::new("Edit…")).clicked() {
            e.item_dialog(e.item_selected);
        }
        if ui.add_enabled(some, egui::Button::new("Clone")).clicked() {
            e.clone_item();
        }
        if ui.add_enabled(some, egui::Button::new("Delete")).clicked() {
            e.delete_item();
        }
        if ui.add_enabled(some, egui::Button::new("Move Up")).clicked() {
            e.move_item(-1);
        }
        if ui
            .add_enabled(some, egui::Button::new("Move Down"))
            .clicked()
        {
            e.move_item(1);
        }
    });
    ui.add_space(6.0);
    ui.label(RichText::new("Elements").strong());
    egui::Grid::new("hk-fe-elements")
        .num_columns(3)
        .show(ui, |ui| {
            ui.label("Element");
            ui.label("Case sensitive");
            ui.label("Important by default");
            ui.end_row();
            for name in e.element_names() {
                let mut info: ElementInfo = e.element_info(&name);
                ui.label(&name);
                let a = ui.checkbox(&mut info.case_sensitive, "").changed();
                let b = ui.checkbox(&mut info.important, "").changed();
                if a || b {
                    e.set_element_info(&name, info);
                    changed = true;
                }
                ui.end_row();
            }
        });
    ui.add_space(6.0);
    ui.label(RichText::new("Line weights (alignment preference)").strong());
    for k in 0..e.format.line_weights.len() {
        let w = &e.format.line_weights[k];
        let text = format!(
            "{}  priority {}{}",
            w.pattern,
            w.priority,
            if w.regex { "  (regex)" } else { "" }
        );
        if ui
            .selectable_label(e.weight_selected == Some(k), text)
            .clicked()
        {
            e.weight_selected = Some(k);
        }
    }
    ui.horizontal(|ui| {
        if e.weight.regex {
            regex_field(
                ui,
                &mut e.weight.pattern,
                egui::Id::unique("hk-fe-weight"),
                200.0,
            );
        } else {
            ui.add(TextEdit::singleline(&mut e.weight.pattern).desired_width(200.0));
        }
        ui.checkbox(&mut e.weight.regex, "Regex");
        ui.checkbox(&mut e.weight.match_case, "Match case");
        ui.label("Priority (1–4)");
        small(ui, &mut e.typed.priority);
        if ui.button("Add").clicked() {
            e.add_weight();
            changed = true;
        }
        if ui
            .add_enabled(e.weight_selected.is_some(), egui::Button::new("Remove"))
            .clicked()
        {
            e.remove_weight();
            changed = true;
        }
    });
    changed
}

fn misc(ui: &mut Ui, e: &mut FormatEditor) -> bool {
    let mut changed = false;
    egui::Grid::new("hk-fe-misc").num_columns(2).show(ui, |ui| {
        ui.label("Tab size (1–32)");
        changed |= small(ui, &mut e.typed.tab_size);
        ui.end_row();
        ui.label("");
        changed |= ui
            .checkbox(
                &mut e.format.text.tab_as_spaces,
                "Insert spaces instead of tabs",
            )
            .changed();
        ui.end_row();
        ui.label("Word characters");
        changed |= ui
            .add(TextEdit::singleline(&mut e.format.text.word_chars))
            .changed();
        ui.end_row();
        ui.label("");
        changed |= ui
            .checkbox(
                &mut e.format.text.lines_independent,
                "Lines are independent",
            )
            .changed();
        ui.end_row();
        ui.label("");
        changed |= ui
            .checkbox(&mut e.format.text.column_based, "Column-based data")
            .changed();
        ui.end_row();
        ui.label("Characters per line limit");
        changed |= ui
            .add(
                TextEdit::singleline(&mut e.typed.cpl)
                    .desired_width(80.0)
                    .hint_text("none"),
            )
            .changed();
        ui.end_row();
        ui.label("Encoding");
        let (mut fixed, mut label) = match &e.format.encoding {
            EncodingSetting::Fixed { label } => (true, label.clone()),
            EncodingSetting::Auto { fallback } => (false, fallback.clone().unwrap_or_default()),
        };
        let before = (fixed, label.clone());
        ui.horizontal(|ui| {
            ui.radio_value(&mut fixed, false, "Auto, fallback");
            ui.radio_value(&mut fixed, true, "Fixed");
            ui.add(
                TextEdit::singleline(&mut label)
                    .desired_width(120.0)
                    .hint_text("label"),
            );
        });
        if (fixed, label.clone()) != before {
            e.set_encoding(fixed.then(|| label.clone()), (!fixed).then_some(label));
            changed = true;
        }
        ui.end_row();
        ui.label("");
        changed |= ui
            .checkbox(&mut e.format.text.ctrl_z_eof, "Ctrl-Z ends the file")
            .changed();
        ui.end_row();
        ui.label("When saving");
        ui.vertical(|ui| {
            changed |= ui
                .checkbox(
                    &mut e.format.text.save_trim_trailing_ws,
                    "Trim trailing whitespace",
                )
                .changed();
            changed |= ui
                .checkbox(
                    &mut e.format.text.save_leading_spaces_to_tabs,
                    "Leading spaces to tabs",
                )
                .changed();
        });
        ui.end_row();
    });
    changed
}

fn table_type(ui: &mut Ui, e: &mut FormatEditor) -> bool {
    let mut changed = false;
    let table = e.format.table.get_or_insert_with(Default::default);
    egui::Grid::new("hk-fe-type").num_columns(2).show(ui, |ui| {
        ui.label("Type");
        ui.horizontal(|ui| {
            for (k, caption) in [
                (TableType::Detect, "Detect"),
                (TableType::Delimited, "Delimited"),
                (TableType::Fixed, "Fixed width"),
                (TableType::Spreadsheet, "Spreadsheet"),
            ] {
                changed |= ui.radio_value(&mut table.kind, k, caption).changed();
            }
        });
        ui.end_row();
        ui.label("Delimiter");
        changed |= small(ui, &mut e.typed.delimiter);
        ui.end_row();
        ui.label("More delimiters");
        changed |= ui
            .add(TextEdit::singleline(&mut table.delimiters))
            .changed();
        ui.end_row();
        ui.label("Text qualifier");
        changed |= small(ui, &mut e.typed.qualifier);
        ui.end_row();
        ui.label("");
        changed |= ui
            .checkbox(
                &mut table.consecutive_as_one,
                "Consecutive delimiters are one",
            )
            .changed();
        ui.end_row();
        ui.label("");
        changed |= ui
            .checkbox(&mut table.trim_whitespace, "Trim whitespace")
            .changed();
        ui.end_row();
        ui.label("Widths");
        changed |= ui
            .add(TextEdit::singleline(&mut e.typed.widths).hint_text("10;20;8"))
            .changed();
        ui.end_row();
        ui.label("First line");
        ui.horizontal(|ui| {
            for (k, caption) in [
                (hikasa_formats::FirstLineSetting::Detect, "Detect"),
                (hikasa_formats::FirstLineSetting::Names, "Column names"),
                (hikasa_formats::FirstLineSetting::Data, "Data"),
            ] {
                changed |= ui.radio_value(&mut table.first_line, k, caption).changed();
            }
        });
        ui.end_row();
        ui.label("");
        changed |= ui
            .checkbox(&mut table.ctrl_z_eof, "Ctrl-Z ends the data")
            .changed();
        ui.end_row();
    });
    changed
}

fn regional(ui: &mut Ui, e: &mut FormatEditor) -> bool {
    let mut changed = false;
    let table = e.format.table.get_or_insert_with(Default::default);
    let r = &mut table.regional;
    egui::Grid::new("hk-fe-regional")
        .num_columns(2)
        .show(ui, |ui| {
            ui.label("");
            changed |= ui
                .checkbox(&mut r.use_system, "Use the system's settings")
                .changed();
            ui.end_row();
            ui.label("Decimal separator");
            changed |= small(ui, &mut r.decimal);
            ui.end_row();
            ui.label("Thousands separator");
            changed |= small(ui, &mut r.thousands);
            ui.end_row();
            ui.label("Date order");
            ui.horizontal(|ui| {
                for (k, caption) in [
                    (hikasa_formats::DateOrderSetting::Ymd, "YMD"),
                    (hikasa_formats::DateOrderSetting::Dmy, "DMY"),
                    (hikasa_formats::DateOrderSetting::Mdy, "MDY"),
                ] {
                    changed |= ui.radio_value(&mut r.date_order, k, caption).changed();
                }
            });
            ui.end_row();
            ui.label("Date separator");
            changed |= small(ui, &mut r.date_sep);
            ui.end_row();
        });
    changed
}

/// The Grammar Item dialog.
pub fn grammar_item(ui: &mut Ui, d: &mut GrammarItemDialog, pal: &Palette) {
    let f = &mut d.fields;
    egui::Grid::new("hk-gi").num_columns(2).show(ui, |ui| {
        ui.label("Element");
        ui.horizontal(|ui| {
            ui.add(TextEdit::singleline(&mut f.element).desired_width(220.0));
            ComboBox::from_id_salt("hk-gi-names")
                .selected_text("Names")
                .show_ui(ui, |ui| {
                    for n in &d.names {
                        if ui.selectable_label(false, n).clicked() {
                            f.element.clone_from(n);
                        }
                    }
                });
        });
        ui.end_row();
        ui.label("Category");
        ComboBox::from_id_salt("hk-gi-category")
            .selected_text(f.category.caption())
            .show_ui(ui, |ui| {
                for c in Category::ALL {
                    ui.selectable_value(&mut f.category, c, c.caption());
                }
            });
        ui.end_row();
        let field = |ui: &mut Ui, text: &mut String, regex: bool, id: &str| {
            if regex {
                regex_field(ui, text, egui::Id::unique(id), 300.0);
            } else {
                ui.add(TextEdit::singleline(text).desired_width(300.0));
            }
        };
        match f.category {
            Category::Basic => {
                ui.label("Text");
                field(ui, &mut f.text, f.regex, "hk-gi-text");
                ui.end_row();
            }
            Category::Delimited => {
                ui.label("From");
                field(ui, &mut f.from, f.regex, "hk-gi-from");
                ui.end_row();
                ui.label("To");
                field(ui, &mut f.to, f.regex, "hk-gi-to");
                ui.end_row();
                ui.label("");
                ui.checkbox(&mut f.stop_at_eol, "Stop at end of line");
                ui.end_row();
                ui.label("Escape character");
                ui.add(TextEdit::singleline(&mut f.escape).desired_width(40.0));
                ui.end_row();
            }
            Category::List => {
                ui.label("Tokens (one a line)");
                ui.add(
                    TextEdit::multiline(&mut f.tokens)
                        .desired_rows(5)
                        .desired_width(300.0),
                );
                ui.end_row();
            }
            Category::Columns => {
                ui.label("From column");
                ui.add(TextEdit::singleline(&mut f.from_column).desired_width(60.0));
                ui.end_row();
                ui.label("To column");
                ui.horizontal(|ui| {
                    ui.add_enabled(
                        !f.to_end,
                        TextEdit::singleline(&mut f.to_column).desired_width(60.0),
                    );
                    ui.checkbox(&mut f.to_end, "to end of line");
                });
                ui.end_row();
            }
            Category::Lines => {
                ui.label("Heading text");
                field(ui, &mut f.heading, f.regex, "hk-gi-heading");
                ui.end_row();
                ui.label("Next lines");
                ui.add(TextEdit::singleline(&mut f.next_lines).desired_width(60.0));
                ui.end_row();
                ui.label("");
                ui.checkbox(&mut f.or_line_one, "or line 1");
                ui.end_row();
            }
        }
        ui.label("");
        ui.vertical(|ui| {
            if f.category != Category::Columns {
                ui.checkbox(&mut f.match_case, "Match case");
            }
            if f.category.has_regex() {
                ui.checkbox(&mut f.regex, "Regular expression");
            }
            ui.checkbox(&mut f.case_sensitive, "This element is case sensitive");
        });
        ui.end_row();
    });
    if let Some(e) = &d.error {
        ui.colored_label(pal.error, e);
    }
}
