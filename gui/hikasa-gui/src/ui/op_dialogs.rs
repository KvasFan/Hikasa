//! Draws the dialogs of folder operations (M2 W6d, §1.6.20): the question of a plan with its counts and "Do not ask
//! again", and the Delete dialog's sides and check boxes. The models decide; this only edits their fields.

use egui::{RichText, Ui};

use crate::op_dialogs::{
    DeleteDialog, OpDialog, OpForm, PlanConfirmDialog, RenameMode, SideChoice,
};
use crate::op_preview::{PreviewList, Tone};
use crate::ui::theme::Palette;
use hikasa_core::Side;
use hikasa_folder::ops::PathStructure;
use hikasa_vfs::ops::OverwritePolicy;

/// The height of one preview line.
const LINE: f32 = 18.0;

/// The confirmation dialog's body.
pub fn confirm(ui: &mut Ui, dialog: &mut PlanConfirmDialog, pal: &Palette) {
    ui.label(RichText::new(dialog.subject()).color(pal.fg).strong());
    ui.add_space(4.0);
    if let Some(question) = dialog.question() {
        ui.label(RichText::new(question).color(pal.fg));
    }
    let remaining = dialog.remaining();
    if remaining > 1 {
        ui.add_space(2.0);
        ui.label(RichText::new(format!("{remaining} questions left")).color(pal.muted));
    }
    if !dialog.counts().is_empty() {
        ui.add_space(6.0);
        for line in dialog.counts() {
            ui.label(RichText::new(line).color(pal.muted));
        }
    }
    if dialog.do_not_ask_key().is_some() {
        ui.add_space(6.0);
        ui.checkbox(&mut dialog.ticked, "Do not ask again");
    }
    if let Some(mut hidden) = dialog.include_hidden() {
        if ui.checkbox(&mut hidden, "Include hidden items").changed() {
            dialog.set_include_hidden(hidden);
        }
    }
    if let Some(list) = dialog.list() {
        ui.add_space(6.0);
        preview(ui, list, pal, 180.0);
    }
}

/// The preview list (D-244): one line per item as `--dry-run` prints it, virtualized (only the lines in view are
/// asked for), failed items in the error colour, skipped ones muted, conflicts highlighted; then the warnings and the
/// archives' backups.
pub fn preview(ui: &mut Ui, list: &PreviewList, pal: &Palette, height: f32) {
    egui::Frame::NONE
        .stroke(egui::Stroke::new(1.0, pal.border))
        .inner_margin(4.0)
        .show(ui, |ui| {
            egui::ScrollArea::both()
                .id_salt("hk-op-preview")
                .max_height(height)
                .auto_shrink([false, true])
                .show_rows(ui, LINE, list.len(), |ui, range| {
                    for line in list.lines(range) {
                        let text = RichText::new(line.text).monospace();
                        let text = match line.tone {
                            Tone::Normal => text.color(pal.fg),
                            Tone::Muted => text.color(pal.muted),
                            Tone::Error => text.color(pal.error),
                            Tone::Conflict => text.color(pal.warning),
                            Tone::Summary => text.color(pal.fg).strong(),
                        };
                        ui.add(egui::Label::new(text).extend());
                    }
                });
        });
    for warning in list.warnings() {
        ui.label(RichText::new(warning).color(pal.warning));
    }
    for backup in list.backups() {
        ui.label(RichText::new(format!("backup: {backup}")).color(pal.muted));
    }
}

/// The side choices of a verb dialog: a greyed choice that would write a read-only side says why.
fn sides(ui: &mut Ui, d: &OpDialog, label: &str, current: SideChoice) -> Option<SideChoice> {
    let mut picked = None;
    ui.horizontal(|ui| {
        ui.label(label);
        for choice in SideChoice::ALL {
            let enabled = d.side_enabled(choice);
            let response = ui.add_enabled(
                enabled,
                egui::RadioButton::new(current == choice, choice.label()),
            );
            let response = if enabled {
                response
            } else {
                let read_only: Vec<Side> = [Side::Left, Side::Right]
                    .into_iter()
                    .filter(|s| !d.writable(*s))
                    .collect();
                match read_only.first() {
                    Some(side) => response.on_disabled_hover_text(OpDialog::read_only_tip(*side)),
                    None => response,
                }
            };
            if response.clicked() {
                picked = Some(choice);
            }
        }
    });
    picked
}

