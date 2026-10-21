//! Token goldens (§6.2.4, Appendix A): `tests/fixtures/tokens/<preset-id>/<case>.<ext>` and
//! `<case>.tokens.txt`, in the text format of `hikasa format tokens`. Every text preset has at least two cases,
//! a typical file and a tricky one; the goldens are the review artefact for grammar changes. Run with
//! `HIKASA_BLESS=1` to rewrite them.

mod common;

use std::fs;

use common::{assert_golden, fixtures, split_lines, token_dump};
use hikasa_core::cancel::CancelToken;
use hikasa_formats::{
    DetectReason, EnabledOverrides, FormatId, FormatKind, FormatRegistry, tokenize_all,
    write_token_dump,
};

#[test]
fn token_goldens() {
    let registry = FormatRegistry::builtin().unwrap();
    let root = fixtures("tokens");
    let mut checked = 0;
    for entry in registry.entries() {
        if !matches!(entry.format.kind, FormatKind::Text | FormatKind::Table) {
            continue;
        }
        let id = entry.id().as_str();
        let dir = root.join(id);
        let mut cases: Vec<_> = fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
            .map(|e| e.unwrap().path())
            .filter(|p| !p.to_string_lossy().ends_with(".tokens.txt"))
            .collect();
        cases.sort();
        assert!(cases.len() >= 2, "{id}: at least two token goldens");
        let format = registry.compiled(entry.id()).unwrap();
        for case in cases {
            let name = case.file_name().unwrap().to_string_lossy().into_owned();
            // The fixture's name is detected as its preset (Sorted is manual-only).
            let detected = registry
                .detect(&name, &EnabledOverrides::default())
                .unwrap();
            match detected.reason {
                DetectReason::Mask { id: got, .. } => assert_eq!(got.as_str(), id, "{name}"),
                DetectReason::Fallback => assert_eq!(id, "sorted", "{name}"),
                DetectReason::Forced { .. } => unreachable!(),
            }
            let text = fs::read_to_string(&case).unwrap();
            let lines = split_lines(&text);
            let dump = token_dump(format.grammar(), &lines);
            let stem = case.file_stem().unwrap().to_string_lossy().into_owned();
            assert_golden(&dir.join(format!("{stem}.tokens.txt")), dump.as_bytes());
            // The parallel driver with tiny chunks produces the same dump.
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(3)
                .build()
                .unwrap();
            let all = hikasa_formats::tokenize_all_with_chunk(
                format.grammar(),
                &lines,
                Some(&pool),
                &CancelToken::never(),
                2,
                |line_no, text, tokens, out| {
                    let mut s = String::new();
                    write_token_dump(
                        &mut s,
                        line_no + 1,
                        text,
                        tokens,
                        format.grammar().elements(),
                    );
                    out.extend_from_slice(s.as_bytes());
                },
            )
            .unwrap();
            let joined: Vec<u8> = all.bytes.iter().flatten().copied().collect();
            assert_eq!(String::from_utf8(joined).unwrap(), dump, "{name}: parallel");
            let seq = tokenize_all(
                format.grammar(),
                &lines,
                None,
                &CancelToken::never(),
                |_, _, _, _| {},
            )
            .unwrap();
            assert_eq!(seq.start_states, all.start_states, "{name}");
            checked += 1;
        }
    }
    assert!(checked >= 34, "{checked} cases");
    assert!(registry.get(&FormatId::new("c-cpp").unwrap()).is_some());
}
