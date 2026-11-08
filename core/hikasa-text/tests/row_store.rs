//! A10.11 (D-270): the chunked row store. Chunk invariants after random provisional splices (at most 1,024 rows a
//! chunk, bases, prefix sums: `check_structure` re-derives every row's lines from the runs), and sharing: a
//! chunk the splice did not touch is the same `Arc` in both comparisons.

mod common;

use common::XorShift;
use common::live::{Gen, Setup, edit_options, editable, live};
use hikasa_core::{Pair, Side};
use hikasa_text::TextRules;
use hikasa_text::edit::{EditRequest, GroupHint, TextPos};

#[test]
fn splices_keep_the_chunks_and_share_the_untouched_ones() {
    let mut rng = XorShift(0x5EED_0011);
    let left: String = (0..20_000).map(|k| format!("line {k}\n")).collect();
    let right: String = (0..20_000)
        .map(|k| {
            if k % 997 == 0 {
                format!("changed {k}\n")
            } else {
                format!("line {k}\n")
            }
        })
        .collect();
    let opts = edit_options(Gen::Clustered);
    let (mut l, r) = (editable(&left, &opts), editable(&right, &opts));
    let setup = Setup {
        format: common::format("ee"),
        rules: TextRules::default(),
        force_tokenize: false,
        anchors: Vec::new(),
    };
    let mut lc = live(&setup, Pair::new(&l, &r));
    let before = lc.current().row_chunks();
    assert!(before.len() >= 19 && before.iter().take(before.len() - 1).all(|c| c.1 == 1024));
    for step in 0..300 {
        let old = lc.current().row_chunks();
        // Small edits: a character, a split line, a joined line, an inserted or removed line.
        let line = rng.below(u64::from(l.line_count() - 1)) as u32;
        let len = l.line_len(hikasa_text::LineIdx(line));
        let at = TextPos::new(line, rng.below(u64::from(len) + 1) as u32);
        let (range, text) = match rng.below(5) {
            0 => (at..at, "x".to_owned()),
            1 => (at..at, "\n".to_owned()),
            2 => (
                TextPos::new(line, len)..TextPos::new(line + 1, 0),
                String::new(),
            ),
            3 => (
                TextPos::new(line, 0)..TextPos::new(line, 0),
                "new line\n".to_owned(),
            ),
            _ => (
                TextPos::new(line, 0)..TextPos::new(line + 1, 0),
                String::new(),
            ),
        };
        let out = l
            .apply(
                &EditRequest::Replace {
                    range,
                    text,
                    keep_eols: false,
                },
                GroupHint::None,
            )
            .unwrap();
        lc.note_edit(Side::Left, &l, &out);
        let cmp = lc.current();
        cmp.check_structure()
            .unwrap_or_else(|e| panic!("step {step}: {e:?}"));
        let new = cmp.row_chunks();
        assert!(new.iter().all(|c| c.1 >= 1 && c.1 <= 1024));
        let shared = new
            .iter()
            .filter(|c| old.iter().any(|o| o.0 == c.0))
            .count();
        assert!(
            shared + 3 >= old.len(),
            "step {step}: {shared} of {} chunks shared",
            old.len()
        );
    }
}
