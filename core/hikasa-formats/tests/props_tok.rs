//! Tokenizer properties (§6.2.4): P-TOK-1..3 (token invariants), P-TOK-4 (re-tokenising a line from its stored
//! start state reproduces the full pass), P-TOK-5 (parallel equals sequential for pools of 1, 3 and 8 threads,
//! with 4-line chunks so that speculative prefixes get patched), and P-TOK-REF (the fast paths — prefilter,
//! next-match cache, keyword hash set, `memchr` jumps, the simple-pattern accelerator — never change a result).
//! Grammars come from a small random item generator; lines from a small alphabet that hits every construct.

mod common;

use std::sync::{Arc, OnceLock};

use common::config;
use hikasa_core::cancel::CancelToken;
use hikasa_formats::{
    CompiledFormat, ElementId, ElementName, FileFormat, FormatId, FormatKind, GrammarItem,
    ItemKind, LineState, RegexLimits, Token, Tokenizer, tokenize_all, tokenize_all_with_chunk,
};
use proptest::prelude::*;

fn lit(s: &str) -> String {
    s.to_owned()
}

/// One random grammar item.
fn item() -> impl Strategy<Value = GrammarItem> {
    let element = prop::sample::select(vec!["A", "B", "C"]);
    let kind = prop_oneof![
        // Basic literal.
        (
            // KELVIN SIGN U+212A and LONG S U+017F fold onto ASCII letters (the prefilter must admit k/s).
            prop::sample::select(vec![
                "a",
                "ab",
                "/",
                "é",
                "x-",
                "#",
                "--",
                "\u{212A}e",
                "k",
                "S"
            ]),
            any::<bool>()
        )
            .prop_map(|(t, mc)| ItemKind::Basic {
                text: lit(t),
                regex: false,
                match_case: mc
            }),
        // Basic regex (standard and fancy).
        (
            prop::sample::select(vec![
                "a+",
                "[ab]x*",
                "x*",
                r"\w+",
                "b(?:a)?",
                "^#.*",
                r"(a)\1",
                "a|ab",
                "[^ ]+",
                "é+",
                r"(?<=a)b",
                r"\d+(?:\.\d+)?",
                r"[ \t]+$",
                r"^\s*[ab]",
                "-[a-]*",
                "\u{17F}t?",
                "\u{212A}e",
                r"[\x{80}-\x{10FFFF}]+",
                r"(?<=k)s",
            ]),
            any::<bool>()
        )
            .prop_map(|(t, mc)| ItemKind::Basic {
                text: lit(t),
                regex: true,
                match_case: mc
            }),
        // Delimited.
        (
            prop::sample::select(vec![
                ("/*", Some("*/"), None),
                ("\"", Some("\""), Some('\\')),
                ("'", Some("'"), Some('\'')),
                ("#", None, None),
                ("<", Some(">"), None),
                ("x", Some("xx"), Some('x')),
            ]),
            any::<bool>(),
            any::<bool>(),
        )
            .prop_map(|((from, to, escape), stop, mc)| ItemKind::Delimited {
                from: lit(from),
                to: to.map(lit),
                stop_at_eol: stop,
                escape,
                regex: false,
                match_case: mc,
            }),
        // Delimited with regexes.
        (any::<bool>(), any::<bool>()).prop_map(|(stop, mc)| ItemKind::Delimited {
            from: lit("[ab]-"),
            to: Some(lit("-[ab]")),
            stop_at_eol: stop,
            escape: None,
            regex: true,
            match_case: mc,
        }),
        // List.
        (
            prop::collection::vec(
                prop::sample::select(vec![
                    "a",
                    "ab",
                    "ba",
                    "x-y",
                    "#a",
                    "é",
                    "b",
                    "\u{212A}e",
                    "s"
                ]),
                1..4
            ),
            any::<bool>()
        )
            .prop_map(|(t, mc)| ItemKind::List {
                tokens: t.into_iter().map(lit).collect(),
                regex: false,
                match_case: mc,
            }),
        (
            prop::collection::vec(prop::sample::select(vec!["a+", "b|ab", "x?y"]), 1..3),
            any::<bool>()
        )
            .prop_map(|(t, mc)| ItemKind::List {
                tokens: t.into_iter().map(lit).collect(),
                regex: true,
                match_case: mc,
            }),
        // Columns.
        (1u32..5, prop::option::of(0u32..4)).prop_map(|(from, span)| ItemKind::Columns {
            from_column: from,
            to_column: span.map(|s| from + s),
        }),
        // Lines.
        (
            prop::sample::select(vec![("##", false), ("^b", true), ("", false)]),
            0u32..3,
            any::<bool>()
        )
            .prop_map(|((heading, regex), next, one)| ItemKind::Lines {
                heading: lit(heading),
                regex,
                match_case: true,
                next_lines: next,
                or_line_one: one || heading.is_empty(),
            }),
    ];
    (element, kind).prop_map(|(e, kind)| GrammarItem {
        element: ElementName::new(e).unwrap(),
        kind,
    })
}

