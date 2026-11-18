//! A10.3 (D-262): undo and redo. P-EDIT-1 — random scripts of every `EditRequest` kind, `copy_rows`, line-ending
//! conversions, the conversions of `transform` (§1.10.15, whole documents and line ranges) and `SetEncoding`, then
//! a full undo: text, terminators, encoding, marks, bookmarks and
//! `is_modified()` equal the start; redo replays to the end state; an edit after an undo drops the redo branch.
//! The coalescing table with explicit `at_ms`.

mod common;

use std::sync::Arc;

use common::config;
use hikasa_core::{Ctl, Side};
use hikasa_text::edit::{
    EditOptions, EditRequest, Editability, EditableDocument, GroupHint, Selection, TextPos,
    copy_rows,
};
use hikasa_text::transform::{Transform, transform};
use hikasa_text::{
    CompareInput, CompiledFormat, LineEnding, LineIdx, TextDocument, TextRules, compare,
};
use proptest::prelude::*;

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

const WORDS: [&str; 8] = ["alpha", "beta", " ", "\t", "é", "x", "{", "}"];
const BREAKS: [&str; 3] = ["\n", "\r\n", "\r"];

fn text(rng: &mut Rng, lines: u64) -> String {
    let mut out = String::new();
    for k in 0..lines {
        for _ in 0..rng.below(4) {
            out.push_str(WORDS[rng.below(8) as usize]);
        }
        if k + 1 < lines || rng.below(2) == 0 {
            out.push_str(BREAKS[rng.below(3) as usize]);
        }
    }
    out
}

fn position(rng: &mut Rng, doc: &EditableDocument) -> TextPos {
    let line = rng.below(u64::from(doc.position_lines())) as u32;
    let text = doc.line(LineIdx(line)).into_owned();
    let mut byte = rng.below(text.len() as u64 + 1) as usize;
    while !text.is_char_boundary(byte) {
        byte -= 1;
    }
    TextPos::new(line, byte as u32)
}

/// Everything P-EDIT-1 compares.
#[derive(Debug, PartialEq)]
struct State {
    text: String,
    eols: Vec<LineEnding>,
    marks: Vec<(u32, Option<u32>)>,
    bookmarks: [Option<LineIdx>; 10],
    encoding: String,
    bom: bool,
    insert_eol: LineEnding,
    modified: bool,
}

fn state(doc: &EditableDocument) -> State {
    let d = doc.document();
    State {
        text: d.rope().to_string(),
        eols: (0..d.line_count())
            .map(|l| d.line_ending(LineIdx(l)))
            .collect(),
        marks: (0..d.line_count())
            .map(|l| {
                let m = d.marks(LineIdx(l));
                (m.generation, m.orig.map(|o| o.0))
            })
            .collect(),
        bookmarks: doc.bookmarks(),
        encoding: doc.encoding().0.name().to_owned(),
        bom: doc.encoding().1,
        insert_eol: doc.insert_eol(),
        modified: doc.is_modified(),
    }
}

/// A random request for `doc` (every kind), sometimes a copy of rows from `other`.
fn request(
    rng: &mut Rng,
    doc: &EditableDocument,
    other: &Arc<TextDocument>,
) -> Option<EditRequest> {
    match rng.below(11) {
        0..=3 => {
            let a = position(rng, doc);
            let b = if rng.below(2) == 0 {
                a
            } else {
                position(rng, doc)
            };
            let mut t = String::new();
            for _ in 0..rng.below(3) {
                t.push_str(WORDS[rng.below(8) as usize]);
                if rng.below(3) == 0 {
                    t.push_str(BREAKS[rng.below(3) as usize]);
                }
            }
            Some(EditRequest::Replace {
                range: a..b,
                text: t,
                keep_eols: rng.below(2) == 0,
            })
        }
        4 => {
            let mut ps: Vec<TextPos> = (0..2 * (1 + rng.below(3)))
                .map(|_| position(rng, doc))
                .collect();
            ps.sort();
            let edits = ps
                .chunks(2)
                .map(|p| (p[0]..p[1], WORDS[rng.below(8) as usize].to_owned()))
                .collect();
            Some(EditRequest::Batch {
                edits,
                keep_eols: false,
            })
        }
        5 if doc.line_count() > 0 => {
            let n = doc.line_count();
            let s = rng.below(u64::from(n)) as u32;
            let e = if rng.below(2) == 0 { n } else { s + 1 };
            let s = if rng.below(2) == 0 { 0 } else { s };
            let eol = [LineEnding::Lf, LineEnding::CrLf, LineEnding::Cr][rng.below(3) as usize];
            Some(EditRequest::SetEols { lines: s..e, eol })
        }
        9 | 10 => {
            // A conversion (TC-063) of every line or of a range.
            let t = match rng.below(6) {
                0 => Transform::TrimTrailing,
                1 => Transform::LeadingSpacesToTabs,
                2 => Transform::TabsToSpaces,
                3 => Transform::LineEndings(LineEnding::Lf),
                4 => Transform::LineEndings(LineEnding::CrLf),
                _ => Transform::LineEndings(LineEnding::Cr),
            };
            let n = doc.line_count();
            let lines = if rng.below(2) == 0 || n == 0 {
                None
            } else {
                let a = rng.below(u64::from(n)) as u32;
                Some(a..a + 1 + rng.below(u64::from(n - a)) as u32)
            };
            transform(doc, t, lines, [1u8, 2, 4, 8][rng.below(4) as usize]).request
        }
        6 => Some(EditRequest::SetEncoding {
            encoding: if rng.below(2) == 0 {
                hikasa_text::TextEncoding::UTF_16LE
            } else {
                hikasa_text::TextEncoding::UTF_8
            },
            bom: rng.below(2) == 0,
        }),
        _ => {
            // Copy rows of a comparison of `other` (left) with the document (right) to the right.
            let cmp = compare(
                CompareInput {
                    left: other.clone(),
                    right: doc.snapshot(),
                    format: Arc::new(CompiledFormat::everything_else()),
                    rules: TextRules::default(),
                    bytes_identical: false,
                    force_tokenize: false,
                    anchors: Vec::new(),
                    right_format: None,
                    ignored: Default::default(),
                },
                None,
                &Ctl::detached(),
                None,
            )
            .ok()?;
            let n = cmp.row_count();
            if n == 0 {
                return None;
            }
            let a = rng.below(u64::from(n)) as u32;
            let b = a + 1 + rng.below(u64::from(n - a).min(5)) as u32;
            copy_rows(&cmp, a..b, Side::Left, doc.insert_eol()).map(|c| c.request)
        }
    }
}

