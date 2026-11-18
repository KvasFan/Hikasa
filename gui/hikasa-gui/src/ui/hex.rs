//! The Hex Compare view (M2 W13a, contract §1.13.4–§1.13.8, D-356…D-360): the Find panel, the File Info headers, the
//! two panes (side by side or over-under) with the address column, the hex digits and the characters, the thumbnail
//! strip, the scroll bar and the value panel, custom-painted with egui's painter for the rows on screen only. Rows,
//! cells, states, addresses and values come from [`HexViewModel`] (core's); this module places them, turns input into
//! model calls, and holds the layout constants and the per-frame galleys.

use egui::text::LayoutJob;
use egui::{
    Align2, Color32, CornerRadius, CursorIcon, EventFilter, FontFamily, FontId, Id, Key, Rect,
    RichText, Sense, Stroke, TextFormat, Ui, UiBuilder, Vec2, pos2, vec2,
};
use hikasa_core::Side;
use hikasa_hex::{ByteState, CharSet};
use hikasa_session::ipc::ViewId;

use crate::commands::{Area, CommandId, Ctx};
use crate::hex::pane::{CaretKey, PaneRect, Part, RowGeometry};
use crate::hex::screen::ShownRow;
use crate::hex::thumbnail::{self, ThumbClass};
use crate::hex::{HexPhase, HexViewModel, PaneLayout, index};
use crate::menus::MenuItem;
use crate::ui::scroll;
use crate::ui::theme::Palette;
use crate::window::{CommandArg, Invocation};

/// Height of a File Info header.
const HEADER_H: f32 = 26.0;
/// Height of the Find panel.
const FIND_H: f32 = 30.0;
/// Width of the thumbnail strip (D-360: 16 points).
pub const THUMB_W: f32 = 16.0;
/// Width of the scroll bar.
const BAR_W: f32 = 12.0;
/// Padding left of a row.
const PAD: f32 = 6.0;
/// Lines of the value panel (both blocks).
const VALUE_LINES: f32 = 13.0;

/// UI state of one hex view (kept by the window between frames).
#[derive(Default)]
pub struct HexUi {
    focused_once: bool,
    /// The header path edits being typed (`None`: the view's spec).
    paths: [Option<String>; 2],
    /// The thumbnail's pixel rows: generation, height, classes.
    thumb: Option<(u64, u32, Vec<ThumbClass>)>,
    /// The pane a context menu was opened on.
    menu: Option<Area>,
    /// The press of a drag selection (the pane).
    pressed: Option<Side>,
    /// Frames drawn with rows on screen (the benchmark's mark).
    pub drawn: u64,
    /// The last frame drew every row from the cache.
    pub complete: bool,
    /// Each pane's header and rows (a drop's target, UI-008).
    pub drop_rects: Vec<(Side, Rect)>,
}

/// The focus id of a hex view's panes (keys go to it).
pub fn panes_id(view: ViewId) -> Id {
    Id::unique(("hikasa-hex", view.0))
}

/// What the view's context menus are built from.
pub struct HexMenus<'a> {
    /// The command context of the frame.
    pub ctx: Ctx,
    /// The items of an area's menu.
    pub menu: &'a dyn Fn(Area, &Ctx) -> Vec<MenuItem>,
}

/// One pane on screen.
struct Pane {
    side: Side,
    header: Rect,
    body: Rect,
}

/// The colours of a cell state: text and background.
fn cell_colours(state: ByteState, pal: &Palette) -> (Color32, Option<Color32>) {
    match state {
        ByteState::Same => (pal.t_text, None),
        ByteState::Different => (pal.imp_fg, Some(pal.imp_hl)),
        ByteState::LeftOnly | ByteState::RightOnly => (pal.t_text, Some(pal.orph_bg)),
    }
}

