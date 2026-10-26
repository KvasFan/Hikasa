//! The Text Compare pane (ARCH-GUI-002): file headers, the thumbnail, the two side-by-side panes and the scroll
//! bar, custom-painted with egui's painter for the visible rows only. The rows come from
//! [`TextViewModel::slice`] (the Rust layout model computes only the slice on screen); this module turns them
//! into backgrounds, line numbers, syntax-coloured text runs, inline highlights and gap bars. The scroll
//! position is a row index (`f64`), so a million-row view scrolls exactly.

use egui::text::{ByteIndex, LayoutJob, LayoutSection, TextFormat};
use egui::{
    Align2, Color32, CornerRadius, CursorIcon, EventFilter, FontId, Id, Key, Painter, Pos2, Rect,
    Sense, Stroke, Ui, Vec2, pos2, vec2,
};
use hikasa_session::ipc::ViewId;

use crate::text_view::{SliceRow, TextPhase, TextSlice, TextStatus, TextViewModel};
use crate::ui::scroll;
use crate::ui::theme::Palette;

/// Height of the pane header strip.
const HEADER_H: f32 = 24.0;
/// Width of the thumbnail column.
const THUMB_W: f32 = 14.0;
/// Width of the scroll bar.
const BAR_W: f32 = 12.0;
/// Rows fetched beyond the visible ones (both directions), so a few lines of scrolling reuse the slice.
const SLICE_MARGIN: u32 = 24;
/// Left padding of the text after the gutter.
const PAD: f32 = 4.0;

/// The thumbnail cells of one generation and height.
struct Thumb {
    generation: u64,
    height: u32,
    cells: Vec<u8>,
}

/// UI state of one text view (kept by the window between frames).
#[derive(Default)]
pub struct TextUi {
    /// First visible display position (fractional: smooth scrolling).
    pub top: f64,
    left: f32,
    meta: Option<crate::text_view::TextMeta>,
    refresh: bool,
    slice: Option<TextSlice>,
    thumb: Option<Thumb>,
    widest: f32,
    pending_scroll: Option<u32>,
    focused_once: bool,
    /// Highest phase drawn so far: 1 the loaded text (unaligned), 2 the aligned comparison with syntax colours.
    pub drawn: u8,
    /// Display positions in the view.
    pub len: u32,
    /// Whole rows that fit on screen.
    pub page_rows: f64,
    /// Time to build the last slice (ms), when one was built this frame.
    pub slice_ms: Option<f64>,
}

impl TextUi {
    /// The model's data or scroll request changed: refetch its meta next frame.
    pub fn mark_dirty(&mut self) {
        self.refresh = true;
    }

    /// The largest valid first row.
    pub fn max_top(&self) -> f64 {
        (f64::from(self.len) - self.page_rows).max(0.0)
    }
}

/// Byte and char offsets of UTF-16 columns of a displayed line (the slices count UTF-16 units). Built once per
/// row side in one pass over the text, so every lookup is O(1) and painting a row is linear in its length.
struct Columns {
    /// `(byte offset, char index)` per UTF-16 unit, plus the end of the text; empty for an ASCII line (columns,
    /// bytes and chars coincide).
    table: Vec<(u32, u32)>,
    /// `(bytes, chars)` of the whole text.
    end: (usize, usize),
}

impl Columns {
    fn new(text: &str) -> Columns {
        if text.is_ascii() {
            return Columns {
                table: Vec::new(),
                end: (text.len(), text.len()),
            };
        }
        let mut table = Vec::with_capacity(text.len() + 1);
        let mut chars = 0u32;
        for (byte, c) in text.char_indices() {
            table.push((byte as u32, chars));
            // The second unit of a surrogate pair maps to the next char (a column inside a char rounds up).
            for _ in 1..c.len_utf16() {
                table.push(((byte + c.len_utf8()) as u32, chars + 1));
            }
            chars += 1;
        }
        table.push((text.len() as u32, chars));
        Columns {
            table,
            end: (text.len(), chars as usize),
        }
    }