fn run(seed: u64, steps: u32) {
    let mut rng = Rng(seed | 1);
    let n = 1 + rng.below(30);
    let base = text(&mut rng, n);
    let n = 1 + rng.below(30);
    let other = Arc::new(TextDocument::from_text(&text(&mut rng, n)));
    let mut doc = EditableDocument::new(
        Arc::new(TextDocument::from_text(&base)),
        EditOptions::default(),
        Editability::EDITABLE,
    );
    for n in 0..4u8 {
        if doc.line_count() > 0 {
            let line = rng.below(u64::from(doc.line_count())) as u32;
            doc.toggle_bookmark(n * 2, LineIdx(line));
        }
    }
    let start = state(&doc);
    let mut at_ms = 0u64;
    let mut applied = 0;
    for _ in 0..steps {
        let Some(req) = request(&mut rng, &doc, &other) else {
            continue;
        };
        at_ms += rng.below(1500);
        let group = match rng.below(4) {
            0 => GroupHint::Typing { at_ms },
            1 => GroupHint::Group(rng.below(2)),
            _ => GroupHint::None,
        };
        if doc.apply(&req, group).is_ok() {
            applied += 1;
        }
        doc.document().check_line_table(None).unwrap();
    }
    let end = state(&doc);
    let mut undone = 0;
    while doc.undo().is_some() {
        undone += 1;
        doc.document().check_line_table(None).unwrap();
    }
    assert!(undone <= applied);
    assert_eq!(state(&doc), start, "seed {seed}: full undo");
    while doc.redo().is_some() {}
    let mut redone = state(&doc);
    // Redo restores the end state (the marks it brings back are those the undo took away).
    redone.modified = end.modified;
    assert_eq!(redone, end, "seed {seed}: full redo");
    // An edit after an undo drops the redo branch.
    if doc.undo().is_some() {
        assert!(doc.can_redo());
        doc.apply(
            &EditRequest::Replace {
                range: TextPos::ZERO..TextPos::ZERO,
                text: "z".into(),
                keep_eols: false,
            },
            GroupHint::None,
        )
        .unwrap();
        assert!(!doc.can_redo());
    }
}

proptest! {
    #![proptest_config(config("tests/proptest-regressions/props_edit.txt"))]

    /// P-EDIT-1 (A10.3).
    #[test]
    fn p_edit_1_edit_then_undo_is_the_identity(seed in any::<u64>()) {
        run(seed, 25);
    }
}

fn typed(doc: &mut EditableDocument, at: TextPos, c: &str, at_ms: u64) -> u64 {
    doc.apply(
        &EditRequest::Replace {
            range: at..at,
            text: c.into(),
            keep_eols: false,
        },
        GroupHint::Typing { at_ms },
    )
    .unwrap()
    .step
    .0
}

fn fresh(text: &str) -> EditableDocument {
    EditableDocument::new(
        Arc::new(TextDocument::from_text(text)),
        EditOptions::default(),
        Editability::EDITABLE,
    )
}