/// A verb dialog's body (D-245): its options, *Include hidden items*, then "Planning…" or the counts and the preview
/// list. True when its Browse button was pressed.
pub fn op_dialog(ui: &mut Ui, d: &mut OpDialog, pal: &Palette) -> bool {
    let mut browse = false;
    let form = d.form().clone();
    match form {
        OpForm::CopyTo {
            moving,
            target,
            structure,
            sides: current,
            overwrite,
        } => {
            ui.horizontal(|ui| {
                ui.label("Target folder");
                let mut text = target.clone();
                if ui
                    .add(egui::TextEdit::singleline(&mut text).desired_width(400.0))
                    .changed()
                {
                    d.edit(|f| {
                        if let OpForm::CopyTo { target, .. } = f {
                            *target = text.clone();
                        }
                    });
                }
                if ui.button("Browse…").clicked() {
                    browse = true;
                }
            });
            for (value, label) in [
                (PathStructure::Relative, "Keep relative folder structure"),
                (PathStructure::Base, "Keep base folder structure"),
                (PathStructure::None, "Don't keep folder structure"),
            ] {
                if ui.radio(structure == value, label).clicked() {
                    d.edit(|f| {
                        if let OpForm::CopyTo { structure, .. } = f {
                            *structure = value;
                        }
                    });
                }
            }
            let label = if moving { "Move from" } else { "Copy from" };
            if let Some(choice) = sides(ui, d, label, current) {
                d.edit(|f| {
                    if let OpForm::CopyTo { sides, .. } = f {
                        *sides = choice;
                    }
                });
            }
            ui.horizontal(|ui| {
                ui.label("If the target exists");
                for (value, label) in [
                    (OverwritePolicy::Fail, "Fail"),
                    (OverwritePolicy::Skip, "Skip"),
                    (OverwritePolicy::Newer, "Replace if newer"),
                    (OverwritePolicy::Always, "Replace"),
                ] {
                    if ui.radio(overwrite == value, label).clicked() {
                        d.edit(|f| {
                            if let OpForm::CopyTo { overwrite, .. } = f {
                                *overwrite = value;
                            }
                        });
                    }
                }
            });
        }
        OpForm::Rename {
            mode,
            dos,
            old,
            new,
            sides: current,
        } => {
            if ui
                .radio(mode == RenameMode::Dos, "DOS compatible")
                .clicked()
            {
                d.edit(|f| {
                    if let OpForm::Rename { mode, .. } = f {
                        *mode = RenameMode::Dos;
                    }
                });
            }
            ui.horizontal(|ui| {
                ui.add_space(20.0);
                let mut text = dos.clone();
                if ui
                    .add_enabled(
                        mode == RenameMode::Dos,
                        egui::TextEdit::singleline(&mut text),
                    )
                    .changed()
                {
                    d.edit(|f| {
                        if let OpForm::Rename { dos, .. } = f {
                            *dos = text.clone();
                        }
                    });
                }
            });
            if ui
                .radio(mode == RenameMode::Regex, "Regular expressions")
                .clicked()
            {
                d.edit(|f| {
                    if let OpForm::Rename { mode, .. } = f {
                        *mode = RenameMode::Regex;
                    }
                });
            }
            ui.horizontal(|ui| {
                ui.add_space(20.0);
                let (mut o, mut n) = (old.clone(), new.clone());
                let enabled = mode == RenameMode::Regex;
                ui.label("Old");
                let a = ui.add_enabled(
                    enabled,
                    egui::TextEdit::singleline(&mut o).desired_width(180.0),
                );
                ui.label("New");
                let b = ui.add_enabled(
                    enabled,
                    egui::TextEdit::singleline(&mut n).desired_width(180.0),
                );
                if a.changed() || b.changed() {
                    d.edit(|f| {
                        if let OpForm::Rename { old, new, .. } = f {
                            *old = o.clone();
                            *new = n.clone();
                        }
                    });
                }
            });
            if let Some(choice) = sides(ui, d, "Rename on", current) {
                d.edit(|f| {
                    if let OpForm::Rename { sides, .. } = f {
                        *sides = choice;
                    }
                });
            }
        }
        OpForm::Touch {
            from_other,
            to,
            text,
            sides: current,
        } => {
            if ui
                .radio(from_other, "Copy timestamps from other side")
                .clicked()
            {
                d.edit(|f| {
                    if let OpForm::Touch { from_other, .. } = f {
                        *from_other = true;
                    }
                });
            }
            ui.horizontal(|ui| {
                ui.add_space(20.0);
                ui.label("to the");
                for (side, label) in [(Side::Left, "Left"), (Side::Right, "Right")] {
                    let response = ui.add_enabled(
                        from_other && d.writable(side),
                        egui::RadioButton::new(to == side, label),
                    );
                    if response.clicked() {
                        d.edit(|f| {
                            if let OpForm::Touch { to, .. } = f {
                                *to = side;
                            }
                        });
                    }
                }
            });
            if ui.radio(!from_other, "Set to").clicked() {
                d.edit(|f| {
                    if let OpForm::Touch { from_other, .. } = f {
                        *from_other = false;
                    }
                });
            }
            ui.horizontal(|ui| {
                ui.add_space(20.0);
                let mut typed = text.clone();
                if ui
                    .add_enabled(!from_other, egui::TextEdit::singleline(&mut typed))
                    .on_hover_text("YYYY-MM-DD HH:MM:SS, local time")
                    .changed()
                {
                    d.edit(|f| {
                        if let OpForm::Touch { text, .. } = f {
                            *text = typed.clone();
                        }
                    });
                }
            });
            if !from_other && let Some(choice) = sides(ui, d, "Touch on", current) {
                d.edit(|f| {
                    if let OpForm::Touch { sides, .. } = f {
                        *sides = choice;
                    }
                });
            }
        }
        OpForm::NewFolder {
            name,
            sides: current,
            parent,
        } => {
            let place = if parent.is_empty() {
                "the base folders".to_owned()
            } else {
                parent.clone()
            };
            ui.label(RichText::new(format!("In {place}")).color(pal.muted));
            ui.horizontal(|ui| {
                ui.label("Name");
                let mut typed = name.clone();
                if ui
                    .add(egui::TextEdit::singleline(&mut typed).desired_width(320.0))
                    .changed()
                {
                    d.edit(|f| {
                        if let OpForm::NewFolder { name, .. } = f {
                            *name = typed.clone();
                        }
                    });
                }
            });
            if let Some(choice) = sides(ui, d, "Create on", current) {
                d.edit(|f| {
                    if let OpForm::NewFolder { sides, .. } = f {
                        *sides = choice;
                    }
                });
            }
        }
    }
    let mut hidden = d.include_hidden();
    if ui.checkbox(&mut hidden, "Include hidden items").changed() {
        d.set_include_hidden(hidden);
    }
    ui.add_space(6.0);
    if let Some(message) = d.message() {
        ui.label(RichText::new(message).color(pal.error));
    } else if let Some(list) = d.list() {
        for line in list.counts() {
            ui.label(RichText::new(line).color(pal.muted));
        }
        preview(ui, list, pal, 200.0);
    } else if d.planning() {
        ui.label(RichText::new("Planning…").color(pal.muted));
    }
    browse
}

