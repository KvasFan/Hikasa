//! The Folder Compare pane: base folders, column headers and the tree, custom-painted for the rows on screen
//! (FC-030…FC-032, FC-060…FC-062): colour spots and names per side, size and time columns, the centre state
//! mark, disclosure triangles, selection and keyboard navigation. The rows come from
//! [`FolderViewModel::window`]; the scroll position is a row index, so a million-row tree scrolls exactly.

use egui::{
    Align2, Color32, CornerRadius, EventFilter, FontId, Id, Key, Modifiers, Rect, Sense, Stroke,
    Ui, Vec2, pos2, vec2,
};
use hikasa_session::ipc::ViewId;

use crate::commands::CommandId;
use crate::folder::columns::{self, ColumnId};
use crate::folder::glyphs::{Glyph, GlyphColour, GlyphView};
use crate::folder::selection::{Hit, Mods, Press, SWIPE_THRESHOLD, Swipe};
use crate::folder_view::{Cell, FolderStatus, FolderViewModel, ROW_HEIGHT, RowView};
use crate::ui::hint::Hint as _;
use crate::ui::icons::{self, IconId};
use crate::ui::keys::shortcuts_of;
use crate::ui::scroll;
use crate::ui::text::{hatch, triangle};
use crate::ui::theme::Palette;
use crate::window::Invocation;
use hikasa_session::settings::Shortcut;

/// Height of the base-folder strip and of the column header.
const STRIP_H: f32 = 24.0;
/// Width of the centre mark column.
const MARK_W: f32 = 34.0;
/// Width of the size column, at least.
const SIZE_W: f32 = 92.0;
/// Width of the time column, at least.
const TIME_W: f32 = 136.0;
/// What the size column has to hold: a thousand gigabytes less a byte.
const SIZE_SAMPLE: &str = "888\u{202f}888\u{202f}888\u{202f}888";
/// What the time column has to hold.
const TIME_SAMPLE: &str = "8888-88-88 88:88:88";
/// The time column's title (D-237: the clock's zone).
const TIME_TITLE: &str = "Modified";
/// Space left of a column's text.
const COLUMN_GAP: f32 = 8.0;
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
    /// A press on a row that may become a swipe (D-238): where it started and its swipe.
    press: Option<(egui::Pos2, Swipe)>,
    /// The press became a swipe (moved ≥ 4 points vertically).
    swiping: bool,
    /// A header drag under way (D-234): a column moved or a separator dragged.
    header_drag: Option<HeaderDrag>,
    /// The widest text of each column in the rows drawn last frame (a double-click on a separator fits to it).
    widest: std::collections::BTreeMap<ColumnId, f32>,
}

/// A drag in the column header (D-234).
#[derive(Debug, Clone, Copy, PartialEq)]
enum HeaderDrag {
    /// A header dragged: the column moves where it is dropped (Name stays first).
    Move(ColumnId),
    /// A column's left separator dragged: the column's width at the press and the press's x.
    Resize(ColumnId, f32, f32),
}

/// How near a column's left edge (points) a press grabs its separator.
const GRIP: f32 = 4.0;

/// What a press in the header at `x` grabs: a column's separator (the column whose left edge it is: the fixed
/// columns grow leftwards from the right edge) or a column's title. Name has no separator of its own.
fn header_grab(spans: &[(ColumnId, egui::Rangef)], x: f32) -> Option<(ColumnId, bool)> {
    if let Some((column, _)) = spans
        .iter()
        .find(|(c, span)| *c != ColumnId::Name && (x - span.min).abs() <= GRIP)
    {
        return Some((*column, true));
    }
    spans
        .iter()
        .find(|(_, span)| span.contains(x))
        .map(|(c, _)| (*c, false))
}

