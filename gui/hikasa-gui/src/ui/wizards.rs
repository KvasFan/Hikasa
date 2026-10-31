//! Draws the Export, Import and Restore wizards (M2 W4c, D-131). The models decide; this only edits their fields.

use egui::{Color32, RichText, ScrollArea, TextEdit, Ui};
use hikasa_session::package::{Category, PlanAction};

use crate::ui::theme::Palette;
use crate::wizards::{
    CategoryRow, ExportWizard, ImportWizard, RestoreWizard, category_caption, delete_caption,
    summary_lines,
};

const LIST_HEIGHT: f32 = 300.0;

fn error_line(ui: &mut Ui, error: Option<&String>, pal: &Palette) {
    if let Some(error) = error {
        ui.add_space(4.0);
        ui.label(RichText::new(error).color(pal.error));
    }
}

fn busy_line(ui: &mut Ui, busy: bool, pal: &Palette) {
    if busy {
        ui.add_space(4.0);
        ui.label(RichText::new("Working…").color(pal.muted));
    }
}

/// One category with its "all or some" choice and its items. `show_all`: the items are listed on "All" too, read
/// only (the Import wizard's actions, §1.4.14).
fn category_row(ui: &mut Ui, row: &mut CategoryRow, pal: &Palette, per_item: bool, show_all: bool) {
    ui.checkbox(&mut row.checked, category_caption(row.category));
    if !row.checked || !per_item {
        return;
    }
    ui.indent(row.category.as_str(), |ui| {
        ui.horizontal(|ui| {
            ui.radio_value(&mut row.all, true, "All");
            ui.radio_value(&mut row.all, false, "Some");
        });
        if row.all && !show_all {
            return;
        }
        let all = row.all;
        for item in &mut row.items {
            let text = RichText::new(&item.name);
            let text = match item.action {
                Some(PlanAction::Replace) => text.color(pal.error),
                Some(PlanAction::SkipLocked) => text.color(pal.muted),
                _ => text,
            };
            ui.horizontal(|ui| {
                if all {
                    ui.label(text);
                } else {
                    ui.checkbox(&mut item.checked, text);
                }
                if let Some(label) = action_label(item.action) {
                    ui.label(RichText::new(label).small().color(pal.muted));
                }
            });
        }
    });
}

fn action_label(action: Option<PlanAction>) -> Option<&'static str> {
    match action? {
        PlanAction::Add => Some("Add"),
        PlanAction::Replace => Some("Replace"),
        PlanAction::SkipLocked => Some("Kept (locked)"),
    }
}

/// The Export wizard.
pub fn export(ui: &mut Ui, w: &mut ExportWizard, pal: &Palette) {
    match w.step {
        0 => {
            ui.label("Choose what to export.");
            ScrollArea::vertical()
                .max_height(LIST_HEIGHT)
                .show(ui, |ui| {
                    for row in &mut w.rows {
                        let per_item = row.category.per_item();
                        category_row(ui, row, pal, per_item, false);
                    }
                });
        }
        1 => {
            ui.label("The package file");
            ui.add(TextEdit::singleline(&mut w.target).desired_width(520.0));
        }
        _ => {
            if let Some(result) = &w.result {
                ui.label(format!("{} items written to", result.items.len()));
                ui.label(RichText::new(&result.path).color(pal.fg));
            }
        }
    }
    busy_line(ui, w.busy(), pal);
    error_line(ui, w.error.as_ref(), pal);
}

/// The Import wizard.
pub fn import(ui: &mut Ui, w: &mut ImportWizard, pal: &Palette) {
    if w.step > 0 {
        ui.label(RichText::new(format!("Package: {}", w.path)).color(pal.muted));
        ui.add_space(4.0);
    }
    match w.step {
        0 => {
            ui.label("The package file");
            ui.add(TextEdit::singleline(&mut w.path).desired_width(580.0));
        }
        1 => {
            ui.label("Choose what to import.");
            ScrollArea::vertical()
                .max_height(LIST_HEIGHT - 60.0)
                .show(ui, |ui| {
                    for row in &mut w.rows {
                        category_row(ui, row, pal, true, true);
                    }
                });
            ui.separator();
            ui.checkbox(&mut w.force, "Include locked sessions");
            ui.horizontal(|ui| {
                for category in [Category::Sessions, Category::Formats, Category::Workspaces] {
                    let mut on = w.delete_existing.contains(&category);
                    if ui.checkbox(&mut on, delete_caption(category)).changed() {
                        if on {
                            w.delete_existing.insert(category);
                        } else {
                            w.delete_existing.remove(&category);
                        }
                    }
                }
            });
        }
        2 => {
            if let Some(plan) = &w.summary {
                for line in summary_lines(plan) {
                    ui.label(line);
                }
            }
        }
        _ => {
            if w.plan_changed() {
                ui.label(
                    RichText::new(
                        "The files changed since the summary: the plan that ran differs.",
                    )
                    .color(pal.warning),
                );
            }
            if let Some(result) = &w.result {
                let failed = result.results.iter().filter(|r| !r.ok).count();
                ui.label(format!(
                    "{} items imported, {failed} failed.",
                    result.results.len() - failed
                ));
                ScrollArea::vertical()
                    .max_height(LIST_HEIGHT)
                    .show(ui, |ui| {
                        for r in &result.results {
                            let colour: Color32 = if r.ok { pal.fg } else { pal.error };
                            let mut text = format!("{}: {}", r.category, r.name);
                            if let Some(e) = &r.error {
                                text.push_str(&format!(" ({e})"));
                            }
                            ui.label(RichText::new(text).color(colour));
                        }
                    });
            }
        }
    }
    busy_line(ui, w.busy(), pal);
    error_line(ui, w.error.as_ref(), pal);
}

/// The Restore wizard.
pub fn restore(ui: &mut Ui, w: &mut RestoreWizard, pal: &Palette) {
    match w.step {
        0 => {
            ui.label("Choose what goes back to the factory defaults.");
            for (category, on) in &mut w.categories {
                ui.checkbox(on, category_caption(*category));
            }
            ui.checkbox(&mut w.session_defaults, "Session defaults");
            ui.separator();
            ui.checkbox(&mut w.delete_sessions, "Delete all named sessions");
            ui.checkbox(&mut w.delete_workspaces, "Delete all workspaces");
            ui.checkbox(&mut w.force, "Include locked sessions");
        }
        1 => {
            if let Some(plan) = &w.plan {
                ui.label(format!("{} files will be deleted.", plan.files.len()));
                ScrollArea::vertical()
                    .max_height(LIST_HEIGHT)
                    .show(ui, |ui| {
                        for file in &plan.files {
                            ui.label(RichText::new(&file.path).small());
                        }
                        for warning in &plan.warnings {
                            ui.label(RichText::new(&warning.message).color(pal.warning));
                        }
                    });
            }
        }
        _ => {
            if w.plan_changed() {
                ui.label(
                    RichText::new("The files changed since the plan: what ran differs.")
                        .color(pal.warning),
                );
            }
            if let Some(result) = &w.result {
                ui.label(format!("{} files deleted.", result.files.len()));
            }
        }
    }
    busy_line(ui, w.busy(), pal);
    error_line(ui, w.error.as_ref(), pal);
}
