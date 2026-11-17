//! The details panel and the ruler of a text view (TC-033; M2 W11b, contract §1.11.15, D-306), toolkit-free. The panel
//! follows the focus pane's current row: *Text Details* shows the row's lines (the panes' own documents and carets: a
//! key typed there is an edit of that line in its pane), *Hex Details* `hex_details` of each side's line (4,096 bytes
//! at most), *Alignment Details* `char_alignment_of_row` (a pending row: `char_alignment` of its live lines) as two
//! lines of cells, editable at a cell. Its contents are computed when the row or the documents change. The ruler's
//! marks are core's `ruler`.

use std::cell::RefCell;

use hikasa_core::Side;
use hikasa_text::InlineOptions;
use hikasa_text::details::{
    CellClass, CharCell, HexDetails, RulerMark, char_alignment, char_alignment_of_row, hex_details,
    ruler,
};
use hikasa_text::edit::{EditRequest, TextPos};
use hikasa_text::{LineIdx, RowIdx};

use super::edit::{self, Origin};
use super::{TextViewModel, index_of, side_of};

/// The most bytes Hex Details shows of a line (D-306).
pub const HEX_MAX: u32 = 4096;

/// One row of Hex Details: the offset, 16 bytes in hex, the bytes as characters (`.` for controls), and which of them
/// were written as `?` (unmappable).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HexRow {
    /// The offset of the first byte.
    pub offset: u32,
    /// The bytes.
    pub bytes: Vec<u8>,
    /// Per byte: written as `?` because the encoding cannot write its character.
    pub unmappable: Vec<bool>,
}

