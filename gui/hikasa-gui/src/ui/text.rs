//! The Text Compare panes (ARCH-GUI-002; M2 W11a, contract §1.11.2–§1.11.12): the File Info headers, the thumbnail,
//! the two editable panes with the copy strip between them, and the scroll bar, custom-painted with egui's painter
//! for the rows on screen only. The rows' states, numbers and colours come from [`TextViewModel::slice`]; the
//! geometry of their lines (visual lines, the caret, hit tests) from the model's [`TextViewModel::screen`] over
//! [`EguiMetrics`]; every edit, move and selection is a model call ([`crate::text::input`], [`crate::text::edit`]).
//! The scroll position is a display position (`f64`), so a million-row view scrolls exactly.

use std::cell::RefCell;
use std::collections::HashMap;

use egui::text::{ByteIndex, LayoutJob, LayoutSection, TextFormat};
use egui::{
    Align2, Color32, CornerRadius, CursorIcon, EventFilter, FontFamily, FontId, Id, Key, Painter,
    PointerButton, Pos2, Rect, Sense, Stroke, Ui, Vec2, pos2, vec2,
};
use hikasa_core::Side;
use hikasa_session::ipc::ViewId;
use hikasa_text::LineIdx;
use hikasa_text::edit::LineMark;

use crate::text::edit::{copy_section, other, sections_starting_in};
use crate::text::input::{self, Clicks, Ime, Press};
use crate::text::screen::ScreenRow;
use crate::text::wrap::Metrics;
use crate::text::{
    SliceRow, TextPhase, TextSlice, TextStatus, TextViewModel, clipboard, index_of, side_of,
};
use crate::ui::scroll;
use crate::ui::theme::Palette;

/// Height of the pane header strip.
const HEADER_H: f32 = 24.0;
/// Width of the thumbnail column.
const THUMB_W: f32 = 14.0;
/// Width of the scroll bar.
const BAR_W: f32 = 12.0;
/// Width of the strip between the panes (the copy buttons, D-299).
const STRIP_W: f32 = crate::text::layout::STRIP_W;
/// Rows fetched beyond the visible ones (both directions), so a few lines of scrolling reuse the slice.
const SLICE_MARGIN: u32 = 24;
/// Left padding of the text after the gutter.
const PAD: f32 = 4.0;
/// The caret blinks this long on and off (D-295).
const BLINK_S: f64 = 0.5;

/// The thumbnail cells of one install and height.
struct Thumb {
    installs: u64,
    height: u32,
    cells: Vec<u8>,
}

/// UI state of one text view (kept by the window between frames).
#[derive(Default)]
pub struct TextUi {
    /// First visible display position (fractional: smooth scrolling).
    pub top: f64,
    left: f32,
    meta: Option<crate::text::TextMeta>,
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
    /// The glyph widths of the editor font (the metrics' cache) and the font they are of.
    widths: RefCell<HashMap<char, f32>>,
    widths_of: Option<FontId>,
    /// The frame time of the last key or edit (the caret is solid for a blink after it).
    last_key: f64,
    /// A drag started in the gutter.
    gutter_drag: bool,
    /// A drag started in a pane.
    text_drag: bool,
    /// The generation the slice's text rows were drawn for (the G16 benches' check).
    pub painted_generation: u64,
    /// The panes had the keyboard in the last frame (the benchmarks' check).
    pub had_focus: bool,
    /// The panes take the keyboard in the next frame (the benchmarks' scripted caret, as a click would).
    pub focus_request: bool,
    /// The focus pane's caret on screen last frame (the IME's cursor rectangle).
    caret_rect: Option<Rect>,
    /// The caret moved: bring it into view horizontally once it is laid out.
    reveal_x: bool,
    /// The area whose context menu is open (D-312).
    menu_area: Option<crate::commands::Area>,
    /// Where it opened.
    menu_anchor: Option<Pos2>,
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

/// The window's [`Metrics`]: egui's glyph widths of the editor font, cached per character (D-293 rule 2).
pub struct EguiMetrics<'a> {
    ctx: &'a egui::Context,
    font: FontId,
    cache: &'a RefCell<HashMap<char, f32>>,
    line_height: f32,
}

