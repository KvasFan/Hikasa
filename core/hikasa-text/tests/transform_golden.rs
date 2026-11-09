//! A10.18 (D-279): conversions (TC-063, §1.10.15). Goldens of every conversion on
//! `tests/fixtures/convert/<case>/input.txt` (tabs and spaces at tab sizes 2, 4 and 8, trailing white space
//! with NBSP, mixed terminators, CPL-wrapped lines when `cpl.txt` gives a limit): each line of the result with
//! its terminator, the *Converted N lines* count and the insert terminator, in `expected.txt` (`HIKASA_BLESS=1`
//! rewrites it). Every conversion is checked to be one undo step whose undo restores the document exactly, and
//! to be idempotent. Line ranges, marks and the insert terminator have their own tests. P-CONVERT: on random
//! documents each conversion equals a line-by-line reference of §1.10.15's table, applied twice equals once,
//! and `TabsToSpaces` after `LeadingSpacesToTabs` restores indents made of spaces.

mod common;

use std::fmt::Write as _;
use std::fs;
use std::sync::Arc;

use common::{assert_golden, config, fixtures};
use hikasa_text::edit::{EditOptions, EditRequest, Editability, EditableDocument, GroupHint};
use hikasa_text::transform::{Transform, TransformOutcome, transform};
use hikasa_text::{LineEnding, LineIdx, LineMark, TextDocument};
use proptest::prelude::*;

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

fn text(doc: &EditableDocument) -> String {
    doc.document().rope().to_string()
}

/// Everything a conversion and its undo must restore.
#[derive(Debug, PartialEq)]
struct State {
    text: String,
    lines: Vec<(String, LineEnding, u32, Option<u32>)>,
    insert_eol: LineEnding,
    modified: bool,
}

fn state(doc: &EditableDocument) -> State {
    State {
        text: text(doc),
        lines: (0..doc.line_count())
            .map(|l| {
                let m = doc.marks(LineIdx(l));
                (
                    doc.line(LineIdx(l)).into_owned(),
                    doc.line_ending(LineIdx(l)),
                    m.generation,
                    m.orig.map(|o| o.0),
                )
            })
            .collect(),
        insert_eol: doc.insert_eol(),
        modified: doc.is_modified(),
    }
}

/// Applies `out` as one undo step, checks that its undo restores the document exactly and that redo brings the
/// result back; returns the converted document.
fn apply_checked(doc: &EditableDocument, out: &TransformOutcome) -> EditableDocument {
    let mut d = doc.clone();
    let Some(req) = &out.request else {
        return d;
    };
    let before = state(&d);
    let steps = d.undo_steps();
    d.apply(req, GroupHint::None).unwrap();
    d.document()
        .check_line_table(d.options().chars_per_line_limit)
        .unwrap();
    assert_eq!(d.undo_steps(), steps + 1, "one undo step");
    let after = state(&d);
    d.undo().unwrap();
    assert_eq!(state(&d), before, "undo restores the document");
    d.redo().unwrap();
    let mut redone = state(&d);
    redone.modified = after.modified;
    assert_eq!(redone.text, after.text);
    assert_eq!(redone.insert_eol, after.insert_eol);
    d
}

/// Every conversion of a golden case, with its name.
fn conversions() -> Vec<(String, Transform, u8)> {
    let mut out = vec![("trim_trailing".to_owned(), Transform::TrimTrailing, 4)];
    for ts in [2u8, 4, 8] {
        out.push((
            format!("leading_spaces_to_tabs tab size {ts}"),
            Transform::LeadingSpacesToTabs,
            ts,
        ));
        out.push((
            format!("tabs_to_spaces tab size {ts}"),
            Transform::TabsToSpaces,
            ts,
        ));
    }
    for e in [LineEnding::Lf, LineEnding::CrLf, LineEnding::Cr] {
        out.push((
            format!("line_endings {}", e.as_str()),
            Transform::LineEndings(e),
            4,
        ));
    }
    out
}