/// §1.10.4: a step never merges across the save point. Typing on after a save (within the second, same word,
/// caret in place) starts a new step, so one undo goes back exactly to the saved text (`is_modified` false), for
/// typing and for group ids alike.
#[test]
fn typing_never_merges_across_the_save_point() {
    let mut doc = fresh("\n");
    let a = typed(&mut doc, TextPos::new(0, 0), "a", 0);
    let b = typed(&mut doc, TextPos::new(0, 1), "b", 10);
    assert_eq!(a, b);
    doc.saved(hikasa_text::LoadStamp {
        size: 0,
        mtime: None,
        crc32: 0,
    });
    assert!(!doc.is_modified());
    let c = typed(&mut doc, TextPos::new(0, 2), "c", 20);
    assert_ne!(b, c, "a step merged across the save point");
    let d = typed(&mut doc, TextPos::new(0, 3), "d", 30);
    assert_eq!(c, d);
    assert!(doc.is_modified());
    doc.undo();
    assert_eq!(doc.line(LineIdx(0)), "ab");
    assert!(!doc.is_modified());
    doc.redo();
    assert_eq!(doc.line(LineIdx(0)), "abcd");
    // The same with a group id spanning the save.
    let mut doc = fresh("x\n");
    let g = |doc: &mut EditableDocument| {
        doc.apply(
            &EditRequest::Replace {
                range: TextPos::ZERO..TextPos::ZERO,
                text: "y".into(),
                keep_eols: false,
            },
            GroupHint::Group(3),
        )
        .unwrap()
        .step
    };
    let s1 = g(&mut doc);
    doc.saved(hikasa_text::LoadStamp {
        size: 0,
        mtime: None,
        crc32: 0,
    });
    let s2 = g(&mut doc);
    assert_ne!(s1, s2);
    doc.undo();
    assert_eq!(doc.line(LineIdx(0)), "yx");
    assert!(!doc.is_modified());
}

/// The coalescing table of §1.10.4 (explicit `at_ms`).
#[test]
fn typing_coalesces_within_a_second_and_a_word() {
    // Merge: consecutive characters within 1,000 ms where the caret is.
    let mut doc = fresh("\n");
    let a = typed(&mut doc, TextPos::new(0, 0), "a", 0);
    let b = typed(&mut doc, TextPos::new(0, 1), "b", 999);
    assert_eq!(a, b);
    // Break at 1,000 ms.
    let c = typed(&mut doc, TextPos::new(0, 2), "c", 1999);
    assert_ne!(b, c);
    // Break at a word start (a word character after a non-word one).
    let sp = typed(&mut doc, TextPos::new(0, 3), " ", 2000);
    assert_eq!(sp, c);
    let d = typed(&mut doc, TextPos::new(0, 4), "d", 2001);
    assert_ne!(d, sp);
    // Break at a caret jump (a non-word character, so only the jump can break it).
    let e = typed(&mut doc, TextPos::new(0, 0), "-", 2002);
    assert_ne!(e, d);
    assert_eq!(doc.line(LineIdx(0)), "-abc d");
    doc.undo();
    assert_eq!(doc.line(LineIdx(0)), "abc d");
    doc.undo();
    assert_eq!(doc.line(LineIdx(0)), "abc ");
    doc.undo();
    assert_eq!(doc.line(LineIdx(0)), "ab");
    // Group: every request of one id merges until another id.
    let mut doc = fresh("x\n");
    let g = |doc: &mut EditableDocument, id: u64| {
        doc.apply(
            &EditRequest::Replace {
                range: TextPos::ZERO..TextPos::ZERO,
                text: "y".into(),
                keep_eols: false,
            },
            GroupHint::Group(id),
        )
        .unwrap()
        .step
    };
    let s1 = g(&mut doc, 7);
    let s2 = g(&mut doc, 7);
    let s3 = g(&mut doc, 8);
    assert_eq!(s1, s2);
    assert_ne!(s2, s3);
    assert_eq!(doc.line(LineIdx(0)), "yyyx");
    doc.undo();
    assert_eq!(doc.line(LineIdx(0)), "yyx");
    doc.undo();
    assert_eq!(doc.line(LineIdx(0)), "x");
    assert!(!doc.is_modified());
    // Deletes merge with deletes, not with inserts.
    let mut doc = fresh("abcd\n");
    let del = |doc: &mut EditableDocument, a: u32, at_ms| {
        doc.apply(
            &EditRequest::Replace {
                range: TextPos::new(0, a - 1)..TextPos::new(0, a),
                text: String::new(),
                keep_eols: false,
            },
            GroupHint::Typing { at_ms },
        )
        .unwrap()
        .step
    };
    let x = del(&mut doc, 4, 0);
    let y = del(&mut doc, 3, 10);
    assert_eq!(x, y);
    let z = typed(&mut doc, TextPos::new(0, 2), "q", 20);
    assert_ne!(y.0, z);
}

/// The undo outcome's selection is the selection before the step; redo's the one after.
#[test]
fn undo_and_redo_restore_the_selections() {
    let mut doc = fresh("hello\n");
    let out = doc
        .apply(
            &EditRequest::Replace {
                range: TextPos::new(0, 1)..TextPos::new(0, 3),
                text: "EY".into(),
                keep_eols: false,
            },
            GroupHint::None,
        )
        .unwrap();
    assert_eq!(out.selection, Selection::caret(TextPos::new(0, 3)));
    let undo = doc.undo().unwrap();
    assert_eq!(
        undo.selection,
        Selection {
            anchor: TextPos::new(0, 1),
            caret: TextPos::new(0, 3)
        }
    );
    let redo = doc.redo().unwrap();
    assert_eq!(redo.selection, out.selection);
}
