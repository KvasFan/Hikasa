//! The Folder Compare pane: base folders, column headers and the tree, custom-painted for the rows on screen
//! (FC-030…FC-032, FC-060…FC-062): colour spots and names per side, size and time columns, the centre state
//! mark, disclosure triangles, selection and keyboard navigation. The rows come from
//! [`FolderViewModel::window`]; the scroll position is a row index, so a million-row tree scrolls exactly.

use egui::{
    Align2, CornerRadius, EventFilter, FontId, Id, Key, Modifiers, Rect, Sense, Stroke, Ui, Vec2,
    pos2, vec2,
};
use hikasa_session::ipc::ViewId;

use crate::commands::{self, CommandId, KeyOrigin, Scope};
use crate::folder_view::{Cell, FolderStatus, FolderViewModel, ROW_HEIGHT, RowView};
use crate::ui::keys::shortcut_of;
use crate::ui::scroll;
use crate::ui::text::{hatch, triangle};
use crate::ui::theme::Palette;

/// Height of the base-folder strip and of the column header.
const STRIP_H: f32 = 24.0;
/// Width of the centre mark column.
const MARK_W: f32 = 34.0;
/// Width of the size column.
const SIZE_W: f32 = 92.0;
/// Width of the time column.
const TIME_W: f32 = 136.0;
/// Width of the scroll bar.
const BAR_W: f32 = 12.0;
/// Indentation per tree level.
const INDENT: f32 = 16.0;

/// UI state of one folder view.
#[derive(Default)]
pub struct FolderUi {
    /// First visible row (fractional).
    pub top: f64,
    scroll_to: Option<u32>,
    focused_once: bool,
    /// Visible rows in the view.
    pub total: u32,
    /// Whole rows that fit on screen.
    pub page_rows: f64,
}

impl FolderUi {
    /// Brings a row into view next frame (the selection moved).
    pub fn scroll_to(&mut self, row: u32) {
        self.scroll_to = Some(row);
    }

    /// The largest valid first row.
    pub fn max_top(&self) -> f64 {
        (f64::from(self.total) - self.page_rows).max(0.0)
    }
}

/// The id of a view's tree (it takes the keyboard focus).
pub fn tree_id(view: ViewId) -> Id {
    Id::unique(("hikasa-tree", view.0))
}