    /// (byte offset, char index) of UTF-16 column `unit` (clamped to the text).
    fn at(&self, unit: u32) -> (usize, usize) {
        if self.table.is_empty() {
            let n = (unit as usize).min(self.end.0);
            return (n, n);
        }
        self.table
            .get(unit as usize)
            .map_or(self.end, |&(b, c)| (b as usize, c as usize))
    }
}

/// The text of one row side as a layout job: one section per syntax segment (`start, end, colour` triples).
fn row_job(
    text: &str,
    cols: &Columns,
    segments: &[u32],
    font: &FontId,
    pal: &Palette,
) -> LayoutJob {
    let mut job = LayoutJob {
        text: text.to_owned(),
        ..LayoutJob::default()
    };
    let format = |colour: u32| TextFormat {
        font_id: font.clone(),
        color: pal.syntax_colour(colour),
        ..TextFormat::default()
    };
    let mut at = 0usize;
    for seg in segments.as_chunks::<3>().0 {
        let (start, _) = cols.at(seg[0]);
        let (end, _) = cols.at(seg[1]);
        let start = start.max(at);
        if end <= start {
            continue;
        }
        if start > at {
            job.sections.push(LayoutSection {
                leading_space: 0.0,
                byte_range: ByteIndex(at)..ByteIndex(start),
                format: format(0),
            });
        }
        job.sections.push(LayoutSection {
            leading_space: 0.0,
            byte_range: ByteIndex(start)..ByteIndex(end),
            format: format(seg[2]),
        });
        at = end;
    }
    if at < text.len() {
        job.sections.push(LayoutSection {
            leading_space: 0.0,
            byte_range: ByteIndex(at)..ByteIndex(text.len()),
            format: format(0),
        });
    }
    job
}

/// Diagonal hatching of an absent side.
pub fn hatch(painter: &Painter, rect: Rect, colour: Color32) {
    let p = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
    let stroke = Stroke::new(1.0, colour);
    let step = 7.0;
    let mut x = rect.left() - rect.height();
    while x < rect.right() {
        p.line_segment(
            [pos2(x, rect.bottom()), pos2(x + rect.height(), rect.top())],
            stroke,
        );
        x += step;
    }
}

/// A small right-pointing triangle (gap bars, closed folders) or a down-pointing one (open folders).
pub fn triangle(painter: &Painter, centre: Pos2, size: f32, open: bool, colour: Color32) {
    let h = size / 2.0;
    let points = if open {
        vec![
            pos2(centre.x - h, centre.y - h * 0.6),
            pos2(centre.x + h, centre.y - h * 0.6),
            pos2(centre.x, centre.y + h * 0.7),
        ]
    } else {
        vec![
            pos2(centre.x - h * 0.6, centre.y - h),
            pos2(centre.x + h * 0.7, centre.y),
            pos2(centre.x - h * 0.6, centre.y + h),
        ]
    };
    painter.add(egui::Shape::convex_polygon(points, colour, Stroke::NONE));
}

