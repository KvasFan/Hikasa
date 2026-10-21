//! Binary fixtures (§2.9, §6.2.4): every magic kind, BOM-less UTF-16/UTF-32 text, text with ESC sequences, an
//! 11 % control-byte buffer (and the 10 % one that is still text), a NUL byte, a BOM before a NUL byte, empty and
//! plain text. `tests/fixtures/binary/expected.toml` lists the verdict of every file.

mod common;

use std::collections::BTreeMap;
use std::fs;

use common::fixtures;
use hikasa_formats::{MAGIC, MagicKind, sniff};

#[test]
fn binary_fixture_table() {
    let dir = fixtures("binary");
    let expected: BTreeMap<String, toml::Table> =
        toml::from_str(&fs::read_to_string(dir.join("expected.toml")).unwrap()).unwrap();
    let mut files: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n != "expected.toml")
        .collect();
    files.sort();
    let listed: Vec<_> = expected.keys().cloned().collect();
    assert_eq!(files, listed, "every fixture has an expected verdict");
    let mut kinds_seen = Vec::new();
    for (name, want) in &expected {
        let verdict = sniff(&fs::read(dir.join(name)).unwrap());
        let json = serde_json::to_value(verdict).unwrap();
        assert_eq!(
            json["is_binary"].as_bool().unwrap(),
            want["binary"].as_bool().unwrap(),
            "{name}"
        );
        let reason = json["reason"].as_object().unwrap();
        assert_eq!(
            reason["rule"].as_str().unwrap(),
            want["rule"].as_str().unwrap(),
            "{name}"
        );
        for (key, value) in want
            .iter()
            .filter(|(k, _)| !matches!(k.as_str(), "binary" | "rule"))
        {
            let got = &reason[key.as_str()];
            match value {
                toml::Value::String(s) => assert_eq!(got.as_str().unwrap(), s, "{name}.{key}"),
                toml::Value::Integer(i) => assert_eq!(got.as_i64().unwrap(), *i, "{name}.{key}"),
                other => panic!("{name}: unexpected {other:?}"),
            }
        }
        if let Some(kind) = want.get("kind") {
            kinds_seen.push(kind.as_str().unwrap().to_owned());
        }
    }
    // Every magic kind of the in-house table has a fixture.
    for (kind, _) in MAGIC {
        let name = serde_json::to_value(kind).unwrap();
        assert!(
            kinds_seen.iter().any(|k| k == name.as_str().unwrap()),
            "no fixture for {kind:?}"
        );
    }
    assert_eq!(
        serde_json::to_value(MagicKind::SevenZip).unwrap(),
        "seven_zip"
    );
}
