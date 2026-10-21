//! Display cells of plain-text output: tab expansion, control-character escapes and column widths from
//! `unicode-width`, plus the streaming line wrapper of side-by-side text (none, character, word).
//!
//! A line becomes a stream of [`Atom`]s (one displayed character each, with its width 0, 1 or 2 and its tone for
//! ANSI colouring). The stream is lazy, so a very long line costs memory proportional to one column, not to the
//! line.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::ops::Range;

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::escape::{control_escape, is_escaped};
use crate::options::Wrap;
use crate::style::Sgr;

/// Colouring class of a displayed character (ANSI output).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tone {
    /// No colour.
    Plain,
    /// Important difference (red).
    Important,
    /// Unimportant difference (blue).
    Unimportant,
}

impl Tone {
    /// The SGR sequence that starts this tone (`""` for plain).
    pub fn sgr(self) -> &'static str {
        match self {
            Tone::Plain => "",
            Tone::Important => Sgr::RED,
            Tone::Unimportant => Sgr::BLUE,
        }
    }
}

/// One displayed character. A cluster of characters that display as one unit (a base character with its
/// combining marks, an emoji ZWJ sequence, a character with a variation selector or an emoji modifier, a pair of
/// regional indicators) is one atom carrying the cluster's width followed by zero-width atoms, which the wrapper
/// never separates from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Atom {
    /// The character.
    pub ch: char,
    /// Display columns (the whole cluster's width on its first character, 0 on the others, 2 for wide
    /// characters).
    pub width: u8,
    /// Colouring.
    pub tone: Tone,
}

/// Display width of text whose control characters are already escaped: the `unicode-width` width of the string,
/// which counts an emoji ZWJ sequence or a character with an emoji variation selector as the two columns it
/// takes, not as the sum of its characters.
pub(crate) fn text_width(s: &str) -> usize {
    s.width()
}

/// A regional indicator symbol (two of them make a flag).
fn is_regional(c: char) -> bool {
    ('\u{1f1e6}'..='\u{1f1ff}').contains(&c)
}

/// True for a character that continues the cluster before it: a zero-width character (combining mark, ZWJ,
/// variation selector, tag character) or an emoji modifier; never a tab or a character shown as an escape.
fn extends_cluster(c: char) -> bool {
    c != '\t'
        && !is_escaped(c)
        && (c.width() == Some(0) || ('\u{1f3fb}'..='\u{1f3ff}').contains(&c))
}

/// Lazily turns one line into atoms: tabs expanded to the next multiple of `tab` columns (counted from the start
/// of the line), control characters shown as their escapes, every byte toned by `tones` (sorted, disjoint byte
/// ranges) or else `base`.
pub(crate) struct Atoms<'a> {
    line: &'a str,
    chars: std::iter::Peekable<std::str::CharIndices<'a>>,
    tab: usize,
    col: usize,
    pending: VecDeque<Atom>,
    tones: &'a [(Range<u32>, Tone)],
    next_tone: usize,
    base: Tone,
}

impl<'a> Atoms<'a> {
    /// Atoms of `line`.
    pub fn new(
        line: &'a str,
        tab: usize,
        tones: &'a [(Range<u32>, Tone)],
        base: Tone,
    ) -> Atoms<'a> {
        Atoms {
            line,
            chars: line.char_indices().peekable(),
            tab: tab.max(1),
            col: 0,
            pending: VecDeque::new(),
            tones,
            next_tone: 0,
            base,
        }
    }

    /// The tone of the byte at `at` (offsets only increase).
    fn tone_at(&mut self, at: usize) -> Tone {
        while let Some((r, _)) = self.tones.get(self.next_tone) {
            if (r.end as usize) <= at {
                self.next_tone += 1;
            } else {
                break;
            }
        }
        match self.tones.get(self.next_tone) {
            Some((r, tone)) if (r.start as usize) <= at => *tone,
            _ => self.base,
        }
    }
}