/// The index among the shown columns where a header dropped at `x` goes (D-234): before the first column whose
/// centre lies right of `x`.
fn drop_index(spans: &[(ColumnId, egui::Rangef)], x: f32) -> usize {
    spans
        .iter()
        .position(|(_, span)| x < span.center())
        .unwrap_or(spans.len())
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

/// The widths of the size and time columns in `font`: wider than the fixed ones for a font that needs it, so a large
/// folder font leaves its columns apart.
fn column_widths(painter: &egui::Painter, font: &FontId) -> Vec2 {
    let width = |text: &str| {
        painter
            .layout_no_wrap(text.to_owned(), font.clone(), Color32::PLACEHOLDER)
            .size()
            .x
    };
    vec2(
        SIZE_W.max(width(SIZE_SAMPLE) + COLUMN_GAP),
        TIME_W.max(width(TIME_SAMPLE).max(width(TIME_TITLE)) + COLUMN_GAP),
    )
}

/// A key of D-238's table other than a move.
enum KeyAct {
    Toggle,
    Mark,
    Restrict(hikasa_core::Side),
    Unmark,
}

/// The id of a view's tree (it takes the keyboard focus).
pub fn tree_id(view: ViewId) -> Id {
    Id::unique(("hikasa-tree", view.0))
}

#[allow(clippy::too_many_arguments)] // the view, its state, its texts, its keys and its look
/// Draws the folder view of `view`; returns the commands its tree ran (Enter, Left, Right with the focus,
/// double-clicks, the header's context menu `header_menu`), for the window to execute through the catalogue.
pub fn show(
    ui: &mut Ui,
    view: ViewId,
    m: &mut FolderViewModel,
    st: &mut FolderUi,
    status: &FolderStatus,
    dispatch: &dyn Fn(&Shortcut) -> Option<CommandId>,
    pal: &Palette,
    tree_font: &FontId,
    keys: bool,
    header_menu: &[crate::menus::MenuItem],
) -> Vec<Invocation> {
    let mut run: Vec<Invocation> = Vec::new();
    let full = ui.available_rect_before_wrap();
    ui.allocate_rect(full, Sense::hover());
    let painter = ui.painter_at(full);
    // The tree's font (`theme.toml [fonts.folder]`, D-113), kept within the fixed row height.
    let tree_size = tree_font.size.clamp(8.0, 17.0);
    let body_font = FontId::new(tree_size, tree_font.family.clone());
    let small = FontId::new((tree_size - 1.0).max(7.0), tree_font.family.clone());
    let mono_small = FontId::monospace(12.0);
    let columns = column_widths(&painter, &small);
    let content_right = full.right() - BAR_W;
    let side_w = ((content_right - full.left() - MARK_W) / 2.0)
        .floor()
        .max(40.0);
    let side_x = [full.left(), full.left() + side_w + MARK_W];

    // Base folders, then the column header.
    let bases = Rect::from_min_size(full.min, vec2(full.width(), STRIP_H));
    painter.rect_filled(bases, CornerRadius::ZERO, pal.chrome);
    let _ = &status.bases;
    for (k, x) in side_x.iter().enumerate() {
        let cell = Rect::from_min_max(pos2(*x, bases.top()), pos2(*x + side_w, bases.bottom()));
        path_edit(
            ui,
            view,
            m,
            k,
            cell.shrink2(vec2(4.0, 2.0)),
            &mono_small,
            pal,
            &mut run,
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
    // The header: the shown columns of each side (D-234), the sort arrow on the sorted side only (D-235).
    let shown = m.shown_columns();
    let sort = m.columns().sort;
    let mut header_hits: Vec<(Rect, ColumnId, hikasa_core::Side)> = Vec::new();
    for (k, x) in side_x.into_iter().enumerate() {
        let side = if k == 0 {
            hikasa_core::Side::Left
        } else {
            hikasa_core::Side::Right
        };
        let right = x + side_w;
        let p = painter.with_clip_rect(Rect::from_min_max(
            pos2(x, header.top()),
            pos2(right, header.bottom()),
        ));
        let y = header.center().y;
        for (column, cx) in column_spans(m, &shown, x, right, columns) {
            let arrow = match sort {
                Some(key) if key.column == column && key.side == side => {
                    if key.descending {
                        " ▼"
                    } else {
                        " ▲"
                    }
                }
                _ => "",
            };
            let title = if column == ColumnId::Modified {
                TIME_TITLE
            } else {
                columns::title(column)
            };
            p.text(
                pos2(cx.min + 8.0, y),
                Align2::LEFT_CENTER,
                format!("{title}{arrow}"),
                small.clone(),
                pal.muted,
            );
            header_hits.push((
                Rect::from_min_max(pos2(cx.min, header.top()), pos2(cx.max, header.bottom())),
                column,
                side,
            ));
        }
    }
    let header_response = ui.interact(
        header,
        tree_id(view).with("header"),
        Sense::click_and_drag(),
    );
    // The spans of the side under the pointer (both sides have the same ones, D-234).
    let side_spans =
        [0, 1].map(|k| column_spans(m, &shown, side_x[k], side_x[k] + side_w, columns));
    let spans_at = |x: f32| side_spans[usize::from(x >= side_x[1])].clone();
    let mut separator_hover = false;
    if let Some(at) = header_response.hover_pos()
        && matches!(header_grab(&spans_at(at.x), at.x), Some((_, true)))
    {
        separator_hover = true;
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    if header_response.drag_started()
        && let Some(at) = ui.input(|i| i.pointer.press_origin())
    {
        st.header_drag = match header_grab(&spans_at(at.x), at.x) {
            Some((column, true)) => {
                Some(HeaderDrag::Resize(column, m.columns().width(column), at.x))
            }
            Some((column, false)) if column != ColumnId::Name => Some(HeaderDrag::Move(column)),
            _ => None,
        };
    }
    if let (Some(HeaderDrag::Resize(column, width, from)), Some(at)) =
        (st.header_drag, header_response.interact_pointer_pos())
        && header_response.dragged()
    {
        // The fixed columns grow leftwards: dragging their left edge left widens them.
        m.change_columns(|c| c.resize(column, width + (from - at.x)));
    }
    if header_response.drag_stopped() {
        if let (Some(HeaderDrag::Move(column)), Some(at)) =
            (st.header_drag, header_response.interact_pointer_pos())
        {
            let index = drop_index(&spans_at(at.x), at.x);
            m.change_columns(|c| c.move_to(column, index));
        }
        st.header_drag = None;
    }
    if header_response.double_clicked()
        && let Some(at) = header_response.interact_pointer_pos()
        && let Some((column, true)) = header_grab(&spans_at(at.x), at.x)
    {
        let widest = st.widest.get(&column).copied().unwrap_or(0.0);
        m.change_columns(|c| c.fit(column, widest));
    } else if header_response.clicked()
        && !separator_hover
        && let Some(at) = header_response.interact_pointer_pos()
        && let Some((_, column, side)) = header_hits.iter().find(|(r, _, _)| r.contains(at))
    {
        m.header_click(*column, *side);
    }
    header_response.context_menu(|ui| {
        let mut hint = None;
        crate::ui::menus::items(ui, header_menu, &mut run, &mut hint);
    });
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
    painter.rect_filled(body, CornerRadius::ZERO, pal.f_bg);

    let rh = ROW_HEIGHT as f32;
    let page = f64::from((tree.height() / rh).floor().max(1.0));
    st.page_rows = page;
    let id = tree_id(view);
    let response = ui.interact(tree, id, Sense::click_and_drag());
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
    // `keys` is false while a dialog is open: the dialog has the keyboard.
    if keys && response.has_focus() {
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
        let mut acts = Vec::new();
        let marked = m.marked();
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
                // A binding of the key in the tree wins (D-111: overrides of focus-bound keys too).
                if let Some(cmd) = shortcuts_of(*key, *modifiers).iter().find_map(dispatch) {
                    run.push(Invocation::new(cmd));
                    return false;
                }
                // D-238's keyboard table: arrows, pages, Home/End move (Shift extends, Ctrl moves the current row
                // only); Ctrl+Space toggles; Space marks; Shift+Left/Right restrict to a side; Esc unmarks.
                let plain_or =
                    |m: Modifiers| m.is_none() || m == Modifiers::SHIFT || m.command_only();
                if plain_or(*modifiers) {
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
                        moves.push((
                            delta,
                            Mods {
                                ctrl: modifiers.command,
                                shift: modifiers.shift,
                            },
                        ));
                        return false;
                    }
                }
                let keyed = match (key, *modifiers) {
                    (Key::Space, m) if m.command_only() => Some(KeyAct::Toggle),
                    (Key::Space, m) if m.is_none() => Some(KeyAct::Mark),
                    (Key::ArrowLeft, Modifiers::SHIFT) => {
                        Some(KeyAct::Restrict(hikasa_core::Side::Left))
                    }
                    (Key::ArrowRight, Modifiers::SHIFT) => {
                        Some(KeyAct::Restrict(hikasa_core::Side::Right))
                    }
                    (Key::Escape, m) if m.is_none() && marked => Some(KeyAct::Unmark),
                    _ => None,
                };
                if let Some(act) = keyed {
                    acts.push(act);
                    return false;
                }
                true
            });
        });
        for (delta, mods) in moves {
            m.move_by(delta, mods);
        }
        for act in acts {
            match act {
                KeyAct::Toggle => m.toggle_current(),
                KeyAct::Mark => m.space(),
                KeyAct::Restrict(side) => m.restrict(side),
                KeyAct::Unmark => m.unmark(),
            }
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
    // CRC on demand (D-237): the frame time decides when the list has been still long enough.
    if let Some(after) = m.crc_frame(std::time::Instant::now()) {
        ui.ctx().request_repaint_after(after);
    }
    // Path completion (D-239): 150 ms after the last key, by the frame time.
    if let Some(after) = m.path_frame(std::time::Instant::now()) {
        ui.ctx().request_repaint_after(after);
    }
    let window = m.window();

    // Clicks: select, open/close a folder (triangle or double-click), open a pair (double-click).
    let top_now = st.top;
    let pointer_row = |p: egui::Pos2| -> Option<(usize, f32)> {
        let index = (top_now + f64::from((p.y - tree.top()) / rh)).floor();
        (index >= 0.0 && index < f64::from(window.total)).then_some((index as usize, p.x))
    };
    let hovered_row = response.hover_pos().and_then(pointer_row);
    let hit_at = |x: f32| -> Hit {
        if x < side_x[0] + side_w {
            Hit::Side(hikasa_core::Side::Left)
        } else if x < side_x[1] {
            Hit::Centre
        } else {
            Hit::Side(hikasa_core::Side::Right)
        }
    };
    let mods = ui.input(|i| Mods {
        ctrl: i.modifiers.command,
        shift: i.modifiers.shift,
    });
    // A press on an unselected row and a vertical drag of ≥ 4 points is a swipe (D-238); on a selected row a drag
    // of the selection (W9c).
    if response.drag_started()
        && let Some(at) = ui.input(|i| i.pointer.press_origin())
        && let Some((index, x)) = pointer_row(at)
    {
        let hit = hit_at(x);
        if m.selection_press(index, hit) == Press::Swipe {
            st.press = Some((at, Swipe::new(index, hit)));
            st.swiping = false;
        }
    }
    if response.dragged()
        && let (Some((start, mut swipe)), Some(at)) = (st.press, response.interact_pointer_pos())
    {
        if (at.y - start.y).abs() >= SWIPE_THRESHOLD {
            st.swiping = true;
        }
        if st.swiping {
            swipe.moved_over(hit_at(at.x));
            st.press = Some((start, swipe));
            let index = (st.top + f64::from((at.y - tree.top()) / rh))
                .floor()
                .max(0.0) as usize;
            m.swipe(&swipe, index);
            // The list scrolls at its edges.
            if at.y < tree.top() + rh {
                st.top -= 1.0;
            } else if at.y > tree.bottom() - rh {
                st.top += 1.0;
            }
        }
    }
    if response.drag_stopped() {
        st.press = None;
        st.swiping = false;
    }
    if (response.clicked() || response.double_clicked())
        && let Some(at) = response.interact_pointer_pos()
    {
        response.request_focus();
        if let Some((index, x)) = pointer_row(at) {
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
                        m.click(index, hit_at(x), Mods::default());
                        run.push(Invocation::new(CommandId::Open));
                    }
                    None => {}
                }
            } else if on_triangle {
                m.toggle(index);
            } else {
                m.click(index, hit_at(x), mods);
            }
        } else {
            // Below the last row: nothing selected.
            m.clear_selection();
        }
    }
    let window = if response.clicked() || response.double_clicked() {
        m.window()
    } else {
        window
    };

    // Rows.
    let clip = painter.with_clip_rect(tree);
    st.widest.clear();
    for row in &window.rows {
        let y = tree.top() + (row.index as f32 - first as f32 - frac) * rh;
        if y + rh < tree.top() || y > tree.bottom() {
            continue;
        }
        let rect = Rect::from_min_max(pos2(tree.left(), y), pos2(tree.right(), y + rh));
        if !row.selected && hovered_row.is_some_and(|(i, _)| i == row.index as usize) {
            clip.rect_filled(rect, CornerRadius::ZERO, pal.hover);
        }
        for (k, cell) in [row.left.as_ref(), row.right.as_ref()]
            .into_iter()
            .enumerate()
        {
            let side = Rect::from_min_max(pos2(side_x[k], y), pos2(side_x[k] + side_w, y + rh));
            let on = if k == 0 {
                hikasa_folder::select::SideSet::LEFT
            } else {
                hikasa_folder::select::SideSet::RIGHT
            };
            if row.sides.contains(on) {
                clip.rect_filled(side, CornerRadius::ZERO, pal.select);
            }
            let spans = column_spans(m, &shown, side.left(), side.right(), columns);
            for (column, w) in draw_side(&clip, side, row, cell, &spans, &body_font, &small, pal) {
                let widest = st.widest.entry(column).or_insert(0.0);
                *widest = widest.max(w);
            }
        }
        if row.current {
            clip.rect_stroke(
                rect.shrink(0.5),
                CornerRadius::ZERO,
                Stroke::new(1.0, pal.accent.gamma_multiply(0.6)),
                egui::StrokeKind::Inside,
            );
        }
        if row.centre.marked {
            clip.circle_stroke(
                pos2(side_x[0] + side_w + MARK_W / 2.0, y + rh / 2.0),
                rh / 2.0 - 2.0,
                Stroke::new(1.5, pal.accent),
            );
        }
        if row.centre.ignored {
            clip.text(
                pos2(side_x[1] - 4.0, y + 4.0),
                Align2::RIGHT_TOP,
                crate::folder::glyphs::IGNORED_BADGE,
                small.clone(),
                pal.muted,
            );
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
        response.hint_at_pointer(tip);
    }
    run
}