/// Draws the hex view of `view` into the remaining space of `ui`; returns the commands its keys and menus ran.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut Ui,
    view: ViewId,
    m: &mut HexViewModel,
    st: &mut HexUi,
    pal: &Palette,
    family: &FontFamily,
    dispatch: &dyn Fn(&hikasa_session::settings::Shortcut) -> Option<CommandId>,
    keys: bool,
    menus: &HexMenus<'_>,
) -> Vec<Invocation> {
    let mut run: Vec<Invocation> = Vec::new();
    let full = ui.available_rect_before_wrap();
    ui.allocate_rect(full, Sense::hover());
    let painter = ui.painter_at(full);
    painter.rect_filled(full, CornerRadius::ZERO, pal.t_bg);
    let font_px = m.display().font_px as f32;
    let font = FontId::new(font_px, family.clone());
    let lh = (font_px * 1.4).round().max(4.0);
    let cw = ui.ctx().fonts_mut(|f| f.glyph_width(&font, '0')).max(1.0);
    let now = ui.input(|i| i.time);
    let d = *m.display();
    m.panel = None;

    // Top: the Find panel.
    let mut top = full.top();
    if m.find.open {
        let rect = Rect::from_min_size(pos2(full.left(), top), vec2(full.width(), FIND_H));
        find_panel(ui, view, m, rect, pal, &mut run);
        top += FIND_H;
    }
    // Bottom: the value panel.
    let mut bottom = full.bottom();
    if d.values != hikasa_session::api::HexValues::None {
        let blocks = f32::from(u8::from(d.values.little()) + u8::from(d.values.big()));
        let h = (VALUE_LINES * blocks / 2.0 + 1.0) * 16.0 + 8.0;
        let rect = Rect::from_min_max(pos2(full.left(), bottom - h), pos2(full.right(), bottom));
        values_panel(ui, view, m, rect, pal, menus, &mut run);
        bottom -= h;
    }
    let area = Rect::from_min_max(pos2(full.left(), top), pos2(full.right(), bottom));
    let thumb_w = if d.thumbnail { THUMB_W } else { 0.0 };
    let panes_rect = Rect::from_min_max(
        area.min,
        pos2(area.right() - BAR_W - thumb_w, area.bottom()),
    );
    let header_h = if m.show_headers() { HEADER_H } else { 0.0 };
    let panes: [Pane; 2] = match d.panes {
        PaneLayout::SideBySide => {
            let w = ((panes_rect.width() - 1.0) / 2.0).floor().max(10.0);
            [0.0, w + 1.0]
                .map(|dx| {
                    let x = panes_rect.left() + dx;
                    let r = Rect::from_min_max(
                        pos2(x, panes_rect.top()),
                        pos2((x + w).min(panes_rect.right()), panes_rect.bottom()),
                    );
                    (r, dx > 0.0)
                })
                .map(|(r, right)| pane_of(r, right, header_h))
        }
        PaneLayout::OverUnder => {
            let h = ((panes_rect.height() - 1.0) / 2.0).floor().max(10.0);
            [0.0, h + 1.0]
                .map(|dy| {
                    let y = panes_rect.top() + dy;
                    let r = Rect::from_min_max(
                        pos2(panes_rect.left(), y),
                        pos2(panes_rect.right(), (y + h).min(panes_rect.bottom())),
                    );
                    (r, dy > 0.0)
                })
                .map(|(r, right)| pane_of(r, right, header_h))
        }
    };

    st.drop_rects = panes
        .iter()
        .map(|p| (p.side, p.header.union(p.body)))
        .collect();
    // The headers.
    if header_h > 0.0 {
        for pane in &panes {
            header(ui, view, m, st, pane, pal, menus, &mut run);
        }
    }

    let rows_visible = ((panes[0].body.height()) / lh).floor().max(1.0) as u32;
    st.complete = m.frame(rows_visible, now);
    let id = panes_id(view);
    let body_union = panes[0].body.union(panes[1].body);
    let response = ui.interact(body_union, id, Sense::click_and_drag());
    if !st.focused_once {
        st.focused_once = true;
        response.request_focus();
    }

    // Phases without rows.
    match m.phase() {
        HexPhase::Empty | HexPhase::Opening | HexPhase::Failed => {
            let text = match m.phase() {
                HexPhase::Empty => "Type or drop the two files into the headers.".to_owned(),
                HexPhase::Opening => "Opening the files…".to_owned(),
                _ => m.error().map_or_else(String::new, |(_, e)| {
                    format!("The files could not be opened: {e}")
                }),
            };
            painter.text(
                body_union.center(),
                Align2::CENTER_CENTER,
                text,
                FontId::proportional(14.0),
                pal.muted,
            );
            return run;
        }
        _ => {}
    }

    // Geometry of a row.
    let view_rows = m.row_count();
    let width_digits = address_chars(m);
    let geometry = RowGeometry::new(
        d.addresses.then_some(width_digits),
        d.layout.bytes_per_line,
        d.layout.group,
        d.layout.chars != CharSet::None,
    );
    let metrics = HexMetrics { cw, lh };
    let row_w = geometry.width() as f32 * cw + 2.0 * PAD;

    // Input: wheel (both panes scroll together), clicks, drags, double-clicks, right-clicks.
    let scroll_x = m.pane(Side::Left).scroll_x;
    if response.hovered() {
        let delta = ui.input(|i| i.smooth_scroll_delta);
        if delta != Vec2::ZERO {
            m.screen.scroll_by(f64::from(-delta.y / lh), view_rows);
            let max_x = (row_w - panes[0].body.width()).max(0.0);
            let x = (scroll_x - delta.x).clamp(0.0, max_x);
            m.set_scroll_x(x);
            ui.ctx().input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
        }
    }
    let pointer = ui.input(|i| i.pointer.interact_pos());
    let shift = ui.input(|i| i.modifiers.shift);
    let hit_at = |m: &HexViewModel, p: egui::Pos2| {
        panes.iter().find_map(|pane| {
            let rect = rect_of(pane.body);
            crate::hex::pane::hit(
                &rect,
                &geometry,
                &metrics,
                m.screen.top,
                m.screen.frac,
                m.pane(pane.side).scroll_x - PAD,
                [p.x, p.y],
            )
            .map(|h| (pane.side, h))
        })
    };
    if response.double_clicked() {
        if let Some((side, hit)) = pointer.and_then(|p| hit_at(m, p)) {
            m.double_click(side, &hit);
        }
    } else if response.drag_started() || response.clicked() {
        response.request_focus();
        if let Some((side, hit)) = pointer.and_then(|p| hit_at(m, p)) {
            m.click(side, &hit, shift);
            st.pressed = Some(side);
        }
    }
    if response.dragged()
        && st.pressed.is_some()
        && let Some(p) = pointer
    {
        // At the edges the view scrolls, both panes together.
        if p.y < body_union.top() {
            m.screen.scroll_by(-1.0, view_rows);
        } else if p.y > body_union.bottom() {
            m.screen.scroll_by(1.0, view_rows);
        }
        let clamped = pos2(
            p.x,
            p.y.clamp(panes[0].body.top() + 1.0, panes[0].body.bottom() - 1.0),
        );
        if let Some((_, hit)) = hit_at(m, clamped) {
            m.drag(&hit);
        }
    }
    if response.drag_stopped() || (st.pressed.is_some() && ui.input(|i| i.pointer.any_released())) {
        st.pressed = None;
        m.release();
    }
    if response.secondary_clicked()
        && let Some((side, hit)) = pointer.and_then(|p| hit_at(m, p))
    {
        m.context_click(side, &hit);
        st.menu = Some(Area::HexPane(side));
    }
    let area_menu = st.menu.unwrap_or(Area::HexPane(m.focus()));
    response.context_menu(|ui| {
        let items = (menus.menu)(area_menu, &menus.ctx);
        menu_items(ui, &items, &mut run);
    });

    // Keys (the focus pane's caret; a dialog has the keyboard while open).
    if keys && response.has_focus() {
        caret_keys(ui, m, id, dispatch, &mut run);
    }

    // The rows.
    let generation = m.generation;
    let top_row = m.screen.top;
    let frac = m.screen.frac;
    let addr_font = FontId::new((font_px - 1.0).max(6.0), family.clone());
    for pane in &panes {
        let p = ui.painter_at(pane.body);
        let side = pane.side;
        let pane_state = *m.pane(side);
        let focused = side == m.focus();
        let x0 = pane.body.left() + PAD - pane_state.scroll_x;
        if d.addresses {
            p.rect_filled(
                Rect::from_min_max(
                    pane.body.min,
                    pos2(
                        x0 + (geometry.address as f32 - 1.0) * cw,
                        pane.body.bottom(),
                    ),
                ),
                CornerRadius::ZERO,
                pal.gutter,
            );
        }
        let sel = m.selection();
        let hit = m.find_hit().cloned();
        for k in 0..=rows_visible {
            let row_k = top_row + u64::from(k);
            if row_k >= view_rows {
                break;
            }
            let y = pane.body.top() + (k as f32 - frac) * lh;
            let band = Rect::from_min_size(pos2(pane.body.left(), y), vec2(pane.body.width(), lh));
            match m.screen.row(generation, row_k) {
                ShownRow::Placeholder => {
                    p.rect_filled(band, CornerRadius::ZERO, pal.gap);
                }
                ShownRow::Error(e) => {
                    p.rect_filled(band, CornerRadius::ZERO, pal.error_bg);
                    p.text(
                        pos2(x0, y + lh / 2.0),
                        Align2::LEFT_CENTER,
                        e,
                        FontId::proportional(12.0),
                        pal.error,
                    );
                }
                ShownRow::Row(row) => {
                    if row.cells.is_empty() {
                        // The marker row past a truncated section list.
                        p.rect_filled(band, CornerRadius::ZERO, pal.gap);
                        p.text(
                            pos2(x0, y + lh / 2.0),
                            Align2::LEFT_CENTER,
                            format!(
                                "{} rows not classified (sections truncated)",
                                row.hidden_before
                            ),
                            FontId::proportional(12.0),
                            pal.gap_fg,
                        );
                        continue;
                    }
                    if row.hidden_before > 0 {
                        p.hline(band.x_range(), y, Stroke::new(1.0, pal.border));
                    }
                    let row_bg = match row.state {
                        hikasa_hex::RowState::Different => Some(pal.imp_bg),
                        hikasa_hex::RowState::Orphan => Some(pal.orph_bg),
                        hikasa_hex::RowState::Same => None,
                    };
                    if let Some(bg) = row_bg {
                        p.rect_filled(
                            Rect::from_min_max(
                                pos2(x0 + geometry.address as f32 * cw - cw / 2.0, y),
                                pos2(x0 + geometry.width() as f32 * cw, y + lh),
                            ),
                            CornerRadius::ZERO,
                            bg,
                        );
                    }
                    // The address of the row's first byte of this side (blank without one).
                    let address = if side == Side::Right {
                        row.right_address
                    } else {
                        row.left_address
                    };
                    if d.addresses
                        && let Some(a) = address
                    {
                        let text = hikasa_hex::nav::address_text(
                            side,
                            a,
                            d.layout.address,
                            m.address_width(),
                        );
                        p.text(
                            pos2(x0 + (geometry.address as f32 - 2.0) * cw, y + lh / 2.0),
                            Align2::RIGHT_CENTER,
                            text,
                            addr_font.clone(),
                            pal.gutter_fg,
                        );
                    }
                    let first_col = row.index * u64::from(geometry.bpl);
                    let mut hex_job = LayoutJob::default();
                    let mut char_job = LayoutJob::default();
                    let mut last_hex_x = geometry.address;
                    let mut last_char_x = geometry.char_x(0);
                    for (i, cell) in row.cells.iter().enumerate() {
                        let i32_ = i as u32;
                        let col = first_col + i as u64;
                        let byte = if side == Side::Right {
                            cell.right
                        } else {
                            cell.left
                        };
                        let (fg, bg) = cell_colours(cell.state, pal);
                        let hx = geometry.hex_x(i32_);
                        let cx = geometry.char_x(i32_);
                        let selected = sel.contains(&col);
                        let found = hit.as_ref().is_some_and(|(s, r)| {
                            *s == side
                                && m.byte_at(side, col)
                                    .is_some_and(|(o, held)| held && r.contains(&o))
                        });
                        let cell_bg = if found {
                            Some(pal.find_hl)
                        } else if selected && focused {
                            Some(pal.t_select)
                        } else if selected {
                            Some(pal.t_inactive_select)
                        } else {
                            bg
                        };
                        let rect_hex =
                            Rect::from_min_size(pos2(x0 + hx as f32 * cw, y), vec2(2.0 * cw, lh));
                        let rect_chr =
                            Rect::from_min_size(pos2(x0 + cx as f32 * cw, y), vec2(cw, lh));
                        match byte {
                            None => {
                                // A gap cell: the other side's byte is missing here.
                                p.rect_filled(rect_hex, CornerRadius::ZERO, pal.gap);
                                crate::ui::text::hatch(&p, rect_hex, pal.hatch);
                                if geometry.chars {
                                    p.rect_filled(rect_chr, CornerRadius::ZERO, pal.gap);
                                }
                            }
                            Some(_) => {
                                if let Some(c) = cell_bg {
                                    p.rect_filled(rect_hex, CornerRadius::ZERO, c);
                                    if geometry.chars {
                                        p.rect_filled(rect_chr, CornerRadius::ZERO, c);
                                    }
                                }
                            }
                        }
                        let spaces = |n: u32| " ".repeat(n as usize);
                        let fmt = TextFormat {
                            font_id: font.clone(),
                            color: fg,
                            ..TextFormat::default()
                        };
                        let gap_fmt = TextFormat {
                            font_id: font.clone(),
                            color: pal.t_text,
                            ..TextFormat::default()
                        };
                        hex_job.append(&spaces(hx - last_hex_x), 0.0, gap_fmt.clone());
                        last_hex_x = hx + 2;
                        match byte {
                            Some(b) => hex_job.append(&format!("{b:02x}"), 0.0, fmt.clone()),
                            None => hex_job.append("  ", 0.0, gap_fmt.clone()),
                        }
                        if geometry.chars {
                            char_job.append(&spaces(cx - last_char_x), 0.0, gap_fmt.clone());
                            last_char_x = cx + 1;
                            let c = byte.and_then(|b| d.layout.char_of(b)).unwrap_or(' ');
                            char_job.append(&c.to_string(), 0.0, fmt);
                        }
                        // The caret: boxed in the focus pane, outlined in the other and in both characters columns.
                        if col == pane_state.caret {
                            let (r_main, r_other) = match pane_state.part {
                                Part::Hex => (rect_hex, rect_chr),
                                Part::Chars => (rect_chr, rect_hex),
                            };
                            if byte.is_none() {
                                p.vline(
                                    r_main.left() + 1.0,
                                    r_main.y_range(),
                                    Stroke::new(2.0, pal.accent),
                                );
                            } else if focused {
                                p.rect_stroke(
                                    r_main,
                                    CornerRadius::same(2),
                                    Stroke::new(1.5, pal.accent),
                                    egui::StrokeKind::Inside,
                                );
                            } else {
                                p.rect_stroke(
                                    r_main,
                                    CornerRadius::same(2),
                                    Stroke::new(1.0, pal.muted),
                                    egui::StrokeKind::Inside,
                                );
                            }
                            if geometry.chars {
                                p.rect_stroke(
                                    r_other,
                                    CornerRadius::same(2),
                                    Stroke::new(1.0, pal.muted),
                                    egui::StrokeKind::Inside,
                                );
                            }
                        }
                    }
                    let galley = ui.ctx().fonts_mut(|f| f.layout_job(hex_job));
                    p.galley(
                        pos2(
                            x0 + geometry.address as f32 * cw,
                            y + (lh - galley.size().y) / 2.0,
                        ),
                        galley,
                        pal.t_text,
                    );
                    if geometry.chars {
                        let galley = ui.ctx().fonts_mut(|f| f.layout_job(char_job));
                        p.galley(
                            pos2(
                                x0 + geometry.char_x(0) as f32 * cw,
                                y + (lh - galley.size().y) / 2.0,
                            ),
                            galley,
                            pal.t_text,
                        );
                    }
                }
            }
        }
        if focused {
            p.rect_stroke(
                pane.body,
                CornerRadius::ZERO,
                Stroke::new(1.0, pal.accent.gamma_multiply(0.5)),
                egui::StrokeKind::Inside,
            );
        }
    }
    // The benchmark's mark: a frame whose rows all came from the cache (no placeholder).
    if view_rows > 0 && st.complete {
        st.drawn += 1;
    }
    // The splitter line.
    match d.panes {
        PaneLayout::SideBySide => painter.vline(
            panes[1].body.left() - 0.5,
            panes_rect.y_range(),
            Stroke::new(1.0, pal.border),
        ),
        PaneLayout::OverUnder => painter.hline(
            panes_rect.x_range(),
            panes[1].header.top() - 0.5,
            Stroke::new(1.0, pal.border),
        ),
    };

    // The thumbnail strip (D-360): drawn from the alignment's buckets; empty with the progress tick before it lands.
    if d.thumbnail {
        let strip = Rect::from_min_max(
            pos2(area.right() - BAR_W - THUMB_W, area.top()),
            pos2(area.right() - BAR_W, area.bottom()),
        );
        thumbnail_strip(ui, view, m, st, strip, pal, rows_visible, menus, &mut run);
    }
    // The scroll bar (rows in f64).
    let bar = Rect::from_min_max(pos2(area.right() - BAR_W, area.top()), area.max);
    let top_f = m.screen.top as f64 + f64::from(m.screen.frac);
    if let Some(top) = scroll::vertical(
        ui,
        id.with("bar"),
        bar,
        view_rows as f64,
        f64::from(rows_visible),
        top_f,
        pal,
    ) {
        let max = m.screen.max_top(view_rows) as f64;
        m.screen
            .thumb_to(if max > 0.0 { top / max } else { 0.0 }, view_rows);
    }
    // Repaint while the alignment or a prefetch runs, and for the primary selection's settle time.
    if m.busy() {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(50));
    }
    run
}

