//! The Table Compare grid (TB-020…TB-024; ARCH-GUI-002): pane headers, the thumbnail, the two grids (side by side or
//! over-under, T-55) with their column heads and row numbers, the scroll bar, the Text Details panel and the sheet
//! tabs, custom-painted with egui's painter for the cells on screen only. Rows, columns and cells come from
//! [`TableViewModel`]; the scroll position is a row index (`f64`), so a million-row sheet scrolls exactly.

use egui::{
    Align2, Color32, CornerRadius, CursorIcon, EventFilter, FontFamily, FontId, Id, Key, Painter,
    Rect, Sense, Stroke, Ui, Vec2, pos2, vec2,
};
use hikasa_core::Side;
use hikasa_session::ipc::ViewId;
use hikasa_table::{CellState, Importance, RowKind, SheetState};

use crate::commands::CommandId;
use crate::table_view::{GridLayout, TablePhase, TableViewModel};
use crate::ui::scroll;
use crate::ui::text::hatch;
use crate::ui::theme::Palette;

/// Height of the pane header strip.
const HEADER_H: f32 = 24.0;
/// Height of the sheet tab strip.
const TABS_H: f32 = 26.0;
/// Width of the thumbnail column.
const THUMB_W: f32 = 14.0;
/// Width of the scroll bar.
const BAR_W: f32 = 12.0;
/// Padding inside a cell.
const PAD: f32 = 4.0;
/// Half the width of the grip on a head's right edge that resizes the column.
const GRIP: f32 = 4.0;
/// Most lines of the Text Details panel.
const DETAIL_LINES: usize = 8;

/// UI state of one table view (kept by the window between frames).
#[derive(Default)]
pub struct TableUi {
    /// First visible position (fractional: smooth scrolling).
    pub top: f64,
    /// Horizontal offset of the grids (points).
    pub left: f32,
    focused_once: bool,
    /// The column being resized: comparison column, pointer x and width (chars) at the start.
    drag: Option<(u32, f32, f32)>,
    thumb: Option<(u64, u32, Vec<u8>)>,
    /// Rows shown.
    pub len: u32,
    /// Whole rows that fit in a pane.
    pub page_rows: f64,
    /// Frames drawn with rows on screen (the benchmark's first-screen mark).
    pub drawn: u64,
    /// The head's context menu: the comparison column right-clicked.
    menu_col: Option<u32>,
    /// The shown columns' widths and start offsets (points), kept per columns generation and glyph width (review
    /// F12: a frame of a 16,384-column sheet touches the columns on screen only).
    columns: Option<(u64, u32, std::sync::Arc<ColumnsLayout>)>,
    /// The first sheet tab shown (the tabs scroll with their arrows and the wheel; review F13).
    pub tab_first: usize,
    /// The active sheet the tabs last brought into view.
    tab_active: Option<usize>,
}

/// The shown columns laid out from the grid's left edge.
#[derive(Default)]
struct ColumnsLayout {
    /// Comparison column, start offset and width (points), in order.
    cols: Vec<(u32, f32, f32)>,
    /// The sum of the widths.
    total: f32,
}

impl ColumnsLayout {
    /// The columns that meet `[left, left + width)`.
    fn visible(&self, left: f32, width: f32) -> &[(u32, f32, f32)] {
        let first = self.cols.partition_point(|&(_, x, w)| x + w <= left);
        let end = self.cols.partition_point(|&(_, x, _)| x < left + width);
        &self.cols[first..end.max(first)]
    }
}

impl TableUi {
    /// The largest valid first row.
    pub fn max_top(&self) -> f64 {
        (f64::from(self.len) - self.page_rows).max(0.0)
    }
}

/// The focus id of a table view's grid (keys go to it).
pub fn grid_id(view: ViewId) -> Id {
    Id::unique(("hikasa-table", view.0))
}

/// One pane (side) of the grid.
struct Pane {
    side: Side,
    /// The pane's whole rectangle (head row included).
    rect: Rect,
    /// The column heads.
    head: Rect,
    /// The rows.
    body: Rect,
    /// Right edge of the row-number gutter.
    gutter_right: f32,
}

