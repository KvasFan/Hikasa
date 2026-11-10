//! The record splitter (contract §2.2): one record of delimited or fixed-width text into its fields. The loader
//! and the compacted sheets (F7: [`crate::Sheet`] re-splits a record on demand) share it, so a field read back is
//! the field the loader stored.

use crate::cell::Cell;
use crate::format::{DelimitedOptions, FixedOptions};

/// Fields between two checkpoints of a wide record (F7: `cell(r, c)` resumes at the checkpoint before `c`).
pub(crate) const CHECKPOINT_EVERY: u32 = 64;

/// Classifies characters for the splitter: ASCII through a table, others through a short list.
#[derive(Debug, Clone)]
pub(crate) struct Delims {
    ascii: [bool; 128],
    other: Vec<char>,
    /// The one delimiter when there is one and it is ASCII (fields are then scanned with `memchr`).
    single: Option<u8>,
}

impl Delims {
    fn new(delims: &[char]) -> Delims {
        let mut ascii = [false; 128];
        let mut other = Vec::new();
        for &d in delims {
            if d.is_ascii() {
                ascii[d as usize] = true;
            } else {
                other.push(d);
            }
        }
        let single = match delims {
            [d] if d.is_ascii() => Some(*d as u8),
            _ => None,
        };
        Delims {
            ascii,
            other,
            single,
        }
    }

    /// Length in bytes of a delimiter at `pos`, or 0.
    #[inline]
    fn at(&self, text: &str, pos: usize) -> usize {
        let b = text.as_bytes()[pos];
        if b < 128 {
            usize::from(self.ascii[b as usize])
        } else if self.other.is_empty() || b & 0xC0 == 0x80 {
            0
        } else {
            match text[pos..].chars().next() {
                Some(c) if self.other.contains(&c) => c.len_utf8(),
                _ => 0,
            }
        }
    }
}

fn is_blank(b: u8) -> bool {
    b == b' ' || b == b'\t'
}

fn is_eol(b: u8) -> bool {
    b == b'\n' || b == b'\r'
}

/// Position after the line end at `pos` (CRLF, CR or LF); `pos` itself when there is none.
pub(crate) fn end_of_line(bytes: &[u8], pos: usize) -> usize {
    match bytes.get(pos) {
        Some(b'\r') => {
            if bytes.get(pos + 1) == Some(&b'\n') {
                pos + 2
            } else {
                pos + 1
            }
        }
        Some(b'\n') => pos + 1,
        _ => pos,
    }
}

/// How a sheet's records are split: the options of its loader, prepared.
#[derive(Debug, Clone)]
pub(crate) enum Splitter {
    /// Delimited text.
    Delimited {
        delims: Delims,
        /// The qualifier's UTF-8 (empty: no qualifier).
        qualifier: Box<str>,
        trim: bool,
        consecutive: bool,
    },
    /// Fixed-width text.
    Fixed { widths: Box<[u32]> },
}

/// One field as written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Field {
    /// No value.
    Empty,
    /// The value is the span `[start, end)` of the text.
    Span(usize, usize),
    /// A qualified field whose value is not a span of the text: `[start, end)` holds doubled qualifiers (`doubled`)
    /// and/or characters follow its closing qualifier (`stray`, kept as data). `lines`: line breaks of the record
    /// before the stray characters.
    Unescaped {
        start: usize,
        end: usize,
        doubled: bool,
        stray: (usize, usize),
        lines: u32,
    },
}

impl Field {
    /// The packed cell of a field that is not [`Field::Unescaped`] (`None` for that one).
    #[inline]
    pub(crate) fn cell(self) -> Option<Cell> {
        match self {
            Field::Empty => Some(Cell::EMPTY),
            Field::Span(s, e) => Some(Cell::text(s as u32, (e - s) as u32)),
            Field::Unescaped { .. } => None,
        }
    }
}

/// One field produced by [`Fields`].
#[derive(Debug, Clone, Copy)]
pub(crate) struct FieldOut {
    /// The field.
    pub field: Field,
    /// Where the field starts (a checkpoint resumes there).
    pub at: usize,
    /// A qualified field that is not closed (it runs to the end of the text).
    pub unterminated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Before the record's first field (leading runs of delimiters are skipped there).
    Start,
    /// At the start of a field.
    Field,
    /// `a,` ends with an empty field.
    TrailingEmpty,
    Done,
}

/// The fields of one record, in order.
#[derive(Debug, Clone)]
pub(crate) struct Fields<'s, 't> {
    splitter: &'s Splitter,
    text: &'t str,
    pos: usize,
    state: State,
    /// Line breaks consumed so far (inside qualified fields).
    lines: u32,
    /// Fixed: index of the next field; the record's line end.
    index: usize,
    end: usize,
}

