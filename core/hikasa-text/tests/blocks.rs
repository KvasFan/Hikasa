//! A10.1 (D-260): the block line table. P-BLOCK — random documents (mixed terminators, CPL wrapping, multi-byte
//! text, empty lines, a last line without terminator) and random edit scripts: after every edit the block table
//! equals a fresh build of the text (starts, terminators, physical lines, artificial breaks), blocks stay within
//! 128…1,024 lines (the last may be shorter), `uniform` is true exactly when it may be, and a snapshot taken
//! before an edit is unchanged after it. The memory table of §1.10.2 by a counting allocator.

mod common;

use std::sync::Arc;

use common::config;
use hikasa_text::edit::{
    EditOptions, EditRequest, Editability, EditableDocument, GroupHint, TextPos,
};
use hikasa_text::{LineEnding, LineIdx, TextDocument};
use proptest::prelude::*;

/// Seeded xorshift64* (§6.1).
pub struct Rng(u64);

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

const PIECES: [&str; 8] = ["a", "bc", "é", "界", "e\u{301}", " ", "\t", "xyz"];
const BREAKS: [&str; 4] = ["\n", "\r\n", "\r", "\n"];

/// A random document text of about `lines` lines.
fn document(rng: &mut Rng, lines: u64) -> String {
    let mut text = String::new();
    for k in 0..lines {
        for _ in 0..rng.below(6) {
            text.push_str(PIECES[rng.below(PIECES.len() as u64) as usize]);
        }
        if k + 1 < lines || rng.below(2) == 0 {
            text.push_str(BREAKS[rng.below(4) as usize]);
        }
    }
    text
}

/// A random insert text (0–3 breaks of every kind, pieces between).
fn insert_text(rng: &mut Rng) -> String {
    let mut text = String::new();
    let lines = if rng.below(10) == 0 {
        rng.below(1500)
    } else {
        rng.below(3)
    };
    for k in 0..=lines {
        for _ in 0..rng.below(4) {
            text.push_str(PIECES[rng.below(PIECES.len() as u64) as usize]);
        }
        if k < lines {
            text.push_str(BREAKS[rng.below(4) as usize]);
        }
    }
    text
}

/// A random valid position.
pub fn position(rng: &mut Rng, doc: &EditableDocument) -> TextPos {
    let line = rng.below(u64::from(doc.position_lines())) as u32;
    let text = doc.line(LineIdx(line)).into_owned();
    let mut byte = rng.below(text.len() as u64 + 1) as usize;
    while !text.is_char_boundary(byte) {
        byte -= 1;
    }
    TextPos::new(line, byte as u32)
}

/// The text with every break of `insert` replaced by `eol` (the model of §1.10.3's break rule).
fn normalized(insert: &str, eol: LineEnding) -> String {
    insert
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', eol.as_text())
}

fn run_script(seed: u64, cpl: Option<u32>, lines: u64, edits: u32) {
    let mut rng = Rng(seed | 1);
    let text = document(&mut rng, lines);
    let doc = TextDocument::from_text(&text);
    let mut doc = EditableDocument::new(
        Arc::new(doc),
        EditOptions {
            chars_per_line_limit: cpl,
            ..EditOptions::default()
        },
        Editability::EDITABLE,
    );
    if cpl.is_some() {
        // `from_text` does not wrap: re-wrap through an edit of everything (a fresh build wraps the same way).
        let all = doc.text(TextPos::ZERO..TextPos::new(u32::MAX, u32::MAX));
        let end = doc.clamp(TextPos::new(u32::MAX, u32::MAX));
        doc.apply(
            &EditRequest::Replace {
                range: TextPos::ZERO..end,
                text: all,
                keep_eols: true,
            },
            GroupHint::None,
        )
        .unwrap();
    }
    doc.document().check_line_table(cpl).unwrap();
    for step in 0..edits {
        let before = doc.snapshot();
        let before_text = before.rope().to_string();
        let before_check = before.check_line_table(cpl);
        let kind = rng.below(10);
        let req = if kind < 8 {
            let a = position(&mut rng, &doc);
            let b = if rng.below(3) == 0 {
                a
            } else {
                position(&mut rng, &doc)
            };
            EditRequest::Replace {
                range: a..b,
                text: insert_text(&mut rng),
                keep_eols: rng.below(2) == 0,
            }
        } else if kind == 8 && doc.line_count() > 0 {
            let n = doc.line_count();
            let s = rng.below(u64::from(n)) as u32;
            let e = s + 1 + rng.below(u64::from(n - s)) as u32;
            let eol = [LineEnding::Lf, LineEnding::CrLf, LineEnding::Cr][rng.below(3) as usize];
            EditRequest::SetEols { lines: s..e, eol }
        } else {
            continue;
        };
        let insert_eol = doc.insert_eol();
        let out = doc.apply(&req, GroupHint::None);
        let after = doc.document();
        after
            .check_line_table(cpl)
            .unwrap_or_else(|e| panic!("seed {seed} step {step} {req:?}: {e}"));
        if let (
            Ok(_),
            EditRequest::Replace {
                range,
                text,
                keep_eols,
            },
        ) = (&out, &req)
        {
            // The text is the old text with the range replaced (breaks normalized unless kept).
            let (a, b) = if range.start <= range.end {
                (range.start, range.end)
            } else {
                (range.end, range.start)
            };
            let start_of = |p: TextPos| -> usize {
                let off = before
                    .line_byte_range(p.line)
                    .map_or(before_text.len(), |r| r.start);
                off + p.byte as usize
            };
            let inserted = if *keep_eols {
                text.clone()
            } else {
                normalized(text, insert_eol)
            };
            let expected = format!(
                "{}{}{}",
                &before_text[..start_of(a)],
                inserted,
                &before_text[start_of(b)..]
            );
            assert_eq!(
                after.rope().to_string(),
                expected,
                "seed {seed} step {step}"
            );
        }
        // The snapshot taken before is unchanged.
        assert_eq!(before.rope().to_string(), before_text);
        assert_eq!(before.check_line_table(cpl), before_check);
    }
    // Untouched blocks are shared between a snapshot and the document.
    let snap = doc.snapshot();
    let (_, _, a) = snap.block_layout();
    let (_, _, b) = doc.document().block_layout();
    assert_eq!(a, b);
}

