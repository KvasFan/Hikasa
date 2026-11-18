//! Draws a window's top dialog (D-112): an egui modal inside its native window (a dimmed backdrop takes the
//! window's input), never an extra viewport. Its title bar moves it, so that what lies under it (the theme preview,
//! D-130) can be seen; the place is remembered per title until the program ends. The model decides; this only draws
//! and reports clicks.

use egui::{Align, Align2, Button, CursorIcon, Label, Layout, RichText, Sense, Ui, Vec2, vec2};

use crate::dialog::{AboutDialog, ButtonRole, DialogModel, Prompt};
use crate::info_dialog::InfoDialog;
use crate::op_dialogs::{DeleteDialog, PlanConfirmDialog};
use crate::options::OptionsDialog;
use crate::script_dialog::{ContinueDialog, RunScriptDialog, ScriptDialog};
use crate::session_dialog::{NameDialog, SaveSessionDialog, SessionSettingsDialog};
use crate::ui::hint::Hint as _;
use crate::ui::theme::Palette;
use crate::ui::{op_dialogs, options, script_dialog, session_dialog, wizards, workspace};
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
    /// The browse dialog's Up or double-click (D-240).
    Browse(crate::ui::browse::BrowseInput),
    /// A Browse button of a dialog: the browse dialog over it (D-240).
    BrowseFor(
        crate::browse::BrowseMode,
        String,
        crate::browse::BrowseTarget,
    ),
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
                // A beep flashes the status window's title (§1.8.9).
                let flash = dialog.as_any().is::<ScriptDialog>() && script_dialog::flashing(&ctx);
                let title = RichText::new(dialog.title())
                    .strong()
                    .size(15.0)
                    .color(if flash { pal.accent } else { pal.fg });
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
        } else if let Some(d) = dialog.as_any().downcast_ref::<InfoDialog>() {
            info(ui, d, pal);
        } else if let Some(d) = dialog
            .as_any_mut()
            .downcast_mut::<crate::browse::BrowseDialog>()
        {
            if let Some(input) = crate::ui::browse::show(ui, d, pal) {
                out.push(DialogInput::Browse(input));
            }
        } else if let Some(d) = dialog.as_any_mut().downcast_mut::<SaveSessionDialog>() {
            session_dialog::save_as(ui, d, pal);
        } else if let Some(d) = dialog.as_any_mut().downcast_mut::<NameDialog>() {
            session_dialog::name(ui, d, pal);
        } else if let Some(d) = dialog.as_any_mut().downcast_mut::<SessionSettingsDialog>() {
            session_dialog::settings(ui, d, pal);
            if let Some((side, start, folder)) = d.take_browse() {
                let mode = if folder {
                    crate::browse::BrowseMode::Folder
                } else {
                    crate::browse::BrowseMode::File
                };
                out.push(DialogInput::BrowseFor(
                    mode,
                    start,
                    crate::browse::BrowseTarget::Specs(side),
                ));
            }
        } else if let Some(d) = dialog
            .as_any_mut()
            .downcast_mut::<crate::drop_dialog::DropDialog>()
        {
            fixed_body(ui, height, |ui| op_dialogs::drop_dialog(ui, d, pal));
        } else if let Some(d) = dialog
            .as_any_mut()
            .downcast_mut::<crate::filters_edit::PresetNameDialog>()
        {
            session_dialog::preset_name(ui, d, pal);
        } else if let Some(d) = dialog.as_any_mut().downcast_mut::<SaveWorkspaceDialog>() {
            workspace::save_as(ui, d, pal);
        } else if let Some(d) = dialog.as_any_mut().downcast_mut::<ManageWorkspacesDialog>() {
            workspace::manage(ui, d, pal);
        } else if let Some(d) = dialog.as_any_mut().downcast_mut::<OptionsDialog>() {
            options::show(ui, d, pal);
            out.push(DialogInput::Edited);
            if let Some((key, text)) = d.browse.take() {
                let mode = if key.contains("folder") {
                    crate::browse::BrowseMode::Folder
                } else {
                    crate::browse::BrowseMode::File
                };
                out.push(DialogInput::BrowseFor(
                    mode,
                    text,
                    crate::browse::BrowseTarget::OptionKey(key),
                ));
            }
        } else if let Some(d) = dialog.as_any_mut().downcast_mut::<ExportWizard>() {
            fixed_body(ui, height, |ui| wizards::export(ui, d, pal));
            if std::mem::take(&mut d.browse) {
                out.push(DialogInput::BrowseFor(
                    crate::browse::BrowseMode::SaveFile {
                        default_name: crate::browse::file_name_of(&d.target),
                    },
                    crate::browse::folder_of(&d.target),
                    crate::browse::BrowseTarget::Export,
                ));
            }
        } else if let Some(d) = dialog.as_any_mut().downcast_mut::<ImportWizard>() {
            fixed_body(ui, height, |ui| wizards::import(ui, d, pal));
        } else if let Some(d) = dialog.as_any_mut().downcast_mut::<RestoreWizard>() {
            fixed_body(ui, height, |ui| wizards::restore(ui, d, pal));
        } else if let Some(d) = dialog.as_any_mut().downcast_mut::<PlanConfirmDialog>() {
            op_dialogs::confirm(ui, d, pal);
        } else if let Some(d) = dialog.as_any_mut().downcast_mut::<DeleteDialog>() {
            op_dialogs::delete(ui, d, pal);
        } else if let Some(d) = dialog
            .as_any_mut()
            .downcast_mut::<crate::content_dialogs::ExcludeDialog>()
        {
            op_dialogs::exclude(ui, d, pal);
        } else if let Some(d) = dialog
            .as_any_mut()
            .downcast_mut::<crate::content_dialogs::CompareContentsDialog>()
        {
            op_dialogs::compare_contents(ui, d, pal);
        } else if let Some(d) = dialog
            .as_any()
            .downcast_ref::<crate::content_dialogs::ContentsResultsDialog>()
        {
            fixed_body(ui, height, |ui| op_dialogs::contents_results(ui, d, pal));
        } else if let Some(d) = dialog
            .as_any_mut()
            .downcast_mut::<crate::content_dialogs::QuickCompareDialog>()
        {
            fixed_body(ui, height, |ui| op_dialogs::quick_compare(ui, d, pal));
        } else if let Some(d) = dialog
            .as_any_mut()
            .downcast_mut::<crate::folder_view::sync::SyncDialog>()
        {
            fixed_body(ui, height, |ui| op_dialogs::sync_dialog(ui, d, pal));
        } else if let Some(d) = dialog
            .as_any_mut()
            .downcast_mut::<crate::op_dialogs::OpDialog>()
        {
            if op_dialogs::op_dialog(ui, d, pal)
                && let crate::op_dialogs::OpForm::CopyTo { target, .. } = d.form()
            {
                out.push(DialogInput::BrowseFor(
                    crate::browse::BrowseMode::Folder,
                    target.clone(),
                    crate::browse::BrowseTarget::OpTarget(d.view()),
                ));
            }
        } else if let Some(d) = dialog
            .as_any_mut()
            .downcast_mut::<crate::table_dialogs::GoToDialog>()
        {
            crate::ui::table_dialogs::go_to(ui, d, pal);
        } else if let Some(d) = dialog
            .as_any_mut()
            .downcast_mut::<crate::hex::nav::HexGoToDialog>()
        {
            crate::ui::hex_panels::go_to(ui, d, pal);
        } else if let Some(d) = dialog
            .as_any_mut()
            .downcast_mut::<crate::hex::layout::LayoutDialog>()
        {
            crate::ui::hex_panels::layout(ui, d);
        } else if let Some(d) = dialog
            .as_any_mut()
            .downcast_mut::<crate::table_dialogs::TableReportDialog>()
        {
            crate::ui::table_dialogs::report(ui, d, pal);
        } else if let Some(d) = dialog
            .as_any_mut()
            .downcast_mut::<crate::text::nav::GoToLineDialog>()
        {
            // Go To of a text view (D-309): `line[:column]` and core's error under the field.
            ui.label(d.range_text());
            let r = ui.text_edit_singleline(&mut d.text);
            r.request_focus();
            if let Some(e) = &d.error {
                ui.colored_label(pal.error, e);
            }
        } else if let Some(d) = dialog
            .as_any_mut()
            .downcast_mut::<crate::text::layout::DisplayFontDialog>()
        {
            // Display Font (D-305): a family the GUI knows, a size, the theme's editor font (a hex view: the size).
            if !d.size_only {
                ui.horizontal(|ui| {
                    ui.label("Family:");
                    egui::ComboBox::from_id_salt("display-font-family")
                        .selected_text(if d.family.is_empty() {
                            "Editor font of the theme".to_owned()
                        } else {
                            d.family.clone()
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut d.family,
                                String::new(),
                                "Editor font of the theme",
                            );
                            for f in d.families.clone() {
                                ui.selectable_value(&mut d.family, f.clone(), f);
                            }
                        });
                });
            }
            ui.horizontal(|ui| {
                ui.label("Size (6–72):");
                ui.add(egui::TextEdit::singleline(&mut d.size).desired_width(60.0));
            });
            if !d.size_only {
                ui.checkbox(&mut d.theme, "Use for the theme's Editor font");
            }
            if let Some(e) = &d.error {
                ui.colored_label(pal.error, e);
            }
        } else if let Some(d) = dialog
            .as_any_mut()
            .downcast_mut::<crate::text::find_files::FindFilesDialog>()
        {
            if find_files_dialog(ui, d, pal) {
                out.push(DialogInput::BrowseFor(
                    crate::browse::BrowseMode::Folder,
                    d.look_in.last().cloned().unwrap_or_default(),
                    crate::browse::BrowseTarget::FindFilesRoot,
                ));
            }
        } else if let Some(d) = dialog
            .as_any_mut()
            .downcast_mut::<crate::formats::FileFormatManager>()
        {
            if let Some((mode, start, target)) = crate::ui::formats::manager(ui, d, pal) {
                out.push(DialogInput::BrowseFor(mode, start, target));
            }
        } else if let Some(d) = dialog
            .as_any_mut()
            .downcast_mut::<crate::formats::editor::FormatEditor>()
        {
            if let Some((mode, start, target)) = crate::ui::formats::editor(ui, d, pal) {
                out.push(DialogInput::BrowseFor(mode, start, target));
            }
        } else if let Some(d) = dialog
            .as_any_mut()
            .downcast_mut::<crate::formats::grammar_item::GrammarItemDialog>()
        {
            crate::ui::formats::grammar_item(ui, d, pal);
        } else if let Some(d) = dialog.as_any().downcast_ref::<ScriptDialog>() {
            fixed_body(ui, height, |ui| script_dialog::status(ui, d, pal));
        } else if let Some(d) = dialog.as_any().downcast_ref::<ContinueDialog>() {
            script_dialog::ask_continue(ui, d, pal);
        } else if let Some(d) = dialog.as_any_mut().downcast_mut::<RunScriptDialog>()
            && script_dialog::run_script(ui, d, pal)
        {
            out.push(DialogInput::BrowseFor(
                crate::browse::BrowseMode::File,
                d.path.clone(),
                crate::browse::BrowseTarget::RunScript,
            ));
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
                let mut button = ui.add_enabled(
                    b.enabled,
                    Button::new(text).min_size(vec2(80.0, BUTTON_HEIGHT)),
                );
                if let Some(tip) = dialog.button_tip(b.role) {
                    button = button.on_disabled_hover_text(tip);
                }
                if button.clicked() {
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

/// The Info dialog's table (D-242): one grid per section, the figures right-aligned.
fn info(ui: &mut Ui, d: &InfoDialog, pal: &Palette) {
    if let Some(note) = d.note() {
        ui.label(RichText::new(note).color(pal.accent));
        ui.add_space(4.0);
    }
    for (k, section) in d.sections().iter().enumerate() {
        ui.add_space(6.0);
        egui::Grid::new(("hk-info", k))
            .num_columns(1 + section.heads.len().max(1))
            .spacing(vec2(18.0, 3.0))
            .show(ui, |ui| {
                ui.label(RichText::new(&section.title).strong().color(pal.fg));
                for head in &section.heads {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new(head).color(pal.muted));
                    });
                }
                ui.end_row();
                for row in &section.rows {
                    ui.label(RichText::new(&row.label).color(pal.fg));
                    for value in &row.values {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(RichText::new(value).color(pal.fg));
                        });
                    }
                    ui.end_row();
                }
            });
    }
    if let Some(message) = &d.message {
        ui.add_space(6.0);
        ui.label(RichText::new(message).color(pal.muted));
    }
}