/// Draws the table view of `view` into the remaining space of `ui`; returns the commands its keys ran.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut Ui,
    view: ViewId,
    m: &mut TableViewModel,
    st: &mut TableUi,
    pal: &Palette,
    family: &FontFamily,
    dispatch: &dyn Fn(&hikasa_session::settings::Shortcut) -> Option<CommandId>,
    keys: bool,
) -> Vec<CommandId> {
    let mut run = Vec::new();
    let full = ui.available_rect_before_wrap();
    ui.allocate_rect(full, Sense::hover());
    let painter = ui.painter_at(full);
    let font_px = m.font_px() as f32;
    let font = FontId::new(font_px, family.clone());
    let small = FontId::proportional(12.0);
    let lh = (font_px * 1.5).round().max(4.0);
    let cw = ui.ctx().fonts_mut(|f| f.glyph_width(&font, '0')).max(1.0);
    let len = m.len();
    st.len = len;
    let layout = m.layout();

    // Bottom: the sheet tabs; above them the Text Details panel.
    let tabs_rect = Rect::from_min_max(pos2(full.left(), full.bottom() - TABS_H), full.max);
    let details = m.show_details().then(|| m.details(DETAIL_LINES));
    let details_h = details.as_ref().map_or(0.0, |(_, total)| {
        ((*total).clamp(1, DETAIL_LINES) as f32 + 1.0) * (lh.min(20.0)) + 6.0
    });
    let details_rect = Rect::from_min_max(
        pos2(full.left(), tabs_rect.top() - details_h),
        pos2(full.right(), tabs_rect.top()),
    );
    let header = Rect::from_min_size(full.min, vec2(full.width(), HEADER_H));
    let thumb_w = if m.show_thumbnail() { THUMB_W } else { 0.0 };
    let body = Rect::from_min_max(
        pos2(full.left(), header.bottom()),
        pos2(full.right(), details_rect.top()),
    );
    painter.rect_filled(body, CornerRadius::ZERO, pal.t_bg);
    let grids = Rect::from_min_max(
        pos2(full.left() + thumb_w, body.top()),
        pos2(full.right() - BAR_W, body.bottom()),
    );
    let head_h = (lh * 1.2).round();
    let gutter_w = if m.row_numbers() {
        let digits = m.len().max(1).ilog10() as f32 + 2.0;
        (digits * cw + 2.0 * PAD).ceil()
    } else {
        0.0
    };
    let pane_of = |side: Side, rect: Rect| {
        let head = Rect::from_min_size(rect.min, vec2(rect.width(), head_h));
        Pane {
            side,
            rect,
            head,
            body: Rect::from_min_max(pos2(rect.left(), head.bottom()), rect.max),
            gutter_right: rect.left() + gutter_w,
        }
    };
    let panes: [Pane; 2] = match layout {
        GridLayout::SideBySide => {
            let w = ((grids.width() - 1.0) / 2.0).floor().max(10.0);
            [
                pane_of(
                    Side::Left,
                    Rect::from_min_size(grids.min, vec2(w, grids.height())),
                ),
                pane_of(
                    Side::Right,
                    Rect::from_min_max(pos2(grids.left() + w + 1.0, grids.top()), grids.max),
                ),
            ]
        }
        GridLayout::OverUnder => {
            let h = ((grids.height() - 1.0) / 2.0).floor().max(10.0);
            [
                pane_of(
                    Side::Left,
                    Rect::from_min_size(grids.min, vec2(grids.width(), h)),
                ),
                pane_of(
                    Side::Right,
                    Rect::from_min_max(pos2(grids.left(), grids.top() + h + 1.0), grids.max),
                ),
            ]
        }
    };

    // Pane headers: the specs and what was read.
    painter.rect_filled(header, CornerRadius::ZERO, pal.chrome);
    painter.hline(
        full.x_range(),
        header.bottom() - 0.5,
        Stroke::new(1.0, pal.border),
    );
    let titles = m.pane_titles();
    let header_cells: [Rect; 2] = match layout {
        GridLayout::SideBySide => [0, 1].map(|k| {
            Rect::from_min_max(
                pos2(panes[k].rect.left(), header.top()),
                pos2(panes[k].rect.right(), header.bottom()),
            )
        }),
        GridLayout::OverUnder => {
            let w = (header.width() / 2.0).floor();
            [
                Rect::from_min_size(header.min, vec2(w, HEADER_H)),
                Rect::from_min_max(pos2(header.left() + w, header.top()), header.max),
            ]
        }
    };
    for (k, (title, detail)) in titles.iter().enumerate() {
        let cell = header_cells[k];
        let p = painter.with_clip_rect(cell);
        let shown = p.text(
            pos2(cell.left() + 8.0, cell.center().y),
            Align2::LEFT_CENTER,
            title,
            FontId::monospace(12.0),
            pal.fg,
        );
        p.text(
            pos2(shown.right() + 10.0, cell.center().y),
            Align2::LEFT_CENTER,
            detail,
            small.clone(),
            pal.muted,
        );
    }

    let page = f64::from(((panes[0].body.height()) / lh).floor().max(1.0));
    st.page_rows = page;
    let id = grid_id(view);

    // Input: wheel, clicks, keys, resizing.
    let response = ui.interact(grids, id, Sense::click());
    if !st.focused_once {
        st.focused_once = true;
        response.request_focus();
    }
    let key = (m.columns_generation(), cw.to_bits());
    if st.columns.as_ref().is_none_or(|(g, c, _)| (*g, *c) != key) {
        let mut layout = ColumnsLayout::default();
        for &c in m.shown_columns() {
            if let Some(h) = m.column_head(c) {
                let w = h.width * cw + 2.0 * PAD;
                layout.cols.push((c, layout.total, w));
                layout.total += w;
            }
        }
        st.columns = Some((key.0, key.1, std::sync::Arc::new(layout)));
    }
    let columns = st
        .columns
        .as_ref()
        .map(|(_, _, l)| std::sync::Arc::clone(l))
        .unwrap_or_default();
    let total_w = columns.total;
    let pane_w = panes[0].body.right() - panes[0].gutter_right;
    let view_w = panes
        .iter()
        .map(|p| p.body.right() - p.gutter_right)
        .fold(0.0, f32::max);
    let max_left = (total_w - pane_w + BAR_W).max(0.0);
    if response.hovered() {
        let delta = ui.input(|i| i.smooth_scroll_delta());
        if delta != Vec2::ZERO {
            st.top -= f64::from(delta.y / lh);
            st.left = (st.left - delta.x).clamp(0.0, max_left);
            ui.ctx().input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
        }
    }
    // Column grips: drag a head's right edge.
    let pointer = ui.input(|i| i.pointer.interact_pos());
    let mut over_grip = None;
    if let Some(p) = pointer {
        for pane in &panes {
            if !pane.head.contains(p) {
                continue;
            }
            for &(col, start, w) in columns.visible(st.left, view_w) {
                let x = pane.gutter_right - st.left + start + w;
                if (p.x - x).abs() <= GRIP && x > pane.gutter_right {
                    over_grip = Some(col);
                }
            }
        }
    }
    if over_grip.is_some() || st.drag.is_some() {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeColumn);
    }
    let (pressed, down, released) = ui.input(|i| {
        (
            i.pointer.primary_pressed(),
            i.pointer.primary_down(),
            i.pointer.primary_released(),
        )
    });
    if pressed
        && let (Some(col), Some(p)) = (over_grip, pointer)
        && let Some(head) = m.column_head(col)
    {
        st.drag = Some((col, p.x, head.width));
    }
    if let Some((col, x0, w0)) = st.drag {
        if let Some(p) = pointer
            && down
        {
            m.set_width(col, w0 + (p.x - x0) / cw);
        }
        if released || !down {
            st.drag = None;
        }
    }
    let mut clicked_head = None;
    if response.clicked() && st.drag.is_none() && over_grip.is_none() {
        response.request_focus();
        if let Some(p) = response.interact_pointer_pos() {
            for pane in &panes {
                if pane.body.contains(p) {
                    let pos = (st.top + f64::from((p.y - pane.body.top()) / lh)).floor();
                    if pos >= 0.0 && pos < f64::from(len) {
                        if p.x < pane.gutter_right {
                            m.click_row(pos as u32, pane.side);
                        } else if let Some(col) =
                            column_at(columns.visible(st.left, view_w), pane, st.left, p.x)
                        {
                            m.click(pos as u32, col, pane.side);
                        }
                    }
                } else if pane.head.contains(p) {
                    clicked_head = column_at(columns.visible(st.left, view_w), pane, st.left, p.x);
                }
            }
        }
    }
    let _ = clicked_head;
    if response.secondary_clicked()
        && let Some(p) = response.interact_pointer_pos()
    {
        st.menu_col = panes
            .iter()
            .find(|pane| pane.head.contains(p) || pane.body.contains(p))
            .and_then(|pane| column_at(columns.visible(st.left, view_w), pane, st.left, p.x));
    }
    response.context_menu(|ui| {
        if let Some(col) = st.menu_col
            && ui.button("Hide Column").clicked()
        {
            m.hide_column(col);
            ui.close();
        }
        let hidden = m.hidden_count();
        if ui
            .add_enabled(
                hidden > 0,
                egui::Button::new(format!("Show Hidden Columns ({hidden})")),
            )
            .clicked()
        {
            m.show_all_columns();
            ui.close();
        }
        if ui.button("Resize Columns to Fit").clicked() {
            run.push(CommandId::ColDetectSize);
            ui.close();
        }
        if ui.button("Copy").clicked() {
            run.push(CommandId::Copy);
            ui.close();
        }
    });

    // `keys` is false while a dialog is open: the dialog has the keyboard.
    if keys && response.has_focus() {
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
                match crate::ui::keys::shortcuts_of(*key, *modifiers)
                    .iter()
                    .find_map(dispatch)
                {
                    Some(cmd) => {
                        run.push(cmd);
                        false
                    }
                    None => true,
                }
            });
        });
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
        let (up, down_k, pg_up, pg_down, home, end, left, right) = ui.input(|i| {
            (
                i.key_pressed(Key::ArrowUp),
                i.key_pressed(Key::ArrowDown),
                i.key_pressed(Key::PageUp),
                i.key_pressed(Key::PageDown),
                i.key_pressed(Key::Home),
                i.key_pressed(Key::End),
                i.key_pressed(Key::ArrowLeft),
                i.key_pressed(Key::ArrowRight),
            )
        });
        let step = (page - 1.0).max(1.0) as i64;
        let rows =
            i64::from(down_k) - i64::from(up) + step * (i64::from(pg_down) - i64::from(pg_up));
        let cols = i64::from(right) - i64::from(left);
        if rows != 0 || cols != 0 {
            m.move_cursor(rows, cols);
        }
        // A page key scrolls a page as well as moving the current cell.
        st.top += step as f64 * (f64::from(u8::from(pg_down)) - f64::from(u8::from(pg_up)));
        if home {
            st.top = 0.0;
        }
        if end {
            st.top = f64::from(len);
        }
    }

    // Scroll bar and thumbnail.
    let bar = Rect::from_min_max(pos2(grids.right(), body.top()), body.max);
    if let Some(top) = scroll::vertical(ui, id.with("bar"), bar, f64::from(len), page, st.top, pal)
    {
        st.top = top;
    }
    let thumb_rect = Rect::from_min_max(body.min, pos2(grids.left(), body.bottom()));
    if m.show_thumbnail() && len > 0 {
        let r = ui
            .interact(thumb_rect, id.with("thumb"), Sense::click_and_drag())
            .on_hover_cursor(CursorIcon::PointingHand);
        if (r.clicked() || r.dragged())
            && let Some(p) = r.interact_pointer_pos()
        {
            let frac = f64::from(((p.y - body.top()) / body.height().max(1.0)).clamp(0.0, 1.0));
            st.top = frac * f64::from(len) - page / 2.0;
        }
    }
    if let Some(pos) = m.take_scroll() {
        let pos = f64::from(pos);
        if pos < st.top || pos > st.top + page - 1.0 {
            st.top = pos - (page / 3.0).floor();
        }
    }
    st.top = st.top.clamp(0.0, st.max_top());
    st.left = st.left.clamp(0.0, max_left);
    m.set_viewport(st.top.floor() as u32, page as u32);

    // The grids.
    let first = st.top.floor() as u32;
    let frac = (st.top - st.top.floor()) as f32;
    let count = ((panes[0].body.height() / lh).ceil() as u32 + 1).min(len.saturating_sub(first));
    let cursor = m.cursor();
    for pane in &panes {
        let shown = columns.visible(st.left, view_w);
        draw_heads(&painter, m, pane, shown, st.left, &font, pal);
        let clip = painter.with_clip_rect(pane.body);
        let cells = painter.with_clip_rect(Rect::from_min_max(
            pos2(pane.gutter_right, pane.body.top()),
            pane.body.max,
        ));
        let k = usize::from(pane.side != Side::Left);
        for i in 0..count {
            let pos = first + i;
            let Some(row) = m.row_at(pos) else {
                break;
            };
            let y = pane.body.top() + (i as f32 - frac) * lh;
            let row_rect =
                Rect::from_min_max(pos2(pane.rect.left(), y), pos2(pane.rect.right(), y + lh));
            let gutter = Rect::from_min_max(row_rect.min, pos2(pane.gutter_right, y + lh));
            let present = row.labels[k].is_some();
            clip.rect_filled(gutter, CornerRadius::ZERO, pal.gutter);
            if let (Some(label), true) = (row.labels[k], gutter_w > 0.0) {
                clip.text(
                    pos2(gutter.right() - PAD, y + lh / 2.0),
                    Align2::RIGHT_CENTER,
                    label.to_string(),
                    font.clone(),
                    pal.gutter_fg,
                );
            }
            let data = Rect::from_min_max(pos2(pane.gutter_right, y), row_rect.max);
            if !present {
                hatch(&cells, data, pal.hatch);
                continue;
            }
            if let Some(bg) = row_background(row.kind, row.importance, pal) {
                cells.rect_filled(data, CornerRadius::ZERO, bg);
            }
            if cursor.is_some_and(|c| c.row == row.row) {
                cells.rect_filled(data, CornerRadius::ZERO, pal.current_line);
            }
            for &(col, start, w) in shown {
                let x = pane.gutter_right - st.left + start;
                let rect = Rect::from_min_max(pos2(x, y), pos2(x + w, y + lh));
                if rect.right() < pane.gutter_right || rect.left() > pane.body.right() {
                    continue;
                }
                let cell = m.cell(row.row, col, pane.side);
                let Some(text) = cell.text else {
                    hatch(&cells, rect, pal.hatch);
                    continue;
                };
                let (fill, colour) = match (row.kind, cell.state) {
                    (RowKind::Changed, CellState::Important) => (Some(pal.imp_hl), pal.imp_fg),
                    (RowKind::Changed, CellState::Unimportant) => {
                        (Some(pal.unimp_hl), pal.unimp_fg)
                    }
                    (RowKind::LeftOnly | RowKind::RightOnly, CellState::Important) => {
                        (None, pal.imp_fg)
                    }
                    (RowKind::LeftOnly | RowKind::RightOnly, CellState::Unimportant) => {
                        (None, pal.unimp_fg)
                    }
                    _ => (None, pal.t_text),
                };
                if let Some(fill) = fill {
                    cells.rect_filled(rect.shrink(1.0), CornerRadius::ZERO, fill);
                }
                cells.vline(
                    rect.right() - 0.5,
                    rect.y_range(),
                    Stroke::new(1.0, pal.border),
                );
                if !text.is_empty() {
                    let text_clip = cells
                        .with_clip_rect(rect.shrink2(vec2(1.0, 0.0)).intersect(cells.clip_rect()));
                    text_clip.text(
                        pos2(rect.left() + PAD, rect.center().y),
                        Align2::LEFT_CENTER,
                        text,
                        font.clone(),
                        colour,
                    );
                }
                if cell.formula {
                    let corner = pos2(rect.right() - 1.0, rect.top() + 1.0);
                    cells.add(egui::Shape::convex_polygon(
                        vec![
                            corner,
                            pos2(corner.x - 5.0, corner.y),
                            pos2(corner.x, corner.y + 5.0),
                        ],
                        pal.muted,
                        Stroke::NONE,
                    ));
                }
                if let Some(c) = cursor
                    && c.row == row.row
                    && (c.whole_row || c.col == col)
                    && c.side == pane.side
                {
                    cells.rect_stroke(
                        rect.shrink(0.5),
                        CornerRadius::ZERO,
                        Stroke::new(2.0, pal.accent),
                        egui::StrokeKind::Inside,
                    );
                }
            }
        }
        // The pane's borders.
        painter.rect_stroke(
            pane.rect,
            CornerRadius::ZERO,
            Stroke::new(1.0, pal.border),
            egui::StrokeKind::Inside,
        );
    }
    if count > 0 {
        st.drawn += 1;
    }
    if len == 0
        && let Some(message) = m.message()
    {
        painter.text(
            pos2(
                grids.center().x,
                grids.top() + 60.0_f32.min(grids.height() / 2.0),
            ),
            Align2::CENTER_CENTER,
            message,
            FontId::proportional(14.0),
            if m.phase() == TablePhase::Failed {
                pal.error
            } else {
                pal.muted
            },
        );
    }
    if m.show_thumbnail() {
        draw_thumbnail(&painter, thumb_rect, m, st, first, count, pal);
    }
    if let Some((lines, total)) = &details {
        draw_details(&painter, details_rect, lines, *total, lh.min(20.0), pal);
    }
    if let Some(k) = draw_tabs(ui, &painter, tabs_rect, m, st, id, pal) {
        m.set_sheet(k);
    }
    run
}