/// One line per document line: its content (JSON-escaped), terminator and mark.
fn render(out: &mut String, doc: &EditableDocument) {
    for l in 0..doc.line_count() {
        let mark = match doc.mark(LineIdx(l)) {
            LineMark::Original => "",
            LineMark::Edited => " *",
            LineMark::SavedEdit => " +",
        };
        writeln!(
            out,
            "{} {}{mark}",
            serde_json::to_string(&doc.line(LineIdx(l))).unwrap(),
            doc.line_ending(LineIdx(l)).as_str()
        )
        .unwrap();
    }
}

#[test]
fn goldens_of_every_conversion() {
    let root = fixtures("convert");
    let mut cases: Vec<_> = fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    cases.sort();
    assert!(cases.len() >= 5, "{cases:?}");
    for dir in cases {
        let input = String::from_utf8(fs::read(dir.join("input.txt")).unwrap()).unwrap();
        let cpl = fs::read_to_string(dir.join("cpl.txt"))
            .ok()
            .map(|s| s.trim().parse::<u32>().unwrap());
        let doc = editable(&input, cpl);
        let mut out = String::new();
        writeln!(out, "== input, insert_eol {}", doc.insert_eol().as_str()).unwrap();
        render(&mut out, &doc);
        for (name, t, ts) in conversions() {
            let outcome = transform(&doc, t, None, ts);
            let converted = apply_checked(&doc, &outcome);
            writeln!(
                out,
                "\n== {name}: converted {} lines, insert_eol {}",
                outcome.lines,
                converted.insert_eol().as_str()
            )
            .unwrap();
            render(&mut out, &converted);
            // Idempotent: a second conversion has nothing to do.
            assert_eq!(
                transform(&converted, t, None, ts),
                TransformOutcome {
                    request: None,
                    lines: 0
                },
                "{}: {name} twice",
                dir.display()
            );
            // The same conversion over an explicit range of every line.
            let n = doc.line_count();
            assert_eq!(transform(&doc, t, Some(0..n), ts), outcome, "{name}: 0..n");
        }
        assert_golden(&dir.join("expected.txt"), out.as_bytes());
    }
}

fn convert(
    doc: &EditableDocument,
    t: Transform,
    lines: Option<std::ops::Range<u32>>,
) -> (EditableDocument, u64) {
    let out = transform(doc, t, lines, 4);
    (apply_checked(doc, &out), out.lines)
}

#[test]
fn a_line_range_converts_the_lines_of_a_selection() {
    let doc = editable("    a  \n    b  \n    c  \n    d  \n", None);
    let (d, n) = convert(&doc, Transform::LeadingSpacesToTabs, Some(1..3));
    assert_eq!(n, 2);
    assert_eq!(text(&d), "    a  \n\tb  \n\tc  \n    d  \n");
    let (d, n) = convert(&doc, Transform::TrimTrailing, Some(3..9));
    assert_eq!(n, 1);
    assert_eq!(text(&d), "    a  \n    b  \n    c  \n    d\n");
    // Nothing in range, an empty range, a range past the end.
    assert_eq!(
        transform(&doc, Transform::TabsToSpaces, Some(0..4), 4).request,
        None
    );
    assert_eq!(
        transform(&doc, Transform::TrimTrailing, Some(2..2), 4).request,
        None
    );
    assert_eq!(
        transform(&doc, Transform::TrimTrailing, Some(7..9), 4).request,
        None
    );
    // Unchanged lines keep their marks; the converted ones are edited.
    let (d, _) = convert(&doc, Transform::TrimTrailing, Some(1..3));
    let marks: Vec<_> = (0..4).map(|l| d.mark(LineIdx(l))).collect();
    assert_eq!(
        marks,
        [
            LineMark::Original,
            LineMark::Edited,
            LineMark::Edited,
            LineMark::Original
        ]
    );
    // A conversion of some lines leaves the insert terminator.
    let doc = editable("a\r\nb\r\nc\r\n", None);
    let (d, n) = convert(&doc, Transform::LineEndings(LineEnding::Lf), Some(1..2));
    assert_eq!((text(&d).as_str(), n), ("a\r\nb\nc\r\n", 1));
    assert_eq!(d.insert_eol(), LineEnding::CrLf);
}

