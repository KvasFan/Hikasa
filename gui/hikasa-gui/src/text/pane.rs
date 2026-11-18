//! One pane's caret, selection and keyboard moves (M2 W11a, contract §1.11.4, D-295): toolkit-free. Character and word
//! steps, smart Home and the line sets come from core (`edit::cursor`, `edit::commands`); the visual lines a move
//! walks (Up/Down at a sticky x, End of a wrapped line, a page) come from [`super::wrap::LineGeom`] over the view's
//! metrics, so every move is tested headless with exact numbers.

use hikasa_core::Side;
use hikasa_text::LineIdx;
use hikasa_text::edit::{
    Dir, EditableDocument, LineSet, Selection, TextPos, cluster_starts, doc_end, line_home,
    next_pos, prev_pos, word_boundary,
};

use super::wrap::LineGeom;

/// The IME's composition shown at the caret (§1.11.5): drawn underlined, never stored in the document.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Preedit {
    /// The composed text.
    pub text: String,
}

/// One pane's caret and selection.
#[derive(Debug, Clone, PartialEq)]
pub struct PaneState {
    /// The side (`Left` in a Text Edit view).
    pub side: Side,
    /// The caret.
    pub caret: TextPos,
    /// The other end of the selection (`== caret`: none).
    pub anchor: TextPos,
    /// A line-set selection (D-307's *Select All*); `None`: the range above.
    pub lines: Option<LineSet>,
    /// The x that Up, Down, PgUp and PgDn keep (from the line's start, display units).
    pub sticky_x: Option<f32>,
    /// The IME composition.
    pub preedit: Option<Preedit>,
    /// The horizontal scroll of the pane (display units).
    pub scroll_x: f32,
}

impl PaneState {
    /// A pane with the caret at the start.
    pub fn new(side: Side) -> PaneState {
        PaneState {
            side,
            caret: TextPos::ZERO,
            anchor: TextPos::ZERO,
            lines: None,
            sticky_x: None,
            preedit: None,
            scroll_x: 0.0,
        }
    }

    /// The selection (anchor .. caret).
    pub fn selection(&self) -> Selection {
        Selection {
            anchor: self.anchor,
            caret: self.caret,
        }
    }

    /// True when text is selected (a range or a line set).
    pub fn has_selection(&self) -> bool {
        self.anchor != self.caret || self.lines.as_ref().is_some_and(|l| !l.is_empty())
    }

    /// Puts the caret at `pos`; `extend` keeps the anchor (Shift), else the selection collapses there.
    pub fn set_caret(&mut self, pos: TextPos, extend: bool) {
        self.caret = pos;
        if !extend {
            self.anchor = pos;
        }
        self.lines = None;
    }

    /// Selects `range` (the caret at its end).
    pub fn select(&mut self, anchor: TextPos, caret: TextPos) {
        self.anchor = anchor;
        self.caret = caret;
        self.lines = None;
        self.sticky_x = None;
    }

    /// Collapses the selection at the caret (Esc).
    pub fn collapse(&mut self) {
        self.anchor = self.caret;
        self.lines = None;
    }

    /// Keeps the caret and the anchor inside `doc` at char boundaries (after an undo, a reload, a relive).
    pub fn clamp(&mut self, doc: &EditableDocument) {
        self.caret = doc.clamp(self.caret);
        self.anchor = doc.clamp(self.anchor);
    }
}

/// A keyboard move of §1.11.4's table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Move {
    /// One grapheme cluster back (with a selection and no Shift: to its start).
    Left,
    /// One grapheme cluster on (with a selection and no Shift: to its end).
    Right,
    /// Ctrl+Left: a word boundary back.
    WordLeft,
    /// Ctrl+Right: a word boundary on.
    WordRight,
    /// The visual line above at the sticky x.
    Up,
    /// The visual line below at the sticky x.
    Down,
    /// Smart Home.
    Home,
    /// The visual line's end, then the line's.
    End,
    /// Ctrl+Home.
    DocStart,
    /// Ctrl+End.
    DocEnd,
    /// One screen of visual lines up.
    PageUp(u32),
    /// One screen of visual lines down.
    PageDown(u32),
}

/// The geometry of a line of the pane's document (the view's metrics, its wrap width and tab size).
pub type GeomOf<'a> = dyn Fn(u32) -> LineGeom + 'a;