/// The comparison column under `x` in `pane`.
fn column_at(columns: &[(u32, f32, f32)], pane: &Pane, left: f32, x: f32) -> Option<u32> {
    if x < pane.gutter_right {
        return None;
    }
    columns.iter().find_map(|&(col, start, w)| {
        let at = pane.gutter_right - left + start;
        (x >= at && x < at + w).then_some(col)
    })
}

/// A row's background: changed rows by importance, orphan rows lighter; same rows none.
fn row_background(kind: RowKind, importance: Importance, pal: &Palette) -> Option<Color32> {
    match (kind, importance) {
        (_, Importance::None) => None,
        (RowKind::Changed | RowKind::Same, Importance::Important) => Some(pal.imp_bg),
        (RowKind::Changed | RowKind::Same, Importance::Unimportant) => Some(pal.unimp_bg),
        (_, Importance::Important) => Some(pal.orph_imp_bg),
        (_, Importance::Unimportant) => Some(pal.orph_unimp_bg),
    }
}

/// The column heads of a pane: the side's header name (else the letters) over the letters, key columns marked.
fn draw_heads(
    painter: &Painter,
    m: &TableViewModel,
    pane: &Pane,
    columns: &[(u32, f32, f32)],
    left: f32,
    font: &FontId,
    pal: &Palette,
) {
    painter.rect_filled(pane.head, CornerRadius::ZERO, pal.chrome);
    let clip = painter.with_clip_rect(Rect::from_min_max(
        pos2(pane.gutter_right, pane.head.top()),
        pane.head.max,
    ));
    let k = usize::from(pane.side != Side::Left);
    for &(col, start, w) in columns {
        let x = pane.gutter_right - left + start;
        let rect = Rect::from_min_max(pos2(x, pane.head.top()), pos2(x + w, pane.head.bottom()));
        if rect.right() < pane.gutter_right || rect.left() > pane.head.right() {
            continue;
        }
        let Some(head) = m.column_head(col) else {
            continue;
        };
        match &head.names[k] {
            None => hatch(&clip, rect, pal.hatch),
            Some(name) => {
                let colour = match head.state {
                    CellState::Important => pal.imp_fg,
                    CellState::Unimportant => pal.unimp_fg,
                    CellState::Same => pal.fg,
                };
                let label = match head.key {
                    Some(n) => format!("#{n} {name}"),
                    None => name.clone(),
                };
                clip.with_clip_rect(rect.intersect(clip.clip_rect())).text(
                    pos2(rect.left() + PAD, rect.center().y),
                    Align2::LEFT_CENTER,
                    label,
                    font.clone(),
                    colour,
                );
            }
        }
        clip.vline(
            rect.right() - 0.5,
            rect.y_range(),
            Stroke::new(1.0, pal.border),
        );
    }
    painter.hline(
        pane.head.x_range(),
        pane.head.bottom() - 0.5,
        Stroke::new(1.0, pal.border),
    );
}