/// Draws the folder view of `view`; returns the commands its tree ran (Enter, Left, Right with the focus,
/// double-clicks), for the window to execute through the catalogue.
pub fn show(
    ui: &mut Ui,
    view: ViewId,
    m: &mut FolderViewModel,
    st: &mut FolderUi,
    status: &FolderStatus,
    enabled: &dyn Fn(CommandId) -> bool,
    pal: &Palette,
) -> Vec<CommandId> {
    let mut run = Vec::new();
    let full = ui.available_rect_before_wrap();
    ui.allocate_rect(full, Sense::hover());
    let painter = ui.painter_at(full);
    let body_font = FontId::proportional(13.0);
    let small = FontId::proportional(12.0);
    let mono_small = FontId::monospace(12.0);
    let content_right = full.right() - BAR_W;
    let side_w = ((content_right - full.left() - MARK_W) / 2.0)
        .floor()
        .max(40.0);
    let side_x = [full.left(), full.left() + side_w + MARK_W];

    // Base folders, then the column header.
    let bases = Rect::from_min_size(full.min, vec2(full.width(), STRIP_H));
    painter.rect_filled(bases, CornerRadius::ZERO, pal.chrome);
    for (k, base) in status.bases.iter().enumerate() {
        let cell = Rect::from_min_max(
            pos2(side_x[k], bases.top()),
            pos2(side_x[k] + side_w, bases.bottom()),
        );
        painter.with_clip_rect(cell).text(
            pos2(cell.left() + 8.0, cell.center().y),
            Align2::LEFT_CENTER,
            base,
            mono_small.clone(),
            pal.fg,
        );
    }
    let header = Rect::from_min_size(
        pos2(full.left(), bases.bottom()),
        vec2(full.width(), STRIP_H),
    );
    painter.rect_filled(header, CornerRadius::ZERO, pal.chrome);
    for line_y in [bases.bottom(), header.bottom()] {
        painter.hline(full.x_range(), line_y - 0.5, Stroke::new(1.0, pal.border));
    }
    for x in side_x {
        let right = x + side_w;
        let p = painter.with_clip_rect(Rect::from_min_max(
            pos2(x, header.top()),
            pos2(right, header.bottom()),
        ));
        let y = header.center().y;
        p.text(
            pos2(x + 8.0, y),
            Align2::LEFT_CENTER,
            "Name",
            small.clone(),
            pal.muted,
        );
        p.text(
            pos2(right - TIME_W - 8.0, y),
            Align2::RIGHT_CENTER,
            "Size",
            small.clone(),
            pal.muted,
        );
        p.text(
            pos2(right - 6.0, y),
            Align2::RIGHT_CENTER,
            "Modified (UTC)",
            small.clone(),
            pal.muted,
        );
    }
    let mut body_top = header.bottom();
    if let Some(error) = &status.error {
        let strip = Rect::from_min_size(pos2(full.left(), body_top), vec2(full.width(), 22.0));
        painter.rect_filled(strip, CornerRadius::ZERO, pal.error_bg);
        painter.with_clip_rect(strip).text(
            pos2(strip.left() + 8.0, strip.center().y),
            Align2::LEFT_CENTER,
            error,
            small.clone(),
            pal.error,
        );
        body_top = strip.bottom();
    }
    let body = Rect::from_min_max(pos2(full.left(), body_top), full.max);
    let tree = Rect::from_min_max(body.min, pos2(content_right, body.bottom()));
    painter.rect_filled(body, CornerRadius::ZERO, pal.bg);

    let rh = ROW_HEIGHT as f32;
    let page = f64::from((tree.height() / rh).floor().max(1.0));
    st.page_rows = page;
    let id = tree_id(view);
    let response = ui.interact(tree, id, Sense::click());
    if !st.focused_once {
        // The tree takes the focus when the view opens, so its keys work at once.
        st.focused_once = true;
        response.request_focus();
    }
    if response.hovered() {
        let delta = ui.input(|i| i.smooth_scroll_delta());
        if delta.y != 0.0 {
            st.top -= f64::from(delta.y / rh);
            ui.ctx().input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
        }
    }

    // Keyboard with the tree focused: arrows and pages move the selection; Enter, Left and Right run their
    // catalogue commands (focus-bound shortcuts, UI-003).
    if response.has_focus() {
        ui.memory_mut(|mem| {
            mem.set_focus_lock_filter(
                id,
                EventFilter {
                    vertical_arrows: true,
                    horizontal_arrows: true,
                    tab: false,
                    escape: false,
                },
            );
        });
        let step = (page - 1.0).max(1.0) as i64;
        let mut moves = Vec::new();
        ui.ctx().input_mut(|i| {
            i.events.retain(|event| {
                let egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } = event
                else {
                    return true;
                };
                if *modifiers == Modifiers::NONE {
                    let delta = match key {
                        Key::ArrowDown => 1,
                        Key::ArrowUp => -1,
                        Key::PageDown => step,
                        Key::PageUp => -step,
                        Key::End => i64::from(u32::MAX),
                        Key::Home => -i64::from(u32::MAX),
                        _ => 0,
                    };
                    if delta != 0 {
                        moves.push(delta);
                        return false;
                    }
                }
                let Some(shortcut) = shortcut_of(*key, *modifiers) else {
                    return true;
                };
                match commands::shortcut_command(Scope::Folder, shortcut, KeyOrigin::View) {
                    Some(c) if enabled(c.id) => {
                        run.push(c.id);
                        false
                    }
                    _ => true,
                }
            });
        });
        for delta in moves {
            m.move_selection(delta);
        }
        if let Some(row) = m.take_scroll() {
            st.scroll_to(row);
        }
    }

    // Scroll bar, then the scroll position.
    st.total = m.visible_count();
    let bar = Rect::from_min_max(pos2(content_right, body.top()), body.max);
    if let Some(top) = scroll::vertical(
        ui,
        id.with("bar"),
        bar,
        f64::from(st.total),
        page,
        st.top,
        pal,
    ) {
        st.top = top;
    }
    if let Some(row) = st.scroll_to.take() {
        let row = f64::from(row);
        if row < st.top {
            st.top = row;
        } else if row > st.top + page - 1.0 {
            st.top = row - page + 1.0;
        }
    }
    st.top = st.top.clamp(0.0, st.max_top());
    let first = st.top.floor() as u32;
    let frac = (st.top - st.top.floor()) as f32;
    m.set_scroll(first, page as u32 + 1);
    let window = m.window();

    // Clicks: select, open/close a folder (triangle or double-click), open a pair (double-click).
    let pointer_row = |p: egui::Pos2| -> Option<(usize, f32)> {
        let index = (st.top + f64::from((p.y - tree.top()) / rh)).floor();
        (index >= 0.0 && index < f64::from(window.total)).then_some((index as usize, p.x))
    };
    let hovered_row = response.hover_pos().and_then(pointer_row);
    if (response.clicked() || response.double_clicked())
        && let Some((index, x)) = response.interact_pointer_pos().and_then(pointer_row)
    {
        response.request_focus();
        let row = window.rows.iter().find(|r| r.index as usize == index);
        let on_triangle = row.is_some_and(|r| {
            r.folder
                && side_x.iter().any(|&sx| {
                    let tx = sx + 4.0 + r.depth as f32 * INDENT;
                    x >= tx - 2.0 && x <= tx + 16.0
                })
        });
        if response.double_clicked() {
            match row {
                Some(r) if r.folder => {
                    if !on_triangle {
                        m.toggle(index);
                    }
                }
                Some(_) => {
                    m.select(index);
                    run.push(CommandId::Open);
                }
                None => {}
            }
        } else if on_triangle {
            m.toggle(index);
        } else {
            m.select(index);
        }
    }
    let window = if response.clicked() || response.double_clicked() {
        m.window()
    } else {
        window
    };

    // Rows.
    let clip = painter.with_clip_rect(tree);
    for row in &window.rows {
        let y = tree.top() + (row.index as f32 - first as f32 - frac) * rh;
        if y + rh < tree.top() || y > tree.bottom() {
            continue;
        }
        let rect = Rect::from_min_max(pos2(tree.left(), y), pos2(tree.right(), y + rh));
        if row.selected {
            clip.rect_filled(rect, CornerRadius::ZERO, pal.select);
        } else if hovered_row.is_some_and(|(i, _)| i == row.index as usize) {
            clip.rect_filled(rect, CornerRadius::ZERO, pal.hover);
        }
        for (k, cell) in [row.left.as_ref(), row.right.as_ref()]
            .into_iter()
            .enumerate()
        {
            let side = Rect::from_min_max(pos2(side_x[k], y), pos2(side_x[k] + side_w, y + rh));
            draw_side(&clip, side, row, cell, &body_font, &small, pal);
        }
        clip.text(
            pos2(side_x[0] + side_w + MARK_W / 2.0, y + rh / 2.0),
            Align2::CENTER_CENTER,
            row.mark,
            body_font.clone(),
            pal.muted,
        );
    }
    // The centre column's separators.
    for x in [side_x[0] + side_w, side_x[1]] {
        clip.vline(
            x - 0.5,
            tree.y_range(),
            Stroke::new(1.0, pal.border.gamma_multiply(0.6)),
        );
    }
    if window.total == 0 {
        let text = if m.busy() {
            "Comparing…"
        } else {
            "No items are shown by the display filter."
        };
        painter.text(
            pos2(tree.center().x, tree.top() + 40.0),
            Align2::CENTER_CENTER,
            text,
            body_font.clone(),
            pal.muted,
        );
    }
    // Tooltips: the state of the hovered row and its errors.
    if let Some((index, _)) = hovered_row
        && let Some(row) = window.rows.iter().find(|r| r.index as usize == index)
    {
        let tip = if row.errors.is_empty() {
            row.mark_title.to_owned()
        } else {
            format!("{}\n{}", row.mark_title, row.errors)
        };
        response.on_hover_text_at_pointer(tip);
    }
    run
}

