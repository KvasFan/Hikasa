//! Draws the Options dialog (M2 W4c, §1.4.13): the searchable page tree, the page, its widgets by `ValueKind`. The
//! model decides; this only edits it and reports that it was edited (the theme preview, D-130).

use egui::{ComboBox, DragValue, RichText, ScrollArea, TextEdit, Ui};
use hikasa_session::settings::theme::{FONT_SIZES, SchemeBase, SchemeGroup};
use hikasa_session::settings::{Colour, FormValue, ValueKind, enum_caption};

use crate::options::OptionsDialog;
use crate::options::commands_page::{SHOWN_VIEWS, ViewFlag};
use crate::options::pages::{Body, Page, Widget, page_of};
use crate::options::theme_page::FontSlot;
use crate::ui::theme::Palette;

const TREE_WIDTH: f32 = 210.0;
const PAGE_WIDTH: f32 = 610.0;
const PAGE_HEIGHT: f32 = 470.0;
/// The columns of the check boxes of the commands list.
const TOOLBAR_COLUMN: f32 = 76.0;
const MENU_COLUMN: f32 = 62.0;

/// Draws the dialog.
pub fn show(ui: &mut Ui, d: &mut OptionsDialog, pal: &Palette) {
    // A short window keeps the buttons on screen.
    let height = PAGE_HEIGHT
        .min(ui.ctx().content_rect().height() - 150.0)
        .max(120.0);
    ui.add_enabled_ui(!d.saving(), |ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(TREE_WIDTH + PAGE_WIDTH + 16.0, height),
            egui::Layout::left_to_right(egui::Align::Min),
            |ui| {
                ui.set_height(height);
                ui.vertical(|ui| {
                    ui.set_width(TREE_WIDTH);
                    let search = ui.add(
                        TextEdit::singleline(&mut d.search)
                            .hint_text("Search")
                            .desired_width(TREE_WIDTH),
                    );
                    if search.gained_focus() {
                        d.end_capture();
                    }
                    if search.changed() {
                        d.search_changed();
                    }
                    ScrollArea::vertical()
                        .id_salt("hk-options-tree")
                        .max_height(height - 28.0)
                        .show(ui, |ui| tree(ui, d));
                });
                ui.separator();
                ui.vertical(|ui| {
                    ui.set_width(PAGE_WIDTH);
                    ScrollArea::vertical()
                        .id_salt(("hk-options-page", d.page))
                        .max_height(height)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            if d.visible_pages().is_empty() {
                                ui.label(
                                    RichText::new("No page matches the search.").color(pal.muted),
                                );
                            } else {
                                page(ui, d, pal);
                            }
                        });
                });
            },
        );
    });
    for problem in d.problems() {
        ui.label(RichText::new(problem).color(pal.error));
    }
    if let Some(message) = &d.message {
        ui.label(RichText::new(message).color(pal.error));
    }
    if d.saving() {
        ui.label(RichText::new("Saving…").color(pal.muted));
    }
}

fn tree(ui: &mut Ui, d: &mut OptionsDialog) {
    let pages = d.visible_pages();
    if pages.is_empty() {
        ui.label("No match.");
    }
    for p in pages {
        ui.horizontal(|ui| {
            if p.parent.is_some() {
                ui.add_space(14.0);
            }
            if ui.selectable_label(d.page == p.id, p.title).clicked() {
                d.show_page(p.id);
            }
        });
    }
}

fn page(ui: &mut Ui, d: &mut OptionsDialog, pal: &Palette) {
    let Some(page) = page_of(d.page) else {
        return;
    };
    ui.label(RichText::new(page.title).strong().size(14.0));
    if let Some(note) = page.note {
        ui.label(RichText::new(note).color(pal.muted));
    }
    for note in d.unreadable_notes() {
        ui.label(RichText::new(note).color(pal.warning));
    }
    ui.add_space(6.0);
    fields(ui, d, page);
    match page.body {
        Body::Fields => {}
        Body::Appearance => appearance(ui, d, pal),
        Body::Scheme(group) => scheme_page(ui, d, group, pal),
        Body::Fonts => fonts(ui, d),
        Body::ArchiveTypes => archive_types(ui, d),
        Body::Commands => commands(ui, d, pal),
        Body::OpenWith => open_with(ui, d, pal),
        Body::NameFilters => name_filters(ui, d),
    }
}

