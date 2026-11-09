//! A10.22 (D-283): P-FIND and the find properties (§1.10.18). On random documents (mixed terminators, CPL pieces,
//! multi-byte letters, word and operator characters):
//!
//! - **P-FIND** — plain search, with and without case and whole word, equals a naive scan of each physical line
//!   (non-overlapping, left to right; whole word moves one character on after a rejected candidate);
//! - **regex** — a random pattern's hits equal the `regex` crate run line by line (empty matches skipped), so the
//!   windows, the line splitting and the CPL pieces change nothing;
//! - **literal = escaped regex** — plain text and its escaped regex find the same hits under every flag (whole word
//!   included: the regex's boundary assertions agree with the plain check);
//! - **next walks all** — `next` forward from every hit's end, and backward from every hit's start, visits exactly
//!   `all`'s hits; from a random position it gives the first (last) hit after (before) it;
//! - **replace + undo = identity** — Replace All equals a line-by-line replacement of the text, is one step, and its
//!   undo restores the text exactly.

mod common;

use std::ops::Range;
use std::sync::Arc;

use common::config;
use hikasa_core::Ctl;
use hikasa_text::edit::{EditOptions, Editability, EditableDocument, GroupHint};
use hikasa_text::find::{Dir, FindQuery, Finder, Replacement, replace_all};
use hikasa_text::{RegexLimits, TextDocument, TextPos};
use proptest::prelude::*;

const ALPHABET: &[&str] = &[
    "a", "b", "A", "B", "é", "É", "ß", " ", "_", "-", "+", "=", "\n", "\r", "\r\n", "ab", "ba",
];

fn text_strategy() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(ALPHABET), 0..120).prop_map(|parts| parts.concat())
}

fn needle_strategy() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop::sample::select(&["a", "b", "A", "é", "É", " ", "_", "-", "+", "="][..]),
        1..4,
    )
    .prop_map(|parts| parts.concat())
}

fn editable(text: &str, cpl: Option<u32>) -> EditableDocument {
    EditableDocument::new(
        Arc::new(TextDocument::from_text(text)),
        EditOptions {
            chars_per_line_limit: cpl,
            ..EditOptions::default()
        },
        Editability::EDITABLE,
    )
}

/// The physical lines of `text` with their start offsets.
fn physical_lines(text: &str) -> Vec<(usize, &str)> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let (mut start, mut i) = (0, 0);
    while i < b.len() {
        match b[i] {
            b'\n' => {
                out.push((start, &text[start..i]));
                i += 1;
                start = i;
            }
            b'\r' => {
                out.push((start, &text[start..i]));
                i += if b.get(i + 1) == Some(&b'\n') { 2 } else { 1 };
                start = i;
            }
            _ => i += 1,
        }
    }
    if start < b.len() {
        out.push((start, &text[start..]));
    }
    out
}

fn is_word(c: char, extra: &str) -> bool {
    c.is_alphanumeric() || c == '_' || extra.contains(c)
}

/// The naive plain scan of one line (absolute offsets).
fn naive_plain(line: &str, base: usize, q: &FindQuery, extra: &str, out: &mut Vec<Range<usize>>) {
    let fold = |s: &str| -> Vec<char> {
        s.chars()
            .map(|c| {
                if q.match_case {
                    c
                } else {
                    // Simple folding of the letters the alphabet uses.
                    c.to_lowercase().next().unwrap_or(c)
                }
            })
            .collect()
    };
    let hay: Vec<(usize, char)> = line.char_indices().collect();
    let hay_f = fold(line);
    let needle = fold(&q.text);
    let n = needle.len();
    let mut i = 0;
    while i + n <= hay.len() {
        if hay_f[i..i + n] == needle[..] {
            let start = hay[i].0;
            let end = hay.get(i + n).map_or(line.len(), |&(k, _)| k);
            let ok = !q.whole_word || {
                let w = |c: char| is_word(c, extra);
                let before = line[..start].chars().next_back();
                let after = line[end..].chars().next();
                let first = line[start..].chars().next();
                let last = line[..end].chars().next_back();
                !(first.is_some_and(w) && before.is_some_and(w))
                    && !(last.is_some_and(w) && after.is_some_and(w))
            };
            if ok {
                out.push(base + start..base + end);
                i += n;
                continue;
            }
        }
        i += 1;
    }
}