/// The panes' monospace metrics (§1.13.2 rule 2): the advance of a digit and the row height.
struct HexMetrics {
    cw: f32,
    lh: f32,
}

impl crate::text::wrap::Metrics for HexMetrics {
    fn advance(&self, _c: char) -> f32 {
        self.cw
    }

    fn line_height(&self) -> f32 {
        self.lh
    }
}

/// Draws a context menu's items.
fn menu_items(ui: &mut Ui, items: &[MenuItem], run: &mut Vec<Invocation>) {
    let mut hint = None;
    crate::ui::menus::items(ui, items, run, &mut hint);
}

fn pane_of(r: Rect, right: bool, header_h: f32) -> Pane {
    let header = Rect::from_min_size(r.min, vec2(r.width(), header_h));
    Pane {
        side: if right { Side::Right } else { Side::Left },
        header,
        body: Rect::from_min_max(pos2(r.left(), header.bottom()), r.max),
    }
}

fn rect_of(r: Rect) -> PaneRect {
    PaneRect {
        x: r.left(),
        y: r.top(),
        w: r.width(),
        h: r.height(),
    }
}

/// The characters of an address text (its widest form for the pair).
fn address_chars(m: &HexViewModel) -> u32 {
    let d = m.display();
    let width = m.address_width();
    match d.layout.address {
        hikasa_hex::Radix::Hex => {
            hikasa_hex::nav::address_text(Side::Left, 0, d.layout.address, width).len() as u32
        }
        hikasa_hex::Radix::Decimal => u32::from(width),
    }
}