impl Splitter {
    /// The splitter of delimited `opts`.
    pub(crate) fn delimited(opts: &DelimitedOptions) -> Splitter {
        let mut buf = [0u8; 4];
        Splitter::Delimited {
            delims: Delims::new(&opts.delimiters),
            qualifier: opts
                .qualifier
                .map_or("", |c| c.encode_utf8(&mut buf))
                .into(),
            trim: opts.trim_whitespace,
            consecutive: opts.consecutive_as_one,
        }
    }

    /// The splitter of fixed-width `opts`.
    pub(crate) fn fixed(opts: &FixedOptions) -> Splitter {
        Splitter::Fixed {
            widths: opts.widths.clone().into_boxed_slice(),
        }
    }

    /// Heap bytes of the splitter.
    pub(crate) fn heap_bytes(&self) -> usize {
        match self {
            Splitter::Delimited {
                delims, qualifier, ..
            } => delims.other.capacity() * 4 + qualifier.len(),
            Splitter::Fixed { widths } => widths.len() * 4,
        }
    }

    /// The fields of the record that starts at `pos` of `text` (`pos < text.len()`).
    pub(crate) fn record<'s, 't>(&'s self, text: &'t str, pos: usize) -> Fields<'s, 't> {
        let end = match self {
            Splitter::Fixed { .. } => line_end(text, pos),
            Splitter::Delimited { .. } => 0,
        };
        Fields {
            splitter: self,
            text,
            pos,
            state: State::Start,
            lines: 0,
            index: 0,
            end,
        }
    }

    /// The fields of a record from field `index` on, which starts at `at` (a [`FieldOut::at`] the record yielded).
    pub(crate) fn resume<'s, 't>(&'s self, text: &'t str, at: usize, index: u32) -> Fields<'s, 't> {
        let mut fields = self.record(text, at);
        fields.state = State::Field;
        fields.index = index as usize;
        fields
    }
}

fn line_end(text: &str, pos: usize) -> usize {
    memchr::memchr2(b'\n', b'\r', &text.as_bytes()[pos..]).map_or(text.len(), |o| pos + o)
}

impl Fields<'_, '_> {
    /// Ends the record: the position of the next record and the line breaks the record consumed (its own end
    /// included). Fields not read yet are skipped.
    pub(crate) fn finish(mut self) -> (usize, u32) {
        while self.next().is_some() {}
        let bytes = self.text.as_bytes();
        let pos = match self.splitter {
            Splitter::Fixed { .. } => self.end,
            Splitter::Delimited { .. } => self.pos,
        };
        if pos < bytes.len() {
            (end_of_line(bytes, pos), self.lines + 1)
        } else {
            (pos, self.lines)
        }
    }

    fn next_fixed(&mut self, widths: &[u32]) -> Option<FieldOut> {
        loop {
            if self.index > widths.len() {
                return None;
            }
            let start = self.pos;
            let cut = match widths.get(self.index) {
                Some(&w) => self.text[start..self.end]
                    .char_indices()
                    .nth(w as usize)
                    .map_or(self.end, |(i, _)| start + i),
                None => self.end,
            };
            self.pos = cut;
            self.index += 1;
            let field = &self.text[start..cut];
            let trimmed = field.trim_matches([' ', '\t']);
            let field = if trimmed.is_empty() {
                if self.index > widths.len() {
                    // Text past the last width that is blank is no column.
                    continue;
                }
                Field::Empty
            } else {
                let lead = field.len() - field.trim_start_matches([' ', '\t']).len();
                Field::Span(start + lead, start + lead + trimmed.len())
            };
            return Some(FieldOut {
                field,
                at: start,
                unterminated: false,
            });
        }
    }