fn fields(ui: &mut Ui, d: &mut OptionsDialog, page: &Page) {
    for def in page.fields {
        let Some(field) = d.field(def.path).cloned() else {
            continue;
        };
        let mut new = None;
        ui.horizontal(|ui| match (&field.value, field.kind) {
            (FormValue::Bool(on), _) => {
                let mut on = *on;
                if ui.checkbox(&mut on, field.label).changed() {
                    new = Some(FormValue::Bool(on));
                }
            }
            (FormValue::Int(n), ValueKind::Int { min, max }) => {
                let mut n = *n;
                ui.label(field.label);
                if ui.add(DragValue::new(&mut n).range(min..=max)).changed() {
                    new = Some(FormValue::Int(n));
                }
            }
            (FormValue::Text(text), ValueKind::Enum(values)) => {
                ui.label(field.label);
                ComboBox::from_id_salt(field.path)
                    .selected_text(enum_caption(text).unwrap_or(text))
                    .show_ui(ui, |ui| {
                        for value in values {
                            let caption = enum_caption(value).unwrap_or(value);
                            if ui.selectable_label(text == value, caption).clicked() {
                                new = Some(FormValue::Text((*value).to_owned()));
                            }
                        }
                    });
            }
            (FormValue::Text(text), _) if field.widget == Widget::Workspace => {
                ui.label(field.label);
                let shown = if text.is_empty() { "(none)" } else { text };
                ComboBox::from_id_salt(field.path)
                    .selected_text(shown)
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(text.is_empty(), "(none)").clicked() {
                            new = Some(FormValue::Text(String::new()));
                        }
                        for name in &d.workspaces {
                            if ui.selectable_label(text == name, name).clicked() {
                                new = Some(FormValue::Text(name.clone()));
                            }
                        }
                    });
            }
            (FormValue::Text(text), _) => {
                let mut text = text.clone();
                ui.label(field.label);
                let response = ui.add(TextEdit::singleline(&mut text).desired_width(260.0));
                if response.changed() {
                    new = Some(FormValue::Text(text));
                }
                // A path field takes a file dropped on it (Browse comes with W9).
                let dropped =
                    ui.input(|i| i.raw.dropped_files.first().map(|f| f.path().to_path_buf()));
                if field.kind == ValueKind::Path
                    && (response.hovered() || response.has_focus())
                    && let Some(path) = dropped
                {
                    new = Some(FormValue::Text(path.display().to_string()));
                }
            }
            _ => {
                ui.label(field.label);
            }
        });
        if let Some(value) = new {
            d.set_field(def.path, value);
        }
    }
}

fn appearance(ui: &mut Ui, d: &mut OptionsDialog, pal: &Palette) {
    ui.add_space(6.0);
    let names = d.scheme_names();
    let choose = |ui: &mut Ui, label: &str, current: &str| -> Option<String> {
        let mut pick = None;
        ui.horizontal(|ui| {
            ui.label(label);
            ComboBox::from_id_salt(label)
                .selected_text(current)
                .show_ui(ui, |ui| {
                    for name in &names {
                        if ui.selectable_label(name == current, name).clicked() {
                            pick = Some(name.clone());
                        }
                    }
                });
        });
        pick
    };
    if let Some(name) = choose(ui, "Light mode scheme", &d.theme.light_scheme.clone()) {
        d.set_light_scheme(&name);
    }
    if let Some(name) = choose(ui, "Dark mode scheme", &d.theme.dark_scheme.clone()) {
        d.set_dark_scheme(&name);
    }
    scheme_header(ui, d, pal);
}