/// One side of a row: indentation, disclosure triangle, colour spot, name, size and time; hatched when the
/// item does not exist on this side.
fn draw_side(
    painter: &egui::Painter,
    rect: Rect,
    row: &RowView,
    cell: Option<&Cell>,
    font: &FontId,
    small: &FontId,
    pal: &Palette,
) {
    let Some(cell) = cell else {
        hatch(painter, rect, pal.hatch);
        return;
    };
    let y = rect.center().y;
    let indent = rect.left() + 4.0 + row.depth as f32 * INDENT;
    if row.folder {
        triangle(painter, pos2(indent + 7.0, y), 8.0, row.open, pal.muted);
    }
    let colour = pal.folder(cell.colour).unwrap_or(pal.fg);
    let spot_colour = if cell.colour == hikasa_folder::ColourClass::Same {
        colour.gamma_multiply(0.35)
    } else {
        colour
    };
    painter.circle_filled(pos2(indent + 20.0, y), 4.5, spot_colour);
    let time_x = rect.right() - 6.0;
    let size_x = rect.right() - TIME_W - 8.0;
    let name_rect = Rect::from_min_max(
        pos2(indent + 28.0, rect.top()),
        pos2(size_x - SIZE_W + 8.0, rect.bottom()),
    );
    if name_rect.width() > 4.0 {
        painter
            .with_clip_rect(name_rect.intersect(painter.clip_rect()))
            .text(
                pos2(name_rect.left(), y),
                Align2::LEFT_CENTER,
                &cell.name,
                font.clone(),
                colour,
            );
    }
    painter.text(
        pos2(size_x, y),
        Align2::RIGHT_CENTER,
        &cell.size,
        small.clone(),
        pal.muted,
    );
    painter.text(
        pos2(time_x, y),
        Align2::RIGHT_CENTER,
        &cell.modified,
        small.clone(),
        pal.muted,
    );
}