/// The horizontal span of each shown column of a side from `left` to `right` (D-234): the fixed-width columns from
/// the right edge, Name the rest. Both sides get the same spans.
fn column_spans(
    m: &FolderViewModel,
    shown: &[ColumnId],
    left: f32,
    right: f32,
    fixed: Vec2,
) -> Vec<(ColumnId, egui::Rangef)> {
    let layout = m.columns();
    let width = |c: ColumnId| match c {
        ColumnId::Size => layout.width(c).max(fixed.x),
        ColumnId::Modified => layout.width(c).max(fixed.y),
        other => layout.width(other),
    };
    let mut spans = Vec::new();
    let mut x = right;
    for &c in shown.iter().rev().filter(|c| **c != ColumnId::Name) {
        let w = width(c);
        spans.push((c, egui::Rangef::new((x - w).max(left), x)));
        x -= w;
    }
    spans.push((ColumnId::Name, egui::Rangef::new(left, x.max(left))));
    spans.reverse();
    spans
}

/// The glyph of a name cell (D-236): the colour spot, a folder (filled, hollow, unopened), the zipper; the red X and
/// the link arrow on top.
pub(crate) fn draw_glyph(
    painter: &egui::Painter,
    at: egui::Pos2,
    glyph: GlyphView,
    colour: Color32,
    pal: &Palette,
) {
    let tint = match glyph.colour {
        GlyphColour::Class => colour,
        GlyphColour::Unknown => pal.c_unknown,
        GlyphColour::Unopened => pal.c_unopened,
        GlyphColour::Error => pal.error,
    };
    let rect = icons::square(pos2(at.x - 7.0, at.y - 7.0), 14.0);
    match glyph.base {
        Glyph::Spot => {
            painter.circle_filled(at, 4.5, tint);
        }
        Glyph::Folder | Glyph::FolderFilled => icons::paint(painter, rect, IconId::Folder, tint),
        Glyph::FolderHollow => {
            painter.rect_stroke(
                rect.shrink(2.0),
                CornerRadius::same(2),
                Stroke::new(1.2, tint),
                egui::StrokeKind::Inside,
            );
        }
        Glyph::Archive => icons::paint(painter, rect, IconId::Zipper, tint),
    }
    if glyph.error {
        let stroke = Stroke::new(1.6, pal.error);
        painter.line_segment(
            [
                rect.left_top() + vec2(3.0, 3.0),
                rect.right_bottom() - vec2(3.0, 3.0),
            ],
            stroke,
        );
        painter.line_segment(
            [
                rect.right_top() + vec2(-3.0, 3.0),
                rect.left_bottom() + vec2(3.0, -3.0),
            ],
            stroke,
        );
    }
    if glyph.link {
        let stroke = Stroke::new(1.2, pal.f_text);
        let base = rect.left_bottom() + vec2(1.0, -1.0);
        painter.line_segment([base, base + vec2(5.0, -5.0)], stroke);
        painter.line_segment([base + vec2(5.0, -5.0), base + vec2(1.5, -5.0)], stroke);
        painter.line_segment([base + vec2(5.0, -5.0), base + vec2(5.0, -1.5)], stroke);
    }
}

