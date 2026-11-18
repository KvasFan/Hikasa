//! The Find in Files results panel of a window (M2 W11b; contract §1.11.17, D-308, help `panel-find-in-files`): the
//! summary line, Stop, Clear, Replace in Files, Close; the virtualized list of files and hits (rows on screen only);
//! Enter or a double click opens a hit, Ctrl+C copies the selected line. The model is
//! [`crate::text::find_files::FindFilesPanel`]; the actions are the window's.

use egui::{Align2, CornerRadius, FontId, Key, Rect, Sense, Ui, pos2, vec2};

use crate::text::find_files::{FindFilesPanel, ListRow};
use crate::ui::theme::Palette;

/// What the panel asks of its window.
#[derive(Debug, Clone, PartialEq)]
pub enum FilesAction {
    /// Open hit `k`.
    Open(usize),
    /// Copy the selected line.
    Copy,
    /// Stop the search.
    Stop,
    /// Empty the panel.
    Clear,
    /// Hide the panel.
    Close,
    /// Replace in Files with this text.
    Replace(String),
    /// The panel's height changed.
    Height(f32),
}

/// Draws the panel into `ui`; returns its actions.
pub fn show(
    ui: &mut Ui,
    panel: &mut FindFilesPanel,
    replace: &mut String,
    pal: &Palette,
) -> Vec<FilesAction> {
    let mut out = Vec::new();
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Find in Files").strong());
        ui.label(panel.summary_text());
        if panel.running() {
            if ui.button("Stop").clicked() {
                out.push(FilesAction::Stop);
            }
        } else {
            ui.label("Replace with:");
            ui.add(egui::TextEdit::singleline(replace).desired_width(160.0));
            if ui
                .add_enabled(
                    panel.request.is_some() && !panel.files.is_empty(),
                    egui::Button::new("Replace in Files…"),
                )
                .clicked()
            {
                out.push(FilesAction::Replace(replace.clone()));
            }
        }
        if ui.button("Clear").clicked() {
            out.push(FilesAction::Clear);
        }
        if ui.button("×").on_hover_text("Close").clicked() {
            out.push(FilesAction::Close);
        }
    });
    let row_h = 18.0;
    let font = FontId::monospace(12.0);
    let count = panel.row_count();
    let id = egui::Id::unique("hk-find-files-list");
    let mut clicked_header = false;
    let mut area = egui::ScrollArea::vertical()
        .id_salt(id)
        .auto_shrink([false, false]);
    if let Some(offset) = panel.scroll_request.take() {
        area = area.vertical_scroll_offset(offset);
    }
    area.show_rows(ui, row_h, count, |ui, range| {
        for index in range {
            let Some(row) = panel.row(index) else {
                continue;
            };
            let (rect, resp) =
                ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click());
            let selected = panel.selected == Some(index);
            if selected {
                ui.painter()
                    .rect_filled(rect, CornerRadius::ZERO, pal.t_select);
            }
            let (indent, colour) = match row {
                ListRow::File(_) | ListRow::SkippedHeader => (4.0, pal.fg),
                ListRow::Hit(_) | ListRow::Skipped(_) => (24.0, pal.t_text),
            };
            let text = panel.row_text(row);
            let galley_pos = pos2(rect.left() + indent, rect.center().y);
            if let ListRow::Hit(k) = row {
                // The hit's range in the preview, highlighted.
                if let Some(h) = panel.hits.get(k as usize) {
                    let head = format!("{}:{}: ", h.line, h.column);
                    let preview = panel.preview(k as usize);
                    let before: String = preview.chars().take(h.range.0 as usize).collect();
                    let hit: String = preview
                        .chars()
                        .skip(h.range.0 as usize)
                        .take((h.range.1 - h.range.0) as usize)
                        .collect();
                    let w = |s: &str| {
                        ui.ctx().fonts_mut(|f| {
                            f.layout_no_wrap(s.to_owned(), font.clone(), colour)
                                .size()
                                .x
                        })
                    };
                    let x0 = galley_pos.x + w(&head) + w(&before);
                    let hl = Rect::from_min_size(
                        pos2(x0, rect.top() + 2.0),
                        vec2(w(&hit).max(2.0), row_h - 4.0),
                    );
                    ui.painter()
                        .rect_filled(hl, CornerRadius::ZERO, pal.find_hl);
                }
            }
            ui.painter().with_clip_rect(rect).text(
                galley_pos,
                Align2::LEFT_CENTER,
                text,
                font.clone(),
                colour,
            );
            if resp.clicked() {
                panel.selected = Some(index);
                if row == ListRow::SkippedHeader {
                    clicked_header = true;
                }
            }
            if resp.double_clicked()
                && let ListRow::Hit(k) = row
            {
                out.push(FilesAction::Open(k as usize));
            }
        }
    });
    if clicked_header {
        panel.skipped_open = !panel.skipped_open;
    }
    let (enter, copy, up, down) = ui.input(|i| {
        (
            i.key_pressed(Key::Enter),
            i.modifiers.command && i.key_pressed(Key::C),
            i.key_pressed(Key::ArrowUp),
            i.key_pressed(Key::ArrowDown),
        )
    });
    let hovered = ui.ui_contains_pointer();
    if hovered || panel.selected.is_some() {
        if down {
            panel.selected = Some(
                panel
                    .selected
                    .map_or(0, |s| (s + 1).min(count.saturating_sub(1))),
            );
        }
        if up {
            panel.selected = Some(panel.selected.map_or(0, |s| s.saturating_sub(1)));
        }
        if enter && let Some(ListRow::Hit(k)) = panel.selected.and_then(|s| panel.row(s)) {
            out.push(FilesAction::Open(k as usize));
        }
        if copy && hovered {
            out.push(FilesAction::Copy);
        }
    }
    out
}