/// The thumbnail of the shown rows (TB-020): one mark per bucket, the visible window outlined.
fn draw_thumbnail(
    painter: &Painter,
    rect: Rect,
    m: &TableViewModel,
    st: &mut TableUi,
    first: u32,
    count: u32,
    pal: &Palette,
) {
    painter.rect_filled(rect, CornerRadius::ZERO, pal.chrome);
    let height = rect.height().max(1.0) as u32;
    let generation = m.generation();
    if st
        .thumb
        .as_ref()
        .is_none_or(|(g, h, _)| *g != generation || *h != height)
    {
        st.thumb = Some((generation, height, m.thumbnail(height)));
    }
    let Some((_, _, cells)) = &st.thumb else {
        return;
    };
    let n = cells.len().max(1) as f32;
    let step = rect.height() / n;
    for (k, &code) in cells.iter().enumerate() {
        let colour = match code {
            1 => pal.thumb_unimp,
            2 => pal.thumb_imp,
            3 => pal.thumb_orph,
            _ => continue,
        };
        let y = rect.top() + k as f32 * step;
        painter.rect_filled(
            Rect::from_min_max(
                pos2(rect.left() + 2.0, y),
                pos2(rect.right() - 2.0, y + step.max(1.0)),
            ),
            CornerRadius::ZERO,
            colour,
        );
    }
    let len = f64::from(st.len.max(1));
    let y0 = rect.top() + (f64::from(first) / len) as f32 * rect.height();
    let y1 = rect.top() + (f64::from(first + count) / len).min(1.0) as f32 * rect.height();
    painter.rect_stroke(
        Rect::from_min_max(
            pos2(rect.left() + 0.5, y0),
            pos2(rect.right() - 0.5, y1.max(y0 + 2.0)),
        ),
        CornerRadius::ZERO,
        Stroke::new(1.0, pal.thumb_view),
        egui::StrokeKind::Inside,
    );
}