#[allow(clippy::too_many_arguments)] // the row, its place and its look
/// One side of a row: indentation, disclosure triangle, glyph, name and the other shown columns; hatched when the
/// item does not exist on this side.
fn draw_side(
    painter: &egui::Painter,
    rect: Rect,
    row: &RowView,
    cell: Option<&Cell>,
    spans: &[(ColumnId, egui::Rangef)],
    font: &FontId,
    small: &FontId,
    pal: &Palette,
) -> Vec<(ColumnId, f32)> {
    let mut widths = Vec::new();
    let Some(cell) = cell else {
        hatch(painter, rect, pal.hatch);
        return widths;
    };
    let y = rect.center().y;
    let indent = rect.left() + 4.0 + row.depth as f32 * INDENT;
    if row.folder {
        triangle(painter, pos2(indent + 7.0, y), 8.0, row.open, pal.muted);
    }
    let colour = pal.folder(cell.colour, row.category).unwrap_or(pal.f_text);
    let spot_colour = if cell.colour == hikasa_folder::ColourClass::Same {
        colour.gamma_multiply(0.35)
    } else {
        colour
    };
    draw_glyph(
        painter,
        pos2(indent + 20.0, y),
        cell.glyph,
        spot_colour,
        pal,
    );
    let ext_shown = spans.iter().any(|(c, _)| *c == ColumnId::Ext);
    for (column, span) in spans {
        let text = cell.text(*column, ext_shown);
        if *column == ColumnId::Name {
            let name_rect = Rect::from_min_max(
                pos2(indent + 28.0, rect.top()),
                pos2(span.max - 4.0, rect.bottom()),
            );
            if name_rect.width() > 4.0 {
                painter
                    .with_clip_rect(name_rect.intersect(painter.clip_rect()))
                    .text(
                        pos2(name_rect.left(), y),
                        Align2::LEFT_CENTER,
                        text,
                        font.clone(),
                        colour,
                    );
            }
            continue;
        }
        let cell_rect = Rect::from_min_max(
            pos2(span.min, rect.top()),
            pos2(span.max - 6.0, rect.bottom()),
        );
        let (anchor, x) = if matches!(
            column,
            ColumnId::Size | ColumnId::Owner | ColumnId::Group | ColumnId::Modified
        ) {
            (Align2::RIGHT_CENTER, cell_rect.right())
        } else {
            (Align2::LEFT_CENTER, cell_rect.left() + 6.0)
        };
        let drawn = painter
            .with_clip_rect(cell_rect.intersect(painter.clip_rect()))
            .text(pos2(x, y), anchor, text, small.clone(), pal.muted);
        widths.push((*column, drawn.width()));
    }
    widths
}

