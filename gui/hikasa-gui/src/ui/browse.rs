//! Draws the browse dialog (D-240): the places at the left; the location with Up; the filter and *Show hidden items*;
//! the listing (folders and archives first, a double-click opens them); SaveFile's name field; the listing's error.

use egui::{Align, Layout, RichText, ScrollArea, TextEdit, Ui, vec2};

use crate::browse::{BrowseDialog, BrowseMode};
use crate::ui::hint::Hint as _;
use crate::ui::theme::Palette;

/// What the user did that needs the service (the window runs it on the model).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowseInput {
    /// Up to the parent.
    Up,
    /// A double-click on entry `index`.
    Open(usize),
}

/// Draws the body; returns what needs the service.
pub fn show(ui: &mut Ui, d: &mut BrowseDialog, pal: &Palette) -> Option<BrowseInput> {
    let mut input = None;
    let height = 360.0;
    ui.horizontal_top(|ui| {
        // Places.
        ui.allocate_ui_with_layout(vec2(190.0, height), Layout::top_down(Align::Min), |ui| {
            ui.label(RichText::new("Places").strong().color(pal.fg));
            ScrollArea::vertical()
                .id_salt("hk-browse-places")
                .show(ui, |ui| {
                    let mut go = None;
                    for place in &d.places {
                        if ui
                            .selectable_label(d.location == place.spec, &place.label)
                            .hint(place.spec.clone())
                            .clicked()
                        {
                            go = Some(place.spec.clone());
                        }
                    }
                    if let Some(spec) = go {
                        d.go(&spec);
                    }
                });
        });
        ui.separator();
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                if ui.button("Up").hint("The parent folder").clicked() {
                    input = Some(BrowseInput::Up);
                }
                let edit = ui.add(TextEdit::singleline(&mut d.typed).desired_width(f32::INFINITY));
                if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    let typed = d.typed.clone();
                    d.go(&typed);
                }
            });
            ui.horizontal(|ui| {
                ui.label("Filter");
                ui.add(TextEdit::singleline(&mut d.filter).desired_width(200.0));
                ui.checkbox(&mut d.show_hidden, "Show hidden items");
                if d.listing() {
                    ui.spinner();
                }
            });
            let list_height = if matches!(d.mode, BrowseMode::SaveFile { .. }) {
                height - 110.0
            } else {
                height - 70.0
            };
            if let Some(error) = &d.error {
                ui.label(RichText::new(error).color(pal.error));
            }
            // Only the rows in view are laid out: a folder may hold 100,000 entries (G15 browse).
            let shown = d.shown();
            let row_height = ui.spacing().interact_size.y;
            let mut area = ScrollArea::vertical()
                .id_salt("hk-browse-list")
                .max_height(list_height)
                .auto_shrink([false, false]);
            if let Some(offset) = d.bench_scroll {
                area = area.vertical_scroll_offset(offset);
            }
            area.show_rows(ui, row_height, shown.len(), |ui, range| {
                for &i in &shown[range] {
                    let entry = &d.entries[i];
                    let mark = if entry.archive {
                        "▣"
                    } else if entry.folder {
                        "▸"
                    } else {
                        " "
                    };
                    let r = ui
                        .selectable_label(d.selected == Some(i), format!("{mark} {}", entry.name));
                    if r.double_clicked() {
                        input = Some(BrowseInput::Open(i));
                    } else if r.clicked() {
                        d.selected = Some(i);
                        if matches!(d.mode, BrowseMode::SaveFile { .. }) && !entry.opens() {
                            d.name.clone_from(&entry.name);
                        }
                    }
                }
            });
            if matches!(d.mode, BrowseMode::SaveFile { .. }) {
                ui.horizontal(|ui| {
                    ui.label("Name");
                    ui.add(TextEdit::singleline(&mut d.name).desired_width(f32::INFINITY));
                });
            }
        });
    });
    input
}
