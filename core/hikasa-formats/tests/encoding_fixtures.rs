//! Encoding fixtures (§2.8 table, §6.2.4): `tests/fixtures/encoding/<case>.bin` + `<case>.expected.toml`, with
//! an optional `[setting]` (the format's `EncodingSetting`, default Auto), optional `[options]` (`override` and
//! `locale` labels) and the `[expected]` encoding, BOM length, source, decoded text and malformed count.

mod common;

use std::fs;

use common::fixtures;
use hikasa_formats::{DecodeOptions, EncodingSetting, TextEncoding, decode, detect};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    #[serde(default)]
    setting: Option<EncodingSetting>,
    #[serde(default)]
    options: Options,
    expected: Expected,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Options {
    #[serde(rename = "override")]
    override_label: Option<String>,
    locale: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Expected {
    encoding: String,
    bom_len: u8,
    source: String,
    text: Option<String>,
    malformed: Option<u64>,
}

#[test]
fn encoding_fixture_table() {
    let dir = fixtures("encoding");
    let mut names: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".bin"))
        .collect();
    names.sort();
    assert!(names.len() >= 18, "{names:?}");
    for name in names {
        let stem = name.trim_end_matches(".bin");
        let bytes = fs::read(dir.join(&name)).unwrap();
        let spec = fs::read_to_string(dir.join(format!("{stem}.expected.toml"))).unwrap();
        let case: Case = toml::from_str(&spec).unwrap_or_else(|e| panic!("{stem}: {e}"));
        let setting = case
            .setting
            .unwrap_or(EncodingSetting::Auto { fallback: None });
        let label = |l: &str| TextEncoding::for_label(l).unwrap_or_else(|| panic!("label {l}"));
        let opts = DecodeOptions {
            override_encoding: case.options.override_label.as_deref().map(label),
            locale_fallback: case
                .options
                .locale
                .as_deref()
                .map_or(TextEncoding::UTF_8, label),
            ..DecodeOptions::default()
        };
        let detected = detect(&bytes, &setting, &opts).unwrap();
        let e = &case.expected;
        assert_eq!(detected.encoding.name(), e.encoding, "{stem}");
        assert_eq!(detected.bom_len, e.bom_len, "{stem}");
        assert_eq!(
            serde_json::to_value(detected.source).unwrap(),
            serde_json::Value::String(e.source.clone()),
            "{stem}"
        );
        let decoded = decode(&bytes, &detected);
        if let Some(text) = &e.text {
            assert_eq!(&decoded.text, text, "{stem}");
        }
        if let Some(malformed) = e.malformed {
            assert_eq!(decoded.malformed, malformed, "{stem}");
        }
    }
}