impl Metrics for EguiMetrics<'_> {
    fn advance(&self, c: char) -> f32 {
        if let Some(w) = self.cache.borrow().get(&c) {
            return *w;
        }
        let w = self.ctx.fonts_mut(|f| f.glyph_width(&self.font, c));
        self.cache.borrow_mut().insert(c, w);
        w
    }

    fn line_height(&self) -> f32 {
        self.line_height
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

/// The part `[a, b)` (bytes) of a job: its text and its sections clipped.
fn sub_job(job: &LayoutJob, a: usize, b: usize) -> LayoutJob {
    let mut out = LayoutJob {
        text: job.text[a..b].to_owned(),
        ..LayoutJob::default()
    };
    for s in &job.sections {
        let (s0, s1) = (s.byte_range.start.0.max(a), s.byte_range.end.0.min(b));
        if s0 < s1 {
            out.sections.push(LayoutSection {
                leading_space: 0.0,
                byte_range: ByteIndex(s0 - a)..ByteIndex(s1 - a),
                format: s.format.clone(),
            });
        }
    }
    out
}

/// A plain job of `text` in the default colour.
fn plain_job(text: &str, font: &FontId, colour: Color32) -> LayoutJob {
    LayoutJob::simple_singleline(text.to_owned(), font.clone(), colour)
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

/// A small left- or right-pointing arrow of the copy strip.
fn arrow(painter: &Painter, centre: Pos2, size: f32, right: bool, colour: Color32) {
    let h = size / 2.0;
    let (tip, back) = if right {
        (centre.x + h, centre.x - h)
    } else {
        (centre.x - h, centre.x + h)
    };
    painter.add(egui::Shape::convex_polygon(
        vec![
            pos2(back, centre.y - h * 0.8),
            pos2(tip, centre.y),
            pos2(back, centre.y + h * 0.8),
        ],
        colour,
        Stroke::NONE,
    ));
}

/// The id of a text view's panes (their keyboard focus, D-111).
pub fn pane_id(view: ViewId) -> Id {
    Id::unique(("hikasa-text", view.0))
}

/// The pane key of an egui key (§1.11.4's table), if it is one.
pub fn pane_key(key: Key) -> Option<input::Key> {
    Some(match key {
        Key::ArrowLeft => input::Key::Left,
        Key::ArrowRight => input::Key::Right,
        Key::ArrowUp => input::Key::Up,
        Key::ArrowDown => input::Key::Down,
        Key::Home => input::Key::Home,
        Key::End => input::Key::End,
        Key::PageUp => input::Key::PageUp,
        Key::PageDown => input::Key::PageDown,
        Key::Enter => input::Key::Enter,
        Key::Tab => input::Key::Tab,
        Key::Backspace => input::Key::Backspace,
        Key::Delete => input::Key::Delete,
        Key::Escape => input::Key::Escape,
        Key::Insert => input::Key::Insert,
        _ => return None,
    })
}

/// Feeds the focused pane the frame's typed text, IME events and keys (§1.11.5); returns the events it did not take
/// and whether it took any.
pub fn feed_events(
    m: &mut TextViewModel,
    events: Vec<egui::Event>,
    metrics: &dyn Metrics,
    page: u32,
    dispatch: &dyn Fn(&hikasa_session::settings::Shortcut) -> Option<crate::commands::CommandId>,
    run: &mut Vec<crate::commands::CommandId>,
) -> (Vec<egui::Event>, bool) {
    let mut kept = Vec::with_capacity(events.len());
    let mut took = false;
    for event in events {
        match &event {
            egui::Event::Text(s) => {
                input::text(m, s);
                took = true;
            }
            egui::Event::Ime(ime) => {
                // egui 0.37: an empty preedit is the IME dismissed; Enabled/Disabled are no longer sent.
                let ime = match ime {
                    egui::ImeEvent::Preedit { text, .. } => Ime::Preedit(text.clone()),
                    egui::ImeEvent::Commit(t) => Ime::Commit(t.clone()),
                    // The surrounding text is the document's: the pane never offers it, so nothing is deleted.
                    _ => {
                        took = true;
                        continue;
                    }
                };
                input::ime(m, ime);
                took = true;
            }
            egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => {
                // A binding of a focus-bound key in the panes wins (D-111: overrides of focus-bound keys too).
                if let Some(cmd) = crate::ui::keys::shortcuts_of(*key, *modifiers)
                    .iter()
                    .find_map(dispatch)
                {
                    run.push(cmd);
                    took = true;
                    continue;
                }
                let Some(k) = pane_key(*key) else {
                    kept.push(event);
                    continue;
                };
                if modifiers.alt {
                    kept.push(event);
                    continue;
                }
                let press = Press {
                    key: k,
                    shift: modifiers.shift,
                    ctrl: modifiers.command || modifiers.ctrl,
                };
                if input::key(m, press, metrics, page) {
                    took = true;
                } else {
                    kept.push(event);
                }
            }
            _ => kept.push(event),
        }
    }
    (kept, took)
}

/// The source byte of display byte `d` of a line's geometry (inline highlights come in display units).
fn source_of_display(g: &crate::text::wrap::LineGeom, d: u32) -> u32 {
    let k = g.bounds.partition_point(|b| b.1 <= d).saturating_sub(1);
    g.bounds.get(k).map_or(0, |b| b.0)
}

/// Height of the find panel strip.
const FIND_H: f32 = 30.0;
/// Height of the Replace panel (two lines).
const REPLACE_H: f32 = 56.0;
/// Height of the ruler strip.
const RULER_H: f32 = 16.0;
/// The grip above the details panel.
const GRIP_H: f32 = 5.0;

/// The menus of a text view's areas (D-312), built by the window when one opens.
pub struct TextMenus<'a> {
    /// The window's command context this frame.
    pub ctx: crate::commands::Ctx,
    /// The items of an area's menu.
    pub menu:
        &'a dyn Fn(crate::commands::Area, &crate::commands::Ctx) -> Vec<crate::menus::MenuItem>,
}

/// One pane's rectangles on screen.
#[derive(Clone, Copy)]
struct PaneRects {
    /// The side index.
    k: usize,
    /// The whole pane (header, ruler, body).
    outer: Rect,
    /// The File Info header (empty when hidden).
    header: Rect,
    /// The ruler (empty when hidden).
    ruler: Rect,
    /// The rows.
    body: Rect,
    /// The line numbers' strip.
    gutter: Rect,
    /// Where the text starts (before the horizontal scroll).
    text_x: f32,
}

/// Draws the text view of `view` into the remaining space of `ui`; returns the invocations its keys and menus ran
/// (bindings in the panes, D-111). `keys` is false while a dialog is open.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub fn show(
    ui: &mut Ui,
    view: ViewId,
    m: &mut TextViewModel,
    st: &mut TextUi,
    status: &TextStatus,
    pal: &Palette,
    family: &FontFamily,
    dispatch: &dyn Fn(&hikasa_session::settings::Shortcut) -> Option<crate::commands::CommandId>,
    keys: bool,
    menus: &TextMenus<'_>,
) -> Vec<crate::window::Invocation> {
    let mut ids: Vec<crate::commands::CommandId> = Vec::new();
    let mut run: Vec<crate::window::Invocation> = Vec::new();
    let now = ui.input(|i| i.time);
    m.set_now(now);
    clipboard::tick(m);
    if st.refresh
        || st.meta.is_none()
        || st.meta.as_ref().map(|x| x.generation) != Some(m.generation())
    {
        st.refresh = false;
        let meta = m.meta();
        if meta.scroll >= 0 {
            st.pending_scroll = u32::try_from(meta.scroll).ok();
        }
        if st.meta.as_ref().map(|x| x.generation) != Some(meta.generation) {
            st.slice = None;
        }
        st.meta = Some(meta);
    }
    let Some(meta) = st.meta.as_ref() else {
        return run;
    };
    let (mut generation, mut len, mut rows, phase, syntax, mut cursor, show_thumb, digits, font_px) = (
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
    let mut tweaks = *m.tweaks();
    tweaks.current_line_across = pal.current_line_across;
    let display = m.display().clone();
    let one_pane = m.is_edit();

    let full = ui.available_rect_before_wrap();
    ui.allocate_rect(full, Sense::hover());
    let painter = ui.painter_at(full);
    let small = FontId::proportional(12.0);
    let mut area = full;
    // The find panel above the panes (D-307), the details panel below them (D-306).
    let find_h = match display.find_panel {
        crate::text::layout::FindShown::None => 0.0,
        crate::text::layout::FindShown::Find => FIND_H,
        crate::text::layout::FindShown::Replace => REPLACE_H,
    };
    if find_h > 0.0 {
        let rect = Rect::from_min_size(area.min, vec2(area.width(), find_h));
        for id in crate::ui::text_panels::find_panel(ui, m, rect, pal) {
            ids.push(id);
        }
        area.min.y = rect.bottom();
    }
    if display.details != crate::text::layout::Details::None {
        let h = display.details_height.min((area.height() - 80.0).max(40.0));
        let rect = Rect::from_min_max(pos2(area.left(), area.bottom() - h), area.max);
        let grip = Rect::from_min_max(
            pos2(rect.left(), rect.top() - GRIP_H),
            pos2(rect.right(), rect.top()),
        );
        let r = ui
            .interact(grip, pane_id(view).with("details-grip"), Sense::drag())
            .on_hover_cursor(CursorIcon::ResizeVertical);
        if r.dragged() {
            let dy = r.drag_delta().y;
            let d = m.display_mut();
            d.details_height = (d.details_height - dy).clamp(
                crate::text::layout::DETAILS_HEIGHT.0,
                crate::text::layout::DETAILS_HEIGHT.1,
            );
        }
        painter.rect_filled(grip, CornerRadius::ZERO, pal.border);
        crate::ui::text_panels::details_panel(ui, view, m, rect, pal, family);
        area.max.y = grip.top();
    }
    if let Some(error) = &status.error {
        let strip = Rect::from_min_size(area.min, vec2(area.width(), 22.0));
        painter.rect_filled(strip, CornerRadius::ZERO, pal.error_bg);
        painter.with_clip_rect(strip).text(
            pos2(strip.left() + 8.0, strip.center().y),
            Align2::LEFT_CENTER,
            error,
            small.clone(),
            pal.error,
        );
        area.min.y = strip.bottom();
    }
    painter.rect_filled(area, CornerRadius::ZERO, pal.t_bg);

    let font = FontId::new(font_px as f32, family_of(&display, family));
    let lh =
        ((font_px as f32 * 1.4).round() + tweaks.extra_line_spacing_px.min(40) as f32).max(4.0);
    if st.widths_of.as_ref() != Some(&font) {
        st.widths.borrow_mut().clear();
        st.widths_of = Some(font.clone());
    }
    let ctx = ui.ctx().clone();
    let metrics = EguiMetrics {
        ctx: &ctx,
        font: font.clone(),
        cache: &st.widths,
        line_height: lh,
    };
    let cw = metrics.advance('M').max(1.0);
    // Inputs queued while a pane was frozen (a save, a relive, a Cut's clipboard write) go first, in order (D-300).
    input::replay(m, &metrics);

    // The panes (D-305): side by side or over under, the splitter, *Zoom Pane*, one pane for Text Edit.
    let thumb_w = if show_thumb { THUMB_W } else { 0.0 };
    let panes_left = area.left() + thumb_w;
    let panes_right = area.right() - BAR_W;
    let panes_area = Rect::from_min_max(
        pos2(panes_left, area.top()),
        pos2(panes_right, area.bottom()),
    );
    let focus = m.focus();
    let geo = crate::text::layout::pane_geometry(
        &display,
        &tweaks,
        one_pane,
        index_of(focus),
        panes_area.width(),
        panes_area.height(),
    );
    let gutter_w = ((digits as f32 + 1.5) * cw).ceil() + 10.0;
    let panes: Vec<PaneRects> = geo
        .panes
        .iter()
        .map(|&(k, [x, y, w, h])| {
            let outer = Rect::from_min_size(
                pos2(panes_area.left() + x, panes_area.top() + y),
                vec2(w, h),
            );
            let mut top = outer.top();
            let header = if display.header {
                let r = Rect::from_min_max(
                    pos2(outer.left(), top),
                    pos2(outer.right(), top + HEADER_H),
                );
                top = r.bottom();
                r
            } else {
                Rect::NOTHING
            };
            let ruler = if display.ruler {
                let r =
                    Rect::from_min_max(pos2(outer.left(), top), pos2(outer.right(), top + RULER_H));
                top = r.bottom();
                r
            } else {
                Rect::NOTHING
            };
            let body = Rect::from_min_max(pos2(outer.left(), top), outer.max);
            let (gutter, text_x) = if geo.gutter_right[k] {
                (
                    Rect::from_min_max(pos2(body.right() - gutter_w, body.top()), body.max),
                    body.left() + PAD,
                )
            } else {
                (
                    Rect::from_min_max(body.min, pos2(body.left() + gutter_w, body.bottom())),
                    body.left() + gutter_w + PAD,
                )
            };
            PaneRects {
                k,
                outer,
                header,
                ruler,
                body,
                gutter,
                text_x,
            }
        })
        .collect();
    // The File Info headers (D-302).
    if display.header {
        for p in &panes {
            painter.rect_filled(p.header, CornerRadius::ZERO, pal.chrome);
            painter.hline(
                p.header.x_range(),
                p.header.bottom() - 0.5,
                Stroke::new(1.0, pal.border),
            );
            crate::ui::text_header::show(
                ui,
                view,
                m,
                side_of(p.k),
                p.header,
                &status.panes[p.k],
                pal,
            );
        }
    }
    // The visible lines fit the shortest pane body.
    let body_h = panes
        .iter()
        .map(|p| p.body.height())
        .fold(f32::MAX, f32::min)
        .max(lh);
    let page = f64::from((body_h / lh).floor().max(1.0));
    st.page_rows = page;
    let id = pane_id(view);
    let text_w_of = |p: &PaneRects| (p.body.width() - gutter_w - PAD * 2.0).max(cw);
    let wrap = display.word_wrap;
    for p in &panes {
        m.set_text_width(side_of(p.k), text_w_of(p));
    }
    let text_w = panes.iter().map(text_w_of).fold(f32::MAX, f32::min);
    if wrap {
        st.left = 0.0;
    }

    // Input: wheel, keys, pointer.
    let text_rect = panes_area;
    let response = ui.interact(text_rect, id, Sense::click_and_drag());
    // The panes take the keyboard when nothing else has it (the first frame, after a dialog or a field let go).
    if !st.focused_once
        || std::mem::take(&mut st.focus_request)
        || (keys && ui.memory(|m| m.focused().is_none()))
    {
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
    if m.scroll_rows != 0.0 {
        st.top += std::mem::take(&mut m.scroll_rows);
    }
    let focused = response.has_focus();
    st.had_focus = focused;
    if keys && focused {
        // The pane takes Tab (no focus traversal while it has the keyboard) and the arrows; Esc collapses.
        ui.memory_mut(|mem| {
            mem.set_focus_lock_filter(
                id,
                EventFilter {
                    vertical_arrows: true,
                    horizontal_arrows: true,
                    tab: true,
                    escape: false,
                },
            );
        });
        let events = ui.ctx().input_mut(|i| std::mem::take(&mut i.events));
        let (kept, took) = feed_events(
            m,
            events,
            &metrics,
            (page as u32).saturating_sub(1).max(1),
            dispatch,
            &mut ids,
        );
        ui.ctx().input_mut(|i| i.events = kept);
        if took {
            st.last_key = now;
        }
        // The key's edit is drawn in this frame (G16: the typed character is in the key frame's slice).
        if m.generation() != generation {
            let meta = m.meta();
            if meta.scroll >= 0 {
                st.pending_scroll = u32::try_from(meta.scroll).ok();
            }
            (generation, len, rows, cursor) = (meta.generation, meta.len, meta.rows, meta.cursor);
            st.len = len;
            st.slice = None;
            st.meta = Some(meta);
        }
        // The platform's candidate window follows the caret (D-296); not over a blocked pane.
        if input::ime_allowed(m) {
            let caret = st.caret_rect.unwrap_or(text_rect);
            ui.ctx().output_mut(|o| {
                o.ime = Some(egui::output::IMEOutput {
                    purpose: egui::IMEPurpose::Normal,
                    rect: text_rect,
                    cursor_rect: caret,
                    should_interrupt_composition: false,
                });
            });
        }
    }

    // A one-shot scroll request (a navigation target, a restored cursor) brings its position into view, centred.
    if let Some(pos) = st.pending_scroll.take() {
        let pos = f64::from(pos);
        if pos < st.top || pos > st.top + page - 1.0 {
            st.top = pos - (page / 2.0).floor();
        }
    }
    if std::mem::take(&mut m.reveal_caret)
        && let Some(pos) = m.caret_pos(m.focus())
    {
        let pos = f64::from(pos);
        if pos < st.top {
            st.top = pos;
        } else if pos > st.top + page - 1.0 {
            st.top = pos - page + 1.0;
        }
        st.reveal_x = true;
    }

    // Scroll bars (OPT-017: one for the view, else one per pane; all move the one position) and the thumbnail.
    let bar = Rect::from_min_max(pos2(panes_right, area.top()), area.max);
    if let Some(top) = scroll::vertical(ui, id.with("bar"), bar, f64::from(len), page, st.top, pal)
    {
        st.top = top;
    }
    if !tweaks.single_vertical_scrollbar && panes.len() == 2 && !display.over_under {
        let p = panes[0];
        let pane_bar = Rect::from_min_max(pos2(p.body.right() - 8.0, p.body.top()), p.body.max);
        if let Some(top) = scroll::vertical(
            ui,
            id.with("bar-left"),
            pane_bar,
            f64::from(len),
            page,
            st.top,
            pal,
        ) {
            st.top = top;
        }
    }
    let thumb_rect = Rect::from_min_max(area.min, pos2(panes_left, area.bottom()));
    if show_thumb && phase == TextPhase::Ready && rows > 0 {
        let r = ui
            .interact(thumb_rect, id.with("thumb"), Sense::click_and_drag())
            .on_hover_cursor(CursorIcon::PointingHand);
        if (r.clicked() || r.dragged())
            && let Some(p) = r.interact_pointer_pos()
        {
            let frac = f64::from(((p.y - area.top()) / area.height().max(1.0)).clamp(0.0, 1.0));
            let row = (frac * f64::from(rows)).floor().min(f64::from(rows - 1)) as u32;
            st.top = f64::from(m.position_of_row(row)) - page / 2.0;
        }
    }
    st.top = st.top.clamp(0.0, st.max_top());
    if wrap {
        st.top = st.top.floor();
    }
    m.set_viewport(st.top.floor() as u32, page as u32);

    // The splitter between side-by-side panes, or over-under ones (D-305): a drag sets the split.
    if let Some([x, y, w, h]) = geo.strip {
        let strip = Rect::from_min_size(
            pos2(panes_area.left() + x, panes_area.top() + y),
            vec2(w, h),
        );
        let r = ui
            .interact(
                strip.shrink2(vec2(2.0, 0.0)),
                id.with("splitter"),
                Sense::drag(),
            )
            .on_hover_cursor(CursorIcon::ResizeHorizontal);
        if r.dragged()
            && let Some(p) = r.interact_pointer_pos()
        {
            let split = (p.x - panes_area.left()) / panes_area.width().max(1.0);
            m.display_mut().set_split(split);
        }
    } else if display.over_under && panes.len() == 2 {
        let divider = Rect::from_min_max(
            pos2(panes_area.left(), panes[0].outer.bottom()),
            pos2(panes_area.right(), panes[1].outer.top()),
        );
        painter.rect_filled(divider, CornerRadius::ZERO, pal.chrome);
        let r = ui
            .interact(divider, id.with("divider"), Sense::drag())
            .on_hover_cursor(CursorIcon::ResizeVertical);
        if r.dragged()
            && let Some(p) = r.interact_pointer_pos()
        {
            let split = (p.y - panes_area.top()) / panes_area.height().max(1.0);
            m.display_mut().set_split(split);
        }
    }

    if len == 0 {
        let message = st.meta.as_ref().map_or("", |x| x.message.as_str());
        painter.text(
            pos2(
                text_rect.center().x,
                area.top() + 60.0_f32.min(area.height() / 2.0),
            ),
            Align2::CENTER_CENTER,
            message,
            FontId::proportional(14.0),
            pal.muted,
        );
        if show_thumb {
            painter.rect_filled(thumb_rect, CornerRadius::ZERO, pal.chrome);
        }
        return finish(ids, run);
    }

    // The rows on screen and their visual lines.
    let first = st.top.floor() as u32;
    let frac = (st.top - st.top.floor()) as f32;
    let fit = (body_h / lh).ceil() as u32 + 1;
    let screen: Vec<ScreenRow> = m.screen(first, fit, &metrics);
    let count = screen.len() as u32;
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
            st.refresh = true;
            ui.ctx().request_repaint();
        }
        st.slice = Some(slice);
    }

    // The pointer (§1.11.4): a press places the caret, a drag selects, the gutter selects lines.
    let locate = |p: Pos2| -> Option<(usize, bool, f32, f32)> {
        let pane = panes.iter().find(|r| r.body.contains(p))?;
        let gutter = pane.gutter.contains(p);
        let y = (p.y - pane.body.top()) / lh + frac;
        let x = p.x - pane.text_x + st.left;
        Some((pane.k, gutter, y, x))
    };
    let (pressed, released, shift) = ui.input(|i| {
        (
            i.pointer.button_pressed(PointerButton::Primary),
            i.pointer.button_released(PointerButton::Primary),
            i.modifiers.shift,
        )
    });
    let pointer = response
        .interact_pointer_pos()
        .or_else(|| response.hover_pos());
    if pressed
        && response.hovered()
        && let Some((k, gutter, y, x)) = pointer.and_then(locate)
    {
        response.request_focus();
        let side = side_of(k);
        let at = screen
            .iter()
            .find(|r| y < (r.top + r.height) as f32)
            .map(|r| r.pos);
        let gap = at.is_some_and(|pos| m.row_at_pos(pos).is_none() && m.filtered().is_some());
        if gap {
            if let Some(pos) = at {
                m.click(pos);
            }
        } else if gutter {
            let line = m.hit(side, &screen, y, 0.0).map_or(0, |p| p.line.0);
            input::gutter_press(m, side, line, shift);
            st.gutter_drag = true;
        } else if let Some(pos) = m.hit(side, &screen, y, x) {
            input::press(m, side, pos, shift, Clicks::One);
            st.text_drag = true;
        }
        st.last_key = now;
    }
    if let Some((k, gutter, y, x)) = pointer.and_then(locate) {
        let side = side_of(k);
        if response.triple_clicked()
            && let Some(pos) = m.hit(side, &screen, y, x)
        {
            input::press(m, side, pos, false, Clicks::Three);
        } else if response.double_clicked()
            && let Some(pos) = m.hit(side, &screen, y, x)
        {
            input::press(m, side, pos, false, Clicks::Two);
        }
        if response.clicked_by(PointerButton::Middle)
            && let Some(pos) = m.hit(side, &screen, y, x)
        {
            input::middle_click(m, side, pos);
        }
        if response.secondary_clicked() {
            // D-312: outside the selection the caret moves first (inside, the selection is kept).
            if let Some(pos) = m.hit(side, &screen, y, x) {
                input::secondary_press(m, side, pos);
            }
            st.menu_area = Some(if gutter {
                crate::commands::Area::TextGutter
            } else {
                crate::commands::Area::TextPane(side)
            });
            st.menu_anchor = pointer;
        }
    }
    // Shift+F10 and the Menu key open the focus pane's menu at the caret.
    if keys && focused && ui.input(|i| i.key_pressed(Key::F10) && i.modifiers.shift) {
        st.menu_area = Some(crate::commands::Area::TextPane(m.focus()));
        st.menu_anchor = st.caret_rect.map(|r| r.left_bottom());
    }
    for p in &panes {
        if display.header {
            let r = ui.interact(p.header, id.with(("header-menu", p.k)), Sense::click());
            if r.secondary_clicked() {
                m.set_focus(side_of(p.k));
                st.menu_area = Some(crate::commands::Area::TextHeader(side_of(p.k)));
                st.menu_anchor = r.interact_pointer_pos();
            }
        }
    }
    if st.menu_area.is_some() && response.secondary_clicked() {
        egui::Popup::open_id(ui.ctx(), egui::Popup::context_menu(&response).get_id());
    }
    if let Some(area_kind) = st.menu_area {
        if !egui::Popup::is_id_open(ui.ctx(), egui::Popup::context_menu(&response).get_id()) {
            egui::Popup::open_id(ui.ctx(), egui::Popup::context_menu(&response).get_id());
        }
        let popup = match st.menu_anchor {
            Some(at) => egui::Popup::context_menu(&response).at_position(at),
            None => egui::Popup::context_menu(&response),
        };
        let opened = popup.show(|ui| {
            let mut ctx = menus.ctx;
            m.ctx_into(&mut ctx);
            ctx.text_pane = true;
            let items = (menus.menu)(area_kind, &ctx);
            let mut hint = None;
            crate::ui::menus::items(ui, &items, &mut run, &mut hint);
        });
        if opened.is_none() {
            st.menu_area = None;
            st.menu_anchor = None;
        }
    }
    if (st.text_drag || st.gutter_drag)
        && response.dragged()
        && let Some(p) = response.interact_pointer_pos()
    {
        let k = index_of(m.focus());
        if let Some(pane) = panes.iter().find(|r| r.k == k).copied() {
            // Scrolling at the edges (the panes in lockstep).
            if p.y < pane.body.top() {
                st.top = (st.top - 1.0).max(0.0);
            } else if p.y > pane.body.bottom() {
                st.top = (st.top + 1.0).min(st.max_top());
            }
            let y = ((p.y.clamp(pane.body.top(), pane.body.bottom() - 1.0) - pane.body.top()) / lh)
                + frac;
            let x = p.x - pane.text_x + st.left;
            if let Some(pos) = m.hit(side_of(k), &screen, y, x) {
                if st.gutter_drag {
                    input::gutter_drag(m, pos.line.0);
                } else {
                    input::drag(m, pos);
                }
            }
        }
        ui.ctx().request_repaint();
    }
    if released && (st.text_drag || st.gutter_drag) {
        st.text_drag = false;
        st.gutter_drag = false;
        input::release(m);
    }

    let Some(slice) = st.slice.as_ref() else {
        return finish(ids, run);
    };
    let blink_on = (now - st.last_key) < BLINK_S || ((now / BLINK_S).floor() as i64) % 2 == 0;
    let overwrite = display.overwrite;
    let mut widest = 0.0f32;
    let mut caret_rect = None;
    let cursor_row = m.cursor().map(|r| r.0);
    for p in &panes {
        let k = p.k;
        let side = side_of(k);
        let clip_body = painter.with_clip_rect(p.body);
        let text_painter = painter.with_clip_rect(if geo.gutter_right[k] {
            Rect::from_min_max(p.body.min, pos2(p.gutter.left(), p.body.bottom()))
        } else {
            Rect::from_min_max(pos2(p.gutter.right(), p.body.top()), p.body.max)
        });
        let Some(doc) = m.doc(side) else {
            continue;
        };
        let pane_state = m.pane(side);
        let x0 = p.text_x - st.left;
        let mut bottom = p.body.top();
        for r in &screen {
            let Some(row) = slice.rows.get((r.pos - slice.start) as usize) else {
                break;
            };
            let y = p.body.top() + (r.top as f32 - frac) * lh;
            let h = r.height as f32 * lh;
            bottom = y + h;
            let row_rect = Rect::from_min_max(pos2(p.body.left(), y), pos2(p.body.right(), y + h));
            if row.k == 1 {
                draw_gap(&clip_body, row_rect, row, gutter_w, pal);
                continue;
            }
            let gutter =
                Rect::from_min_max(pos2(p.gutter.left(), y), pos2(p.gutter.right(), y + h));
            let text_bg = Rect::from_min_max(
                pos2(
                    if geo.gutter_right[k] {
                        p.body.left()
                    } else {
                        gutter.right()
                    },
                    y,
                ),
                pos2(
                    if geo.gutter_right[k] {
                        gutter.left()
                    } else {
                        p.body.right()
                    },
                    y + h,
                ),
            );
            clip_body.rect_filled(gutter, CornerRadius::ZERO, pal.gutter);
            let (Some(line), Some(g)) = (r.lines[k], r.geoms[k].as_ref()) else {
                hatch(&clip_body, text_bg, pal.hatch);
                continue;
            };
            if let Some(bg) = row_background(row, pal, &tweaks) {
                clip_body.rect_filled(text_bg, CornerRadius::ZERO, bg);
            }
            // The current line: the focus pane's; over under with the theme's flag, across both panes (D-305).
            let current = if side == focus {
                pane_state.caret.line.0 == line
            } else {
                display.over_under
                    && tweaks.current_line_across
                    && r.row.is_some()
                    && r.row == cursor_row
            };
            if current && row.st == 0 {
                clip_body.rect_filled(text_bg, CornerRadius::ZERO, pal.current_line);
            }
            // Edited and saved lines (TC-031): the background tinted, a bar in the gutter.
            let mark = match doc.mark(LineIdx(line)) {
                LineMark::Edited => Some(pal.t_edited),
                LineMark::SavedEdit => Some(pal.t_saved),
                LineMark::Original => None,
            };
            if let Some(colour) = mark {
                clip_body.rect_filled(text_bg, CornerRadius::ZERO, colour.gamma_multiply(0.12));
                let bar_x = if geo.gutter_right[k] {
                    gutter.left()
                } else {
                    gutter.right() - 3.0
                };
                clip_body.rect_filled(
                    Rect::from_min_max(pos2(bar_x, y), pos2(bar_x + 3.0, y + h)),
                    CornerRadius::ZERO,
                    colour,
                );
            }
            if row.ln[k] > 0 {
                clip_body.text(
                    pos2(gutter.right() - cw * 0.5 - 4.0, y + lh / 2.0),
                    Align2::RIGHT_CENTER,
                    row.ln[k],
                    font.clone(),
                    pal.gutter_fg,
                );
            }
            // The bookmarks' digits (D-309).
            let marks = crate::text::nav::bookmarks_at(doc, line);
            if let Some(n) = marks.first() {
                clip_body.text(
                    pos2(gutter.left() + 2.0, y + lh / 2.0),
                    Align2::LEFT_CENTER,
                    n.to_string(),
                    small.clone(),
                    pal.accent,
                );
            }
            // The selection (a range, or a line set of Select All, D-307).
            let set_selected = pane_state
                .lines
                .as_ref()
                .is_some_and(|set| set.0.contains(line));
            let sel = pane_state.selection().range();
            let sel_colour = if side == focus {
                pal.t_select
            } else {
                pal.t_inactive_select
            };
            if set_selected {
                text_painter.rect_filled(text_bg, CornerRadius::ZERO, sel_colour);
            } else if sel.start != sel.end && sel.start.line.0 <= line && line <= sel.end.line.0 {
                let a = if sel.start.line.0 == line {
                    sel.start.byte
                } else {
                    0
                };
                let b = if sel.end.line.0 == line {
                    sel.end.byte
                } else {
                    doc.line_len(LineIdx(line))
                };
                for v in 0..g.visuals.len() {
                    if let Some((xa, xb)) = g.span_x(v, a, b) {
                        let extra = if sel.end.line.0 > line && v + 1 == g.visuals.len() {
                            cw * 0.5
                        } else {
                            0.0
                        };
                        let vy = y + v as f32 * lh;
                        text_painter.rect_filled(
                            Rect::from_min_max(pos2(x0 + xa, vy), pos2(x0 + xb + extra, vy + lh)),
                            CornerRadius::ZERO,
                            sel_colour,
                        );
                    }
                }
            }
            // The find panel's hits on screen (D-307's highlight; `hits_in_line` of this line only).
            if display.find_panel != crate::text::layout::FindShown::None {
                for hit in crate::text::find::hits_on_screen(m, side, line) {
                    for v in 0..g.visuals.len() {
                        if let Some((xa, xb)) = g.span_x(v, hit.start, hit.end) {
                            let vy = y + v as f32 * lh;
                            text_painter.rect_filled(
                                Rect::from_min_max(
                                    pos2(x0 + xa, vy + 1.0),
                                    pos2(x0 + xb.max(xa + 2.0), vy + lh - 1.0),
                                ),
                                CornerRadius::ZERO,
                                pal.find_hl,
                            );
                        }
                    }
                }
            }
            // Inline differences.
            if let Some(t) = row.t[k].as_deref() {
                let cols = Columns::new(t);
                for hl in row.hl[k].as_chunks::<3>().0 {
                    let (d0, _) = cols.at(hl[0]);
                    let (d1, _) = cols.at(hl[1]);
                    if g.cut_before || d1 > g.text.len() {
                        continue;
                    }
                    let (s0, s1) = (
                        source_of_display(g, d0 as u32),
                        source_of_display(g, d1 as u32),
                    );
                    let colour = if hl[2] != 0 { pal.imp_hl } else { pal.unimp_hl };
                    for v in 0..g.visuals.len() {
                        if let Some((xa, xb)) = g.span_x(v, s0, s1) {
                            let vy = y + v as f32 * lh;
                            text_painter.rect_filled(
                                Rect::from_min_max(
                                    pos2(x0 + xa, vy + 1.0),
                                    pos2(x0 + xb.max(xa + 2.0), vy + lh - 1.0),
                                ),
                                CornerRadius::ZERO,
                                colour,
                            );
                        }
                    }
                }
            }
            // The text, one galley per visual line; the syntax colours of the slice when its text is the line's, the
            // view shows them and the row may have them (`syntax_on_difference_lines`).
            let text_colour = if row.ig { pal.t_ignored } else { pal.t_text };
            let colours_on =
                display.syntax && !row.ig && (row.st == 0 || tweaks.syntax_on_difference_lines);
            let coloured = row.t[k]
                .as_deref()
                .filter(|t| colours_on && !g.cut_before && t.len() == g.text.len())
                .map(|t| row_job(t, &Columns::new(t), &row.sp[k], &font, pal));
            for v in 0..g.visuals.len() {
                let vis = g.visuals[v];
                let (a, b) = (
                    g.bounds[vis.start].1 as usize,
                    g.bounds[vis.end.min(g.bounds.len() - 1)].1 as usize,
                );
                if a >= b {
                    continue;
                }
                let job = match &coloured {
                    Some(job) => sub_job(job, a, b),
                    None => plain_job(&g.text[a..b], &font, text_colour),
                };
                let galley = text_painter.layout_job(job);
                let vy = y + v as f32 * lh;
                widest = widest.max(g.draw_x(v) + galley.size().x + PAD * 2.0 - text_w);
                text_painter.galley(
                    pos2(x0 + g.draw_x(v), vy + (lh - galley.size().y) / 2.0),
                    galley,
                    text_colour,
                );
            }
            // Visible whitespace (D-305): a mark over each space, tab and other blank, and the terminator's.
            if display.whitespace {
                draw_whitespace(
                    &text_painter,
                    doc,
                    line,
                    g,
                    x0,
                    y,
                    lh,
                    &small,
                    pal.whitespace,
                );
            }
            if g.continues {
                let vy = y + g.visuals.len() as f32 * lh;
                text_painter.text(
                    pos2(x0, vy + lh / 2.0),
                    Align2::LEFT_CENTER,
                    "… (the line continues)",
                    small.clone(),
                    pal.muted,
                );
            }
            // The caret (a bar; overwrite: a block; the pane without focus: hollow) and the IME's preedit.
            if pane_state.caret.line.0 == line {
                let (v, xr) = g.caret(pane_state.caret.byte);
                let cx = x0 + xr;
                let cy = y + v as f32 * lh;
                let mut rect =
                    Rect::from_min_max(pos2(cx, cy + 1.0), pos2(cx + 2.0, cy + lh - 1.0));
                if let Some(pre) = &pane_state.preedit {
                    let galley =
                        text_painter.layout_no_wrap(pre.text.clone(), font.clone(), pal.t_text);
                    let w = galley.size().x;
                    text_painter.rect_filled(
                        Rect::from_min_max(pos2(cx, cy), pos2(cx + w, cy + lh)),
                        CornerRadius::ZERO,
                        pal.t_bg,
                    );
                    text_painter.galley(
                        pos2(cx, cy + (lh - galley.size().y) / 2.0),
                        galley,
                        pal.t_text,
                    );
                    text_painter.hline(cx..=cx + w, cy + lh - 2.0, Stroke::new(1.0, pal.t_text));
                    rect = Rect::from_min_max(
                        pos2(cx + w, cy + 1.0),
                        pos2(cx + w + 2.0, cy + lh - 1.0),
                    );
                } else if overwrite && side == focus {
                    let next = doc
                        .line(LineIdx(line))
                        .get(pane_state.caret.byte as usize..)
                        .and_then(|s| s.chars().next())
                        .map_or(cw, |c| metrics.advance(c).max(2.0));
                    rect = Rect::from_min_max(pos2(cx, cy + 1.0), pos2(cx + next, cy + lh - 1.0));
                }
                if side == focus {
                    caret_rect = Some(rect);
                    if blink_on || !focused {
                        let colour = if overwrite {
                            pal.t_text.gamma_multiply(0.5)
                        } else {
                            pal.t_text
                        };
                        text_painter.rect_filled(rect, CornerRadius::ZERO, colour);
                    }
                } else {
                    text_painter.rect_stroke(
                        rect,
                        CornerRadius::ZERO,
                        Stroke::new(1.0, pal.muted),
                        egui::StrokeKind::Inside,
                    );
                }
            }
        }
        // Below the last line (OPT-014 `crosshatch_beyond_eof`).
        if tweaks.crosshatch_beyond_eof && bottom < p.body.bottom() && first + count >= len {
            let rest = Rect::from_min_max(pos2(p.body.left(), bottom), p.body.max);
            hatch(&clip_body, rest, pal.hatch);
        }
        // The column guide (OPT-014 `column_line_at`).
        if tweaks.column_line_at > 0 {
            let x = x0 + tweaks.column_line_at as f32 * cw;
            if x > p.text_x && x < p.body.right() {
                text_painter.vline(x, p.body.y_range(), Stroke::new(1.0, pal.border));
            }
        }
        // The ruler of the visible display columns (D-306).
        if display.ruler {
            crate::ui::text_panels::ruler(&painter, m, side, p.ruler, p.text_x, st.left, cw, pal);
        }
        // The pane without the keyboard dimmed (OPT-014 `dim_inactive_pane_percent`).
        if side != focus && tweaks.dim_inactive_pane_percent > 0 && panes.len() > 1 {
            let alpha = tweaks.dim_inactive_pane_percent as f32 / 100.0;
            clip_body.rect_filled(p.body, CornerRadius::ZERO, pal.t_bg.gamma_multiply(alpha));
        }
    }
    // The caret comes into view horizontally after a move (no wrap).
    if std::mem::take(&mut st.reveal_x)
        && !wrap
        && let Some(rect) = caret_rect
        && let Some(p) = panes.iter().find(|p| p.k == index_of(focus))
    {
        let x = rect.left() - p.text_x + st.left;
        if x < st.left {
            st.left = (x - cw * 4.0).max(0.0);
        } else if x > st.left + text_w - cw {
            st.left = x - text_w + cw * 4.0;
        }
        m.pane_mut(focus).scroll_x = st.left;
        ui.ctx().request_repaint();
    }
    st.caret_rect = caret_rect;
    st.widest = st.widest.max(widest).max(st.left);
    // The copy buttons (D-299): in the strip between side-by-side panes, else in each pane's gutter (over under); the
    // alignment overrides' marks in the strip (D-310).
    let rows_on_screen = screen.iter().filter_map(|r| r.row);
    let strip = geo.strip.map(|[x, y, w, h]| {
        Rect::from_min_size(
            pos2(panes_area.left() + x, panes_area.top() + y),
            vec2(w, h),
        )
    });
    if let Some(strip) = strip {
        let clip = painter.with_clip_rect(strip);
        clip.rect_filled(strip, CornerRadius::ZERO, pal.chrome);
        clip.vline(
            strip.left() + 0.5,
            strip.y_range(),
            Stroke::new(1.0, pal.border),
        );
        clip.vline(
            strip.right() - 0.5,
            strip.y_range(),
            Stroke::new(1.0, pal.border),
        );
    }
    if !one_pane
        && !display.zoom
        && let (Some(lo), Some(hi)) = (rows_on_screen.clone().min(), rows_on_screen.max())
    {
        let editable = [Side::Left, Side::Right].map(|s| {
            m.doc(s)
                .is_some_and(|d| d.editability().editable() || crate::text::only_binary(d))
        });
        let mut clicks: Vec<(Side, std::ops::Range<u32>)> = Vec::new();
        for (start, rows_of) in sections_starting_in(m, lo..hi + 1) {
            let Some(r) = screen.iter().find(|r| r.row == Some(start)) else {
                continue;
            };
            for (to_right, target) in [(true, 1usize), (false, 0usize)] {
                if !editable[target] {
                    continue;
                }
                // Side by side: the strip; over under: the gutter of the pane copied from.
                let (cx, top) = match strip {
                    Some(strip) => (strip.center().x, panes[0].body.top()),
                    None => {
                        let from = 1 - target;
                        let Some(p) = panes.iter().find(|p| p.k == from) else {
                            continue;
                        };
                        (p.gutter.left() + 7.0, p.body.top())
                    }
                };
                let y = top + (r.top as f32 - frac) * lh;
                let half = lh.min(STRIP_W);
                let cy = if to_right || strip.is_none() {
                    y + half / 2.0
                } else {
                    y + half * 1.5
                };
                let button = Rect::from_center_size(pos2(cx, cy), vec2(STRIP_W - 4.0, half - 1.0));
                let hint = if to_right {
                    "Copy Section to Right (Ctrl+R)"
                } else {
                    "Copy Section to Left (Ctrl+L)"
                };
                let resp = ui
                    .interact(button, id.with(("copy", start, to_right)), Sense::click())
                    .on_hover_text(hint);
                let colour = if resp.hovered() {
                    pal.accent
                } else {
                    pal.muted
                };
                arrow(
                    &painter,
                    button.center(),
                    (half - 4.0).max(6.0),
                    to_right,
                    colour,
                );
                if resp.clicked() {
                    clicks.push((other(side_of(target)), rows_of.clone()));
                }
            }
        }
        for (from, rows_of) in clicks {
            copy_section(m, from, rows_of);
        }
        // The overrides' marks: a link for a pair's rows, a slash for isolated rows.
        if let Some(strip) = strip {
            for r in &screen {
                let Some(row) = r.row else { continue };
                let Some(pair) = crate::text::marks::anchor_mark(m, row) else {
                    continue;
                };
                let y = panes[0].body.top() + (r.top as f32 - frac) * lh;
                let c = pos2(strip.center().x, y + lh / 2.0);
                if pair {
                    painter.circle_stroke(c, 3.0, Stroke::new(1.0, pal.accent));
                } else {
                    painter.line_segment(
                        [c + vec2(-4.0, 4.0), c + vec2(4.0, -4.0)],
                        Stroke::new(1.0, pal.accent),
                    );
                }
            }
        }
    }
    let _ = cursor;
    if show_thumb {
        draw_thumbnail(ui, &painter, thumb_rect, m, st, rows, first, count, pal);
    }
    if count > 0 {
        let drawn = match phase {
            TextPhase::Loaded => 1,
            TextPhase::Ready if syntax => 2,
            TextPhase::Ready => 1,
            _ => 0,
        };
        st.drawn = st.drawn.max(drawn);
    }
    st.painted_generation = generation;
    // Repaints: the blink while the pane has the keyboard, the debounce, the primary selection's settle.
    if focused {
        let next = BLINK_S - now.rem_euclid(BLINK_S);
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs_f64(next.max(0.01)));
    }
    if let Some(at) = m.wake_at() {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs_f64((at - now).max(0.0)));
    }
    finish(ids, run)
}

