//! The delimited and fixed-width parsers (contract §2.2).

use crate::cell::Cell;
use crate::error::{TableError, TableWarning, TableWarningCode};
use crate::format::{DelimitedOptions, FixedOptions};
use crate::sheet::SheetBuilder;
use hikasa_core::CancelToken;

/// Rows between two cancellation checks.
const CHECK_EVERY: u32 = 65_536;

/// Classifies characters for the parser: ASCII through a table, others through a short list.
struct Delims {
    ascii: [bool; 128],
    other: Vec<char>,
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
        Delims { ascii, other }
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

/// Parses `builder`'s arena as delimited text into rows. Returns the warnings.
pub(crate) fn parse_delimited(
    builder: &mut SheetBuilder,
    opts: &DelimitedOptions,
    cancel: &CancelToken,
) -> Result<Vec<TableWarning>, TableError> {
    let text = std::mem::take(builder.arena_mut());
    let result = parse_into(builder, &text, opts, cancel);
    *builder.arena_mut() = text;
    result
}

fn parse_into(
    b: &mut SheetBuilder,
    text: &str,
    opts: &DelimitedOptions,
    cancel: &CancelToken,
) -> Result<Vec<TableWarning>, TableError> {
    let delims = Delims::new(&opts.delimiters);
    let q = opts.qualifier;
    let mut qbuf = [0u8; 4];
    let qs: &str = q.map_or("", |c| c.encode_utf8(&mut qbuf));
    let qlen = qs.len();
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut pos = 0usize;
    let mut line: u32 = 1;
    let mut warnings = Vec::new();
    let mut stray_warned = false;
    let mut rows: u32 = 0;
    b.reserve(len / 8, len / 64);
    while pos < len {
        rows += 1;
        if rows.is_multiple_of(CHECK_EVERY) {
            cancel.check()?;
        }
        let record_line = line;
        if opts.consecutive_as_one {
            loop {
                if pos < len && opts.trim_whitespace && is_blank(bytes[pos]) {
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
        if pos >= len || bytes[pos] == b'\n' || bytes[pos] == b'\r' {
            pos = end_of_line(bytes, pos);
            line += 1;
            b.end_row(Some(record_line));
            continue;
        }
        loop {
            // One field.
            if opts.trim_whitespace {
                while pos < len && is_blank(bytes[pos]) && delims.at(text, pos) == 0 {
                    pos += 1;
                }
            }
            if qlen > 0 && text[pos..].starts_with(qs) {
                pos += qlen;
                let start = pos;
                let mut doubled = false;
                let end;
                loop {
                    let Some(off) = find_any(&bytes[pos..], qs.as_bytes()[0], b'\n', b'\r') else {
                        warnings.push(TableWarning::new(
                            TableWarningCode::UnterminatedQualifier,
                            format!("a qualified field opened on line {record_line} is not closed"),
                        ));
                        end = len;
                        pos = len;
                        break;
                    };
                    pos += off;
                    match bytes[pos] {
                        b'\n' => {
                            line += 1;
                            pos += 1;
                        }
                        b'\r' => {
                            line += 1;
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
                while pos < len && delims.at(text, pos) == 0 && bytes[pos] != b'\n' && bytes[pos] != b'\r'
                {
                    pos += 1;
                    if !(opts.trim_whitespace && is_blank(bytes[pos - 1])) {
                        stray_end = pos;
                    }
                }
                let stray = &text[after..stray_end];
                let content = &text[start..end];
                if !doubled && stray.is_empty() {
                    if content.is_empty() {
                        b.push(Cell::EMPTY)?;
                    } else {
                        b.push_span(start, end)?;
                    }
                } else {
                    let mut value = if doubled {
                        content.replace(&format!("{qs}{qs}"), qs)
                    } else {
                        content.to_owned()
                    };
                    if !stray.is_empty() {
                        if !stray_warned {
                            stray_warned = true;
                            warnings.push(TableWarning::new(
                                TableWarningCode::StrayAfterQualifier,
                                format!(
                                    "characters after a closing qualifier on line {line} are kept as data"
                                ),
                            ));
                        }
                        value.push_str(stray);
                    }
                    if value.is_empty() {
                        b.push(Cell::EMPTY)?;
                    } else {
                        b.push_text(&value)?;
                    }
                }
            } else {
                let start = pos;
                while pos < len
                    && bytes[pos] != b'\n'
                    && bytes[pos] != b'\r'
                    && delims.at(text, pos) == 0
                {
                    pos += 1;
                }
                let mut end = pos;
                if opts.trim_whitespace {
                    while end > start && is_blank(bytes[end - 1]) {
                        end -= 1;
                    }
                }
                if end == start {
                    b.push(Cell::EMPTY)?;
                } else {
                    b.push_span(start, end)?;
                }
            }
            // At a delimiter, a line end or the end.
            if pos < len {
                let d = delims.at(text, pos);
                if d > 0 {
                    pos += d;
                    if opts.consecutive_as_one {
                        loop {
                            if pos < len && opts.trim_whitespace && is_blank(bytes[pos]) {
                                // Blanks between delimiters belong to the separator.
                                let mut p = pos;
                                while p < len && is_blank(bytes[p]) {
                                    p += 1;
                                }
                                if p < len && delims.at(text, p) > 0 {
                                    pos = p;
                                    continue;
                                }
                                if p >= len || bytes[p] == b'\n' || bytes[p] == b'\r' {
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
                        if pos >= len || bytes[pos] == b'\n' || bytes[pos] == b'\r' {
                            // A trailing run separates nothing.
                            break;
                        }
                    } else if pos >= len || bytes[pos] == b'\n' || bytes[pos] == b'\r' {
                        // `a,` ends with an empty field.
                        b.push(Cell::EMPTY)?;
                        break;
                    }
                    continue;
                }
            }
            break;
        }
        if pos < len {
            pos = end_of_line(bytes, pos);
            line += 1;
        }
        b.end_row(Some(record_line));
    }
    Ok(warnings)
}

/// Position after the line end at `pos` (CRLF, CR or LF); `pos` itself when there is none.
fn end_of_line(bytes: &[u8], pos: usize) -> usize {
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

fn find_any(hay: &[u8], a: u8, b: u8, c: u8) -> Option<usize> {
    memchr::memchr3(a, b, c, hay)
}

/// Parses `builder`'s arena as fixed-width text (§2.2 rule 5).
pub(crate) fn parse_fixed(
    b: &mut SheetBuilder,
    opts: &FixedOptions,
    cancel: &CancelToken,
) -> Result<(), TableError> {
    let text = std::mem::take(b.arena_mut());
    let result = fixed_into(b, &text, opts, cancel);
    *b.arena_mut() = text;
    result
}

fn fixed_into(
    b: &mut SheetBuilder,
    text: &str,
    opts: &FixedOptions,
    cancel: &CancelToken,
) -> Result<(), TableError> {
    let bytes = text.as_bytes();
    let mut pos = 0usize;
    let mut line: u32 = 0;
    while pos < bytes.len() {
        line += 1;
        if line.is_multiple_of(CHECK_EVERY) {
            cancel.check()?;
        }
        let end = memchr::memchr2(b'\n', b'\r', &bytes[pos..]).map_or(bytes.len(), |o| pos + o);
        let mut start = pos;
        let mut chars = text[pos..end].char_indices().map(|(i, _)| pos + i).chain([end]);
        let mut cuts = Vec::with_capacity(opts.widths.len() + 1);
        let mut consumed = 0u64;
        let mut current = chars.next().unwrap_or(end);
        for &w in &opts.widths {
            let target = consumed + u64::from(w);
            while consumed < target && current < end {
                current = chars.next().unwrap_or(end);
                consumed += 1;
            }
            cuts.push((start, current));
            start = current;
        }
        cuts.push((start, end));
        let last = cuts.len() - 1;
        for (i, (s, e)) in cuts.into_iter().enumerate() {
            let field = &text[s..e];
            let trimmed = field.trim_matches([' ', '\t']);
            if trimmed.is_empty() {
                if i == last {
                    continue;
                }
                b.push(Cell::EMPTY)?;
            } else {
                let lead = field.len() - field.trim_start_matches([' ', '\t']).len();
                b.push_span(s + lead, s + lead + trimmed.len())?;
            }
        }
        b.end_row(Some(line));
        pos = end_of_line(bytes, end);
    }
    Ok(())
}
