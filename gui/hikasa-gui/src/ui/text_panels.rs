//! The panels of a text view (M2 W11b; contract §1.11.15, §1.11.16): the Find and Replace panel above the panes, the
//! details panel below them (Text, Hex and Alignment Details) and the ruler above each pane. Every value is the
//! model's ([`crate::text::find`], [`crate::text::details`]); this file lays them out and turns clicks and keys into
//! model calls.

use egui::{
    Align2, Color32, CornerRadius, FontFamily, FontId, Key, Painter, Rect, Sense, Stroke, Ui, pos2,
    vec2,
};
use hikasa_core::Side;
use hikasa_session::ipc::ViewId;
use hikasa_text::details::CellClass;
use hikasa_text::find::FindSides;

use crate::commands::CommandId;
use crate::text::find::FindAction;
use crate::text::layout::{Details, FindShown};
use crate::text::{TextViewModel, details, find, side_of};
use crate::ui::theme::Palette;

/// The Find (and Replace) panel (D-307): the query, its options, the sides (Text Compare), *Close automatically*, *Select
/// All*, ▲ ▼; with Replace, *Replace with*, *Replace*, *Replace All*. Enter is Find Next, Shift+Enter Find Previous, Esc
/// closes. Returns the commands it ran (F3's); the panel's own buttons are model actions run at the next poll.
pub fn find_panel(ui: &mut Ui, m: &mut TextViewModel, rect: Rect, pal: &Palette) -> Vec<CommandId> {
    let out = Vec::new();
    ui.painter()
        .rect_filled(rect, CornerRadius::ZERO, pal.chrome);
    ui.painter().hline(
        rect.x_range(),
        rect.bottom() - 0.5,
        Stroke::new(1.0, pal.border),
    );
    let replace = m.display().find_panel == FindShown::Replace;
    let edit = m.is_edit();
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(vec2(6.0, 3.0))));
    let ui = &mut child;
    ui.horizontal(|ui| {
        ui.label("Find:");
        let field = ui.add(
            egui::TextEdit::singleline(&mut m.find.query.text)
                .desired_width(220.0)
                .id_salt("text-find"),
        );
        if std::mem::take(&mut m.find.focus) {
            field.request_focus();
        }
        if field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
            let back = ui.input(|i| i.modifiers.shift);
            m.find.ask = Some(if back {
                FindAction::Prev
            } else {
                FindAction::Next
            });
            field.request_focus();
        }
        if field.has_focus() && ui.input(|i| i.key_pressed(Key::Escape)) {
            find::close(m);
        }
        ui.checkbox(&mut m.find.query.match_case, "Match case");
        ui.checkbox(&mut m.find.query.whole_word, "Whole words");
        ui.checkbox(&mut m.find.query.regex, "Regular expression");
        ui.checkbox(&mut m.find.wrap, "Wrap around");
        if !edit {
            for (sides, label) in [
                (FindSides::Left, "Left"),
                (FindSides::Right, "Right"),
                (FindSides::Both, "Both"),
            ] {
                ui.radio_value(&mut m.find.sides, sides, label);
            }
        }
        ui.checkbox(&mut m.find.close_automatically, "Close automatically");
        if ui.button("Select All").clicked() {
            m.find.ask = Some(FindAction::SelectAll);
        }
        if ui
            .button("▲")
            .on_hover_text("Find Previous (Shift+F3)")
            .clicked()
        {
            m.find.ask = Some(FindAction::Prev);
        }
        if ui.button("▼").on_hover_text("Find Next (F3)").clicked() {
            m.find.ask = Some(FindAction::Next);
        }
        if ui.button("×").on_hover_text("Close (Esc)").clicked() {
            find::close(m);
        }
        if let Some(e) = &m.find.error {
            ui.colored_label(pal.error, e);
        } else if let Some(note) = &m.find.note {
            ui.colored_label(pal.muted, note);
        }
    });
    if replace {
        ui.horizontal(|ui| {
            ui.label("Replace with:");
            ui.add(
                egui::TextEdit::singleline(&mut m.find.replace)
                    .desired_width(220.0)
                    .id_salt("text-replace"),
            );
            if ui.button("Replace").clicked() {
                m.find.ask = Some(FindAction::Replace);
            }
            if ui.button("Replace All").clicked() {
                m.find.ask = Some(FindAction::ReplaceAll);
            }
        });
    }
    out
}