/// Draws the text view of `view` into the remaining space of `ui`.
pub fn show(
    ui: &mut Ui,
    view: ViewId,
    m: &mut TextViewModel,
    st: &mut TextUi,
    status: &TextStatus,
    pal: &Palette,
) {
    if st.refresh || st.meta.is_none() {
        st.refresh = false;
        let meta = m.meta();
        if meta.scroll >= 0 {
            st.pending_scroll = u32::try_from(meta.scroll).ok();
        }
        if st.meta.as_ref().map(|x| x.generation) != Some(meta.generation) {
            st.slice = None;
            st.widest = 0.0;
        }
        st.meta = Some(meta);
    }
    let Some(meta) = st.meta.as_ref() else {
        return;
    };
    let (generation, len, rows, phase, syntax, cursor, show_thumb, digits, font_px) = (
        meta.generation,
        meta.len,
        meta.rows,
        meta.phase,
        meta.syntax,
        meta.cursor,
        meta.thumb,
        meta.digits,
        meta.font,
    );
    st.len = len;
    st.slice_ms = None;

    let full = ui.available_rect_before_wrap();
    ui.allocate_rect(full, Sense::hover());
    let painter = ui.painter_at(full);
    let thumb_w = if show_thumb { THUMB_W } else { 0.0 };
    let panes_left = full.left() + thumb_w;
    let panes_right = full.right() - BAR_W;
    let pane_w = ((panes_right - panes_left - 1.0) / 2.0).floor().max(10.0);
    let pane_x = [panes_left, panes_left + pane_w + 1.0];

    // Header: title and file details per pane.
    let header = Rect::from_min_size(full.min, vec2(full.width(), HEADER_H));
    painter.rect_filled(header, CornerRadius::ZERO, pal.chrome);
    painter.hline(
        full.x_range(),
        header.bottom() - 0.5,
        Stroke::new(1.0, pal.border),
    );
    let small = FontId::proportional(12.0);
    let mono_small = FontId::monospace(12.0);
    for (k, pane) in status.panes.iter().enumerate() {
        let cell = Rect::from_min_max(
            pos2(pane_x[k], header.top()),
            pos2(pane_x[k] + pane_w, header.bottom()),
        );
        let p = painter.with_clip_rect(cell);
        let title = p.text(
            pos2(cell.left() + 8.0, cell.center().y),
            Align2::LEFT_CENTER,
            &pane.title,
            mono_small.clone(),
            pal.fg,
        );
        p.text(
            pos2(title.right() + 10.0, cell.center().y),
            Align2::LEFT_CENTER,
            &pane.detail,
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
    let text_rect = Rect::from_min_max(
        pos2(panes_left, body.top()),
        pos2(panes_right, body.bottom()),
    );
    painter.rect_filled(body, CornerRadius::ZERO, pal.bg);

    let font = FontId::monospace(font_px as f32);
    let lh = (font_px as f32 * 1.4).round().max(4.0);
    let cw = ui.ctx().fonts_mut(|f| f.glyph_width(&font, 'M')).max(1.0);
    let page = f64::from((body.height() / lh).floor().max(1.0));
    st.page_rows = page;
    let id = Id::unique(("hikasa-text", view.0));

    // Input: wheel, clicks, keys.
    let response = ui.interact(text_rect, id, Sense::click());
    if !st.focused_once {
        st.focused_once = true;
        response.request_focus();
    }
    if response.hovered() {
        let delta = ui.input(|i| i.smooth_scroll_delta());
        if delta != Vec2::ZERO {
            st.top -= f64::from(delta.y / lh);
            st.left = (st.left - delta.x).clamp(0.0, st.widest.max(0.0));
            ui.ctx().input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
        }
    }
    if response.clicked() {
        response.request_focus();
        if let Some(p) = response.interact_pointer_pos() {
            let pos = (st.top + f64::from((p.y - body.top()) / lh)).floor();
            if pos >= 0.0 && pos < f64::from(len) {
                m.click(pos as u32);
                st.refresh = true;
            }
        }
    }
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
        let (up, down, pg_up, pg_down, home, end, left, right) = ui.input(|i| {
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
        let step = (page - 1.0).max(1.0);
        st.top += f64::from(u8::from(down)) - f64::from(u8::from(up))
            + step * (f64::from(u8::from(pg_down)) - f64::from(u8::from(pg_up)));
        if home {
            st.top = 0.0;
        }
        if end {
            st.top = f64::from(len);
        }
        st.left = (st.left + 4.0 * cw * (f32::from(u8::from(right)) - f32::from(u8::from(left))))
            .clamp(0.0, st.widest.max(0.0));
    }

    // Scroll bar and thumbnail.
    let bar = Rect::from_min_max(pos2(panes_right, body.top()), body.max);
    if let Some(top) = scroll::vertical(ui, id.with("bar"), bar, f64::from(len), page, st.top, pal)
    {
        st.top = top;
    }
    let thumb_rect = Rect::from_min_max(body.min, pos2(panes_left, body.bottom()));
    if show_thumb && phase == TextPhase::Ready && rows > 0 {
        let r = ui
            .interact(thumb_rect, id.with("thumb"), Sense::click_and_drag())
            .on_hover_cursor(CursorIcon::PointingHand);
        if (r.clicked() || r.dragged())
            && let Some(p) = r.interact_pointer_pos()
        {
            let frac = f64::from(((p.y - body.top()) / body.height().max(1.0)).clamp(0.0, 1.0));
            let row = (frac * f64::from(rows)).floor().min(f64::from(rows - 1)) as u32;
            st.top = f64::from(m.position_of_row(row)) - page / 2.0;
        }
    }

    // A one-shot scroll request (next/previous section, a restored cursor) brings its position into view.
    if let Some(pos) = st.pending_scroll.take() {
        let pos = f64::from(pos);
        if pos < st.top || pos > st.top + page - 1.0 {
            st.top = pos - (page / 3.0).floor();
        }
    }
    st.top = st.top.clamp(0.0, st.max_top());
    m.set_viewport(st.top.floor() as u32, page as u32);

    if len == 0 {
        let message = st.meta.as_ref().map_or("", |x| x.message.as_str());
        painter.text(
            pos2(
                text_rect.center().x,
                body.top() + 60.0_f32.min(body.height() / 2.0),
            ),
            Align2::CENTER_CENTER,
            message,
            FontId::proportional(14.0),
            pal.muted,
        );
        if show_thumb {
            painter.rect_filled(thumb_rect, CornerRadius::ZERO, pal.chrome);
        }
        return;
    }

    // The slice on screen.
    let first = st.top.floor() as u32;
    let frac = (st.top - st.top.floor()) as f32;
    let count = ((body.height() / lh).ceil() as u32 + 1).min(len - first);
    let covered = st.slice.as_ref().is_some_and(|s| {
        s.generation == generation
            && s.start <= first
            && first + count <= s.start + s.rows.len() as u32
    });
    if !covered {
        let start = first.saturating_sub(SLICE_MARGIN);
        let want = (first - start) + count + SLICE_MARGIN;
        let began = std::time::Instant::now();
        let slice = m.slice(start, want);
        st.slice_ms = Some(began.elapsed().as_secs_f64() * 1000.0);
        if slice.generation != generation {
            // The model moved on since the meta was read: refetch both next frame.
            st.refresh = true;
            ui.ctx().request_repaint();
        }
        st.slice = Some(slice);
    }
    let Some(slice) = st.slice.as_ref() else {
        return;
    };

    let gutter_w = ((digits as f32 + 1.5) * cw).ceil();
    let clip_body = painter.with_clip_rect(body);
    let pane_rects = [0usize, 1].map(|k| {
        Rect::from_min_max(
            pos2(pane_x[k], body.top()),
            pos2(pane_x[k] + pane_w, body.bottom()),
        )
    });
    let text_painters = [0usize, 1].map(|k| {
        painter.with_clip_rect(Rect::from_min_max(
            pos2(pane_rects[k].left() + gutter_w, body.top()),
            pane_rects[k].max,
        ))
    });
    let mut widest = st.widest;
    let mut drawn_text = false;
    for i in 0..count {
        let pos = first + i;
        let Some(row) = slice.rows.get((pos - slice.start) as usize) else {
            break;
        };
        let y = body.top() + (i as f32 - frac) * lh;
        let full_row = Rect::from_min_max(pos2(panes_left, y), pos2(panes_right, y + lh));
        if row.k == 1 {
            draw_gap(&clip_body, full_row, row, gutter_w, pal);
            continue;
        }
        for side in 0..2 {
            let pane = pane_rects[side];
            let row_rect = Rect::from_min_max(pos2(pane.left(), y), pos2(pane.right(), y + lh));
            let gutter = Rect::from_min_max(row_rect.min, pos2(pane.left() + gutter_w, y + lh));
            let text_bg = Rect::from_min_max(pos2(gutter.right(), y), row_rect.max);
            clip_body.rect_filled(gutter, CornerRadius::ZERO, pal.gutter);
            let Some(text) = row.t[side].as_deref() else {
                hatch(&clip_body, text_bg, pal.hatch);
                continue;
            };
            if let Some(bg) = row_background(row, pal) {
                clip_body.rect_filled(text_bg, CornerRadius::ZERO, bg);
            }
            if row.ln[side] > 0 {
                clip_body.text(
                    pos2(gutter.right() - cw * 0.5, y + lh / 2.0),
                    Align2::RIGHT_CENTER,
                    row.ln[side],
                    font.clone(),
                    pal.gutter_fg,
                );
            }
            if text.is_empty() {
                continue;
            }
            let cols = Columns::new(text);
            let job = row_job(text, &cols, &row.sp[side], &font, pal);
            let galley = text_painters[side].layout_job(job);
            let origin = pos2(
                text_bg.left() + PAD - st.left,
                y + (lh - galley.size().y) / 2.0,
            );
            widest = widest.max(galley.size().x + PAD * 2.0 - pane_w + gutter_w);
            let hl = &row.hl[side];
            if !hl.is_empty()
                && let Some(placed) = galley.rows.first()
            {
                for h in hl.as_chunks::<3>().0 {
                    let (_, c0) = cols.at(h[0]);
                    let (_, c1) = cols.at(h[1]);
                    let x0 = placed.glyphs.get(c0).map_or(placed.size.x, |g| g.pos.x);
                    let x1 = placed.glyphs.get(c1).map_or(placed.size.x, |g| g.pos.x);
                    let rect = Rect::from_min_max(
                        pos2(origin.x + x0, y + 1.0),
                        pos2(origin.x + x1.max(x0 + 2.0), y + lh - 1.0),
                    );
                    let colour = if h[2] != 0 { pal.imp_hl } else { pal.unimp_hl };
                    text_painters[side].rect_filled(rect, CornerRadius::ZERO, colour);
                }
            }
            text_painters[side].galley(origin, galley, pal.fg);
            drawn_text = true;
        }
    }
    st.widest = widest.max(0.0);
    // The divider between the panes and the cursor row.
    clip_body.vline(
        pane_x[1] - 0.5,
        body.y_range(),
        Stroke::new(1.0, pal.border),
    );
    if cursor >= 0 {
        let c = cursor as f64 - st.top;
        if c > -1.0 && c < f64::from(count) {
            let y = body.top() + c as f32 * lh;
            clip_body.rect_stroke(
                Rect::from_min_max(
                    pos2(panes_left + 0.5, y + 0.5),
                    pos2(panes_right - 0.5, y + lh - 0.5),
                ),
                CornerRadius::ZERO,
                Stroke::new(1.0, pal.accent),
                egui::StrokeKind::Inside,
            );
        }
    }
    if show_thumb {
        draw_thumbnail(
            ui, &painter, thumb_rect, m, st, generation, rows, first, count, pal,
        );
    }
    if drawn_text || count > 0 {
        let drawn = match phase {
            TextPhase::Loaded => 1,
            TextPhase::Ready if syntax => 2,
            TextPhase::Ready => 1,
            _ => 0,
        };
        st.drawn = st.drawn.max(drawn);
    }
}

/// The background of a row's text (TC-031): changed rows by importance, orphans lighter; same rows none.
fn row_background(row: &SliceRow, pal: &Palette) -> Option<Color32> {
    match (row.st, row.im) {
        (1, 2) => Some(pal.imp_bg),
        (1, _) => Some(pal.unimp_bg),
        (2 | 3, 2) => Some(pal.orph_imp_bg),
        (2 | 3, _) => Some(pal.orph_unimp_bg),
        _ => None,
    }
}

/// A gap bar: the hidden rows of the display filter, expandable by a click (TC-035).
fn draw_gap(painter: &Painter, rect: Rect, row: &SliceRow, indent: f32, pal: &Palette) {
    painter.rect_filled(rect.shrink2(vec2(0.0, 1.0)), CornerRadius::ZERO, pal.gap);
    let x = rect.left() + indent + PAD;
    triangle(
        painter,
        pos2(x + 4.0, rect.center().y),
        8.0,
        false,
        pal.gap_fg,
    );
    let label = if row.g == 1 {
        "1 hidden line".to_owned()
    } else {
        format!("{} hidden lines", row.g)
    };
    painter.text(
        pos2(x + 14.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(12.0),
        pal.gap_fg,
    );
}

/// The thumbnail (TC-032): one bucket per pixel row, coloured by the most significant change in it, with the
/// visible part marked.
#[allow(clippy::too_many_arguments)] // one call site; the values come from the frame being drawn
fn draw_thumbnail(
    ui: &Ui,
    painter: &Painter,
    rect: Rect,
    m: &TextViewModel,
    st: &mut TextUi,
    generation: u64,
    rows: u32,
    first: u32,
    count: u32,
    pal: &Palette,
) {
    painter.rect_filled(rect, CornerRadius::ZERO, pal.chrome);
    painter.vline(
        rect.right() - 0.5,
        rect.y_range(),
        Stroke::new(1.0, pal.border),
    );
    if rows == 0 || m.response().is_none() {
        return;
    }
    let ppp = ui.ctx().pixels_per_point();
    let height = ((rect.height() * ppp).round() as u32).max(1);
    let fresh = st
        .thumb
        .as_ref()
        .is_some_and(|t| t.generation == generation && t.height == height);
    if !fresh {
        st.thumb = Some(Thumb {
            generation,
            height,
            cells: m.thumbnail(height).cells,
        });
    }
    let Some(thumb) = st.thumb.as_ref() else {
        return;
    };
    let scale = rect.height() / thumb.cells.len().max(1) as f32;
    let inner = rect.shrink2(vec2(2.0, 0.0));
    // Runs of equal colour become one rectangle.
    let mut run: Option<(usize, Color32)> = None;
    let colour_of = |c: u8| {
        if c & 4 != 0 {
            Some(pal.thumb_imp)
        } else if c & 24 != 0 {
            Some(pal.thumb_orph)
        } else if c & 2 != 0 {
            Some(pal.thumb_unimp)
        } else {
            None
        }
    };
    let flush = |start: usize, end: usize, colour: Color32| {
        let y0 = rect.top() + start as f32 * scale;
        let y1 = (rect.top() + end as f32 * scale).max(y0 + 1.0 / ppp);
        painter.rect_filled(
            Rect::from_min_max(pos2(inner.left(), y0), pos2(inner.right(), y1)),
            CornerRadius::ZERO,
            colour,
        );
    };
    for (i, &c) in thumb.cells.iter().enumerate() {
        let colour = colour_of(c);
        match (run, colour) {
            (Some((_, a)), Some(b)) if a == b => {}
            (prev, next) => {
                if let Some((start, a)) = prev {
                    flush(start, i, a);
                }
                run = next.map(|b| (i, b));
            }
        }
    }
    if let Some((start, a)) = run {
        flush(start, thumb.cells.len(), a);
    }
    // The visible rows.
    let Some(slice) = st.slice.as_ref() else {
        return;
    };
    let row_at = |pos: u32| {
        slice
            .rows
            .get(pos.saturating_sub(slice.start) as usize)
            .map(|r| (r.r, r.g.max(1)))
    };
    let (r0, _) = row_at(first).unwrap_or((first, 1));
    let (r1, g1) = row_at(first + count.saturating_sub(2)).unwrap_or((first + count, 1));
    let y0 = rect.top() + rect.height() * (r0 as f32 / rows as f32);
    let y1 = (rect.top() + rect.height() * ((r1 + g1) as f32 / rows as f32)).max(y0 + 2.0);
    painter.rect_filled(
        Rect::from_min_max(
            pos2(rect.left(), y0),
            pos2(rect.right(), y1.min(rect.bottom())),
        ),
        CornerRadius::ZERO,
        pal.thumb_view,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_columns_map_to_bytes_and_chars_and_jobs_cover_the_line() {
        let ascii = Columns::new("int x;");
        assert_eq!(ascii.at(3), (3, 3));
        assert_eq!(ascii.at(99), (6, 6));
        // "é" is one UTF-16 unit and two bytes; "😀" two units and four bytes.
        let wide = Columns::new("é😀x");
        assert_eq!(wide.at(1), (2, 1));
        assert_eq!(wide.at(3), (6, 2));
        assert_eq!(wide.at(4), (7, 3));
        let font = FontId::monospace(13.0);
        let job = row_job(
            "int x = 1;",
            &Columns::new("int x = 1;"),
            &[0, 3, 2, 3, 8, 0, 8, 9, 3, 9, 10, 0],
            &font,
            &crate::ui::theme::LIGHT,
        );
        let ranges: Vec<_> = job
            .sections
            .iter()
            .map(|s| (s.byte_range.start.0, s.byte_range.end.0))
            .collect();
        assert_eq!(ranges, vec![(0, 3), (3, 8), (8, 9), (9, 10)]);
        assert_eq!(
            job.sections[0].format.color,
            crate::ui::theme::LIGHT.syntax[1]
        );
        let bare = row_job(
            "abc",
            &Columns::new("abc"),
            &[],
            &font,
            &crate::ui::theme::LIGHT,
        );
        assert_eq!(bare.sections.len(), 1);
        assert_eq!(bare.sections[0].byte_range, ByteIndex(0)..ByteIndex(3));
    }

    /// The column of a UTF-16 unit by walking the text (the definition the table must match).
    fn walk(text: &str, unit: u32) -> (usize, usize) {
        let mut units = 0u32;
        for (k, (byte, c)) in text.char_indices().enumerate() {
            if units >= unit {
                return (byte, k);
            }
            units += c.len_utf16() as u32;
        }
        (text.len(), text.chars().count())
    }

    #[test]
    fn column_table_matches_the_definition() {
        for text in [
            "",
            "abc",
            "é",
            "😀",
            "aé😀b",
            "😀😀",
            "x\u{2400}é\t😀\u{202e}z",
        ] {
            let cols = Columns::new(text);
            let units = text.encode_utf16().count() as u32;
            for unit in 0..=units + 2 {
                assert_eq!(cols.at(unit), walk(text, unit), "{text:?} unit {unit}");
            }
        }
    }

    /// Regression (wave-6b review): a row of 4096 non-ASCII columns with a syntax segment and an inline
    /// highlight every three columns (the reviewer's `1é ` × 1365 fixtures) used to cost a scan of the line per
    /// lookup, quadratic per row. 200 such rows, both sides, finish in a few milliseconds now; the bound is
    /// generous (the quadratic version took seconds even in release builds).
    #[test]
    fn long_non_ascii_rows_are_linear() {
        let text = "1é ".repeat(1365);
        let units = text.encode_utf16().count() as u32;
        assert_eq!(units, 4095);
        let segments: Vec<u32> = (0..1365u32)
            .flat_map(|k| [3 * k, 3 * k + 2, 1 + k % 8, 3 * k + 2, 3 * k + 3, 0])
            .collect();
        let font = FontId::monospace(13.0);
        let began = std::time::Instant::now();
        let mut checksum = 0usize;
        for _ in 0..400 {
            let cols = Columns::new(&text);
            let job = row_job(&text, &cols, &segments, &font, &crate::ui::theme::LIGHT);
            checksum += job.sections.len();
            for k in 0..1365u32 {
                checksum += cols.at(3 * k).1 + cols.at(3 * k + 2).1;
            }
        }
        let elapsed = began.elapsed();
        assert_eq!(
            checksum,
            400 * (2730 + (0..1365).map(|k| 6 * k + 2).sum::<usize>())
        );
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "400 rows of 4096 non-ASCII columns took {elapsed:?}"
        );
    }
}