#[test]
fn a_range_inside_a_wrapped_line_converts_the_whole_physical_line() {
    // CPL 4: "        x  " is the pieces "    ", "    ", "x  ".
    let doc = editable("ab\n        x  \ncd\n", Some(4));
    assert_eq!(doc.line_count(), 5);
    let (d, n) = convert(&doc, Transform::TrimTrailing, Some(3..4));
    assert_eq!((text(&d).as_str(), n), ("ab\n        x\ncd\n", 1));
    let (d, n) = convert(&doc, Transform::LeadingSpacesToTabs, Some(1..2));
    assert_eq!((text(&d).as_str(), n), ("ab\n\t\tx  \ncd\n", 1));
    d.document().check_line_table(Some(4)).unwrap();
    // Tab stops count from the start of the physical line, not of a piece.
    let doc = editable("abcde\tf\n", Some(4));
    let (d, _) = convert(&doc, Transform::TabsToSpaces, Some(1..2));
    assert_eq!(text(&d), "abcde   f\n");
}

#[test]
fn line_endings_keep_the_last_line_and_set_the_insert_terminator() {
    let doc = editable("a\nb\r\nc", None);
    let (d, n) = convert(&doc, Transform::LineEndings(LineEnding::Cr), None);
    assert_eq!((text(&d).as_str(), n), ("a\rb\rc", 2));
    assert_eq!(d.insert_eol(), LineEnding::Cr);
    // No terminated line: the insert terminator alone, as one undo step, 0 lines.
    for input in ["", "only"] {
        let doc = editable(input, None);
        assert_eq!(doc.insert_eol(), LineEnding::Lf);
        let out = transform(&doc, Transform::LineEndings(LineEnding::CrLf), None, 4);
        assert_eq!(out.lines, 0);
        let mut d = apply_checked(&doc, &out);
        assert_eq!(
            (text(&d).as_str(), d.insert_eol()),
            (input, LineEnding::CrLf)
        );
        d.undo().unwrap();
        assert_eq!(d.insert_eol(), LineEnding::Lf);
        // Already the insert terminator: nothing to do.
        assert_eq!(
            transform(&doc, Transform::LineEndings(LineEnding::Lf), None, 4).request,
            None
        );
    }
    // Every terminator already the kind, the insert terminator not (a CRLF block pasted as is).
    let mut doc = editable("a\n", None);
    doc.apply(
        &EditRequest::Replace {
            range: hikasa_text::TextPos::ZERO..hikasa_text::TextPos::new(1, 0),
            text: "x\r\ny\r\n".into(),
            keep_eols: true,
        },
        GroupHint::None,
    )
    .unwrap();
    assert_eq!(doc.insert_eol(), LineEnding::Lf);
    let out = transform(&doc, Transform::LineEndings(LineEnding::CrLf), None, 4);
    assert_eq!(out.lines, 0);
    let d = apply_checked(&doc, &out);
    assert_eq!(d.insert_eol(), LineEnding::CrLf);
    // Not a terminator kind: nothing.
    for e in [LineEnding::None, LineEnding::Artificial] {
        assert_eq!(
            transform(&doc, Transform::LineEndings(e), None, 4).request,
            None
        );
    }
    // Lines whose terminator is already the kind keep their marks.
    let doc = editable("a\nb\r\nc\nd\r\n", None);
    let (d, n) = convert(&doc, Transform::LineEndings(LineEnding::Lf), None);
    assert_eq!(n, 2);
    let marks: Vec<_> = (0..4).map(|l| d.mark(LineIdx(l))).collect();
    assert_eq!(
        marks,
        [
            LineMark::Original,
            LineMark::Edited,
            LineMark::Original,
            LineMark::Edited
        ]
    );
}