/// The scheme selector and its buttons.
fn scheme_header(ui: &mut Ui, d: &mut OptionsDialog, pal: &Palette) {
    ui.separator();
    ui.horizontal(|ui| {
        ui.label("Edit scheme");
        ComboBox::from_id_salt("hk-edit-scheme")
            .selected_text(d.theme_page.scheme.clone())
            .show_ui(ui, |ui| {
                for name in d.scheme_names() {
                    if ui
                        .selectable_label(d.theme_page.scheme == name, &name)
                        .clicked()
                    {
                        d.select_scheme(&name);
                    }
                }
            });
        if ui.button("Reset All").clicked() {
            d.reset_all();
        }
        if ui
            .add_enabled(!d.edits_builtin(), egui::Button::new("Delete"))
            .clicked()
        {
            d.delete_scheme();
        }
    });
    ui.horizontal(|ui| {
        ui.label("New name");
        ui.add(TextEdit::singleline(&mut d.theme_page.new_name).desired_width(140.0));
        ComboBox::from_id_salt("hk-new-base")
            .selected_text(d.theme_page.new_base.as_str())
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut d.theme_page.new_base, SchemeBase::Light, "light");
                ui.selectable_value(&mut d.theme_page.new_base, SchemeBase::Dark, "dark");
            });
        if ui.button("New").clicked() {
            d.new_scheme();
        }
        if ui.button("Clone").clicked() {
            d.clone_scheme();
        }
    });
    if let Some(error) = &d.theme_page.error {
        ui.label(RichText::new(error).color(pal.error));
    }
}

/// The stored colour, unmultiplied: it must come back as it was stored (`Color32` is premultiplied).
pub fn colour_rgba(value: Colour) -> [u8; 4] {
    let [r, g, b] = value.rgb;
    [r, g, b, value.alpha.unwrap_or(255)]
}

/// A colour button: `Some` when it changed.
fn colour_button(ui: &mut Ui, value: Colour) -> Option<Colour> {
    let mut rgba = colour_rgba(value);
    if ui.color_edit_button_srgba_unmultiplied(&mut rgba).changed() {
        Some(Colour {
            rgb: [rgba[0], rgba[1], rgba[2]],
            alpha: (rgba[3] != 255 || value.alpha.is_some()).then_some(rgba[3]),
        })
    } else {
        None
    }
}

fn scheme_page(ui: &mut Ui, d: &mut OptionsDialog, group: SchemeGroup, pal: &Palette) {
    scheme_header(ui, d, pal);
    let colours = d.theme_page.resolved.colours(group);
    let flags = d.theme_page.resolved.flags(group);
    egui::Grid::new(("hk-scheme", d.page))
        .num_columns(3)
        .show(ui, |ui| {
            for (name, value) in colours {
                ui.label(name.replace('_', " "));
                if let Some(new) = colour_button(ui, value) {
                    d.set_element_colour(group, name, new);
                }
                if ui
                    .add_enabled(
                        d.element_set(group, name),
                        egui::Button::new("Reset").small(),
                    )
                    .clicked()
                {
                    d.reset_element(group, name);
                }
                ui.end_row();
            }
            for (name, value) in flags {
                let mut on = value;
                if ui.checkbox(&mut on, name.replace('_', " ")).changed() {
                    d.set_element_flag(group, name, on);
                }
                ui.label("");
                if ui
                    .add_enabled(
                        d.element_set(group, name),
                        egui::Button::new("Reset").small(),
                    )
                    .clicked()
                {
                    d.reset_element(group, name);
                }
                ui.end_row();
            }
        });
    if group == SchemeGroup::File {
        syntax(ui, d);
    }
    for warning in d.contrast_warnings() {
        ui.label(RichText::new(warning).color(pal.warning));
    }
}