/// The finder's hits as absolute offsets in the text.
fn hits_abs(f: &Finder, doc: &EditableDocument) -> Vec<Range<usize>> {
    f.all(doc, u64::MAX, &Ctl::detached())
        .unwrap()
        .into_iter()
        .map(|h| abs(doc, h.range.start)..abs(doc, h.range.end))
        .collect()
}

fn abs(doc: &EditableDocument, p: TextPos) -> usize {
    match doc.document().line_byte_range(p.line) {
        Some(r) => r.start + p.byte as usize,
        None => doc.document().rope().len_bytes(),
    }
}

fn cpl_strategy() -> impl Strategy<Value = Option<u32>> {
    prop_oneof![Just(None), (2u32..9).prop_map(Some)]
}

fn regex_strategy() -> impl Strategy<Value = String> {
    let atom = prop::sample::select(
        &[
            "a", "b", "A", "é", " ", "_", "-", r"\+", "=", ".", "[ab]", "[^a ]", r"\w", r"\s",
            r"\b", "^", "$", "(?:ab)",
        ][..],
    );
    let quant = prop::sample::select(&["", "", "", "*", "+", "?", "{2}"][..]);
    prop::collection::vec((atom, quant), 1..5).prop_map(|parts| {
        parts
            .into_iter()
            .map(|(a, q)| {
                if q.is_empty() || matches!(a, "^" | "$" | r"\b") {
                    a.to_owned()
                } else {
                    format!("(?:{a}){q}")
                }
            })
            .collect()
    })
}

