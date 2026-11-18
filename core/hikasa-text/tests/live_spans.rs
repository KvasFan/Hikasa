//! M2 W11a (§1.11.32, typing at 1M lines): the live comparison keeps the tokenizer start states exact through edits,
//! so the syntax spans a view draws in a key frame need no tokenization of the document. After every noted edit and
//! after every install, each line's spans equal those of a fresh comparison of the documents; the states stay
//! prepared (no O(document) pass on the next `spans` call) unless an edit's effect reaches past the splice bound.

mod common;

use common::XorShift;
use common::live::{Gen, Setup, edit_options, editable, fresh, live, pair, random_edit, side};
use hikasa_core::{Ctl, Pair, Side};
use hikasa_text::edit::{EditRequest, GroupHint, TextPos};
use hikasa_text::{LineIdx, TextRules};

fn spans_equal(
    live: &hikasa_text::LiveComparison,
    fresh: &hikasa_text::TextComparison,
    what: &str,
) {
    for side in [Side::Left, Side::Right] {
        let n = fresh.document(side).line_count();
        assert_eq!(live.current().document(side).line_count(), n, "{what}");
        for l in 0..n {
            let a: Vec<_> = live
                .current()
                .spans(side, LineIdx(l))
                .into_iter()
                .map(|s| (s.range, s.element, s.class))
                .collect();
            let b: Vec<_> = fresh
                .spans(side, LineIdx(l))
                .into_iter()
                .map(|s| (s.range, s.element, s.class))
                .collect();
            assert_eq!(a, b, "{what}: {side:?} line {l}");
        }
    }
}

#[test]
fn spans_stay_exact_through_edits_and_installs() {
    for seed in 1..=12u64 {
        let mut rng = XorShift(0x5EED_5A45 ^ seed);
        let (l, r) = pair(&mut rng, Gen::Source, 120);
        let opts = edit_options(Gen::Source);
        let mut docs = Pair::new(editable(&l, &opts), editable(&r, &opts));
        let setup = Setup {
            format: common::format("c-cpp"),
            rules: TextRules::default(),
            force_tokenize: false,
            anchors: Vec::new(),
            right_format: None,
            ignored: Default::default(),
        };
        let mut lc = live(&setup, docs.as_ref());
        let mut kept = 0;
        for step in 0..40 {
            let s = side(&mut rng);
            let doc = if s == Side::Right {
                &mut docs.right
            } else {
                &mut docs.left
            };
            let Some(out) = random_edit(&mut rng, Gen::Source, doc) else {
                continue;
            };
            lc.note_edit(s, doc, &out);
            kept += usize::from(lc.current().spans_prepared(s));
            let f = fresh(&setup, docs.as_ref(), lc.anchors());
            spans_equal(&lc, &f, &format!("seed {seed} step {step} (noted)"));
            if step % 7 == 6 {
                let done = lc
                    .recompare_job(docs.as_ref())
                    .run(None, &Ctl::detached())
                    .unwrap();
                lc.install(done, docs.as_ref());
                spans_equal(&lc, &f, &format!("seed {seed} step {step} (installed)"));
            }
        }
        assert!(kept > 0, "seed {seed}: the states were never kept");
    }
}

#[test]
fn typing_keeps_the_states_and_an_opened_comment_reaches_the_end() {
    let mut text = String::new();
    for k in 0..300 {
        text.push_str(&format!("int v{k} = {k};\n"));
    }
    let opts = edit_options(Gen::Source);
    let mut docs = Pair::new(editable(&text, &opts), editable(&text, &opts));
    let setup = Setup {
        format: common::format("c-cpp"),
        rules: TextRules::default(),
        force_tokenize: false,
        anchors: Vec::new(),
        right_format: None,
        ignored: Default::default(),
    };
    let mut lc = live(&setup, docs.as_ref());
    // Make sure the states exist (a view's worker does this once).
    lc.current()
        .prepare_spans(Side::Left, &hikasa_core::CancelToken::never())
        .unwrap();
    for (k, c) in "x /* y".chars().enumerate() {
        let out = docs
            .left
            .apply(
                &EditRequest::Replace {
                    range: TextPos::new(10, k as u32)..TextPos::new(10, k as u32),
                    text: c.to_string(),
                    keep_eols: false,
                },
                GroupHint::Typing { at_ms: k as u64 },
            )
            .unwrap();
        lc.note_edit(Side::Left, &docs.left, &out);
        assert!(lc.current().spans_prepared(Side::Left), "char {k}");
    }
    let f = fresh(&setup, docs.as_ref(), lc.anchors());
    spans_equal(&lc, &f, "an opened block comment");
}