/// The Delete dialog's body.
pub fn delete(ui: &mut Ui, dialog: &mut DeleteDialog, pal: &Palette) {
    ui.label(RichText::new(dialog.path()).color(pal.fg).strong());
    for (side, name) in [(Side::Left, "Left"), (Side::Right, "Right")] {
        let text = dialog.holds(side).unwrap_or("nothing");
        ui.label(RichText::new(format!("{name}: {text}")).color(pal.muted));
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label("Delete on");
        for choice in SideChoice::ALL {
            let on = dialog.choice() == Some(choice);
            let enabled = dialog.enabled(choice);
            if ui
                .add_enabled(enabled, egui::RadioButton::new(on, choice.label()))
                .clicked()
            {
                dialog.set_choice(choice);
            }
        }
    });
    ui.add_space(4.0);
    let mut bin = dialog.use_bin();
    if ui
        .add_enabled(
            !dialog.permanent(),
            egui::Checkbox::new(&mut bin, "Use recycle bin if possible"),
        )
        .changed()
    {
        dialog.set_use_bin(bin);
    }
    if dialog.shows_fallback() {
        let mut fallback = dialog.fallback();
        if ui
            .add_enabled(
                dialog.use_bin(),
                egui::Checkbox::new(
                    &mut fallback,
                    "If the recycle bin fails, delete permanently",
                ),
            )
            .changed()
        {
            dialog.set_fallback(fallback);
        }
    }
}

