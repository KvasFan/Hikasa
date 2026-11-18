//! Positions and selections (§1.10.3): [`TextPos`], [`Selection`].

use std::ops::Range;

use serde::{Deserialize, Serialize};

use crate::document::LineIdx;

/// A position in a document: a line and a UTF-8 byte offset in its content (terminator excluded), at a char
/// boundary. `TextPos { line, byte: len }` is the end of the line. Ordered by line, then byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TextPos {
    /// Line.
    pub line: LineIdx,
    /// Byte offset in the line's content.
    pub byte: u32,
}

impl TextPos {
    /// `(line, byte)`.
    pub const fn new(line: u32, byte: u32) -> TextPos {
        TextPos {
            line: LineIdx(line),
            byte,
        }
    }

    /// The start of the document.
    pub const ZERO: TextPos = TextPos::new(0, 0);
}

/// A selection: the fixed end (`anchor`) and the moving end (`caret`); empty when they are equal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Selection {
    /// Where the selection started.
    pub anchor: TextPos,
    /// Where the caret is.
    pub caret: TextPos,
}

impl Selection {
    /// An empty selection at `pos`.
    pub const fn caret(pos: TextPos) -> Selection {
        Selection {
            anchor: pos,
            caret: pos,
        }
    }

    /// True when anchor and caret are equal.
    pub fn is_empty(&self) -> bool {
        self.anchor == self.caret
    }

    /// The selected range, start before end.
    pub fn range(&self) -> Range<TextPos> {
        if self.anchor <= self.caret {
            self.anchor..self.caret
        } else {
            self.caret..self.anchor
        }
    }
}
