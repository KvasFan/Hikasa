//! Document properties: INV-DOC-1 (lines end only at CR, LF and CRLF; the lines plus their non-artificial
//! endings reproduce the decoded text, with any characters-per-line limit), INV-DOC-2 (at most one `None`
//! ending, on the last line), physical line numbers, the sequential line cursor against random access, and
//! totality of `from_bytes` over arbitrary bytes (T9).

mod common;

use common::config;
use hikasa_formats::CompiledFormat;
use hikasa_text::{DocOrigin, LineEnding, LineIdx, LoadOptions, TextDocument};
use proptest::prelude::*;

fn text() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop_oneof![
            Just("\n".to_owned()),
            Just("\r\n".to_owned()),
            Just("\r".to_owned()),
            Just("\u{2028}".to_owned()),
            Just("\u{85}".to_owned()),
            Just("\u{0B}\u{0C}".to_owned()),
            "[a-zé€ \t]{0,12}",
        ],
        0..30,
    )
    .prop_map(|parts| parts.concat())
}

proptest! {
    #![proptest_config(config("tests/proptest-regressions/props_document.txt"))]

    /// INV-DOC-1 and INV-DOC-2, with and without a characters-per-line limit.
    #[test]
    fn inv_doc_1_2_lines_reproduce_the_text(text in text(), cpl in prop::option::of(0u32..6)) {
        let load = LoadOptions { chars_per_line_limit: cpl, ..LoadOptions::default() };
        let doc = TextDocument::from_bytes(
            text.as_bytes(),
            &CompiledFormat::everything_else(),
            &load,
            DocOrigin::default(),
        )
        .unwrap();
        let mut rebuilt = String::new();
        let mut nones = Vec::new();
        let mut physical = 1u32;
        for k in 0..doc.line_count() {
            let line = doc.line(LineIdx(k));
            prop_assert!(!line.contains(['\n', '\r']));
            rebuilt.push_str(&line);
            let eol = doc.line_ending(LineIdx(k));
            rebuilt.push_str(eol.as_text());
            if eol == LineEnding::None {
                nones.push(k);
            }
            prop_assert_eq!(doc.physical_line(LineIdx(k)), physical);
            if eol != LineEnding::Artificial {
                physical += 1;
            }
            if let (Some(limit), true) = (cpl, eol == LineEnding::Artificial) {
                prop_assert_eq!(line.chars().count(), limit as usize);
            }
        }
        prop_assert_eq!(&rebuilt, &text);
        prop_assert_eq!(doc.rope().to_string(), text);
        prop_assert!(nones.len() <= 1);
        if let Some(&k) = nones.first() {
            prop_assert_eq!(k + 1, doc.line_count());
        }
        let e = doc.eol();
        let terminated = (0..doc.line_count())
            .filter(|&k| !matches!(doc.line_ending(LineIdx(k)), LineEnding::Artificial))
            .count() as u64;
        prop_assert_eq!(e.lf + e.crlf + e.cr + e.none, terminated);
    }

    /// `from_bytes` is total over arbitrary bytes (decoding is lossy, never failing, within the limits).
    #[test]
    fn from_bytes_is_total(bytes in prop::collection::vec(any::<u8>(), 0..300), cpl in 0u32..4) {
        let load = LoadOptions { chars_per_line_limit: Some(cpl), ..LoadOptions::default() };
        let doc = TextDocument::from_bytes(
            &bytes,
            &CompiledFormat::everything_else(),
            &load,
            DocOrigin::default(),
        )
        .unwrap();
        let total: usize = (0..doc.line_count())
            .map(|k| doc.line(LineIdx(k)).len() + doc.line_ending(LineIdx(k)).as_text().len())
            .sum();
        prop_assert_eq!(total, doc.rope().len_bytes());
    }
}