proptest! {
    #![proptest_config(config("tests/proptest-regressions/props_find.txt"))]

    #[test]
    fn p_find_plain_equals_a_naive_scan(
        text in text_strategy(),
        needle in needle_strategy(),
        match_case in any::<bool>(),
        whole_word in any::<bool>(),
        extra in prop::sample::select(&["", "-", "+="][..]),
        cpl in cpl_strategy(),
    ) {
        let q = FindQuery { text: needle, regex: false, match_case, whole_word };
        let doc = editable(&text, cpl);
        let f = Finder::new(&q, extra, &RegexLimits::default()).unwrap();
        let mut expected = Vec::new();
        for (base, line) in physical_lines(&text) {
            naive_plain(line, base, &q, extra, &mut expected);
        }
        prop_assert_eq!(hits_abs(&f, &doc), expected);
    }

    #[test]
    fn regex_hits_equal_the_engine_line_by_line(
        text in text_strategy(),
        pattern in regex_strategy(),
        match_case in any::<bool>(),
        cpl in cpl_strategy(),
    ) {
        let q = FindQuery { text: pattern.clone(), regex: true, match_case, whole_word: false };
        let Ok(f) = Finder::new(&q, "", &RegexLimits::default()) else {
            return Ok(());
        };
        let re = regex::RegexBuilder::new(&pattern).case_insensitive(!match_case).build().unwrap();
        let doc = editable(&text, cpl);
        let mut expected = Vec::new();
        for (base, line) in physical_lines(&text) {
            let mut at = 0;
            while at <= line.len() {
                let Some(m) = re.find_at(line, at) else { break };
                if m.is_empty() {
                    match line[m.start()..].chars().next() {
                        Some(c) => at = m.start() + c.len_utf8(),
                        None => break,
                    }
                    continue;
                }
                expected.push(base + m.start()..base + m.end());
                at = m.end();
            }
        }
        prop_assert_eq!(hits_abs(&f, &doc), expected);
    }

    #[test]
    fn a_literal_equals_its_escaped_regex(
        text in text_strategy(),
        needle in needle_strategy(),
        match_case in any::<bool>(),
        whole_word in any::<bool>(),
        extra in prop::sample::select(&["", "-", "+="][..]),
    ) {
        let doc = editable(&text, None);
        let plain = FindQuery { text: needle.clone(), regex: false, match_case, whole_word };
        let regex = FindQuery { text: regex::escape(&needle), regex: true, match_case, whole_word };
        let a = Finder::new(&plain, extra, &RegexLimits::default()).unwrap();
        let b = Finder::new(&regex, extra, &RegexLimits::default()).unwrap();
        prop_assert_eq!(hits_abs(&a, &doc), hits_abs(&b, &doc));
    }

    #[test]
    fn next_walks_every_hit_both_ways(
        text in text_strategy(),
        needle in needle_strategy(),
        match_case in any::<bool>(),
        cpl in cpl_strategy(),
        probe in 0usize..200,
    ) {
        let doc = editable(&text, cpl);
        let q = FindQuery { text: needle, regex: false, match_case, whole_word: false };
        let f = Finder::new(&q, "", &RegexLimits::default()).unwrap();
        let all = f.all(&doc, u64::MAX, &Ctl::detached()).unwrap();
        let mut walked = Vec::new();
        let mut at = TextPos::ZERO;
        while let Some(h) = f.next(&doc, at, Dir::Forward, false).unwrap() {
            at = h.range.end;
            walked.push(h);
        }
        prop_assert_eq!(&walked, &all);
        let end = TextPos::new(doc.line_count(), 0);
        let mut back = Vec::new();
        let mut at = end;
        while let Some(h) = f.next(&doc, at, Dir::Backward, false).unwrap() {
            at = h.range.start;
            back.push(h);
        }
        back.reverse();
        prop_assert_eq!(&back, &all);
        // From a random position (as an absolute offset): the first hit at or after it, the last before it.
        let p = probe.min(text.len());
        let line = (0..doc.line_count())
            .rev()
            .find(|&l| abs(&doc, TextPos::new(l, 0)) <= p);
        if let Some(line) = line {
            let start = abs(&doc, TextPos::new(line, 0));
            let len = doc.line_len(hikasa_text::LineIdx(line)) as usize;
            if p - start <= len && text.is_char_boundary(p) {
                let from = TextPos::new(line, (p - start) as u32);
                let fwd = f.next(&doc, from, Dir::Forward, false).unwrap();
                let want = all.iter().find(|h| abs(&doc, h.range.start) >= p).cloned();
                prop_assert_eq!(fwd, want);
                let bwd = f.next(&doc, from, Dir::Backward, false).unwrap();
                let want = all.iter().rev().find(|h| abs(&doc, h.range.start) < p).cloned();
                prop_assert_eq!(bwd, want);
            }
        }
    }

    #[test]
    fn replace_then_undo_is_the_identity(
        text in text_strategy(),
        needle in needle_strategy(),
        with in prop::sample::select(&["", "x", "XY", "é", "$$", "a\nb"][..]),
        match_case in any::<bool>(),
        cpl in cpl_strategy(),
    ) {
        let mut doc = editable(&text, cpl);
        let all_text = |d: &EditableDocument| d.text(TextPos::ZERO..TextPos::new(d.line_count(), 0));
        let before = all_text(&doc);
        prop_assert_eq!(&before, &text);
        let q = FindQuery { text: needle, regex: false, match_case, whole_word: false };
        let f = Finder::new(&q, "", &RegexLimits::default()).unwrap();
        let hits = hits_abs(&f, &doc);
        let r = Replacement::new(with, &f).unwrap();
        let (req, count) = replace_all(&doc, &f, &r, None, &Ctl::detached()).unwrap();
        prop_assert_eq!(count as usize, hits.len());
        let Some(req) = req else {
            prop_assert!(hits.is_empty());
            return Ok(());
        };
        // The expected text: each hit replaced; a line break in the replacement becomes the insert terminator.
        let eol = doc.insert_eol().as_text().to_owned();
        let mut expected = String::new();
        let mut at = 0;
        for h in &hits {
            expected.push_str(&text[at..h.start]);
            expected.push_str(&with.replace('\n', &eol));
            at = h.end;
        }
        expected.push_str(&text[at..]);
        let steps = doc.undo_steps();
        doc.apply(&req, GroupHint::None).unwrap();
        prop_assert_eq!(doc.undo_steps(), steps + 1);
        let after = all_text(&doc);
        // A lone CR before an inserted LF joins into CRLF exactly as a load reads it: compare the bytes.
        prop_assert_eq!(after.as_bytes(), expected.as_bytes());
        doc.undo().unwrap();
        prop_assert_eq!(all_text(&doc), before);
        prop_assert!(!doc.is_modified());
    }
}