/// The Text Details panel: the current row's cells, one column a line.
fn draw_details(
    painter: &Painter,
    rect: Rect,
    lines: &[crate::table_view::DetailLine],
    total: usize,
    lh: f32,
    pal: &Palette,
) {
    painter.rect_filled(rect, CornerRadius::ZERO, pal.chrome);
    painter.hline(
        rect.x_range(),
        rect.top() + 0.5,
        Stroke::new(1.0, pal.border),
    );
    let font = FontId::proportional(12.0);
    let clip = painter.with_clip_rect(rect);
    if lines.is_empty() {
        clip.text(
            pos2(rect.left() + 8.0, rect.top() + lh / 2.0 + 3.0),
            Align2::LEFT_CENTER,
            "Click a row to see its cells here.",
            font,
            pal.muted,
        );
        return;
    }
    let name_w = (rect.width() * 0.18).clamp(60.0, 220.0);
    let value_w = (rect.width() - name_w - 16.0) / 2.0;
    let mut y = rect.top() + 3.0;
    for line in lines.iter().take(DETAIL_LINES) {
        let colour = match line.state {
            CellState::Important => pal.imp_fg,
            CellState::Unimportant => pal.unimp_fg,
            CellState::Same => pal.fg,
        };
        clip.text(
            pos2(rect.left() + 8.0, y + lh / 2.0),
            Align2::LEFT_CENTER,
            &line.name,
            font.clone(),
            pal.muted,
        );
        for (k, text) in line.texts.iter().enumerate() {
            let x = rect.left() + 8.0 + name_w + k as f32 * value_w;
            let cell = Rect::from_min_size(pos2(x, y), vec2(value_w - 8.0, lh));
            match text {
                Some(t) => {
                    clip.with_clip_rect(cell.intersect(rect)).text(
                        pos2(x, y + lh / 2.0),
                        Align2::LEFT_CENTER,
                        t,
                        font.clone(),
                        colour,
                    );
                }
                None => hatch(&clip, cell, pal.hatch),
            }
        }
        y += lh;
    }
    if total > DETAIL_LINES {
        clip.text(
            pos2(rect.left() + 8.0, y + lh / 2.0),
            Align2::LEFT_CENTER,
            format!("… {} more columns", total - DETAIL_LINES),
            font,
            pal.muted,
        );
    }
}