impl Iterator for Atoms<'_> {
    type Item = Atom;

    fn next(&mut self) -> Option<Atom> {
        if let Some(a) = self.pending.pop_front() {
            return Some(a);
        }
        let (at, c) = self.chars.next()?;
        let tone = self.tone_at(at);
        if c == '\t' {
            let n = self.tab - self.col % self.tab;
            self.col += n;
            for _ in 1..n {
                self.pending.push_back(Atom {
                    ch: ' ',
                    width: 1,
                    tone,
                });
            }
            return Some(Atom {
                ch: ' ',
                width: 1,
                tone,
            });
        }
        if let Some(esc) = control_escape(c) {
            let mut chars = esc.chars();
            let first = chars.next().unwrap_or('\\');
            for e in chars {
                self.pending.push_back(Atom {
                    ch: e,
                    width: 1,
                    tone,
                });
            }
            self.col += esc.chars().count();
            return Some(Atom {
                ch: first,
                width: 1,
                tone,
            });
        }
        // A cluster: the base character, then every character that continues it (after a ZWJ, the next
        // character too); the base carries the cluster's width.
        let single = at + c.len_utf8();
        let mut end = single;
        let mut regional = is_regional(c);
        while let Some(&(i, n)) = self.chars.peek() {
            if n.is_ascii() {
                break; // no ASCII character continues a cluster
            }
            let joins = if regional && is_regional(n) {
                regional = false;
                true
            } else {
                extends_cluster(n)
            };
            if !joins {
                break;
            }
            self.chars.next();
            end = i + n.len_utf8();
            let tone = self.tone_at(i);
            self.pending.push_back(Atom {
                ch: n,
                width: 0,
                tone,
            });
            if n == '\u{200d}'
                && let Some(&(j, m)) = self.chars.peek()
                && m != '\t'
                && !is_escaped(m)
            {
                self.chars.next();
                end = j + m.len_utf8();
                let tone = self.tone_at(j);
                self.pending.push_back(Atom {
                    ch: m,
                    width: 0,
                    tone,
                });
            }
        }
        let width = if end == single {
            c.width().unwrap_or(1)
        } else {
            self.line.get(at..end).map_or(1, UnicodeWidthStr::width)
        };
        let width = u8::try_from(width).unwrap_or(u8::MAX);
        self.col += usize::from(width);
        Some(Atom { ch: c, width, tone })
    }
}

/// Breaks a stream of atoms into segments of at most `width` columns (a single atom wider than the column is a
/// segment of its own; zero-width atoms stay with the preceding one).
pub(crate) struct Wrapper<'a> {
    atoms: Atoms<'a>,
    buf: VecDeque<Atom>,
    buf_width: usize,
    width: usize,
    mode: Wrap,
    exhausted: bool,
    emitted: bool,
}

