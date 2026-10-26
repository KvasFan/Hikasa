//! Encoding and binary-sniffing properties (§6.2.4): P-ENC-1 (decoding round-trips for UTF-8, UTF-16 and
//! UTF-32), P-ENC-2 (`detect` and `decode` are total over arbitrary bytes), P-ENC-3 (`detect` and `sniff` agree
//! on every input that `utf_pattern` accepts).

mod common;

use common::config;
use hikasa_formats::{
    BinaryReason, DecodeOptions, DetectedEncoding, EncodingSetting, EncodingSource, SNIFF_LEN,
    TextEncoding, UTF_PATTERN_SAMPLE, bom_sniff, decode, detect, encode, magic, sniff, utf_pattern,
};
use proptest::prelude::*;
use proptest::strategy::ValueTree;

const UTFS: [TextEncoding; 5] = [
    TextEncoding::UTF_8,
    TextEncoding::UTF_16LE,
    TextEncoding::UTF_16BE,
    TextEncoding::Utf32Le,
    TextEncoding::Utf32Be,
];

fn settings() -> impl Strategy<Value = EncodingSetting> {
    prop_oneof![
        Just(EncodingSetting::Auto { fallback: None }),
        prop::sample::select(vec!["windows-1252", "utf-16le", "koi8-r", "utf-32be"]).prop_map(
            |l| EncodingSetting::Auto {
                fallback: Some(l.into())
            }
        ),
        prop::sample::select(vec![
            "utf-8",
            "shift_jis",
            "utf-16be",
            "UTF-32LE",
            "gb18030",
            "iso-2022-jp"
        ])
        .prop_map(|l| EncodingSetting::Fixed { label: l.into() }),
    ]
}

fn options() -> impl Strategy<Value = DecodeOptions> {
    let enc = prop::sample::select(vec![
        TextEncoding::UTF_8,
        TextEncoding::UTF_16LE,
        TextEncoding::Utf32Be,
        TextEncoding::for_label("windows-1251").unwrap(),
        TextEncoding::for_label("euc-kr").unwrap(),
    ]);
    (prop::option::of(enc.clone()), enc, 1usize..64).prop_map(|(o, l, limit)| DecodeOptions {
        override_encoding: o,
        locale_fallback: l,
        detect_limit: limit,
    })
}

/// Bytes biased towards the interesting shapes: BOMs, zero-heavy UTF-16/32 patterns, high bytes.
fn bytes() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        prop::collection::vec(any::<u8>(), 0..64),
        prop::collection::vec(
            prop::sample::select(vec![
                0u8, 0, 0, b'a', 0xFF, 0xFE, 0xEF, 0xBB, 0xBF, 0xD8, 0x80
            ]),
            0..64
        ),
        (".{0,20}", prop::sample::select(UTFS.to_vec())).prop_map(|(s, e)| encode(&s, e).0),
        // Mostly-ASCII text in UTF-16/32: what the BOM-less pattern test is for.
        (
            "[a-z \\r\\n\\u{e9}\\u{4e2d}]{2,40}",
            prop::sample::select(UTFS[1..].to_vec())
        )
            .prop_map(|(s, e)| encode(&s, e).0),
    ]
}

