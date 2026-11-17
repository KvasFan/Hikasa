//! The rows on screen with their visual lines (M2 W11a, contract §1.11.4, D-295), toolkit-free: for the display
//! positions the panes show, the lines of each side, their [`LineGeom`] (word wrap at the pane's width, the long-line
//! window), the row heights (the tallest side's visual lines), and the hit test and caret geometry over them. Only the
//! rows on screen are laid out (ARCH-GUI-002); the scroll position stays a display position plus a fraction.

use hikasa_core::Side;
use hikasa_text::edit::TextPos;
use hikasa_text::{LineIdx, RowIdx, ViewItem};

use super::wrap::{LineGeom, Metrics};
use super::{TextViewModel, index_of};

/// One display position on screen.
#[derive(Debug, Clone, PartialEq)]
pub struct ScreenRow {
    /// The display position.
    pub pos: u32,
    /// The alignment row (`None`: a gap bar, or the unaligned preview).
    pub row: Option<u32>,
    /// The line of each side (`None`: a gap of that side).
    pub lines: [Option<u32>; 2],
    /// The geometry of each side's line.
    pub geoms: [Option<LineGeom>; 2],
    /// Visual lines of the row (the tallest side's; 1 for a gap bar).
    pub height: u32,
    /// The visual line the row starts at, counted from the first row of the screen.
    pub top: u32,
}

impl TextViewModel {
    /// The text width of each pane (display units) word wrap breaks at; set by the window each frame.
    pub fn set_text_width(&mut self, side: Side, width: f32) {
        let k = index_of(side);
        if (self.text_width[k] - width).abs() > 0.5 {
            self.text_width[k] = width;
            if self.layout.word_wrap {
                self.dirty = true;
            }
        }
    }

    /// The geometry of `line` of `side` with the view's wrap and the pane's caret or horizontal scroll position.
    pub fn line_geom(&self, side: Side, line: u32, metrics: &dyn Metrics) -> LineGeom {
        let Some(doc) = self.doc(side) else {
            return LineGeom::new("", 4, metrics, None, 0);
        };
        let text = if line < doc.position_lines() {
            doc.line(LineIdx(line))
        } else {
            std::borrow::Cow::Borrowed("")
        };
        let pane = self.pane(side);
        let wrap = self
            .layout
            .word_wrap
            .then_some(self.text_width[index_of(side)].max(metrics.line_height()));
        let focus = if pane.caret.line.0 == line {
            pane.caret.byte
        } else {
            // The column of the horizontal scroll position (no wrap): the window follows the scroll bar.
            let per = metrics.advance('M').max(0.01);
            (pane.scroll_x / per) as u32
        };
        LineGeom::new(&text, doc.options().tab_size, metrics, wrap, focus)
    }

    /// The line of `side` at display position `pos` (`None`: a gap of that side or a gap bar).
    pub fn line_at_pos(&self, side: Side, pos: u32) -> Option<u32> {
        match (self.filtered(), self.comparison()) {
            (Some(view), Some(cmp)) => match view.get(pos)? {
                ViewItem::Row { row } => {
                    let r = cmp.row(row)?;
                    match side {
                        Side::Right => r.right.map(|l| l.0),
                        _ => r.left.map(|l| l.0),
                    }
                }
                ViewItem::Gap { .. } => None,
            },
            _ => None,
        }
    }

    /// The rows from display position `first` that fill `lines` visual lines (at least one row).
    pub fn screen(&self, first: u32, lines: u32, metrics: &dyn Metrics) -> Vec<ScreenRow> {
        let mut out = Vec::new();
        let len = self.len();
        let mut top = 0u32;
        let mut pos = first;
        while pos < len && (top < lines.max(1) || out.is_empty()) {
            let (row, gap) = match self.filtered().and_then(|v| v.get(pos)) {
                Some(ViewItem::Row { row }) => (Some(row.0), false),
                Some(ViewItem::Gap { .. }) => (None, true),
                None => (None, false),
            };
            let mut lines_of = [None, None];
            let mut geoms = [None, None];
            let mut height = 1u32;
            if !gap && row.is_some() {
                for (k, side) in [Side::Left, Side::Right].into_iter().enumerate() {
                    lines_of[k] = self.line_at_pos(side, pos);
                    if let Some(l) = lines_of[k] {
                        let g = self.line_geom(side, l, metrics);
                        height = height.max(g.visuals.len() as u32 + u32::from(g.continues));
                        geoms[k] = Some(g);
                    }
                }
            }
            out.push(ScreenRow {
                pos,
                row,
                lines: lines_of,
                geoms,
                height,
                top,
            });
            top += height;
            pos += 1;
        }
        out
    }

    /// The caret position under a point of `side`'s pane: `y` in visual lines from the screen's top, `x` in display
    /// units from the text's start (the pane's horizontal scroll added by the caller). A gap of that side goes to the
    /// next line of that side (D-295's open default); below the last row, to the document's end.
    pub fn hit(&self, side: Side, screen: &[ScreenRow], y: f32, x: f32) -> Option<TextPos> {
        let doc = self.doc(side)?;
        let k = index_of(side);
        let y = y.max(0.0);
        let at = screen
            .iter()
            .find(|r| y < (r.top + r.height) as f32)
            .or_else(|| screen.last())?;
        let past_end = y >= (at.top + at.height) as f32;
        match (&at.lines[k], &at.geoms[k]) {
            (Some(line), Some(g)) if !past_end => {
                let v =
                    ((y - at.top as f32).floor() as usize).min(g.visuals.len().saturating_sub(1));
                Some(doc.clamp(TextPos::new(*line, g.hit(v, x))))
            }
            _ if past_end => Some(hikasa_text::edit::doc_end(doc)),
            _ => {
                // A gap of this side: the next line of this side, else the end.
                let row = at.row?;
                let line = self.line_at_row(side, row)?;
                Some(doc.clamp(TextPos::new(line, 0)))
            }
        }
    }

    /// The display position of the caret's row (to bring it into view).
    pub fn caret_pos(&self, side: Side) -> Option<u32> {
        let line = self.pane(side).caret.line.0;
        let row = self.row_of_line(side, line)?;
        Some(self.position_of_row(row))
    }

    /// The row index of display position `pos`, if it shows a row.
    pub fn row_at_pos(&self, pos: u32) -> Option<RowIdx> {
        match self.filtered()?.get(pos)? {
            ViewItem::Row { row } => Some(row),
            ViewItem::Gap { .. } => None,
        }
    }
}