/// The caret keys of the focus pane, then the catalogue's keys (Ctrl+C, Ctrl+A, F3, Ctrl+N, …).
fn caret_keys(
    ui: &mut Ui,
    m: &mut HexViewModel,
    id: Id,
    dispatch: &dyn Fn(&hikasa_session::settings::Shortcut) -> Option<CommandId>,
    run: &mut Vec<Invocation>,
) {
    ui.memory_mut(|mem| {
        mem.set_focus_lock_filter(
            id,
            EventFilter {
                vertical_arrows: true,
                horizontal_arrows: true,
                tab: true,
                escape: true,
            },
        );
    });
    let events: Vec<(Key, egui::Modifiers)> = ui.input(|i| {
        i.events
            .iter()
            .filter_map(|e| match e {
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } => Some((*key, *modifiers)),
                _ => None,
            })
            .collect()
    });
    for (key, mods) in events {
        let caret = match (key, mods.command || mods.ctrl) {
            (Key::ArrowLeft, false) => Some(CaretKey::Left),
            (Key::ArrowRight, false) => Some(CaretKey::Right),
            (Key::ArrowUp, false) if !mods.alt => Some(CaretKey::Up),
            (Key::ArrowDown, false) if !mods.alt => Some(CaretKey::Down),
            (Key::Home, false) => Some(CaretKey::Home),
            (Key::End, false) => Some(CaretKey::End),
            (Key::Home, true) => Some(CaretKey::First),
            (Key::End, true) => Some(CaretKey::Last),
            (Key::PageUp, false) => Some(CaretKey::PageUp),
            (Key::PageDown, false) => Some(CaretKey::PageDown),
            _ => None,
        };
        if let Some(k) = caret {
            m.key(k, mods.shift);
            if matches!(k, CaretKey::PageUp | CaretKey::PageDown) {
                let rows = m.row_count();
                m.screen
                    .page(if k == CaretKey::PageUp { -1 } else { 1 }, rows);
            }
            continue;
        }
        match key {
            Key::Tab if !mods.ctrl => {
                m.switch_part();
                continue;
            }
            Key::Escape => {
                m.escape();
                continue;
            }
            _ => {}
        }
        if let Some(cmd) = crate::ui::keys::shortcuts_of(key, mods)
            .iter()
            .find_map(dispatch)
        {
            run.push(Invocation::new(cmd));
        }
    }
}