/// The path edit of side `k` above its pane (D-239): the field, the recent specs, Accept and Cancel while the text
/// differs from the committed spec, Browse; the completion list below it; the message under it.
#[allow(clippy::too_many_arguments)] // the field, its place and its look
fn path_edit(
    ui: &mut Ui,
    view: ViewId,
    m: &mut FolderViewModel,
    k: usize,
    rect: Rect,
    font: &FontId,
    pal: &Palette,
    run: &mut Vec<Invocation>,
) {
    use crate::path_edit::{PathKey, PathKeyOutcome};
    let now = std::time::Instant::now();
    let id = egui::Id::unique(("hk-path", view.0, k));
    let recent_open = id.with("recent-open");
    let focused = ui.memory(|mem| mem.has_focus(id));
    if focused {
        for (key, pk) in [
            (egui::Key::ArrowDown, PathKey::Down),
            (egui::Key::ArrowUp, PathKey::Up),
            (egui::Key::Tab, PathKey::Tab),
            (egui::Key::Enter, PathKey::Enter),
            (egui::Key::Escape, PathKey::Esc),
        ] {
            let open = m.path_edit(k).open;
            let ours = open || matches!(pk, PathKey::Enter | PathKey::Esc);
            if ours
                && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, key))
                && m.path_key(k, pk, now) == PathKeyOutcome::Accept
            {
                ui.memory_mut(|mem| mem.surrender_focus(id));
            }
        }
    }
    let changed = m.path_edit(k).changed();
    let buttons = if changed { 4.0 } else { 2.0 } * 24.0;
    let field = Rect::from_min_max(
        rect.min,
        pos2(
            (rect.right() - buttons).max(rect.left() + 40.0),
            rect.bottom(),
        ),
    );
    let edit = m.path_edit(k);
    let before = edit.text.clone();
    let response = ui.put(
        field,
        egui::TextEdit::singleline(&mut edit.text)
            .id(id)
            .font(font.clone())
            .text_color(pal.fg)
            .margin(vec2(4.0, 2.0)),
    );
    if edit.text != before {
        edit.edited(now);
    }
    if let Some(error) = &edit.error {
        response.clone().hint(error.clone());
        ui.painter().rect_stroke(
            field,
            CornerRadius::same(2),
            Stroke::new(1.0, pal.error),
            egui::StrokeKind::Inside,
        );
    }
    let mut x = field.right() + 2.0;
    let mut button = |ui: &mut Ui, text: &str, hint: &str| {
        let r = Rect::from_min_size(pos2(x, rect.top()), vec2(22.0, rect.height()));
        x += 24.0;
        ui.put(r, egui::Button::new(text).small())
            .hint(hint)
            .clicked()
    };
    let recent = edit.recent.clone();
    let recent_button = button(ui, "▾", "Recent folders");
    if recent_button && !recent.is_empty() {
        let open = ui
            .data(|d| d.get_temp::<bool>(recent_open))
            .unwrap_or(false);
        ui.data_mut(|d| d.insert_temp(recent_open, !open));
    }
    if changed {
        if button(ui, "✓", "Accept (Enter)") {
            m.accept_paths();
        }
        if button(ui, "✕", "Cancel (Esc)") {
            m.path_edit(k).cancel();
        }
    }
    if button(ui, "…", "Browse") {
        run.push(Invocation::new(if k == 0 {
            CommandId::BrowseLeft
        } else {
            CommandId::BrowseRight
        }));
    }
    let below = pos2(field.left(), field.bottom() + 1.0);
    if ui
        .data(|d| d.get_temp::<bool>(recent_open))
        .unwrap_or(false)
    {
        let mut close = false;
        egui::Area::new(id.with("recent-area"))
            .fixed_pos(below)
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    for text in &recent {
                        if ui.selectable_label(false, text).clicked() {
                            m.path_edit(k).set_text(text);
                            close = true;
                        }
                    }
                });
            });
        if close || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            ui.data_mut(|d| d.insert_temp(recent_open, false));
        }
    }
    let edit = m.path_edit(k);
    if edit.open && focused {
        let mut take = None;
        egui::Area::new(id.with("completions"))
            .fixed_pos(below)
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_min_width(field.width());
                    egui::ScrollArea::vertical()
                        .max_height(240.0)
                        .show(ui, |ui| {
                            for (i, item) in edit.completions.iter().enumerate() {
                                let mark = if item.archive {
                                    "▣ "
                                } else if item.folder {
                                    "▸ "
                                } else {
                                    "  "
                                };
                                let label = format!("{mark}{}", item.name);
                                if ui
                                    .selectable_label(edit.highlighted == Some(i), label)
                                    .clicked()
                                {
                                    take = Some(i);
                                }
                            }
                        });
                });
            });
        if let Some(i) = take {
            m.path_edit(k).take(i, now);
            ui.memory_mut(|mem| mem.request_focus(id));
        }
    }
}