#[test]
fn an_undo_after_a_save_marks_only_the_lines_it_changes() {
    let input: String = (0..300)
        .map(|k| {
            if k % 3 == 0 {
                format!("{k}  \n")
            } else {
                format!("{k}\n")
            }
        })
        .collect();
    let mut doc = editable(&input, None);
    let out = transform(&doc, Transform::TrimTrailing, None, 4);
    assert_eq!(out.lines, 100);
    doc.apply(&out.request.unwrap(), GroupHint::None).unwrap();
    doc.saved();
    doc.undo().unwrap();
    assert_eq!(text(&doc), input);
    for l in 0..300 {
        let want = if l % 3 == 0 {
            LineMark::Edited
        } else {
            LineMark::Original
        };
        assert_eq!(doc.mark(LineIdx(l)), want, "line {l}");
    }
    // Redo returns to the save point: the converted lines are saved edits again.
    doc.redo().unwrap();
    assert!(!doc.is_modified());
    for l in 0..300 {
        let want = if l % 3 == 0 {
            LineMark::SavedEdit
        } else {
            LineMark::Original
        };
        assert_eq!(doc.mark(LineIdx(l)), want, "line {l} after redo");
    }
}

#[test]
fn a_range_never_joins_a_line_into_crlf() {
    // "a" becoming CR before the empty LF lines: they become CR too (else "a\r\n" would be one line).
    let doc = editable("a\n\n\nb\n", None);
    let (d, n) = convert(&doc, Transform::LineEndings(LineEnding::Cr), Some(0..1));
    assert_eq!((text(&d).as_str(), n), ("a\r\r\rb\n", 3));
    assert_eq!(d.line_count(), 4);
    // An empty line becoming LF after a lone CR: the CR line becomes LF too.
    let doc = editable("a\r\r\nb\n", None);
    assert_eq!(doc.line_count(), 3);
    let (d, n) = convert(&doc, Transform::LineEndings(LineEnding::Lf), Some(1..2));
    assert_eq!((text(&d).as_str(), n), ("a\n\nb\n", 2));
    assert_eq!(d.line_count(), 3);
}

// ---------------------------------------------------------------------------------------------------------------
// P-CONVERT

/// Seeded xorshift64* (§6.1).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

const PIECES: [&str; 11] = [
    " ", " ", "  ", "\t", "a", "bc", "界", "e\u{301}", "\u{a0}", "\u{3000}", "x y",
];
const BREAKS: [&str; 4] = ["\n", "\r\n", "\r", "\n"];

fn document(rng: &mut Rng, tabs: bool) -> String {
    let lines = rng.below(25);
    let mut out = String::new();
    for k in 0..lines {
        for _ in 0..rng.below(7) {
            let p = PIECES[rng.below(PIECES.len() as u64) as usize];
            if tabs || p != "\t" {
                out.push_str(p);
            }
        }
        if k + 1 < lines || rng.below(2) == 0 {
            out.push_str(BREAKS[rng.below(4) as usize]);
        }
    }
    out
}

/// The physical lines of `text` as (content, terminator).
fn split(text: &str) -> Vec<(String, &'static str)> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' if chars.peek() == Some(&'\n') => {
                chars.next();
                out.push((std::mem::take(&mut cur), "\r\n"));
            }
            '\r' => out.push((std::mem::take(&mut cur), "\r")),
            '\n' => out.push((std::mem::take(&mut cur), "\n")),
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push((cur, ""));
    }
    out
}

fn col_width(c: char) -> usize {
    // Test-side display widths: the fixtures' wide and combining characters.
    match c {
        '界' | '\u{3000}' => 2,
        '\u{301}' => 0,
        _ => 1,
    }
}