/// The File Info header of a pane: the path edit (Enter reopens that side), *Browse…*, the format, the facts.
#[allow(clippy::too_many_arguments)]
fn header(
    ui: &mut Ui,
    view: ViewId,
    m: &mut HexViewModel,
    st: &mut HexUi,
    pane: &Pane,
    pal: &Palette,
    menus: &HexMenus<'_>,
    run: &mut Vec<Invocation>,
) {
    let side = pane.side;
    let k = index(side);
    let cell = pane.header;
    ui.painter_at(cell)
        .rect_filled(cell, CornerRadius::ZERO, pal.chrome);
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(cell.shrink2(vec2(4.0, 1.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center))
            .id_salt(("hex-header", view.0, k)),
    );
    let ui = &mut child;
    ui.spacing_mut().item_spacing.x = 6.0;
    let spec = m.inputs()[k].clone();
    let mut text = st.paths[k].clone().unwrap_or_else(|| spec.clone());
    let resp = ui.add(
        egui::TextEdit::singleline(&mut text)
            .font(FontId::monospace(12.0))
            .desired_width((cell.width() * 0.45).max(60.0)),
    );
    if resp.changed() || resp.has_focus() {
        st.paths[k] = Some(text.clone());
    }
    if resp.gained_focus() {
        m.focus_side(side);
    }
    if resp.lost_focus() {
        let accepted = ui.input(|i| i.key_pressed(Key::Enter));
        let typed = st.paths[k].take();
        if accepted && let Some(typed) = typed.filter(|t| t.trim() != spec && !t.trim().is_empty())
        {
            let os = std::ffi::OsString::from(typed.trim());
            match side {
                Side::Right => m.replace_specs(None, Some(os)),
                _ => m.replace_specs(Some(os), None),
            }
        }
    }
    if ui.small_button("…").on_hover_text("Browse…").clicked() {
        m.focus_side(side);
        run.push(Invocation::new(CommandId::Open));
    }
    let facts = m.header_facts(side);
    let format = facts
        .as_ref()
        .and_then(|f| f.format.clone())
        .unwrap_or_else(|| "Auto".to_owned());
    let fmt = ui.add(egui::Button::new(RichText::new(format).size(12.0)).small());
    if fmt.clicked() {
        m.focus_side(side);
        st.menu = Some(Area::FileHeader(side));
    }
    let area = Area::FileHeader(side);
    fmt.context_menu(|ui| {
        let items = (menus.menu)(area, &menus.ctx);
        menu_items(ui, &items, run);
    });
    match (m.error(), facts) {
        (Some((Some(s), e)), _) if *s == side => {
            ui.label(RichText::new(e).size(12.0).color(pal.error));
        }
        (_, Some(f)) => {
            ui.label(RichText::new(f.line()).size(12.0).color(pal.muted));
        }
        _ => {}
    }
}