/// The invocations of a frame: the commands of keys first, then the menus' and panels'.
fn finish(
    ids: Vec<crate::commands::CommandId>,
    mut run: Vec<crate::window::Invocation>,
) -> Vec<crate::window::Invocation> {
    let mut out: Vec<crate::window::Invocation> = ids
        .into_iter()
        .map(crate::window::Invocation::new)
        .collect();
    out.append(&mut run);
    out
}

/// The font family of the view: its own (Display Font, D-305) when the GUI knows it, else the editor font.
fn family_of(display: &crate::text::layout::Display, editor: &FontFamily) -> FontFamily {
    match &display.font_family {
        Some(name) if name.eq_ignore_ascii_case("monospace") => FontFamily::Monospace,
        Some(name) if name.eq_ignore_ascii_case("proportional") => FontFamily::Proportional,
        Some(name) => FontFamily::Name(name.as_str().into()),
        None => editor.clone(),
    }
}

/// The marks of visible whitespace in one line on screen (D-305): `·` a space, `→` a tab, `°` another blank, then
/// the terminator's (`¶` LF, `¤` CR, `¶` with a dot CRLF). Drawn over the text in `[file] whitespace`; display only.
#[allow(clippy::too_many_arguments)]
fn draw_whitespace(
    painter: &Painter,
    doc: &hikasa_text::EditableDocument,
    line: u32,
    g: &crate::text::wrap::LineGeom,
    x0: f32,
    y: f32,
    lh: f32,
    font: &FontId,
    colour: Color32,
) {
    if line >= doc.line_count() {
        return;
    }
    let text = doc.line(LineIdx(line));
    for (byte, c) in text.char_indices() {
        let Some(mark) = crate::text::layout::blank_mark(c) else {
            continue;
        };
        let b = byte as u32;
        for v in 0..g.visuals.len() {
            if let Some((xa, xb)) = g.span_x(v, b, b + c.len_utf8() as u32) {
                let vy = y + v as f32 * lh;
                let cx = if c == '\t' {
                    x0 + xa + 2.0
                } else {
                    x0 + (xa + xb) / 2.0
                };
                let align = if c == '\t' {
                    Align2::LEFT_CENTER
                } else {
                    Align2::CENTER_CENTER
                };
                painter.text(pos2(cx, vy + lh / 2.0), align, mark, font.clone(), colour);
            }
        }
    }
    let Some(eol) = crate::text::layout::eol_mark(doc.line_ending(LineIdx(line))) else {
        return;
    };
    let len = text.len() as u32;
    let v = g.visuals.len().saturating_sub(1);
    let x = g.span_x(v, len, len).map_or(0.0, |(xa, _)| xa);
    painter.text(
        pos2(x0 + x + 2.0, y + v as f32 * lh + lh / 2.0),
        Align2::LEFT_CENTER,
        eol,
        font.clone(),
        colour,
    );
}