fn grammar() -> impl Strategy<Value = Arc<CompiledFormat>> {
    (prop::collection::vec(item(), 0..7), any::<bool>()).prop_map(|(items, hyphen)| {
        let mut f = FileFormat::new(FormatId::new("p").unwrap(), "P", FormatKind::Text);
        f.grammar = items;
        if hyphen {
            f.text.word_chars = "-".into();
        }
        Arc::new(CompiledFormat::compile(Arc::new(f), &RegexLimits::default()).unwrap())
    })
}

fn line() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop::sample::select(vec![
            'a', 'b', 'x', 'y', '-', '/', '*', '"', '\'', '#', '<', '>', '\\', ' ', ' ', 'é', '\t',
            '日', '1', '.', 'A', 'B', 'k', 'K', 'e', 's', 'S', 't', '\u{212A}', '\u{17F}',
        ]),
        0..24,
    )
    .prop_map(|c| c.into_iter().collect())
}

fn doc() -> impl Strategy<Value = Vec<String>> {
    prop::collection::vec(line(), 0..30)
}

fn check_invariants(line: &str, tokens: &[Token]) -> Result<(), TestCaseError> {
    let mut pos = 0u32;
    for (k, t) in tokens.iter().enumerate() {
        prop_assert!(t.end > t.start, "INV-TOK-1: {tokens:?}");
        prop_assert_eq!(t.start, pos, "INV-TOK-2: {:?}", tokens);
        prop_assert!(line.is_char_boundary(t.start as usize));
        prop_assert!(line.is_char_boundary(t.end as usize));
        prop_assert_eq!(t.item.is_none(), t.element == ElementId::EVERYTHING_ELSE);
        if k > 0 {
            prop_assert!(
                t.item.is_some() || tokens[k - 1].item.is_some(),
                "INV-TOK-3: {tokens:?}"
            );
        }
        pos = t.end;
    }
    prop_assert_eq!(pos as usize, line.len(), "INV-TOK-2 coverage: {:?}", tokens);
    Ok(())
}

/// Serialises tokens (the callback output used by the parallel properties).
fn encode(tokens: &[Token], out: &mut Vec<u8>) {
    for t in tokens {
        out.extend_from_slice(&t.start.to_le_bytes());
        out.extend_from_slice(&t.end.to_le_bytes());
        out.extend_from_slice(&t.element.0.to_le_bytes());
        out.extend_from_slice(&t.item.unwrap_or(u16::MAX).to_le_bytes());
    }
}

fn pools() -> &'static [rayon::ThreadPool; 3] {
    static POOLS: OnceLock<[rayon::ThreadPool; 3]> = OnceLock::new();
    POOLS.get_or_init(|| {
        [1, 3, 8].map(|n| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(n)
                .build()
                .unwrap()
        })
    })
}