/// The Find panel above the panes (D-359).
fn find_panel(
    ui: &mut Ui,
    view: ViewId,
    m: &mut HexViewModel,
    rect: Rect,
    pal: &Palette,
    run: &mut Vec<Invocation>,
) {
    ui.painter_at(rect)
        .rect_filled(rect, CornerRadius::ZERO, pal.chrome);
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(rect.shrink2(vec2(6.0, 2.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center))
            .id_salt(("hex-find", view.0)),
    );
    let ui = &mut child;
    ui.label("Find");
    let mut changed = false;
    let r = ui.add(
        egui::TextEdit::singleline(&mut m.find.pattern)
            .font(FontId::monospace(12.0))
            .desired_width(220.0)
            .hint_text(if m.find.hex { "DE AD ?? EF" } else { "text" }),
    );
    changed |= r.changed();
    if r.has_focus() {
        m.panel = Some(crate::hex::find::FIND_TOPIC);
    }
    if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
        run.push(Invocation::new(CommandId::FindNext));
    }
    changed |= ui.radio_value(&mut m.find.hex, false, "Text").changed();
    changed |= ui.radio_value(&mut m.find.hex, true, "Hex bytes").changed();
    ui.add_enabled_ui(!m.find.hex, |ui| {
        let shown = crate::hex::find::ENCODINGS
            .iter()
            .find(|(w, _)| *w == m.find.encoding)
            .map_or("UTF-8", |(_, c)| c);
        egui::ComboBox::from_id_salt(("hex-find-enc", view.0))
            .selected_text(shown)
            .show_ui(ui, |ui| {
                for (word, caption) in crate::hex::find::ENCODINGS {
                    changed |= ui
                        .selectable_value(&mut m.find.encoding, word, caption)
                        .changed();
                }
            });
        changed |= ui.checkbox(&mut m.find.match_case, "Match case").changed();
    });
    ui.checkbox(&mut m.find.wrap, "Wrap around");
    egui::ComboBox::from_id_salt(("hex-find-side", view.0))
        .selected_text(if m.find.side == Side::Right {
            "Right"
        } else {
            "Left"
        })
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut m.find.side, Side::Left, "Left");
            ui.selectable_value(&mut m.find.side, Side::Right, "Right");
        });
    if changed {
        m.validate_find();
    }
    let ok = m.find.finder().is_some();
    if ui.add_enabled(ok, egui::Button::new("Find Next")).clicked() {
        run.push(Invocation::new(CommandId::FindNext));
    }
    if ui
        .add_enabled(ok, egui::Button::new("Find Previous"))
        .clicked()
    {
        run.push(Invocation::new(CommandId::FindPrev));
    }
    if let Some(f) = m.searching() {
        ui.label(
            RichText::new(match f {
                Some(f) => format!("Searching… {} %", (f * 100.0).round() as u32),
                None => "Searching…".to_owned(),
            })
            .color(pal.muted),
        );
    }
    if let Some(e) = &m.find.message {
        ui.label(RichText::new(e).size(12.0).color(pal.error));
    }
    if ui.small_button("✕").on_hover_text("Close").clicked() {
        m.cancel_searches();
        m.find.open = false;
    }
}