    fn next_delimited(
        &mut self,
        delims: &Delims,
        qs: &str,
        trim: bool,
        consecutive: bool,
    ) -> Option<FieldOut> {
        let text = self.text;
        let bytes = text.as_bytes();
        let len = bytes.len();
        let mut pos = self.pos;
        match self.state {
            State::Done => return None,
            State::TrailingEmpty => {
                self.state = State::Done;
                return Some(FieldOut {
                    field: Field::Empty,
                    at: pos,
                    unterminated: false,
                });
            }
            State::Start => {
                if consecutive {
                    loop {
                        if pos < len && trim && is_blank(bytes[pos]) {
                            pos += 1;
                            continue;
                        }
                        let d = if pos < len { delims.at(text, pos) } else { 0 };
                        if d == 0 {
                            break;
                        }
                        pos += d;
                    }
                }
                // An empty record (or one of only skipped delimiters).
                if pos >= len || is_eol(bytes[pos]) {
                    self.pos = pos;
                    self.state = State::Done;
                    return None;
                }
            }
            State::Field => {}
        }
        let at = pos;
        let qlen = qs.len();
        let mut unterminated = false;
        // One field.
        if trim {
            while pos < len && is_blank(bytes[pos]) && delims.at(text, pos) == 0 {
                pos += 1;
            }
        }
        let field = if qlen > 0 && text[pos..].starts_with(qs) {
            pos += qlen;
            let start = pos;
            let mut doubled = false;
            let end;
            loop {
                let Some(off) = memchr::memchr3(qs.as_bytes()[0], b'\n', b'\r', &bytes[pos..])
                else {
                    unterminated = true;
                    end = len;
                    pos = len;
                    break;
                };
                pos += off;
                match bytes[pos] {
                    b'\n' => {
                        self.lines += 1;
                        pos += 1;
                    }
                    b'\r' => {
                        self.lines += 1;
                        pos += 1;
                        if pos < len && bytes[pos] == b'\n' {
                            pos += 1;
                        }
                    }
                    _ if text[pos..].starts_with(qs) => {
                        if text[pos + qlen..].starts_with(qs) {
                            doubled = true;
                            pos += 2 * qlen;
                        } else {
                            end = pos;
                            pos += qlen;
                            break;
                        }
                    }
                    // A character sharing the first byte of a multi-byte qualifier.
                    _ => pos += text[pos..].chars().next().map_or(1, char::len_utf8),
                }
            }
            // After the closing qualifier: blanks (trim), then stray characters up to the delimiter.
            let after = pos;
            let mut stray_end = pos;
            while pos < len && delims.at(text, pos) == 0 && !is_eol(bytes[pos]) {
                pos += 1;
                if !(trim && is_blank(bytes[pos - 1])) {
                    stray_end = pos;
                }
            }
            if !doubled && stray_end == after {
                if end == start {
                    Field::Empty
                } else {
                    Field::Span(start, end)
                }
            } else {
                Field::Unescaped {
                    start,
                    end,
                    doubled,
                    stray: (after, stray_end),
                    lines: self.lines,
                }
            }
        } else {
            let start = pos;
            match delims.single {
                Some(d) => {
                    pos = memchr::memchr3(d, b'\n', b'\r', &bytes[pos..]).map_or(len, |o| pos + o);
                }
                None => {
                    while pos < len && !is_eol(bytes[pos]) && delims.at(text, pos) == 0 {
                        pos += 1;
                    }
                }
            }
            let mut end = pos;
            if trim {
                while end > start && is_blank(bytes[end - 1]) {
                    end -= 1;
                }
            }
            if end == start {
                Field::Empty
            } else {
                Field::Span(start, end)
            }
        };
        // At a delimiter, a line end or the end.
        self.state = State::Done;
        if pos < len {
            let d = delims.at(text, pos);
            if d > 0 {
                pos += d;
                self.state = State::Field;
                if consecutive {
                    loop {
                        if pos < len && trim && is_blank(bytes[pos]) {
                            // Blanks between delimiters belong to the separator.
                            let mut p = pos;
                            while p < len && is_blank(bytes[p]) {
                                p += 1;
                            }
                            if p < len && delims.at(text, p) > 0 {
                                pos = p;
                                continue;
                            }
                            if p >= len || is_eol(bytes[p]) {
                                pos = p;
                            }
                            break;
                        }
                        let d = if pos < len { delims.at(text, pos) } else { 0 };
                        if d == 0 {
                            break;
                        }
                        pos += d;
                    }
                    if pos >= len || is_eol(bytes[pos]) {
                        // A trailing run separates nothing.
                        self.state = State::Done;
                    }
                } else if pos >= len || is_eol(bytes[pos]) {
                    self.state = State::TrailingEmpty;
                }
            }
        }
        self.pos = pos;
        Some(FieldOut {
            field,
            at,
            unterminated,
        })
    }
}

impl Iterator for Fields<'_, '_> {
    type Item = FieldOut;

    #[inline]
    fn next(&mut self) -> Option<FieldOut> {
        match self.splitter {
            Splitter::Delimited {
                delims,
                qualifier,
                trim,
                consecutive,
            } => self.next_delimited(delims, qualifier, *trim, *consecutive),
            Splitter::Fixed { widths } => self.next_fixed(widths),
        }
    }
}