proptest! {
    #![proptest_config(config("tests/proptest-regressions/props_enc.txt"))]

    /// P-ENC-1: `decode(encode(s)) == s` for UTF-8, UTF-16LE/BE and UTF-32LE/BE.
    #[test]
    fn p_enc_1_round_trip(s in "\\PC{0,40}|[\\u{0}-\\u{10FFFF}]{0,12}") {
        for enc in UTFS {
            let (bytes, unmappable) = encode(&s, enc);
            prop_assert!(!unmappable);
            let d = DetectedEncoding { encoding: enc, bom_len: 0, source: EncodingSource::Override };
            let decoded = decode(&bytes, &d);
            prop_assert_eq!(&decoded.text, &s, "{}", enc);
            prop_assert_eq!(decoded.malformed, 0);
            // With its BOM, detection finds the encoding again and strips the BOM.
            let mut with_bom = enc.bom().to_vec();
            with_bom.extend_from_slice(&bytes);
            let found = detect(&with_bom, &EncodingSetting::Auto { fallback: None }, &DecodeOptions::default()).unwrap();
            if enc == TextEncoding::UTF_16LE && s.starts_with('\0') {
                // FF FE 00 00 is the UTF-32LE BOM (§2.8 tests UTF-32LE first): a documented ambiguity.
                continue;
            }
            prop_assert_eq!(found.encoding, enc);
            prop_assert_eq!(decode(&with_bom, &found).text, s.clone());
        }
    }

    /// P-ENC-2: `detect` and `decode` are total over arbitrary bytes (no panic; decoding never loses the
    /// replacement count).
    #[test]
    fn p_enc_2_total(input in bytes(), setting in settings(), opts in options()) {
        let d = detect(&input, &setting, &opts).unwrap();
        prop_assert!(usize::from(d.bom_len) <= input.len());
        let decoded = decode(&input, &d);
        prop_assert_eq!(decoded.encoding, d);
        if decoded.malformed == 0 && d.encoding == TextEncoding::UTF_8 {
            prop_assert_eq!(decoded.text.as_bytes(), &input[usize::from(d.bom_len)..]);
        }
        let v = sniff(&input);
        prop_assert_eq!(v.is_binary, matches!(
            v.reason,
            BinaryReason::Magic { .. } | BinaryReason::NulByte { .. } | BinaryReason::ControlRatio { .. }
        ));
    }

    /// P-ENC-3: on every input that `utf_pattern` accepts (no BOM, no magic number), detection reports that
    /// encoding from the pattern and sniffing classifies the content as text by the same rule.
    #[test]
    fn p_enc_3_detect_and_sniff_agree(input in bytes(), tail in prop::collection::vec(any::<u8>(), 0..32)) {
        let mut input = input;
        input.extend_from_slice(&tail);
        let Some(e) = utf_pattern(&input[..input.len().min(UTF_PATTERN_SAMPLE)]) else {
            return Ok(());
        };
        if bom_sniff(&input).is_some() || magic(&input).is_some() {
            return Ok(());
        }
        let d = detect(&input, &EncodingSetting::Auto { fallback: None }, &DecodeOptions::default()).unwrap();
        prop_assert_eq!((d.encoding, d.source), (e, EncodingSource::UtfPattern));
        let v = sniff(&input);
        prop_assert!(!v.is_binary);
        prop_assert_eq!(v.reason, BinaryReason::UtfPattern);
    }
}

/// P-ENC-3 is not vacuous: with the default 256 cases, a good share of the inputs has a UTF pattern.
#[test]
fn p_enc_3_reaches_the_check() {
    use proptest::test_runner::TestRunner;
    let mut runner = TestRunner::deterministic();
    let mut accepted = 0;
    for _ in 0..256 {
        let input = bytes().new_tree(&mut runner).unwrap().current();
        if utf_pattern(&input[..input.len().min(UTF_PATTERN_SAMPLE)]).is_some() {
            accepted += 1;
        }
    }
    assert!(
        accepted >= 40,
        "{accepted} of 256 inputs have a UTF pattern"
    );
}

/// Long inputs: detection and sniffing look at bounded prefixes (4096 bytes for the pattern, 8192 for sniffing).
#[test]
fn prefixes_are_bounded() {
    let mut utf16: Vec<u8> = encode(&"text ".repeat(1000), TextEncoding::UTF_16LE).0;
    utf16.extend_from_slice(&[0xC3, 0x28, 0xFF].repeat(5000));
    let d = detect(
        &utf16,
        &EncodingSetting::Auto { fallback: None },
        &DecodeOptions::default(),
    )
    .unwrap();
    assert_eq!(d.encoding, TextEncoding::UTF_16LE);
    assert_eq!(sniff(&utf16).reason, BinaryReason::UtfPattern);
    let mut late = vec![b'a'; SNIFF_LEN + 10];
    late[SNIFF_LEN + 5] = 0;
    assert!(!sniff(&late).is_binary);
}