/// §1.10.15's table, line by line, written independently of the crate.
fn reference(line: &str, t: Transform, ts: usize) -> String {
    match t {
        Transform::TrimTrailing => line.trim_end_matches(char::is_whitespace).to_owned(),
        Transform::TabsToSpaces => {
            let mut out = String::new();
            let mut col = 0;
            for c in line.chars() {
                if c == '\t' {
                    let n = ts - col % ts;
                    out.push_str(&" ".repeat(n));
                    col += n;
                } else {
                    col += col_width(c);
                    out.push(c);
                }
            }
            out
        }
        Transform::LeadingSpacesToTabs => {
            let lead = line.len() - line.trim_start_matches([' ', '\t']).len();
            let mut out = String::new();
            let mut col = 0;
            let mut spaces = String::new();
            for c in line[..lead].chars() {
                if c == '\t' {
                    out.push_str(&spaces);
                    spaces.clear();
                    out.push('\t');
                    col = (col / ts + 1) * ts;
                } else {
                    spaces.push(' ');
                    col += 1;
                    if col.is_multiple_of(ts) {
                        spaces.clear();
                        out.push('\t');
                    }
                }
            }
            out.push_str(&spaces);
            out.push_str(&line[lead..]);
            out
        }
        Transform::LineEndings(_) => line.to_owned(),
    }
}

/// The reference result of `t` over the whole text and its count.
fn reference_doc(text: &str, t: Transform, ts: usize) -> (String, u64) {
    reference_range(text, t, ts, 0..u32::MAX)
}

/// The reference result of `t` over the physical lines `range` (no CPL) and its count. A range conversion of
/// line endings also takes the neighbours that would join a converted line into CRLF.
fn reference_range(
    text: &str,
    t: Transform,
    ts: usize,
    range: std::ops::Range<u32>,
) -> (String, u64) {
    let lines = split(text);
    let n = lines.len() as u32;
    let (mut a, mut b) = (range.start.min(n), range.end.min(n));
    match t {
        Transform::LineEndings(LineEnding::Cr) => {
            while b < n && lines[b as usize].0.is_empty() && lines[b as usize].1 == "\n" {
                b += 1;
            }
        }
        Transform::LineEndings(LineEnding::Lf) => {
            while a > 0
                && a < n
                && lines[a as usize].0.is_empty()
                && !lines[a as usize].1.is_empty()
                && lines[a as usize - 1].1 == "\r"
            {
                a -= 1;
            }
        }
        _ => {}
    }
    let mut out = String::new();
    let mut count = 0;
    for (k, (line, eol)) in lines.into_iter().enumerate() {
        let inside = (a..b).contains(&(k as u32));
        let new = if inside {
            reference(&line, t, ts)
        } else {
            line.clone()
        };
        let new_eol = match t {
            Transform::LineEndings(e) if inside && !eol.is_empty() => e.as_text(),
            _ => eol,
        };
        if new != line || new_eol != eol {
            count += 1;
        }
        out.push_str(&new);
        out.push_str(new_eol);
    }
    (out, count)
}