/// The Find in Files dialog's fields (D-308); true when *Browse…* was pressed.
fn find_files_dialog(
    ui: &mut Ui,
    d: &mut crate::text::find_files::FindFilesDialog,
    pal: &Palette,
) -> bool {
    let mut browse = false;
    egui::Grid::new("find-in-files")
        .num_columns(2)
        .show(ui, |ui| {
            ui.label("Find what:");
            ui.horizontal(|ui| {
                let field = ui.add(egui::TextEdit::singleline(&mut d.text).desired_width(320.0));
                if d.regex {
                    crate::ui::formats::helper_button(ui, &mut d.text, field.id);
                }
            });
            ui.end_row();
            ui.label("");
            ui.horizontal(|ui| {
                ui.checkbox(&mut d.match_case, "Match case");
                ui.checkbox(&mut d.whole_word, "Whole words");
                ui.checkbox(&mut d.regex, "Regular expression");
            });
            ui.end_row();
            ui.label("Look in:");
            ui.vertical(|ui| {
                if d.look_in.is_empty() {
                    d.look_in.push(String::new());
                }
                for root in &mut d.look_in {
                    ui.add(egui::TextEdit::singleline(root).desired_width(320.0));
                }
                ui.horizontal(|ui| {
                    if ui.button("Add").clicked() {
                        d.look_in.push(String::new());
                    }
                    if ui.button("Browse…").clicked() {
                        browse = true;
                    }
                });
            });
            ui.end_row();
            ui.label("File masks:");
            ui.add(egui::TextEdit::singleline(&mut d.masks).desired_width(320.0));
            ui.end_row();
            ui.label("");
            ui.vertical(|ui| {
                ui.checkbox(&mut d.subfolders, "Include subfolders");
                ui.checkbox(&mut d.hidden, "Include hidden items");
                ui.checkbox(&mut d.archives, "Search inside archives");
                ui.checkbox(&mut d.convert, "Convert with the format's converter");
            });
            ui.end_row();
            ui.label("Encoding:");
            let mut detect = d.encoding.is_none();
            ui.horizontal(|ui| {
                ui.checkbox(&mut detect, "Detect");
                if detect {
                    d.encoding = None;
                } else {
                    let label = d.encoding.get_or_insert_with(String::new);
                    ui.add(egui::TextEdit::singleline(label).desired_width(140.0));
                }
            });
            ui.end_row();
        });
    egui::CollapsingHeader::new("Advanced").show(ui, |ui| {
        egui::Grid::new("find-in-files-advanced").show(ui, |ui| {
            ui.label("Max hits:");
            ui.add(egui::TextEdit::singleline(&mut d.max_hits).desired_width(100.0));
            ui.end_row();
            ui.label("Hits per file:");
            ui.add(egui::TextEdit::singleline(&mut d.per_file).desired_width(100.0));
            ui.end_row();
            ui.label("Max file size (bytes):");
            ui.add(egui::TextEdit::singleline(&mut d.max_size).desired_width(100.0));
            ui.end_row();
        });
    });
    if let Some(e) = &d.error {
        ui.colored_label(pal.error, e);
    }
    browse
}
