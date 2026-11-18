//! A hex pane's caret, selection, keys and geometry (M2 W13a, contract §1.13.5, D-357; §1.13.2 rule 2), toolkit-free.
//!
//! The caret is an aligned column (D-327): it survives a filter change and is ready for HX-006's editing. The geometry
//! of a row is computed here over W11's [`Metrics`] (a monospace advance; tests use `FixedMetrics`): the address
//! column, the hex digits (two a byte; one space between groups of `group` bytes, and with `group` 1 one space between
//! bytes and two after each 8), two spaces, then one character a byte.

use hikasa_core::Side;

use crate::text::wrap::Metrics;

/// Where the caret is drawn and what Copy copies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Part {
    /// The hex digits.
    #[default]
    Hex,
    /// The characters column.
    Chars,
}

/// One pane's caret and selection.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaneState {
    /// The side.
    pub side: Side,
    /// The caret's aligned column.
    pub caret: u64,
    /// The selection's other end (selection = `anchor..=caret`, one column when equal).
    pub anchor: u64,
    /// Where the caret is drawn.
    pub part: Part,
    /// The horizontal scroll (points; both panes scroll together, the window keeps them equal).
    pub scroll_x: f32,
}

impl PaneState {
    /// A pane at column 0.
    pub fn new(side: Side) -> PaneState {
        PaneState {
            side,
            caret: 0,
            anchor: 0,
            part: Part::Hex,
            scroll_x: 0.0,
        }
    }

    /// The selected columns (`anchor..=caret` as a half-open range).
    pub fn selection(&self) -> std::ops::Range<u64> {
        self.anchor.min(self.caret)..self.anchor.max(self.caret) + 1
    }

    /// More than one column is selected.
    pub fn has_selection(&self) -> bool {
        self.anchor != self.caret
    }
}

/// A caret key of a hex pane (§1.13.5's table).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaretKey {
    /// One column back.
    Left,
    /// One column on.
    Right,
    /// The column a row back (under a filter: the same position in the previous shown row).
    Up,
    /// The column a row on.
    Down,
    /// The row's first column.
    Home,
    /// The row's last column.
    End,
    /// The first column of the pair.
    First,
    /// The last column of the pair.
    Last,
    /// A screen of rows back, the position in the row kept.
    PageUp,
    /// A screen of rows on.
    PageDown,
}

/// The x positions of a row's parts, in characters (multiply by the advance for points).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RowGeometry {
    /// Characters of the address column with its trailing gap (0 when hidden).
    pub address: u32,
    /// Bytes per line.
    pub bpl: u32,
    /// The group size.
    pub group: u32,
    /// The characters column is shown.
    pub chars: bool,
}

impl RowGeometry {
    /// The geometry of a layout: `address_digits` is the width of the address text (`0x` and `_` included), `None`
    /// when the address column is hidden.
    pub fn new(address_digits: Option<u32>, bpl: u16, group: u8, chars: bool) -> RowGeometry {
        RowGeometry {
            address: address_digits.map_or(0, |d| d + 2),
            bpl: u32::from(bpl.clamp(1, 256)),
            group: u32::from(group.max(1)),
            chars,
        }
    }

    /// The x of byte `i`'s first hex digit (characters from the row's start).
    pub fn hex_x(&self, i: u32) -> u32 {
        self.address
            + if self.group == 1 {
                i * 3 + i / 8
            } else {
                i * 2 + i / self.group
            }
    }

    /// The end of the hex digits of a full row.
    pub fn hex_end(&self) -> u32 {
        self.hex_x(self.bpl - 1) + 2
    }

    /// The x of byte `i`'s character.
    pub fn char_x(&self, i: u32) -> u32 {
        self.hex_end() + 2 + i
    }

    /// The width of a row (characters).
    pub fn width(&self) -> u32 {
        if self.chars {
            self.char_x(self.bpl)
        } else {
            self.hex_end()
        }
    }

    /// The part and byte index under `x` characters from the row's start (`None` on the address column). A point
    /// between two bytes' digits belongs to the byte on its left; past the row, the last byte.
    pub fn hit(&self, x: f32) -> Option<(Part, u32)> {
        if x < self.address as f32 {
            return None;
        }
        if self.chars && x >= (self.hex_end() + 1) as f32 {
            let i = (x - self.char_x(0) as f32).floor().max(0.0) as u32;
            return Some((Part::Chars, i.min(self.bpl - 1)));
        }
        // The last byte whose digits start at or before x.
        let (mut lo, mut hi) = (0u32, self.bpl - 1);
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            if self.hex_x(mid) as f32 <= x {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        Some((Part::Hex, lo))
    }
}

/// A pane's rectangle on screen in points: where its rows start and its size (the window passes it; tests pass fixed
/// ones).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PaneRect {
    /// Left edge of the rows.
    pub x: f32,
    /// Top edge of the first row.
    pub y: f32,
    /// Width.
    pub w: f32,
    /// Height.
    pub h: f32,
}

impl PaneRect {
    /// True when `p` lies inside.
    pub fn contains(&self, p: [f32; 2]) -> bool {
        p[0] >= self.x && p[0] < self.x + self.w && p[1] >= self.y && p[1] < self.y + self.h
    }
}

/// What a point of a pane hits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hit {
    /// The filtered row on screen.
    pub row: u64,
    /// The part and byte index in the row (`None`: the address column).
    pub cell: Option<(Part, u32)>,
}

/// The row and cell under `p` in a pane whose screen starts at row `top` with `frac` of it scrolled off.
pub fn hit(
    rect: &PaneRect,
    geometry: &RowGeometry,
    metrics: &dyn Metrics,
    top: u64,
    frac: f32,
    scroll_x: f32,
    p: [f32; 2],
) -> Option<Hit> {
    if !rect.contains(p) {
        return None;
    }
    let lh = metrics.line_height().max(1e-3);
    let cw = metrics.advance('0').max(1e-3);
    let rows = f64::from((p[1] - rect.y) / lh) + f64::from(frac);
    let row = top + rows.floor().max(0.0) as u64;
    let x = (p[0] - rect.x + scroll_x) / cw;
    Some(Hit {
        row,
        cell: geometry.hit(x),
    })
}
