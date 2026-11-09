//! M2 W10b review fix (W10b note 66): P-SAVE-2, the review's save probe made a property. Random edits inside and
//! across lines (breaks inserted and joined), `SetEols`, encoding changes, undo and redo across saves, and several
//! saves of one document, CPL-wrapped or not. Oracles:
//!
//! - the file is the BOM plus the encoding of the document's whole text, and the Ctrl-Z tail of the file as loaded
//!   after it (A10.13, note 28: also across an encoding change); with malformed UTF-8 bytes the lossy decode equals
//!   the text and an untouched first line keeps its malformed bytes;
//! - a save asks `Unmappable` exactly when the text holds a character the encoding cannot write (GB18030's U+E5E5,
//!   ISO-2022-JP's half-width katakana, note 62), and answered yes writes `?` for it; `LossyRewritten` is asked only in
//!   a document whose decode replaced malformed bytes (a literal U+FFFD is text).
//!
//! 256 cases unless `PROPTEST_CASES` says otherwise.

mod common;

use common::save::{confirm_all, execute, forced, open, plan, spec, text_format};
use hikasa_core::CaseSensitivity;
use hikasa_formats::{Encoder, EncodingSetting, TextEncoding};
use hikasa_text::edit::{EditRequest, EditableDocument, GroupHint, TextPos};
use hikasa_text::save::{SaveQuestion, SaveRequest};
use hikasa_text::{LineEnding, LineIdx, LineMark};
use hikasa_vfs::{MemMeta, MemoryVfs};
use proptest::prelude::*;

/// The tail after a Ctrl-Z that ends the text (kept by every save).
const TAIL: &[u8] = b"\x1Atail\xFE\n";

fn put(vfs: &MemoryVfs, path: &str, bytes: &[u8]) {
    vfs.insert_file(
        &common::host(path),
        bytes.to_vec(),
        MemMeta {
            mtime: hikasa_core::FileTime::from_unix(1_700_000_000, 0),
            ..MemMeta::default()
        },
    )
    .unwrap();
}

fn whole_text(doc: &EditableDocument) -> String {
    let n = doc.line_count();
    if n == 0 {
        return String::new();
    }
    let last = n - 1;
    let mut s = doc.text(TextPos::new(0, 0)..TextPos::new(last, doc.line_len(LineIdx(last))));
    s.push_str(doc.line_ending(LineIdx(last)).as_text());
    s
}

#[derive(Debug, Clone)]
enum Op {
    Replace(u32, u32, u32, u32, String),
    Eols(u8),
    /// Switches to the case's other encoding (index into `Case::encodings`).
    Encoding(u8),
    Undo,
    Redo,
    Save,
}

fn pos(doc: &EditableDocument, l: u32, c: u32) -> TextPos {
    let n = doc.line_count().max(1);
    let l = l % n;
    let text = doc.line(LineIdx(l)).into_owned();
    let mut b = (c as usize) % (text.len() + 1);
    while !text.is_char_boundary(b) {
        b -= 1;
    }
    TextPos::new(l, b as u32)
}

fn op() -> impl Strategy<Value = Op> {
    let text = proptest::collection::vec(
        proptest::sample::select(vec![
            "a", "\u{e9}", "\u{65e5}", "\n", "\r", "\r\n", " ", "\t", "\u{E5E5}", "\u{FF76}",
            "\u{FFFD}",
        ]),
        0..5,
    )
    .prop_map(|v| v.concat());
    prop_oneof![
        5 => (any::<u32>(), any::<u32>(), any::<u32>(), any::<u32>(), text)
            .prop_map(|(a, b, c, d, t)| Op::Replace(a, b, c % 3, d, t)),
        1 => (0u8..3).prop_map(Op::Eols),
        1 => (0u8..2).prop_map(Op::Encoding),
        2 => Just(Op::Undo),
        1 => Just(Op::Redo),
        2 => Just(Op::Save),
    ]
}

/// The cases: the encoding loaded, the one an `Encoding` op switches to, a BOM, malformed bytes, a Ctrl-Z tail.
fn cases() -> Vec<(&'static str, &'static str, bool, bool, bool)> {
    vec![
        ("UTF-8", "Shift_JIS", false, false, false),
        ("UTF-8", "UTF-8", true, false, false),
        ("UTF-8", "UTF-8", false, true, false),
        ("UTF-16LE", "UTF-16BE", true, false, false),
        ("UTF-16BE", "UTF-16BE", false, false, false),
        ("Shift_JIS", "EUC-JP", false, false, true),
        ("EUC-JP", "UTF-8", false, false, false),
        ("gb18030", "UTF-8", false, false, true),
        ("windows-1252", "UTF-8", false, false, true),
        ("UTF-8", "gb18030", false, false, true),
        ("ISO-2022-JP", "UTF-8", false, false, false),
        ("UTF-8", "ISO-2022-JP", false, false, false),
    ]
}

/// True for a character `enc` cannot write, by `encoding_rs` itself (independent of Hikasa's tables), and the
/// half-width katakana ISO-2022-JP would write as other characters.
fn unmappable(enc: TextEncoding, c: char) -> bool {
    let TextEncoding::Whatwg(e) = enc else {
        return false;
    };
    if e == encoding_rs::UTF_16LE || e == encoding_rs::UTF_16BE {
        return false;
    }
    let (_, _, unmappable) = e.encode(c.encode_utf8(&mut [0; 4]));
    unmappable || (e == encoding_rs::ISO_2022_JP && ('\u{FF61}'..='\u{FF9F}').contains(&c))
}