fn syntax(ui: &mut Ui, d: &mut OptionsDialog) {
    ui.separator();
    ui.label(RichText::new("Syntax").strong());
    let elements: Vec<_> = d
        .theme_page
        .resolved
        .file
        .syntax
        .iter()
        .map(|(k, v)| (k.clone(), *v))
        .collect();
    egui::Grid::new("hk-syntax").num_columns(5).show(ui, |ui| {
        for (name, style) in elements {
            let mut style2 = style;
            ui.label(&name);
            if let Some(new) = colour_button(ui, style.color) {
                style2.color = new;
            }
            ui.checkbox(&mut style2.bold, "Bold");
            ui.checkbox(&mut style2.italic, "Italic");
            if style2 != style {
                d.set_syntax(&name, style2);
            }
            if ui
                .add_enabled(d.syntax_set(&name), egui::Button::new("Reset").small())
                .clicked()
            {
                d.reset_syntax(&name);
            }
            ui.end_row();
        }
    });
    ui.horizontal(|ui| {
        ui.label("Fallback cycle");
        let cycle = d.theme_page.resolved.file.fallback.clone();
        for (slot, value) in cycle.into_iter().enumerate() {
            if let Some(new) = colour_button(ui, value) {
                d.set_fallback(slot, new);
            }
        }
        if ui.small_button("Reset").clicked() {
            d.reset_fallback();
        }
    });
}

fn fonts(ui: &mut Ui, d: &mut OptionsDialog) {
    let f = d.theme.fonts.clone();
    let rows = [
        (
            "Editor font",
            FontSlot::Editor,
            f.editor.family,
            f.editor.size,
        ),
        (
            "Listing font",
            FontSlot::Listing,
            f.listing.family,
            f.listing.size,
        ),
        ("Hex font", FontSlot::Hex, f.hex.family, f.hex.size),
        (
            "Folder font",
            FontSlot::Folder,
            f.folder.family,
            f.folder.size,
        ),
    ];
    egui::Grid::new("hk-fonts").num_columns(3).show(ui, |ui| {
        for (label, slot, family, size) in rows {
            let (mut family, mut size) = (family, size);
            ui.label(label);
            if ui
                .add(
                    TextEdit::singleline(&mut family)
                        .hint_text("built-in")
                        .desired_width(220.0),
                )
                .changed()
            {
                d.set_font_family(slot, &family);
            }
            if ui
                .add(
                    DragValue::new(&mut size)
                        .range(FONT_SIZES.0..=FONT_SIZES.1)
                        .speed(0.25),
                )
                .changed()
            {
                d.set_font_size(slot, size);
            }
            ui.end_row();
        }
    });
    let mut system = f.folder.use_system;
    if ui
        .checkbox(&mut system, "Folder views use the system font")
        .changed()
    {
        d.set_folder_system_font(system);
    }
}

fn archive_types(ui: &mut Ui, d: &mut OptionsDialog) {
    use hikasa_session::settings::archives::ArchiveType;
    egui::Grid::new("hk-archives")
        .num_columns(2)
        .show(ui, |ui| {
            for kind in ArchiveType::ORDER {
                let mut mask = d.archives.mask(kind).to_owned();
                ui.label(kind.as_str());
                if ui
                    .add(TextEdit::singleline(&mut mask).desired_width(420.0))
                    .changed()
                {
                    d.set_archive_mask(kind, &mask);
                }
                ui.end_row();
            }
        });
}

