//! The File Info header of a text pane (M2 W11a, contract §1.11.11, D-302): the path edit (Accept reopens that side,
//! the window asks D-301 first), the encoding menu (*Reopen with Encoding*, *Save with Encoding*, *Byte Order
//! Mark*), the line-ending menu (the conversions), the format picker (a relive keeping the edits) and the badges.
//! The texts and the actions are the model's ([`crate::text::header`]); this module only draws them.

use egui::{Rect, RichText, Ui, UiBuilder};
use hikasa_core::Side;
use hikasa_session::ipc::ViewId;
use hikasa_text::LineEnding;

use crate::text::header::{self, Badge, ENCODINGS};
use crate::text::{PaneInfo, TextViewModel, edit, index_of};
use crate::ui::theme::Palette;

/// Draws the header of `side` into `cell`.
pub fn show(
    ui: &mut Ui,
    view: ViewId,
    m: &mut TextViewModel,
    side: Side,
    cell: Rect,
    info: &PaneInfo,
    pal: &Palette,
) {
    let Some(items) = header::items(m, side) else {
        // Before the documents exist: the M1 title line.
        let painter = ui.painter_at(cell);
        painter.text(
            egui::pos2(cell.left() + 8.0, cell.center().y),
            egui::Align2::LEFT_CENTER,
            format!("{}   {}", info.title, info.detail),
            egui::FontId::monospace(12.0),
            pal.fg,
        );
        return;
    };
    let k = index_of(side);
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(cell.shrink2(egui::vec2(4.0, 1.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center))
            .id_salt(("text-header", view.0, k)),
    );
    let ui = &mut child;
    ui.spacing_mut().item_spacing.x = 6.0;
    let small = |t: &str| RichText::new(t).size(12.0);

    // The path edit (D-239's edit; Enter accepts and reopens that side).
    let mut text = m.headers[k]
        .path
        .clone()
        .unwrap_or_else(|| items.path.clone());
    let width = (cell.width() * 0.42).max(60.0);
    let edit_resp = ui.add(
        egui::TextEdit::singleline(&mut text)
            .font(egui::FontId::monospace(12.0))
            .desired_width(width),
    );
    if edit_resp.changed() || edit_resp.has_focus() {
        m.headers[k].path = Some(text.clone());
    }
    if edit_resp.lost_focus() {
        let accepted = ui.input(|i| i.key_pressed(egui::Key::Enter));
        let typed = m.headers[k].path.take();
        if accepted
            && let Some(typed) = typed.filter(|t| t.trim() != items.path && !t.trim().is_empty())
        {
            m.request_reopen(side, Some(std::ffi::OsString::from(typed.trim())), None);
        }
    }
    edit_resp.on_hover_text("The file of this pane: Enter opens the typed one");

    // The encoding.
    ui.menu_button(small(&items.encoding), |ui| {
        ui.menu_button("Reopen with Encoding", |ui| {
            if ui.button("Detect").clicked() {
                m.request_reopen(side, None, Some(None));
                ui.close();
            }
            for label in ENCODINGS {
                if ui.button(*label).clicked() {
                    m.request_reopen(side, None, Some(Some((*label).to_owned())));
                    ui.close();
                }
            }
        });
        ui.menu_button("Save with Encoding", |ui| {
            for label in ENCODINGS {
                if ui.button(*label).clicked() {
                    header::save_with_encoding(m, side, label);
                    ui.close();
                }
            }
        });
        if items.utf {
            let mut bom = items.bom;
            if ui.checkbox(&mut bom, "Byte Order Mark").clicked() {
                header::toggle_bom(m, side);
                ui.close();
            }
        }
    });
    // The line endings.
    ui.menu_button(small(items.eol), |ui| {
        for (label, eol) in [
            ("To Windows/DOS", LineEnding::CrLf),
            ("To Unix", LineEnding::Lf),
            ("To Macintosh", LineEnding::Cr),
        ] {
            if ui.button(label).clicked() {
                header::set_line_endings(m, side, eol);
                ui.close();
            }
        }
    });
    // The format (FF-003): a relive keeping the edits.
    let names = m.format_names().to_vec();
    ui.menu_button(small(&items.format), |ui| {
        if ui.button("Detect").clicked() {
            m.request_format(side, None);
            ui.close();
        }
        egui::ScrollArea::vertical()
            .max_height(320.0)
            .show(ui, |ui| {
                for name in &names {
                    if ui.button(name).clicked() {
                        m.request_format(side, Some(name.clone()));
                        ui.close();
                    }
                }
            });
    });
    // The badges.
    for badge in &items.badges {
        match badge {
            Badge::Blocked {
                text,
                tooltip,
                binary,
            } => {
                let r = ui
                    .label(RichText::new(*text).size(11.0).color(pal.warning))
                    .on_hover_text(tooltip);
                if *binary {
                    let edit_anyway = ui.small_button("Edit anyway");
                    if edit_anyway.clicked() {
                        edit::answer_binary(m, side, true);
                    }
                    // M2 W13c (D-369): the binary pair in a Hex Compare view.
                    if !m.is_edit()
                        && !m.is_text_pair()
                        && ui.small_button("Compare in Hex").clicked()
                    {
                        m.compare_in_hex();
                    }
                }
                let _ = r;
            }
            Badge::Converted => {
                ui.label(RichText::new("Converted").size(11.0).color(pal.muted))
                    .on_hover_text("The format converts this file for the comparison");
            }
            Badge::Modified => {
                ui.label(RichText::new("Modified").size(11.0).color(pal.t_edited));
            }
        }
    }
}