/// The details panel (D-306): the current row's Text, Hex or Alignment Details, and *Lock Visible Whitespace*.
pub fn details_panel(
    ui: &mut Ui,
    view: ViewId,
    m: &mut TextViewModel,
    rect: Rect,
    pal: &Palette,
    family: &FontFamily,
) {
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::ZERO, pal.t_bg);
    let font = FontId::new(13.0, family.clone());
    let small = FontId::proportional(11.0);
    let kind = m.display().details;
    let title = match kind {
        Details::Text => "Text Details",
        Details::Hex => "Hex Details",
        Details::Alignment => "Alignment Details",
        Details::None => return,
    };
    let head = Rect::from_min_size(rect.min, vec2(rect.width(), 20.0));
    painter.rect_filled(head, CornerRadius::ZERO, pal.chrome);
    painter.text(
        pos2(head.left() + 6.0, head.center().y),
        Align2::LEFT_CENTER,
        title,
        small.clone(),
        pal.fg,
    );
    let lock = Rect::from_min_size(pos2(head.right() - 170.0, head.top()), vec2(170.0, 20.0));
    let mut on = m.display().details_whitespace;
    if ui
        .put(
            lock,
            egui::Checkbox::new(&mut on, "Lock Visible Whitespace"),
        )
        .changed()
    {
        m.display_mut().details_whitespace = on;
    }
    let body = Rect::from_min_max(pos2(rect.left(), head.bottom()), rect.max);
    let marks = m.display().whitespace || m.display().details_whitespace;
    let data = details::data(m);
    let lh = 18.0;
    let sides: Vec<usize> = if m.is_edit() { vec![0] } else { vec![0, 1] };
    match kind {
        Details::Text => {
            for (n, k) in sides.iter().enumerate() {
                let y = body.top() + 4.0 + n as f32 * lh;
                let text = data.lines[*k].as_ref().map_or("", |l| l.1.as_str());
                let shown = if marks {
                    show_blanks(text)
                } else {
                    text.to_owned()
                };
                let row =
                    Rect::from_min_size(pos2(body.left() + 6.0, y), vec2(body.width() - 12.0, lh));
                painter.text(
                    row.left_center(),
                    Align2::LEFT_CENTER,
                    &shown,
                    font.clone(),
                    pal.t_text,
                );
                let r = ui.interact(
                    row,
                    egui::Id::unique(("text-details", view.0, *k)),
                    Sense::click(),
                );
                if r.clicked()
                    && let Some(p) = r.interact_pointer_pos()
                {
                    // The byte under the pointer (whole characters, the panel's font).
                    let x = p.x - row.left();
                    let mut byte = 0u32;
                    let mut w = 0.0;
                    for (b, c) in text.char_indices() {
                        let cw = ui.ctx().fonts_mut(|f| f.glyph_width(&font, c));
                        if w + cw / 2.0 > x {
                            byte = b as u32;
                            break;
                        }
                        w += cw;
                        byte = (b + c.len_utf8()) as u32;
                    }
                    details::text_press(m, side_of(*k), byte);
                }
            }
        }
        Details::Hex => {
            let mut y = body.top() + 4.0;
            for k in &sides {
                let Some(hex) = &data.hex[*k] else {
                    continue;
                };
                for row in details::hex_rows(hex) {
                    if y > body.bottom() {
                        break;
                    }
                    let line = format!(
                        "{:08X}  {:<48}  {}",
                        row.offset,
                        row.hex_text(),
                        row.chars_text()
                    );
                    let colour = if row.unmappable.iter().any(|u| *u) {
                        pal.imp_fg
                    } else {
                        pal.t_text
                    };
                    let r = painter.text(
                        pos2(body.left() + 6.0, y),
                        Align2::LEFT_TOP,
                        line,
                        font.clone(),
                        colour,
                    );
                    if row.unmappable.iter().any(|u| *u) {
                        ui.interact(
                            r,
                            egui::Id::unique(("hex-unmappable", view.0, *k, row.offset)),
                            Sense::hover(),
                        )
                        .on_hover_text("written as ?");
                    }
                    y += lh;
                }
                if !hex.eol.is_empty() {
                    let eol: Vec<String> = hex.eol.iter().map(|b| format!("{b:02X}")).collect();
                    painter.text(
                        pos2(body.left() + 6.0, y),
                        Align2::LEFT_TOP,
                        format!("          {}", eol.join(" ")),
                        font.clone(),
                        pal.muted,
                    );
                    y += lh;
                }
                if hex.truncated {
                    painter.text(
                        pos2(body.left() + 6.0, y),
                        Align2::LEFT_TOP,
                        "(truncated)",
                        small.clone(),
                        pal.muted,
                    );
                    y += lh;
                }
                y += lh / 2.0;
            }
        }
        Details::Alignment => {
            let Some(cells) = data.cells.clone() else {
                painter.text(
                    body.left_top() + vec2(6.0, 6.0),
                    Align2::LEFT_TOP,
                    "No row is current.",
                    small,
                    pal.muted,
                );
                return;
            };
            let cw = ui.ctx().fonts_mut(|f| f.glyph_width(&font, 'M')).max(6.0) + 2.0;
            let id = egui::Id::unique(("align-details", view.0));
            let state_id = id.with("cell");
            let mut caret: Option<(Side, usize)> = ui.data(|d| d.get_temp(state_id));
            for (n, k) in [0usize, 1].into_iter().enumerate() {
                let y = body.top() + 6.0 + n as f32 * (lh + 4.0);
                for (i, cell) in cells.iter().enumerate() {
                    let x = body.left() + 6.0 + i as f32 * cw;
                    if x > body.right() {
                        break;
                    }
                    let c = if k == 0 { cell.left } else { cell.right };
                    let r = Rect::from_min_size(pos2(x, y), vec2(cw, lh));
                    let bg = match cell.class {
                        CellClass::Same => None,
                        CellClass::Changed => Some(pal.imp_hl),
                        CellClass::Unimportant => Some(pal.unimp_hl),
                        CellClass::LeftOnly | CellClass::RightOnly => Some(pal.orph_bg),
                    };
                    if let Some(bg) = bg {
                        painter.rect_filled(r, CornerRadius::ZERO, bg);
                    }
                    if let Some(c) = c {
                        let shown = if marks && c == ' ' { '·' } else { c };
                        painter.text(
                            r.center(),
                            Align2::CENTER_CENTER,
                            shown,
                            font.clone(),
                            pal.t_text,
                        );
                    }
                    if caret == Some((side_of(k), i)) {
                        painter.vline(r.left(), r.y_range(), Stroke::new(2.0, pal.t_text));
                    }
                    if ui.interact(r, id.with((k, i)), Sense::click()).clicked() {
                        caret = Some((side_of(k), i));
                    }
                }
            }
            // A key at a cell edits that cell's side (D-306's open default); arrows move by cells.
            if let Some((side, i)) = caret {
                let events = ui.input(|inp| inp.events.clone());
                for e in events {
                    match e {
                        egui::Event::Text(t) => {
                            if details::align_type(m, side, i, &t) {
                                caret = Some((side, i + t.chars().count()));
                            }
                        }
                        egui::Event::Key {
                            key: Key::Backspace,
                            pressed: true,
                            ..
                        } => {
                            if details::align_delete(m, side, i, true) {
                                caret = Some((side, i.saturating_sub(1)));
                            }
                        }
                        egui::Event::Key {
                            key: Key::Delete,
                            pressed: true,
                            ..
                        } => {
                            details::align_delete(m, side, i, false);
                        }
                        egui::Event::Key {
                            key: Key::ArrowLeft,
                            pressed: true,
                            ..
                        } => {
                            caret = Some((side, i.saturating_sub(1)));
                        }
                        egui::Event::Key {
                            key: Key::ArrowRight,
                            pressed: true,
                            ..
                        } => {
                            caret = Some((side, (i + 1).min(cells.len())));
                        }
                        egui::Event::Key {
                            key: Key::Escape,
                            pressed: true,
                            ..
                        } => caret = None,
                        _ => {}
                    }
                }
            }
            ui.data_mut(|d| d.insert_temp(state_id, caret));
        }
        Details::None => {}
    }
}