fn commands(ui: &mut Ui, d: &mut OptionsDialog, pal: &Palette) {
    use hikasa_session::settings::commands::DisplayFilterMode as Mode;
    ui.horizontal(|ui| {
        ui.label("View");
        let current = SHOWN_VIEWS
            .iter()
            .find(|(v, _)| *v == d.commands_page.view)
            .map_or("", |(_, c)| *c);
        ComboBox::from_id_salt("hk-commands-view")
            .selected_text(current)
            .show_ui(ui, |ui| {
                for (view, caption) in SHOWN_VIEWS {
                    if ui
                        .selectable_label(d.commands_page.view == view, caption)
                        .clicked()
                    {
                        d.select_view(view);
                    }
                }
            });
        let mut search = d.commands_page.search.clone();
        let field = ui.add(
            TextEdit::singleline(&mut search)
                .hint_text("Search")
                .desired_width(220.0),
        );
        if field.gained_focus() {
            d.end_capture();
        }
        if field.changed() {
            d.set_command_search(&search);
        }
        if ui.button("Reset View").clicked() {
            d.reset_commands();
        }
    });
    let custom = d.view_custom();
    ui.horizontal(|ui| {
        ui.label("Display filters");
        for (mode, caption) in [
            (Mode::Dropdown, "Drop-down"),
            (Mode::Favorites, "Favorites"),
            (Mode::Toggles, "Toggles"),
        ] {
            if ui
                .radio(custom.display_filter_mode == mode, caption)
                .clicked()
            {
                d.set_filter_mode(mode);
            }
        }
    });
    ui.horizontal(|ui| {
        for (flag, on, caption) in [
            (
                ViewFlag::LargeButtons,
                custom.large_buttons,
                "Large buttons",
            ),
            (
                ViewFlag::ShowAllInMenu,
                custom.show_all_in_menu,
                "Show all in menu",
            ),
            (
                ViewFlag::ShowAllOnToolbar,
                custom.show_all_on_toolbar,
                "Show all on toolbar",
            ),
        ] {
            let mut on = on;
            if ui.checkbox(&mut on, caption).changed() {
                d.set_view_flag(flag, on);
            }
        }
    });
    ui.separator();
    let rows = d.commands_page.rows.clone();
    ScrollArea::vertical()
        .id_salt("hk-commands-list")
        .max_height(250.0)
        .auto_shrink([false, false])
        .show_rows(ui, 22.0, rows.len(), |ui, range| {
            for row in &rows[range] {
                ui.horizontal(|ui| {
                    let selected = d.commands_page.selected.as_deref() == Some(row.id.as_str());
                    let shortcuts = row
                        .shortcuts
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ");
                    let mut text = RichText::new(format!("{}   {shortcuts}", row.caption));
                    if !row.conflicts.is_empty() {
                        text = text.color(pal.warning);
                    }
                    if ui.selectable_label(selected, text).clicked() {
                        d.select_command(&row.id);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Each box has its column, whether the row has that box or not.
                        let column = |ui: &mut Ui, width: f32, add: &mut dyn FnMut(&mut Ui)| {
                            ui.allocate_ui_with_layout(
                                egui::vec2(width, ui.available_height()),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.set_min_width(width);
                                    add(ui);
                                },
                            );
                        };
                        column(ui, TOOLBAR_COLUMN, &mut |ui| {
                            if let Some(on) = row.toolbar {
                                let mut on2 = on;
                                if ui.checkbox(&mut on2, "Toolbar").changed() {
                                    d.set_command_toolbar(&row.id, on2);
                                }
                            }
                        });
                        column(ui, MENU_COLUMN, &mut |ui| {
                            if let Some(on) = row.menu {
                                let mut on2 = on;
                                if ui.checkbox(&mut on2, "Menu").changed() {
                                    d.set_command_menu(&row.id, on2);
                                }
                            }
                        });
                    });
                });
            }
        });
    let selected = d
        .commands_page
        .selected
        .clone()
        .and_then(|id| d.commands_page.rows.iter().find(|r| r.id == id).cloned());
    if let Some(row) = selected {
        ui.separator();
        ui.label(RichText::new(&row.description).color(pal.muted));
        ui.horizontal(|ui| {
            for (i, s) in row.shortcuts.iter().enumerate() {
                if ui
                    .selectable_label(d.commands_page.shortcut == Some(i), s.to_string())
                    .clicked()
                {
                    d.commands_page.shortcut = Some(i);
                }
            }
            let adding = d.commands_page.capturing;
            if ui
                .button(if adding { "Press a key…" } else { "Add" })
                .clicked()
            {
                d.begin_capture();
            }
            if ui
                .add_enabled(
                    d.commands_page.shortcut.is_some(),
                    egui::Button::new("Remove"),
                )
                .clicked()
            {
                d.remove_shortcut();
            }
            if ui
                .add_enabled(row.customized, egui::Button::new("Default"))
                .clicked()
            {
                d.default_shortcuts();
            }
        });
        for conflict in &row.conflicts {
            ui.label(RichText::new(conflict).color(pal.warning));
        }
    }
}