impl HexRow {
    /// `41 42 0D` (display only).
    pub fn hex_text(&self) -> String {
        self.bytes
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// The bytes as characters: printable ASCII as itself, every other byte `.`.
    pub fn chars_text(&self) -> String {
        self.bytes
            .iter()
            .map(|b| {
                if (0x20..0x7f).contains(b) {
                    char::from(*b)
                } else {
                    '.'
                }
            })
            .collect()
    }
}

/// The rows of a line's Hex Details (16 bytes a row), the terminator's bytes after them.
pub fn hex_rows(hex: &HexDetails) -> Vec<HexRow> {
    hex.bytes
        .chunks(16)
        .enumerate()
        .map(|(k, chunk)| {
            let offset = (k * 16) as u32;
            HexRow {
                offset,
                bytes: chunk.to_vec(),
                unmappable: (0..chunk.len() as u32)
                    .map(|i| hex.unmappable.iter().any(|r| r.contains(&(offset + i))))
                    .collect(),
            }
        })
        .collect()
}

/// What the panel shows for the current row.
#[derive(Debug, Clone, PartialEq)]
pub struct DetailsData {
    /// The row (`None`: no comparison or no row).
    pub row: Option<u32>,
    /// Each side's line and its text (`None`: a gap of that side).
    pub lines: [Option<(u32, String)>; 2],
    /// Hex Details of each side's line.
    pub hex: [Option<HexDetails>; 2],
    /// Alignment Details' cells (Text Compare with a row).
    pub cells: Option<Vec<CharCell>>,
}

/// The cache of the panel's data: computed when the row or a document changes, never per frame.
#[derive(Debug, Default)]
pub struct Cache {
    inner: RefCell<Option<((Option<u32>, [u64; 2], u64), DetailsData)>>,
    /// Computations so far (tests).
    pub builds: std::cell::Cell<u64>,
}

/// The current row's lines of each side: the row's lines, or (Text Edit) the caret's line.
fn row_lines(view: &TextViewModel) -> (Option<u32>, [Option<u32>; 2]) {
    if view.is_edit() {
        let line = view.pane(Side::Left).caret.line.0;
        return (Some(line), [Some(line), None]);
    }
    let Some(row) = view.cursor() else {
        let focus = view.focus();
        let line = view.pane(focus).caret.line.0;
        let mut out = [None, None];
        out[index_of(focus)] = Some(line);
        return (None, out);
    };
    let lines = view
        .comparison()
        .and_then(|c| c.row(row))
        .map_or([None, None], |r| {
            [r.left.map(|l| l.0), r.right.map(|l| l.0)]
        });
    (Some(row.0), lines)
}

/// The panel's data of now (from the cache when neither the row nor the documents changed).
pub fn data(view: &TextViewModel) -> DetailsData {
    let (row, lines) = row_lines(view);
    let gens = [Side::Left, Side::Right].map(|s| view.doc(s).map_or(0, |d| d.generation()));
    let key = (row, gens, view.installs());
    if let Some((k, d)) = view.details.inner.borrow().as_ref()
        && *k == key
        && d.lines
            .iter()
            .map(|l| l.as_ref().map(|x| x.0))
            .eq(lines.iter().copied())
    {
        return d.clone();
    }
    let mut out = DetailsData {
        row,
        lines: [None, None],
        hex: [None, None],
        cells: None,
    };
    for k in 0..2 {
        let side = side_of(k);
        let (Some(line), Some(doc)) = (lines[k], view.doc(side)) else {
            continue;
        };
        if line >= doc.line_count() {
            continue;
        }
        out.lines[k] = Some((line, doc.line(LineIdx(line)).into_owned()));
        out.hex[k] = Some(hex_details(doc, LineIdx(line), HEX_MAX));
    }
    if let (Some(r), Some(live)) = (row, view.live_view()) {
        out.cells = if live.live.is_pending(RowIdx(r)) {
            // A pending row: the cells of its live lines (the comparison's documents are those of the last install).
            let l = out.lines[0].as_ref().map_or("", |x| x.1.as_str());
            let rt = out.lines[1].as_ref().map_or("", |x| x.1.as_str());
            Some(match (&out.lines[0], &out.lines[1]) {
                (Some(_), Some(_)) => char_alignment(l, rt, &InlineOptions::DEFAULT),
                (Some(_), None) => one_sided(l, true),
                (None, Some(_)) => one_sided(rt, false),
                (None, None) => Vec::new(),
            })
        } else {
            char_alignment_of_row(live.live.current(), RowIdx(r))
        };
    }
    view.details.builds.set(view.details.builds.get() + 1);
    *view.details.inner.borrow_mut() = Some((key, out.clone()));
    out
}

fn one_sided(text: &str, left: bool) -> Vec<CharCell> {
    text.chars()
        .map(|c| CharCell {
            left: left.then_some(c),
            right: (!left).then_some(c),
            class: if left {
                CellClass::LeftOnly
            } else {
                CellClass::RightOnly
            },
        })
        .collect()
}

/// The position in `side`'s line of cell `k` of `cells`: the byte of that cell's character on that side, a gap cell
/// the next character's (the line's end after the last).
pub fn cell_pos(cells: &[CharCell], side: Side, k: usize, line: u32) -> TextPos {
    let mut byte = 0u32;
    for cell in cells.iter().take(k) {
        let c = if side == Side::Right {
            cell.right
        } else {
            cell.left
        };
        if let Some(c) = c {
            byte += c.len_utf8() as u32;
        }
    }
    TextPos::new(line, byte)
}

/// A typed character at cell `k` of the Alignment Details (D-306, **open default**: editable): inserted in `side`'s
/// line at the cell's position, as an edit of that pane; the cells are computed again from the row.
pub fn align_type(view: &mut TextViewModel, side: Side, k: usize, text: &str) -> bool {
    let d = data(view);
    let (Some(cells), Some((line, _))) = (d.cells.as_ref(), d.lines[index_of(side)].as_ref())
    else {
        return false;
    };
    let at = cell_pos(cells, side, k, *line);
    let req = EditRequest::Replace {
        range: at..at,
        text: text.to_owned(),
        keep_eols: false,
    };
    edit::edit(view, side, req, Origin::Single, None)
}

/// Backspace (`back`) or Delete at cell `k`: the character before the cell's position, or the cell's own.
pub fn align_delete(view: &mut TextViewModel, side: Side, k: usize, back: bool) -> bool {
    let d = data(view);
    let (Some(cells), Some((line, text))) = (d.cells.as_ref(), d.lines[index_of(side)].as_ref())
    else {
        return false;
    };
    let at = cell_pos(cells, side, k, *line);
    let b = at.byte as usize;
    let range = if back {
        let Some(c) = text[..b.min(text.len())].chars().next_back() else {
            return false;
        };
        TextPos::new(*line, (b - c.len_utf8()) as u32)..at
    } else {
        let Some(c) = text.get(b..).and_then(|s| s.chars().next()) else {
            return false;
        };
        at..TextPos::new(*line, (b + c.len_utf8()) as u32)
    };
    let req = EditRequest::Replace {
        range,
        text: String::new(),
        keep_eols: false,
    };
    edit::edit(view, side, req, Origin::Single, None)
}

/// A click in the Text Details panel at byte `byte` of `side`'s line: that pane takes the keyboard with its caret
/// there (the panel and the pane are one editor, D-306).
pub fn text_press(view: &mut TextViewModel, side: Side, byte: u32) {
    let d = data(view);
    let Some((line, _)) = d.lines[index_of(side)].as_ref() else {
        return;
    };
    super::input::press(
        view,
        side,
        TextPos::new(*line, byte),
        false,
        super::input::Clicks::One,
    );
}

/// The ruler's marks of display columns `from..=to` of `side` (its document's tab size).
pub fn ruler_marks(view: &TextViewModel, side: Side, from: u32, to: u32) -> Vec<RulerMark> {
    let tab = view.doc(side).map_or(4, |d| d.options().tab_size);
    ruler(tab, from, to)
}

/// The caret's display column of `side` (1-based; the ruler's mark), from core's `column_of_tabs`.
pub fn caret_column(view: &TextViewModel, side: Side) -> Option<u32> {
    let doc = view.doc(side)?;
    let caret = view.pane(side).caret;
    if caret.line.0 >= doc.line_count() {
        return Some(1);
    }
    let line = doc.line(caret.line);
    Some(hikasa_text::nav::column_of_tabs(&line, caret.byte, doc.options().tab_size).1)
}