/// The value panel under the panes (D-360).
fn values_panel(
    ui: &mut Ui,
    view: ViewId,
    m: &mut HexViewModel,
    rect: Rect,
    pal: &Palette,
    menus: &HexMenus<'_>,
    run: &mut Vec<Invocation>,
) {
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, CornerRadius::ZERO, pal.chrome);
    painter.hline(
        rect.x_range(),
        rect.top() + 0.5,
        Stroke::new(1.0, pal.border),
    );
    let (text, stale) = m
        .value_lines()
        .cloned()
        .unwrap_or_else(|| (String::new(), false));
    let colour = if stale { pal.muted } else { pal.fg };
    let response = ui.interact(rect, Id::unique(("hex-values", view.0)), Sense::click());
    if response.hovered() {
        m.panel = Some(crate::hex::values::VALUES_TOPIC);
    }
    let columns: Vec<&str> = text.split("Big Endian Values").collect();
    let (left, right) = match columns.as_slice() {
        [a, b] => (a.to_string(), format!("Big Endian Values{b}")),
        _ => (text.clone(), String::new()),
    };
    for (i, block) in [left, right].iter().enumerate() {
        painter.text(
            pos2(
                rect.left() + 10.0 + i as f32 * rect.width() / 2.0,
                rect.top() + 6.0,
            ),
            Align2::LEFT_TOP,
            block,
            FontId::monospace(12.0),
            colour,
        );
    }
    response.context_menu(|ui| {
        let items = (menus.menu)(Area::HexValues, &menus.ctx);
        // The panel's Copy copies its lines.
        let items: Vec<MenuItem> = items
            .into_iter()
            .map(|item| match item {
                MenuItem::Command {
                    invocation,
                    caption,
                    shortcut,
                    check,
                    enabled,
                    description,
                } if invocation.id == CommandId::Copy => MenuItem::Command {
                    invocation: Invocation::with(
                        CommandId::Copy,
                        CommandArg::Page(crate::hex::values::VALUES_COPY),
                    ),
                    caption,
                    shortcut,
                    check,
                    enabled,
                    description,
                },
                other => other,
            })
            .collect();
        menu_items(ui, &items, run);
    });
}