proptest! {
    #![proptest_config(config("tests/proptest-regressions/blocks.txt"))]

    /// P-BLOCK (A10.1).
    #[test]
    fn p_block_the_table_equals_a_fresh_build_after_every_edit(
        seed in any::<u64>(),
        cpl in prop_oneof![Just(None), (1u32..8).prop_map(Some)],
        lines in prop_oneof![0u64..20, 500u64..3000],
    ) {
        run_script(seed, cpl, lines, 12);
    }
}

/// An edit in one block shares every other block with the snapshot taken before it; a loaded document is
/// uniform (512-line blocks) and an edit that changes a block's line count makes it non-uniform.
#[test]
fn snapshots_share_untouched_blocks() {
    let text: String = (0..5000).map(|k| format!("line {k}\n")).collect();
    let mut doc = EditableDocument::new(
        Arc::new(TextDocument::from_text(&text)),
        EditOptions::default(),
        Editability::EDITABLE,
    );
    let (sizes, uniform, _) = doc.document().block_layout();
    assert!(uniform);
    assert_eq!(sizes.len(), 10);
    assert!(sizes[..9].iter().all(|&s| s == 512));
    let before = doc.snapshot();
    let at = TextPos::new(2000, 0);
    doc.apply(
        &EditRequest::Replace {
            range: at..at,
            text: "new\n".into(),
            keep_eols: false,
        },
        GroupHint::None,
    )
    .unwrap();
    let (_, uniform, after) = doc.document().block_layout();
    let (_, _, old) = before.block_layout();
    assert!(!uniform);
    let shared = after.iter().filter(|p| old.contains(p)).count();
    assert_eq!(shared, 9, "every block but the edited one is shared");
    assert_eq!(doc.line(LineIdx(2000)), "new");
    assert_eq!(before.line(LineIdx(2000)), "line 2000");
}

/// Splits and merges: a paste of 3,000 lines into one block splits it into blocks of at most 1,024 lines; removing
/// most of a block merges the rest with a neighbour.
#[test]
fn blocks_split_and_merge() {
    let text: String = (0..2048).map(|k| format!("{k}\n")).collect();
    let mut doc = EditableDocument::new(
        Arc::new(TextDocument::from_text(&text)),
        EditOptions::default(),
        Editability::EDITABLE,
    );
    let paste: String = (0..3000).map(|k| format!("p{k}\n")).collect();
    let at = TextPos::new(100, 0);
    doc.apply(
        &EditRequest::Replace {
            range: at..at,
            text: paste,
            keep_eols: false,
        },
        GroupHint::None,
    )
    .unwrap();
    doc.document().check_line_table(None).unwrap();
    let (sizes, _, _) = doc.document().block_layout();
    assert!(
        sizes.iter().all(|&s| (128..=1024).contains(&s)),
        "{sizes:?}"
    );
    doc.apply(
        &EditRequest::Replace {
            range: TextPos::new(10, 0)..TextPos::new(3500, 0),
            text: String::new(),
            keep_eols: false,
        },
        GroupHint::None,
    )
    .unwrap();
    doc.document().check_line_table(None).unwrap();
    let (sizes, _, _) = doc.document().block_layout();
    let last = sizes.len() - 1;
    assert!(
        sizes[..last].iter().all(|&s| (128..=1024).contains(&s)),
        "{sizes:?}"
    );
}