/// A line with its blanks shown (`·` a space, `→` a tab): Lock Visible Whitespace of the Text Details (display only).
fn show_blanks(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            ' ' => '·',
            '\t' => '→',
            c => c,
        })
        .collect()
}

/// The ruler above a pane (D-306): a tick every display column, a label every ten, the tab stops marked and the caret's
/// column; scrolled with the pane.
#[allow(clippy::too_many_arguments)]
pub fn ruler(
    painter: &Painter,
    m: &TextViewModel,
    side: Side,
    rect: Rect,
    text_x: f32,
    scroll: f32,
    cw: f32,
    pal: &Palette,
) {
    let p = painter.with_clip_rect(rect);
    p.rect_filled(rect, CornerRadius::ZERO, pal.chrome);
    let first = (scroll / cw).floor() as u32 + 1;
    let last = first + ((rect.width() / cw).ceil() as u32).max(1);
    let small = FontId::proportional(9.0);
    let x_of = |col: u32| text_x - scroll + (col - 1) as f32 * cw;
    for mark in details::ruler_marks(m, side, first, last) {
        let x = x_of(mark.column);
        let h = if mark.label.is_some() {
            7.0
        } else if mark.tab_stop {
            5.0
        } else {
            2.0
        };
        p.vline(
            x,
            rect.bottom() - h..=rect.bottom(),
            Stroke::new(1.0, pal.muted),
        );
        if let Some(label) = mark.label {
            p.text(
                pos2(x + 1.0, rect.top()),
                Align2::LEFT_TOP,
                label.to_string(),
                small.clone(),
                pal.muted,
            );
        }
    }
    if let Some(col) = details::caret_column(m, side) {
        let x = x_of(col);
        p.rect_filled(
            Rect::from_min_max(pos2(x, rect.top() + 2.0), pos2(x + cw, rect.bottom())),
            CornerRadius::ZERO,
            Color32::from_rgba_unmultiplied(pal.accent.r(), pal.accent.g(), pal.accent.b(), 60),
        );
    }
}