fn open_with(ui: &mut Ui, d: &mut OptionsDialog, pal: &Palette) {
    let rows: Vec<String> = d
        .openwith
        .app
        .iter()
        .map(|a| a.description.clone())
        .collect();
    for (i, description) in rows.iter().enumerate() {
        if ui
            .selectable_label(d.open_with.selected == Some(i), description)
            .clicked()
        {
            d.open_with.selected = Some(i);
        }
    }
    ui.horizontal(|ui| {
        if ui.button("New").clicked() {
            d.open_with_new();
        }
        let one = d.open_with.selected.is_some();
        if ui.add_enabled(one, egui::Button::new("Edit")).clicked() {
            d.open_with_edit();
        }
        if ui.add_enabled(one, egui::Button::new("Clone")).clicked() {
            d.open_with_clone();
        }
        if ui.add_enabled(one, egui::Button::new("Delete")).clicked() {
            d.open_with_delete();
        }
    });
    let mut commit = false;
    let mut cancel = false;
    let mut capture = false;
    let capturing = d.open_with.capturing;
    if let Some(edit) = d.open_with.editing.as_mut() {
        ui.separator();
        egui::Grid::new("hk-open-with")
            .num_columns(2)
            .show(ui, |ui| {
                ui.label("Description");
                ui.add(TextEdit::singleline(&mut edit.app.description).desired_width(420.0));
                ui.end_row();
                ui.label("Command line");
                ui.add(TextEdit::singleline(&mut edit.app.command).desired_width(420.0));
                ui.end_row();
                ui.label("Shortcut");
                ui.horizontal(|ui| {
                    let shown = if capturing {
                        "Press a key…"
                    } else if edit.app.shortcut.is_empty() {
                        "(none)"
                    } else {
                        &edit.app.shortcut
                    };
                    capture = ui.button(shown).clicked();
                    if ui
                        .add_enabled(
                            !edit.app.shortcut.is_empty(),
                            egui::Button::new("Clear").small(),
                        )
                        .clicked()
                    {
                        edit.app.shortcut.clear();
                    }
                });
                ui.end_row();
                ui.label("Path delimiter");
                ui.add(TextEdit::singleline(&mut edit.app.path_delimiter).desired_width(40.0));
                ui.end_row();
            });
        ui.horizontal(|ui| {
            ui.checkbox(&mut edit.app.accepts_files, "Accepts files");
            ui.checkbox(&mut edit.app.accepts_folders, "Accepts folders");
            ui.checkbox(&mut edit.app.refresh_when_finished, "Refresh when finished");
            ui.checkbox(
                &mut edit.app.wait_for_previous,
                "Wait for previous instance",
            );
        });
        if let Some(error) = &edit.error {
            ui.label(RichText::new(error).color(pal.error));
        }
        ui.horizontal(|ui| {
            commit = ui.button("OK").clicked();
            cancel = ui.button("Cancel").clicked();
        });
    }
    if capture {
        d.open_with_begin_capture();
    }
    if commit {
        d.open_with_commit();
    }
    if cancel {
        d.open_with_cancel();
    }
}

fn name_filters(ui: &mut Ui, d: &mut OptionsDialog) {
    let mut delete = None;
    let mut changed = false;
    egui::Grid::new("hk-presets").num_columns(3).show(ui, |ui| {
        for (i, preset) in d.filters.preset.iter_mut().enumerate() {
            changed |= ui
                .add(TextEdit::singleline(&mut preset.name).desired_width(150.0))
                .changed();
            changed |= ui
                .add(TextEdit::singleline(&mut preset.mask).desired_width(340.0))
                .changed();
            if ui.small_button("Delete").clicked() {
                delete = Some(i);
            }
            ui.end_row();
        }
    });
    if changed {
        d.edited();
    }
    if let Some(i) = delete {
        d.preset_delete(i);
    }
    if ui.button("Add").clicked() {
        d.preset_add();
    }
}