fn row_background(
    row: &SliceRow,
    pal: &Palette,
    tweaks: &crate::text::layout::Tweaks,
) -> Option<Color32> {
    if row.ig {
        // An ignored row counts as same (TC-024): no difference colour.
        return None;
    }
    match (row.st, row.im) {
        (1, 2) => Some(pal.imp_bg),
        (1, _) => Some(pal.unimp_bg),
        // OPT-014 `use_orphan_color`: orphan rows in `[file] orphan`, else in the difference colours.
        (2 | 3, _) if tweaks.use_orphan_color => Some(pal.orph_bg),
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
    // Of the last install (D-294): never per keystroke.
    let installs = m.installs();
    let fresh = st
        .thumb
        .as_ref()
        .is_some_and(|t| t.installs == installs && t.height == height);
    if !fresh {
        st.thumb = Some(Thumb {
            installs,
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
            &crate::ui::theme::light(),
        );
        let ranges: Vec<_> = job
            .sections
            .iter()
            .map(|s| (s.byte_range.start.0, s.byte_range.end.0))
            .collect();
        assert_eq!(ranges, vec![(0, 3), (3, 8), (8, 9), (9, 10)]);
        assert_eq!(
            job.sections[0].format.color,
            crate::ui::theme::light().syntax[1]
        );
        let bare = row_job(
            "abc",
            &Columns::new("abc"),
            &[],
            &font,
            &crate::ui::theme::light(),
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
            let job = row_job(&text, &cols, &segments, &font, &crate::ui::theme::light());
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