fn p_convert(seed: u64) {
    let mut rng = Rng(seed | 1);
    let input = document(&mut rng, true);
    let cpl = [None, None, Some(3), Some(8)][rng.below(4) as usize];
    let doc = editable(&input, cpl);
    let ts = [1u8, 2, 3, 4, 8][rng.below(5) as usize];
    let all = [
        Transform::TrimTrailing,
        Transform::LeadingSpacesToTabs,
        Transform::TabsToSpaces,
        Transform::LineEndings(LineEnding::Lf),
        Transform::LineEndings(LineEnding::CrLf),
        Transform::LineEndings(LineEnding::Cr),
    ];
    for t in all {
        let out = transform(&doc, t, None, ts);
        let once = apply_checked(&doc, &out);
        once.document().check_line_table(cpl).unwrap();
        // The reference of the normative table.
        let (want, n) = reference_doc(&input, t, usize::from(ts));
        assert_eq!(text(&once), want, "seed {seed}: {t:?} ts {ts} cpl {cpl:?}");
        assert_eq!(out.lines, n, "seed {seed}: {t:?} count");
        if let Transform::LineEndings(e) = t {
            assert_eq!(once.insert_eol(), e, "seed {seed}");
        }
        // Applied twice equals once.
        let again = transform(&once, t, None, ts);
        assert_eq!(again.request, None, "seed {seed}: {t:?} twice");
        // A random range (a selection's lines) against the reference over those lines.
        let n_lines = doc.line_count();
        if n_lines > 0 && cpl.is_none() {
            let a = rng.below(u64::from(n_lines)) as u32;
            let b = a + 1 + rng.below(u64::from(n_lines - a)) as u32;
            let part = transform(&doc, t, Some(a..b), ts);
            let converted = apply_checked(&doc, &part);
            let (want, n) = reference_range(&input, t, usize::from(ts), a..b);
            assert_eq!(text(&converted), want, "seed {seed}: {t:?} {a}..{b}");
            assert_eq!(part.lines, n, "seed {seed}: {t:?} {a}..{b} count");
            if matches!(t, Transform::LineEndings(_)) {
                assert_eq!(converted.line_count(), n_lines, "seed {seed}: no line lost");
            }
            // Exactly the changed lines are marked edited.
            if converted.line_count() == n_lines {
                for l in 0..n_lines {
                    let same = converted.line(LineIdx(l)) == doc.line(LineIdx(l))
                        && converted.line_ending(LineIdx(l)) == doc.line_ending(LineIdx(l));
                    assert_eq!(
                        converted.mark(LineIdx(l)) == LineMark::Original,
                        same,
                        "seed {seed}: {t:?} {a}..{b} mark of line {l}"
                    );
                }
            }
            // Applied twice equals once on the same lines.
            if converted.line_count() == n_lines {
                assert_eq!(
                    transform(&converted, t, Some(a..b), ts).request,
                    None,
                    "seed {seed}: {t:?} {a}..{b} twice"
                );
            }
        }
    }
    // TabsToSpaces after LeadingSpacesToTabs restores indents made of spaces.
    let plain = document(&mut rng, false);
    let doc = editable(&plain, cpl);
    let tabs = apply_checked(
        &doc,
        &transform(&doc, Transform::LeadingSpacesToTabs, None, ts),
    );
    let back = apply_checked(&tabs, &transform(&tabs, Transform::TabsToSpaces, None, ts));
    assert_eq!(text(&back), plain, "seed {seed}: ts {ts}");
    // With tabs elsewhere: every line whose leading blanks are spaces gets them back.
    doc_with_tabs(&mut rng, cpl, ts);
}

/// A document with tabs: leading blanks of space-only indents come back after the two conversions.
fn doc_with_tabs(rng: &mut Rng, cpl: Option<u32>, ts: u8) {
    let input = document(rng, true);
    let doc = editable(&input, cpl);
    let tabs = apply_checked(
        &doc,
        &transform(&doc, Transform::LeadingSpacesToTabs, None, ts),
    );
    let back = apply_checked(&tabs, &transform(&tabs, Transform::TabsToSpaces, None, ts));
    let lead = |s: &str| s.len() - s.trim_start_matches([' ', '\t']).len();
    for ((before, _), (after, _)) in split(&input).iter().zip(split(&text(&back)).iter()) {
        let indent = &before[..lead(before)];
        if !indent.contains('\t') {
            assert_eq!(&after[..lead(after)], indent, "{input:?}");
        }
    }
}

proptest! {
    #![proptest_config(config("tests/proptest-regressions/transform_golden.txt"))]

    /// P-CONVERT (A10.18).
    #[test]
    fn p_convert_conversions_follow_the_table_and_are_idempotent(seed in any::<u64>()) {
        p_convert(seed);
    }
}