/// Applies `mv` to the pane (`extend`: Shift). Vertical moves keep `sticky_x`, every other move resets it.
pub fn apply_move(
    pane: &mut PaneState,
    doc: &EditableDocument,
    mv: Move,
    extend: bool,
    geom: &GeomOf<'_>,
) {
    let caret = doc.clamp(pane.caret);
    let sel = pane.selection();
    let collapsing = !extend && sel.anchor != sel.caret;
    let target = match mv {
        Move::Left if collapsing => sel.range().start,
        Move::Right if collapsing => sel.range().end,
        Move::Left => prev_pos(doc, caret),
        Move::Right => next_pos(doc, caret),
        // Core's word steps go by characters (a combining mark is a class of its own): the caret keeps to clusters.
        Move::WordLeft => snap_toward(doc, word_boundary(doc, caret, Dir::Backward), false),
        Move::WordRight => snap_toward(doc, word_boundary(doc, caret, Dir::Forward), true),
        Move::Home => line_home(doc, caret),
        Move::End => end_of(doc, caret, geom),
        Move::DocStart => TextPos::ZERO,
        Move::DocEnd => doc_end(doc),
        Move::Up => vertical(pane, doc, caret, -1, geom),
        Move::Down => vertical(pane, doc, caret, 1, geom),
        Move::PageUp(n) => vertical(pane, doc, caret, -i64::from(n.max(1)), geom),
        Move::PageDown(n) => vertical(pane, doc, caret, i64::from(n.max(1)), geom),
    };
    if !matches!(
        mv,
        Move::Up | Move::Down | Move::PageUp(_) | Move::PageDown(_)
    ) {
        pane.sticky_x = None;
    }
    pane.set_caret(target, extend);
}

/// End: the visual line's end, then (already there) the line's end.
fn end_of(doc: &EditableDocument, caret: TextPos, geom: &GeomOf<'_>) -> TextPos {
    let line_end = TextPos::new(caret.line.0, doc.line_len(caret.line));
    let g = geom(caret.line.0);
    let (v, _) = g.caret(caret.byte);
    if v + 1 >= g.visuals.len() {
        return line_end;
    }
    // The last cluster boundary of the visual line before the break (the break belongs to the next line).
    let (start, end) = (g.visuals[v].start, g.visuals[v].end);
    let k = (start..end).rev().find(|k| g.clusters[*k]).unwrap_or(start);
    let byte = g.bounds[k].0;
    if byte == caret.byte {
        line_end
    } else {
        TextPos::new(caret.line.0, byte)
    }
}

/// `steps` visual lines up (negative) or down at the sticky x, walking the pane's own lines (gap rows of the
/// comparison are not lines of the pane, so they are skipped).
fn vertical(
    pane: &mut PaneState,
    doc: &EditableDocument,
    caret: TextPos,
    steps: i64,
    geom: &GeomOf<'_>,
) -> TextPos {
    let mut line = caret.line.0;
    let mut g = geom(line);
    let (mut v, x) = g.caret(caret.byte);
    let sx = *pane.sticky_x.get_or_insert(x);
    let last_line = doc.position_lines().saturating_sub(1);
    let mut left = steps.unsigned_abs();
    while left > 0 {
        if steps < 0 {
            if v > 0 {
                v -= 1;
            } else if line > 0 {
                line -= 1;
                g = geom(line);
                v = g.visuals.len().saturating_sub(1);
            } else {
                return TextPos::ZERO;
            }
        } else if v + 1 < g.visuals.len() {
            v += 1;
        } else if line < last_line {
            line += 1;
            g = geom(line);
            v = 0;
        } else {
            return doc_end(doc);
        }
        left -= 1;
    }
    doc.clamp(TextPos::new(line, g.hit(v, sx)))
}

/// The physical line of `line` with its terminator, as a selection (triple-click, a gutter click): from its start
/// to the next line's start, or to the document's end.
pub fn line_selection(doc: &EditableDocument, line: u32) -> (TextPos, TextPos) {
    let start = TextPos::new(line, 0);
    let end = if line + 1 < doc.position_lines() {
        TextPos::new(line + 1, 0)
    } else {
        doc_end(doc)
    };
    (start, end)
}

/// `pos` inside `doc` at the start of a grapheme cluster (a hit, a mapped position: the caret never stands inside a
/// cluster, D-295).
pub fn snap(doc: &EditableDocument, pos: TextPos) -> TextPos {
    let pos = doc.clamp(pos);
    let line = doc.line(pos.line);
    if line.is_ascii() {
        return pos;
    }
    let starts = cluster_starts(&line);
    let k = starts.partition_point(|b| *b <= pos.byte);
    TextPos::new(
        pos.line.0,
        starts.get(k.wrapping_sub(1)).copied().unwrap_or(0),
    )
}

/// `pos` at a grapheme cluster's start: the next one (`forward`) or the previous one when it is inside a cluster.
fn snap_toward(doc: &EditableDocument, pos: TextPos, forward: bool) -> TextPos {
    let pos = doc.clamp(pos);
    let line = doc.line(pos.line);
    if line.is_ascii() {
        return pos;
    }
    let starts = cluster_starts(&line);
    match starts.binary_search(&pos.byte) {
        Ok(_) => pos,
        Err(k) if forward => TextPos::new(pos.line.0, starts.get(k).copied().unwrap_or(pos.byte)),
        Err(k) => TextPos::new(
            pos.line.0,
            starts.get(k.wrapping_sub(1)).copied().unwrap_or(0),
        ),
    }
}

/// The caret's line (the status bar's `Ln`): 1-based physical line.
pub fn physical_line(doc: &EditableDocument, line: LineIdx) -> u32 {
    if line.0 >= doc.line_count() {
        doc.line_count() + 1
    } else {
        doc.document().physical_line(line)
    }
}