/// The thumbnail strip at the view's right edge (D-360).
#[allow(clippy::too_many_arguments)]
fn thumbnail_strip(
    ui: &mut Ui,
    view: ViewId,
    m: &mut HexViewModel,
    st: &mut HexUi,
    strip: Rect,
    pal: &Palette,
    rows_visible: u32,
    menus: &HexMenus<'_>,
    run: &mut Vec<Invocation>,
) {
    let painter = ui.painter_at(strip);
    painter.rect_filled(strip, CornerRadius::ZERO, pal.chrome);
    let height = strip.height().max(1.0) as u32;
    let Some(al) = m.alignment().cloned() else {
        // Before the alignment lands: the progress tick of the job.
        if let Some((_, Some(f))) = m.aligning() {
            let y = strip.top() + strip.height() * f;
            painter.hline(strip.x_range(), y, Stroke::new(2.0, pal.accent));
        }
        return;
    };
    let generation = m.generation;
    if st
        .thumb
        .as_ref()
        .is_none_or(|(g, h, _)| *g != generation || *h != height)
    {
        st.thumb = Some((
            generation,
            height,
            thumbnail::pixel_rows(&al.thumbnail, height),
        ));
    }
    if let Some((_, _, rows)) = &st.thumb {
        for (y, class) in rows.iter().enumerate() {
            let colour = match class {
                ThumbClass::Empty => continue,
                ThumbClass::Same => pal.t_bg,
                ThumbClass::Orphan => pal.thumb_orph,
                ThumbClass::Different => pal.thumb_imp,
            };
            let y = strip.top() + y as f32;
            painter.hline(strip.x_range(), y + 0.5, Stroke::new(1.0, colour));
        }
    }
    // The shown rows framed.
    let total = m.columns();
    if let (Some(a), Some(b)) = (
        m.first_column_of(m.screen.top),
        m.first_column_of(m.screen.top + u64::from(rows_visible))
            .or(Some(total)),
    ) {
        let (y0, y1) = thumbnail::frame_of(a..b, total, height);
        painter.rect_stroke(
            Rect::from_min_max(
                pos2(strip.left() + 1.0, strip.top() + y0 as f32),
                pos2(strip.right() - 1.0, strip.top() + y1 as f32),
            ),
            CornerRadius::ZERO,
            Stroke::new(1.0, pal.accent),
            egui::StrokeKind::Inside,
        );
    }
    let r = ui
        .interact(
            strip,
            Id::unique(("hex-thumb", view.0)),
            Sense::click_and_drag(),
        )
        .on_hover_cursor(CursorIcon::PointingHand);
    if (r.clicked() || r.dragged())
        && let Some(p) = r.interact_pointer_pos()
    {
        m.thumbnail_to(f64::from((p.y - strip.top()) / strip.height().max(1.0)));
    }
    r.context_menu(|ui| {
        let items = (menus.menu)(Area::HexThumbnail, &menus.ctx);
        menu_items(ui, &items, run);
    });
}