/// Width of a tab-scrolling arrow.
const TAB_ARROW_W: f32 = 22.0;

/// The sheet tabs (TB-004): a spot coloured by the sheet pair's state; returns the tab clicked. Tabs past the right
/// edge are reached with the arrows at the right end or the wheel over the strip; the active sheet is brought into
/// view when it changes (review F13).
fn draw_tabs(
    ui: &mut Ui,
    painter: &Painter,
    rect: Rect,
    m: &TableViewModel,
    st: &mut TableUi,
    id: Id,
    pal: &Palette,
) -> Option<usize> {
    painter.rect_filled(rect, CornerRadius::ZERO, pal.chrome);
    painter.hline(
        rect.x_range(),
        rect.top() + 0.5,
        Stroke::new(1.0, pal.border),
    );
    let font = FontId::proportional(12.0);
    let active = m.active_sheet();
    let tabs = m.sheet_tabs();
    let galleys: Vec<_> = tabs
        .iter()
        .map(|tab| painter.layout_no_wrap(tab.caption.clone(), font.clone(), pal.fg))
        .collect();
    let widths: Vec<f32> = galleys.iter().map(|g| g.size().x + 28.0 + 4.0).collect();
    let total: f32 = widths.iter().sum::<f32>() + 6.0;
    let overflow = total > rect.width();
    let strip_right = if overflow {
        rect.right() - 2.0 * TAB_ARROW_W - 4.0
    } else {
        rect.right()
    };
    let room = strip_right - rect.left() - 6.0;
    // The tabs from `first` that fit.
    let fits = |first: usize| -> usize {
        let mut used = 0.0;
        let mut n = 0;
        for w in widths.iter().skip(first) {
            used += w;
            if used > room && n > 0 {
                break;
            }
            n += 1;
        }
        n
    };
    // The first tab that shows the last one at the right end.
    let last_first = (0..tabs.len())
        .find(|&k| fits(k) >= tabs.len() - k)
        .unwrap_or(0);
    if !overflow {
        st.tab_first = 0;
    }
    if st.tab_active != Some(active) {
        st.tab_active = Some(active);
        if active < st.tab_first {
            st.tab_first = active;
        }
        while st.tab_first < active && st.tab_first + fits(st.tab_first) <= active {
            st.tab_first += 1;
        }
    }
    let strip = Rect::from_min_max(rect.min, pos2(strip_right, rect.bottom()));
    let wheel = ui.interact(strip, id.with("sheet-strip"), Sense::hover());
    if overflow && wheel.hovered() {
        let delta = ui.input(|i| i.smooth_scroll_delta);
        let step = delta.y + delta.x;
        if step < 0.0 {
            st.tab_first += 1;
        } else if step > 0.0 {
            st.tab_first = st.tab_first.saturating_sub(1);
        }
        if step != 0.0 {
            ui.ctx().input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
        }
    }
    st.tab_first = st.tab_first.min(last_first);
    let clip = painter.with_clip_rect(strip);
    let mut x = rect.left() + 6.0;
    let mut clicked = None;
    for (k, (tab, galley)) in tabs.iter().zip(galleys).enumerate().skip(st.tab_first) {
        let w = galley.size().x + 28.0;
        let r = Rect::from_min_size(pos2(x, rect.top() + 3.0), vec2(w, rect.height() - 6.0));
        x += w + 4.0;
        // Whole tabs only (the first one always): the arrows reach the others.
        if r.right() > strip_right && k > st.tab_first {
            break;
        }
        let response = ui
            .interact(r.intersect(strip), id.with(("sheet", k)), Sense::click())
            .on_hover_text(format!(
                "{}: {}",
                tab.caption,
                crate::table_view::sheet_state_text(tab.state)
            ));
        let bg = if k == active {
            pal.t_bg
        } else if response.hovered() {
            pal.hover
        } else {
            pal.chrome
        };
        clip.rect_filled(r, CornerRadius::same(3), bg);
        clip.rect_stroke(
            r,
            CornerRadius::same(3),
            Stroke::new(1.0, pal.border),
            egui::StrokeKind::Inside,
        );
        let spot = match tab.state {
            SheetState::Same => pal.c_same,
            SheetState::Unimportant => pal.unimp_fg,
            SheetState::Important => pal.imp_fg,
            SheetState::LeftOnly | SheetState::RightOnly => pal.c_orphan,
        };
        clip.circle_filled(pos2(r.left() + 10.0, r.center().y), 4.0, spot);
        clip.galley(
            pos2(r.left() + 20.0, r.center().y - galley.size().y / 2.0),
            galley,
            pal.fg,
        );
        if response.clicked() {
            clicked = Some(k);
        }
    }
    if overflow {
        let arrow = |k: usize, label: &str, enabled: bool| {
            let r = Rect::from_min_size(
                pos2(strip_right + 2.0 + k as f32 * TAB_ARROW_W, rect.top() + 3.0),
                vec2(TAB_ARROW_W - 2.0, rect.height() - 6.0),
            );
            let response = ui.interact(r, id.with(("sheet-arrow", k)), Sense::click());
            let colour = if !enabled {
                pal.muted
            } else if response.hovered() {
                pal.fg
            } else {
                pal.muted
            };
            painter.text(
                r.center(),
                Align2::CENTER_CENTER,
                label,
                font.clone(),
                colour,
            );
            enabled && response.clicked()
        };
        if arrow(0, "◀", st.tab_first > 0) {
            st.tab_first -= 1;
        }
        if arrow(1, "▶", st.tab_first < last_first) {
            st.tab_first += 1;
        }
    }
    clicked
}