impl<'a> Wrapper<'a> {
    /// A wrapper over `atoms` for a column of `width` (≥ 1) columns.
    pub fn new(atoms: Atoms<'a>, width: usize, mode: Wrap) -> Wrapper<'a> {
        Wrapper {
            atoms,
            buf: VecDeque::new(),
            buf_width: 0,
            width: width.max(1),
            mode,
            exhausted: false,
            emitted: false,
        }
    }

    /// Fills the buffer until it is wider than the column or the line ends.
    fn fill(&mut self) {
        while !self.exhausted && self.buf_width <= self.width {
            match self.atoms.next() {
                Some(a) => {
                    self.buf_width += usize::from(a.width);
                    self.buf.push_back(a);
                }
                None => self.exhausted = true,
            }
        }
    }

    /// Takes the first `k` atoms of the buffer.
    fn take(&mut self, k: usize) -> Vec<Atom> {
        let seg: Vec<Atom> = self.buf.drain(..k.min(self.buf.len())).collect();
        self.buf_width -= seg.iter().map(|a| usize::from(a.width)).sum::<usize>();
        seg
    }

    /// The next segment; an empty line gives one empty segment; `None` after the last one.
    pub fn next_segment(&mut self) -> Option<Vec<Atom>> {
        if self.mode == Wrap::None && self.emitted {
            return None;
        }
        self.fill();
        if self.buf.is_empty() {
            if self.emitted {
                return None;
            }
            self.emitted = true;
            return Some(Vec::new());
        }
        self.emitted = true;
        if self.exhausted && self.buf_width <= self.width {
            let n = self.buf.len();
            return Some(self.take(n));
        }
        // The longest prefix that fits, plus the zero-width atoms that follow it.
        let mut k = 0;
        let mut w = 0usize;
        while let Some(a) = self.buf.get(k) {
            if w + usize::from(a.width) > self.width {
                break;
            }
            w += usize::from(a.width);
            k += 1;
        }
        if k == 0 {
            k = 1;
        }
        while self.buf.get(k).is_some_and(|a| a.width == 0) {
            k += 1;
        }
        if self.mode == Wrap::Word {
            let fits_with_next =
                |i: usize| self.buf.get(i + 1).is_some_and(|a| !a.ch.is_whitespace());
            if let Some(j) = (0..k).rev().find(|&j| {
                self.buf.get(j).is_some_and(|a| a.ch.is_whitespace()) && fits_with_next(j)
            }) {
                k = j + 1;
            }
        }
        let seg = self.take(k);
        if self.mode == Wrap::None {
            self.buf.clear();
            self.buf_width = 0;
        }
        Some(seg)
    }
}

/// Display width of a segment.
pub(crate) fn segment_width(seg: &[Atom]) -> usize {
    seg.iter().map(|a| usize::from(a.width)).sum()
}

/// Appends a segment to `out`, with SGR sequences at tone changes when `ansi` (reset at the end).
pub(crate) fn push_segment(out: &mut String, seg: &[Atom], ansi: bool) {
    let mut current = Tone::Plain;
    for a in seg {
        if ansi && a.tone != current {
            if current != Tone::Plain {
                out.push_str(Sgr::RESET);
            }
            out.push_str(a.tone.sgr());
            current = a.tone;
        }
        out.push(a.ch);
    }
    if ansi && current != Tone::Plain {
        out.push_str(Sgr::RESET);
    }
}

/// Appends `n` spaces.
pub(crate) fn pad(out: &mut String, n: usize) {
    let _ = write!(out, "{:n$}", "");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn atoms(s: &str) -> String {
        Atoms::new(s, 4, &[], Tone::Plain).map(|a| a.ch).collect()
    }

    fn wrap(s: &str, width: usize, mode: Wrap) -> Vec<String> {
        let mut w = Wrapper::new(Atoms::new(s, 4, &[], Tone::Plain), width, mode);
        let mut out = Vec::new();
        while let Some(seg) = w.next_segment() {
            out.push(seg.iter().map(|a| a.ch).collect());
        }
        out
    }

    #[test]
    fn tabs_and_controls() {
        assert_eq!(atoms("a\tb"), "a   b");
        assert_eq!(atoms("abcd\te"), "abcd    e");
        assert_eq!(atoms("\t"), "    ");
        assert_eq!(atoms("x\u{1b}y"), "x\\u{1b}y");
        // A tab after an escape counts the escape's columns.
        assert_eq!(atoms("\u{1}\tz"), "\\u{1}   z");
        assert_eq!(text_width("日本"), 4);
        assert_eq!(text_width("e\u{301}"), 1);
    }

    /// The widths of the atoms of `s`.
    fn widths(s: &str) -> Vec<u8> {
        Atoms::new(s, 4, &[], Tone::Plain)
            .map(|a| a.width)
            .collect()
    }

    #[test]
    fn clusters_take_their_string_width() {
        let family = "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}";
        let smiley = "\u{263a}\u{fe0f}";
        let flag = "\u{1f1e9}\u{1f1ea}";
        let thumbs = "\u{1f44d}\u{1f3fd}";
        for s in [family, smiley, flag, thumbs, "e\u{301}", "日", "a"] {
            let w: usize = widths(s).iter().map(|&w| usize::from(w)).sum();
            assert_eq!(w, text_width(s), "{s:?}");
        }
        assert_eq!(text_width(family), 2);
        assert_eq!(text_width(smiley), 2);
        assert_eq!(widths(family), [2, 0, 0, 0, 0]);
        assert_eq!(widths(smiley), [2, 0]);
        assert_eq!(widths("e\u{301}x"), [1, 0, 1]);
        // Two flags are two clusters.
        assert_eq!(widths(&format!("{flag}{flag}")), [2, 0, 2, 0]);
        // A bidirectional control is escaped, never joined.
        assert_eq!(atoms("a\u{202e}b"), "a\\u{202e}b");
        // A cluster never straddles a break.
        let line = format!("ab{family}cd");
        assert_eq!(
            wrap(&line, 3, Wrap::Character),
            ["ab".to_owned(), format!("{family}c"), "d".to_owned()]
        );
        // Text width over a mixed line agrees with the atoms.
        let mixed = format!("x{family}y{smiley}z");
        let w: usize = widths(&mixed).iter().map(|&w| usize::from(w)).sum();
        assert_eq!(w, text_width(&mixed));
    }

    #[test]
    fn character_wrapping() {
        assert_eq!(wrap("abcdefg", 3, Wrap::Character), ["abc", "def", "g"]);
        assert_eq!(wrap("abc", 3, Wrap::Character), ["abc"]);
        assert_eq!(wrap("", 3, Wrap::Character), [""]);
        // A wide character never straddles a break.
        assert_eq!(wrap("ab日本", 3, Wrap::Character), ["ab", "日", "本"]);
        // A combining mark stays with its base.
        assert_eq!(
            wrap("abe\u{301}cd", 3, Wrap::Character),
            ["abe\u{301}", "cd"]
        );
        // A character wider than the column still makes progress.
        assert_eq!(wrap("日本", 1, Wrap::Character), ["日", "本"]);
    }

    #[test]
    fn word_wrapping() {
        assert_eq!(
            wrap("the quick brown fox", 10, Wrap::Word),
            ["the quick ", "brown fox"]
        );
        assert_eq!(
            wrap("abcdefghijkl mn", 5, Wrap::Word),
            ["abcde", "fghij", "kl mn"]
        );
        assert_eq!(wrap("a  b", 2, Wrap::Word), ["a ", " b"]);
    }

    #[test]
    fn no_wrapping_truncates() {
        assert_eq!(wrap("abcdefg", 3, Wrap::None), ["abc"]);
        assert_eq!(wrap("", 3, Wrap::None), [""]);
        assert_eq!(wrap("a日", 2, Wrap::None), ["a"]);
    }

    #[test]
    fn tones_follow_byte_ranges() {
        let tones = [(1..2, Tone::Important), (3..5, Tone::Unimportant)];
        let got: Vec<Tone> = Atoms::new("abcdé", 4, &tones, Tone::Plain)
            .map(|a| a.tone)
            .collect();
        assert_eq!(
            got,
            [
                Tone::Plain,
                Tone::Important,
                Tone::Plain,
                Tone::Unimportant,
                Tone::Unimportant
            ]
        );
        let seg: Vec<Atom> = Atoms::new("abc", 4, &tones, Tone::Plain).collect();
        let mut out = String::new();
        push_segment(&mut out, &seg, true);
        assert_eq!(out, "a\x1b[31mb\x1b[0mc");
        let mut out = String::new();
        push_segment(&mut out, &seg, false);
        assert_eq!(out, "abc");
        assert_eq!(segment_width(&seg), 3);
    }
}