proptest! {
    #![proptest_config(config("tests/proptest-regressions/props_tok.txt"))]

    /// P-TOK-1..3 and P-TOK-REF.
    #[test]
    fn p_tok_1_3_invariants_and_reference(format in grammar(), lines in doc()) {
        let g = format.grammar();
        let mut fast = Tokenizer::new(g);
        let mut slow = Tokenizer::reference(g);
        let (mut a, mut b) = (Vec::new(), Vec::new());
        let mut state = LineState::Normal;
        for (k, line) in lines.iter().enumerate() {
            let oa = fast.tokenize_line(line, k as u32, state, &mut a);
            let ob = slow.tokenize_line(line, k as u32, state, &mut b);
            check_invariants(line, &a)?;
            prop_assert_eq!(&a, &b, "fast paths changed the tokens of {:?}", line);
            prop_assert_eq!(oa.end_state, ob.end_state);
            prop_assert_eq!(oa.regex_errors, ob.regex_errors);
            if !g.has_multiline() {
                prop_assert_eq!(oa.end_state, LineState::Normal);
            }
            state = oa.end_state;
        }
    }

    /// P-TOK-4: re-tokenising any line from its stored start state reproduces the full pass.
    #[test]
    fn p_tok_4_random_access(format in grammar(), lines in doc()) {
        let g = format.grammar();
        let all = tokenize_all(g, &lines, None, &CancelToken::never(), |_, _, tokens, out| encode(tokens, out))
            .unwrap();
        prop_assert_eq!(all.bytes.len() as usize, lines.len());
        if g.has_multiline() {
            prop_assert_eq!(all.start_states.len(), lines.len());
        } else {
            prop_assert!(all.start_states.is_empty());
        }
        let mut t = Tokenizer::new(g);
        let mut tokens = Vec::new();
        for (k, line) in lines.iter().enumerate() {
            let state = all.start_states.get(k).copied().unwrap_or_default();
            t.tokenize_line(line, k as u32, state, &mut tokens);
            let mut bytes = Vec::new();
            encode(&tokens, &mut bytes);
            prop_assert_eq!(all.bytes.get(k as u32), &bytes[..], "line {}", k);
        }
    }

    /// P-TOK-5: parallel equals sequential (pools of 1, 3 and 8 threads; 4-line chunks).
    #[test]
    fn p_tok_5_parallel_equals_sequential(format in grammar(), lines in doc(), chunk in 1u32..6) {
        let g = format.grammar();
        let cancel = CancelToken::never();
        let f = |_: u32, _: &str, tokens: &[Token], out: &mut Vec<u8>| encode(tokens, out);
        let sequential = tokenize_all(g, &lines, None, &cancel, f).unwrap();
        let small = tokenize_all_with_chunk(g, &lines, None, &cancel, chunk, f).unwrap();
        prop_assert_eq!(&small, &sequential);
        for pool in pools() {
            let parallel = tokenize_all_with_chunk(g, &lines, Some(pool), &cancel, 4, f).unwrap();
            prop_assert_eq!(&parallel, &sequential);
            let parallel = tokenize_all_with_chunk(g, &lines, Some(pool), &cancel, chunk, f).unwrap();
            prop_assert_eq!(&parallel, &sequential);
            prop_assert_eq!(parallel.bytes.iter().count(), lines.len());
        }
    }
}

/// A multi-line comment opened near the top makes every speculative chunk wrong: all of them get patched, and
/// the result still equals the sequential run.
#[test]
fn every_chunk_patched() {
    let mut f = FileFormat::new(FormatId::new("c").unwrap(), "C", FormatKind::Text);
    f.grammar.push(GrammarItem {
        element: ElementName::new("Comment").unwrap(),
        kind: ItemKind::Delimited {
            from: "/*".into(),
            to: Some("*/".into()),
            stop_at_eol: false,
            escape: None,
            regex: false,
            match_case: true,
        },
    });
    let c = CompiledFormat::compile(Arc::new(f), &RegexLimits::default()).unwrap();
    let mut lines = vec!["x /* never closed".to_owned()];
    lines.extend((0..200).map(|k| format!("plain line {k}")));
    lines.push("*/ y".into());
    let cancel = CancelToken::never();
    let f = |_: u32, _: &str, tokens: &[Token], out: &mut Vec<u8>| encode(tokens, out);
    let sequential = tokenize_all(c.grammar(), &lines, None, &cancel, f).unwrap();
    for pool in pools() {
        let parallel =
            tokenize_all_with_chunk(c.grammar(), &lines, Some(pool), &cancel, 4, f).unwrap();
        assert_eq!(parallel, sequential);
        assert!(
            parallel.bytes.patched_lines() >= 190,
            "{}",
            parallel.bytes.patched_lines()
        );
    }
    assert_eq!(
        sequential.start_states[100],
        LineState::InDelimited { item: 0 }
    );
}

/// Cancellation is observed per chunk.
#[test]
fn cancelled_runs_return_err() {
    let c = CompiledFormat::everything_else();
    let lines: Vec<String> = (0..20_000).map(|k| k.to_string()).collect();
    let cancel = CancelToken::new();
    cancel.cancel();
    let f = |_: u32, _: &str, _: &[Token], _: &mut Vec<u8>| {};
    assert!(tokenize_all(c.grammar(), &lines, None, &cancel, f).is_err());
    assert!(tokenize_all(c.grammar(), &lines, Some(&pools()[1]), &cancel, f).is_err());
    let empty: Vec<String> = Vec::new();
    let out = tokenize_all(c.grammar(), &empty, None, &CancelToken::never(), f).unwrap();
    assert!(out.bytes.is_empty());
    assert_eq!(out.bytes.get(5), b"");
}
