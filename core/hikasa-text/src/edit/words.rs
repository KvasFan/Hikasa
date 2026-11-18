//! Word characters, character classes and grapheme steps of the editing commands (D-58, D-264).

use unicode_segmentation::UnicodeSegmentation;

/// A word character: a letter, a digit, `_`, or one of the format's extra word characters (D-58).
pub(crate) fn is_word(c: char, extra: &str) -> bool {
    c.is_alphanumeric() || c == '_' || extra.contains(c)
}

/// A blank: space or tab.
pub(crate) fn is_blank(c: char) -> bool {
    c == ' ' || c == '\t'
}

/// The class of a character for word runs: words, blanks, and every other character on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Class {
    Word,
    Blank,
    Other,
}

pub(crate) fn class(c: char, extra: &str) -> Class {
    if is_word(c, extra) {
        Class::Word
    } else if is_blank(c) {
        Class::Blank
    } else {
        Class::Other
    }
}

/// The byte offset of the grapheme cluster boundary after `byte` in `line` (`line.len()` at the end).
pub(crate) fn next_grapheme(line: &str, byte: usize) -> usize {
    line[byte..]
        .grapheme_indices(true)
        .nth(1)
        .map_or(line.len(), |(k, _)| byte + k)
}

/// The byte offset of the grapheme cluster boundary before `byte` in `line` (0 at the start).
pub(crate) fn prev_grapheme(line: &str, byte: usize) -> usize {
    line[..byte]
        .grapheme_indices(true)
        .next_back()
        .map_or(0, |(k, _)| k)
}

/// The run `[start, end)` of characters of the same class around `byte` (preferring the run before `byte`
/// when `byte` ends a word or is at the line end); `Other` characters are runs of one.
pub(crate) fn run_at(line: &str, byte: usize, extra: &str) -> (usize, usize) {
    if line.is_empty() {
        return (0, 0);
    }
    let after = line[byte..].chars().next();
    let before = line[..byte].chars().next_back();
    let pick_before = match (before, after) {
        (Some(b), Some(a)) => class(b, extra) == Class::Word && class(a, extra) != Class::Word,
        (Some(_), None) => true,
        _ => false,
    };
    let (anchor, c) = if pick_before {
        let c = before.unwrap_or(' ');
        (byte - c.len_utf8(), c)
    } else {
        (byte, after.unwrap_or(' '))
    };
    let k = class(c, extra);
    if k == Class::Other {
        return (anchor, anchor + c.len_utf8());
    }
    let mut start = anchor;
    for ch in line[..anchor].chars().rev() {
        if class(ch, extra) != k {
            break;
        }
        start -= ch.len_utf8();
    }
    let mut end = anchor;
    for ch in line[anchor..].chars() {
        if class(ch, extra) != k {
            break;
        }
        end += ch.len_utf8();
    }
    (start, end)
}

/// The display column of `byte` in `line`: tabs advance to the next multiple of `tab_size`, other characters by
/// their display width (wide characters 2, combining marks 0).
pub(crate) fn column(line: &str, byte: usize, tab_size: u8) -> u32 {
    let ts = u32::from(tab_size.max(1));
    let mut col = 0u32;
    for c in line[..byte].chars() {
        if c == '\t' {
            col = (col / ts + 1) * ts;
        } else {
            col += unicode_width::UnicodeWidthChar::width(c).unwrap_or(0) as u32;
        }
    }
    col
}

/// Byte length of the leading blanks of `line`.
pub(crate) fn indent_len(line: &str) -> usize {
    line.len() - line.trim_start_matches(is_blank).len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_and_columns() {
        assert_eq!(run_at("foo  bar", 1, ""), (0, 3));
        assert_eq!(run_at("foo  bar", 3, ""), (0, 3));
        assert_eq!(run_at("foo  bar", 4, ""), (3, 5));
        assert_eq!(run_at("a+=b", 1, ""), (0, 1));
        assert_eq!(run_at("a+=b", 2, ""), (2, 3));
        assert_eq!(run_at("a+=b", 1, "+="), (0, 4));
        assert_eq!(run_at("ab", 2, ""), (0, 2));
        assert_eq!(column("\tx", 1, 4), 4);
        assert_eq!(column("ab\tx", 3, 4), 4);
        assert_eq!(column("界x", 3, 4), 2);
        assert_eq!(next_grapheme("e\u{301}x", 0), 3);
        assert_eq!(prev_grapheme("e\u{301}x", 3), 0);
        assert_eq!(indent_len(" \t x"), 3);
    }
}