/// The Exclude dialog's body (FC-052): what is excluded and the masks it adds.
pub fn exclude(ui: &mut Ui, d: &mut crate::content_dialogs::ExcludeDialog, pal: &Palette) {
    use hikasa_folder::select::ExcludeWhat;
    for (what, label) in [
        (ExcludeWhat::Items, "Selected items"),
        (ExcludeWhat::Type, "All files of these types"),
    ] {
        if ui.radio(d.what() == what, label).clicked() {
            d.set_what(what);
        }
    }
    ui.add_space(6.0);
    match d.masks() {
        None => {
            ui.label(RichText::new("Reading the selection…").color(pal.muted));
        }
        Some(Err(message)) => {
            ui.label(RichText::new(message).color(pal.error));
        }
        Some(Ok((files, folders))) => {
            for (title, list) in [("Files", files), ("Folders", folders)] {
                if !list.is_empty() {
                    ui.label(RichText::new(format!("{title}: {}", list.join("; "))).color(pal.fg));
                }
            }
        }
    }
}

/// The Compare Contents dialog's body (FC-029 b): the method, *Show results dialog*, the pairs it reads.
pub fn compare_contents(
    ui: &mut Ui,
    d: &mut crate::content_dialogs::CompareContentsDialog,
    pal: &Palette,
) {
    use hikasa_core::ContentMethod;
    ui.horizontal(|ui| {
        ui.label("Method");
        for method in [
            ContentMethod::Crc,
            ContentMethod::Binary,
            ContentMethod::Rules,
        ] {
            if ui.radio(d.method == method, method_label(method)).clicked() {
                d.method = method;
            }
        }
    });
    ui.checkbox(&mut d.show_results, "Show results dialog");
    ui.add_space(6.0);
    match d.count() {
        None => {
            ui.label(RichText::new("Counting the file pairs…").color(pal.muted));
        }
        Some(Ok(count)) => {
            ui.label(
                RichText::new(format!("{} file pairs, {} bytes", count.pairs, count.bytes))
                    .color(pal.muted),
            );
        }
        Some(Err(message)) => {
            ui.label(RichText::new(message).color(pal.error));
        }
    }
}

/// The results of a content compare: the counts, then one line per item (virtualized).
pub fn contents_results(
    ui: &mut Ui,
    d: &crate::content_dialogs::ContentsResultsDialog,
    pal: &Palette,
) {
    for line in d.counts() {
        ui.label(RichText::new(line).color(pal.fg));
    }
    ui.add_space(6.0);
    egui::ScrollArea::both()
        .id_salt("hk-contents-results")
        .max_height(260.0)
        .auto_shrink([false, true])
        .show_rows(ui, LINE, d.len(), |ui, range| {
            for i in range {
                if let Some((path, result, failed)) = d.item(i) {
                    let text = RichText::new(format!("{path}: {result}")).monospace();
                    ui.add(
                        egui::Label::new(if failed {
                            text.color(pal.error)
                        } else {
                            text.color(pal.fg)
                        })
                        .extend(),
                    );
                }
            }
        });
}