proptest! {
    #![proptest_config(common::config("tests/proptest-regressions/save_intra_line.txt"))]

    /// P-SAVE-2 (W10b note 66).
    #[test]
    fn p_save_intra_line(
        case_i in 0usize..12,
        lines in proptest::collection::vec(
            (proptest::sample::select(vec!["", "ab", "a\u{e9}b", "\u{65e5}\u{65e5}x", "  a\t"]),
             proptest::sample::select(vec!["\n", "\r\n", "\r", ""])), 1..9),
        cpl in proptest::option::of(2u32..6),
        ops in proptest::collection::vec(op(), 1..14),
    ) {
        let (label, other, bom, bad, tail) = cases()[case_i];
        let first = TextEncoding::for_label(label).unwrap();
        let second = TextEncoding::for_label(other).unwrap();
        // The source: characters the loaded encoding writes.
        let mut src = String::new();
        let n = lines.len();
        for (k, (t, e)) in lines.iter().enumerate() {
            let t: String = t.chars().map(|c| if unmappable(first, c) { 'o' } else { c }).collect();
            src.push_str(&t);
            if k + 1 < n || !e.is_empty() {
                src.push_str(if e.is_empty() { "\n" } else { e });
            }
        }
        let mut bytes = Vec::new();
        if bom {
            bytes.extend_from_slice(first.bom());
        }
        Encoder::new(first).encode(&src, true, &mut bytes);
        if bad {
            bytes.splice(0..0, b"z\xC3\xFFz".iter().copied());
        }
        if tail {
            bytes.extend_from_slice(TAIL);
        }
        let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
        put(&vfs, "/r.txt", &bytes);
        let format = text_format(EncodingSetting::Auto { fallback: None }, tail);
        let mut doc = open(&vfs, &spec("/r.txt"), &format, forced(first), cpl);
        let mut replaced_before = false;
        for op in &ops {
            match op {
                Op::Replace(a, b, span, d, t) => {
                    let from = pos(&doc, *a, *b);
                    let to = pos(&doc, from.line.0 + span, *d);
                    let (from, to) = if to < from { (to, from) } else { (from, to) };
                    let _ = doc.apply(
                        &EditRequest::Replace { range: from..to, text: t.clone(), keep_eols: true },
                        GroupHint::None,
                    );
                }
                Op::Eols(k) => {
                    let eol = [LineEnding::Lf, LineEnding::CrLf, LineEnding::Cr][*k as usize];
                    let lines = doc.line_count();
                    let _ = doc.apply(&EditRequest::SetEols { lines: 0..lines, eol }, GroupHint::None);
                }
                Op::Encoding(k) => {
                    let encoding = if *k == 0 { first } else { second };
                    let bom = !encoding.bom().is_empty() && bom;
                    let _ = doc.apply(&EditRequest::SetEncoding { encoding, bom }, GroupHint::None);
                }
                Op::Undo => {
                    let _ = doc.undo();
                }
                Op::Redo => {
                    let _ = doc.redo();
                }
                Op::Save => {
                    let text = whole_text(&doc);
                    let (enc, with_bom) = doc.encoding();
                    let planned = plan(&vfs, &doc, &SaveRequest::default());
                    prop_assert!(planned.plan.items[0].error.is_none(), "{:?}", planned.plan.items[0].error);
                    let questions = planned.items[0].questions();
                    // Asked exactly when the text holds such a character, until a save wrote `?` for one (a line
                    // unchanged since is spliced from that file, `?` and all).
                    let asked = questions.contains(&SaveQuestion::Unmappable);
                    let holds = text.chars().any(|c| unmappable(enc, c));
                    if replaced_before {
                        prop_assert!(!asked || holds, "asked without a cause: {:?}", text);
                    } else {
                        prop_assert_eq!(asked, holds, "unmappable in {}: {:?}", enc.name(), text);
                    }
                    replaced_before |= asked;
                    if questions.contains(&SaveQuestion::LossyRewritten) {
                        prop_assert!(bad, "a lossy question without malformed bytes: {:?}", text);
                    }
                    let report = execute(&vfs, confirm_all(planned), &mut doc);
                    prop_assert!(report.ok(), "{:?}", report.op.results);
                    let file = vfs.read_back(&common::host("/r.txt")).unwrap().to_vec();
                    // The tail stays after the text, whatever the encoding (an ASCII-compatible one cuts it).
                    let kept_tail = tail && first.is_ascii_compatible();
                    let body = if kept_tail {
                        prop_assert!(file.ends_with(TAIL), "the Ctrl-Z tail lost: {:?}", file);
                        &file[..file.len() - TAIL.len()]
                    } else {
                        &file[..]
                    };
                    if bad {
                        let (dec, _, _) = encoding_rs::UTF_8.decode(body);
                        prop_assert_eq!(dec.into_owned(), text);
                        let mut l = 0u32;
                        let mut orig = doc.marks(LineIdx(0)).orig == Some(LineIdx(0))
                            && doc.line(LineIdx(0)).starts_with("z\u{FFFD}");
                        loop {
                            orig &= doc.mark(LineIdx(l)) == LineMark::Original;
                            if doc.line_ending(LineIdx(l)) != LineEnding::Artificial || l + 1 >= doc.line_count() {
                                break;
                            }
                            l += 1;
                        }
                        if orig {
                            prop_assert!(body.starts_with(b"z\xC3\xFFz"), "malformed bytes lost: {:?}", &body[..body.len().min(8)]);
                        }
                    } else {
                        let mut want = Vec::new();
                        if with_bom {
                            want.extend_from_slice(enc.bom());
                        }
                        // What the encoder cannot write is `?`, never a numeric reference.
                        let written: String = text.chars().map(|c| if unmappable(enc, c) { '?' } else { c }).collect();
                        Encoder::new(enc).encode(&written, true, &mut want);
                        prop_assert_eq!(body, &want[..], "label {} → {} ops {:?}", label, enc.name(), ops);
                    }
                }
            }
        }
    }
}