/// The Quick Compare dialog's body: the method and one line per pair (click selects it for *Open*).
pub fn quick_compare(
    ui: &mut Ui,
    d: &mut crate::content_dialogs::QuickCompareDialog,
    pal: &Palette,
) {
    use hikasa_core::ContentMethod;
    use hikasa_session::api::QuickMethod;
    ui.horizontal(|ui| {
        ui.label("Method");
        for (method, content) in [
            (QuickMethod::Binary, ContentMethod::Binary),
            (QuickMethod::Rules, ContentMethod::Rules),
        ] {
            if ui
                .radio(d.method() == method, method_label(content))
                .clicked()
            {
                d.set_method(method);
            }
        }
    });
    ui.add_space(6.0);
    if let Some(error) = d.error() {
        ui.label(RichText::new(error).color(pal.error));
    } else if d.running() {
        ui.label(RichText::new("Comparing…").color(pal.muted));
    }
    let rows = d.rows().to_vec();
    egui::ScrollArea::both()
        .id_salt("hk-quick-compare")
        .max_height(240.0)
        .auto_shrink([false, true])
        .show_rows(ui, LINE, rows.len(), |ui, range| {
            for i in range {
                let row = &rows[i];
                let (result, colour) = match &row.result {
                    Ok(kind) => (
                        crate::content_dialogs::result_text(*kind).to_owned(),
                        pal.fg,
                    ),
                    Err(message) => (message.clone(), pal.error),
                };
                let text = RichText::new(format!("{}: {result}", row.path)).color(colour);
                if ui.selectable_label(d.selected == Some(i), text).clicked() {
                    d.selected = Some(i);
                }
            }
        });
    if d.more() > 0 {
        ui.label(RichText::new(format!("… and {} more", d.more())).color(pal.muted));
    }
}

/// The Synchronize preview's body (FC-078): *Just selection*, the bin, the confirmation mode, *Include hidden items*,
/// then the counts and the preview list (or the core message, Run disabled).
pub fn sync_dialog(ui: &mut Ui, d: &mut crate::folder_view::sync::SyncDialog, pal: &Palette) {
    use hikasa_folder::ops::ConfirmMode;
    let has_selection = d.has_selection();
    let mut just = d.just_selection;
    if ui
        .add_enabled(
            has_selection,
            egui::Checkbox::new(&mut just, "Just selection"),
        )
        .changed()
    {
        d.edit(|d| d.just_selection = just);
    }
    let mut bin = d.options.recycle;
    if ui
        .checkbox(&mut bin, "Delete to recycle bin if possible")
        .changed()
    {
        d.edit(|d| d.options.recycle = bin);
    }
    let mut fallback = d.options.recycle_fallback;
    if ui
        .add_enabled(
            d.options.recycle,
            egui::Checkbox::new(
                &mut fallback,
                "If the recycle bin fails, delete permanently",
            ),
        )
        .changed()
    {
        d.edit(|d| d.options.recycle_fallback = fallback);
    }
    ui.horizontal(|ui| {
        ui.label("Confirmations");
        for (mode, label) in [
            (ConfirmMode::Prompt, "Prompt"),
            (ConfirmMode::YesToAll, "Yes to All"),
            (ConfirmMode::NoToAll, "No to All"),
        ] {
            if ui.radio(d.mode == mode, label).clicked() {
                d.edit(|d| d.mode = mode);
            }
        }
    });
    let mut hidden = d.include_hidden;
    if ui.checkbox(&mut hidden, "Include hidden items").changed() {
        d.edit(|d| d.include_hidden = hidden);
    }
    ui.add_space(6.0);
    if let Some(message) = d.message() {
        ui.label(RichText::new(message).color(pal.error));
    } else if let Some(list) = d.list() {
        for line in list.counts() {
            ui.label(RichText::new(line).color(pal.muted));
        }
        preview(ui, list, pal, 220.0);
    } else if d.planning() {
        ui.label(RichText::new("Planning…").color(pal.muted));
    }
}

/// The label of a content method.
fn method_label(method: hikasa_core::ContentMethod) -> &'static str {
    use hikasa_core::ContentMethod;
    match method {
        ContentMethod::Crc => "CRC",
        ContentMethod::Binary => "Binary",
        ContentMethod::Rules => "Rules-based",
        ContentMethod::None => "",
    }
}
