//! M2 W7 acceptance A7.3 (D-197): the interleaved layout's goldens. Over every text fixture of `tests/fixtures/text`
//! (with its display, ignore-unimportant, width, wrap and line numbers): `expected.interleaved.txt`, `.ansi.txt`,
//! `.html`, `.mono.html`, and for the fixtures with differences `.strike-left.html`, `.strike-right.html`,
//! `.strike-both.txt`, `.strike-both.ansi.txt`, `.strike-both.mono.html`. Then the variants of
//! `tests/fixtures/interleaved/<case>`: each display mode, ignore-unimportant, no line numbers, `wrap-none` and
//! `wrap-word` at width 40 (text and HTML), a hidden run, characters-per-line pieces, controls and bidirectional
//! characters, and the mark table row by row. Every document renders twice alike and every HTML one passes the
//! checker of P-RPT-3. The bytes are the same on every platform (Windows host paths are spelled as on Unix before the
//! comparison). `HIKASA_BLESS=1` rewrites the goldens.

mod common;

use std::path::Path;

use common::{
    assert_golden, cases, check_html, compare_bytes, compare_bytes_with, compare_case, fixtures,
    sibling_fixtures, unix_paths,
};
use hikasa_report::{
    HtmlStyle, ReportDefaults, ReportInput, ReportKind, ReportMeta, ReportRequest, ReportWords,
    render_report,
};
use hikasa_text::{LoadOptions, TextComparison, TextRules};
use serde::Deserialize;

/// The options of a text case (`case.json`, as `golden_text.rs` reads it).
#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct Case {
    description: String,
    source: Option<String>,
    context: u32,
    display: String,
    ignore_unimportant: bool,
    width: u16,
    wrap: String,
    line_numbers: bool,
    mono: bool,
    ansi: bool,
}

impl Default for Case {
    fn default() -> Case {
        Case {
            description: String::new(),
            source: None,
            context: 3,
            display: "all".into(),
            ignore_unimportant: false,
            width: 80,
            wrap: "character".into(),
            line_numbers: true,
            mono: false,
            ansi: false,
        }
    }
}

fn inputs(dir: &Path, case: &Case) -> std::path::PathBuf {
    match case.source.as_deref() {
        None => dir.to_path_buf(),
        Some(source) => match source.split_once('/') {
            Some(("hikasa-text", name)) => sibling_fixtures("hikasa-text", "compare").join(name),
            Some(("text", name)) => fixtures("text").join(name),
            _ => panic!("{}: unknown source {source}", dir.display()),
        },
    }
}

/// Renders an interleaved request of `options` and `output` words (twice: identical bytes, INV-RPT-1).
fn render(
    cmp: &TextComparison,
    defaults: &ReportDefaults,
    options: &[String],
    output: &[String],
) -> Vec<u8> {
    let words = ReportWords {
        layout: Some("interleaved".to_owned()),
        options: options.to_vec(),
        output_options: output.to_vec(),
        title: None,
    };
    let req = ReportRequest::parse(ReportKind::Text, &words, defaults).unwrap();
    let hikasa_report::ReportBody::Text(opts) = &req.body else {
        unreachable!()
    };
    let meta = ReportMeta::for_text(cmp, opts, None, "hikasa test");
    let mut first = Vec::new();
    let warnings = render_report(
        ReportInput::Text(cmp),
        &req,
        &meta,
        &HtmlStyle::default(),
        None,
        &mut first,
    )
    .unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let mut second = Vec::new();
    render_report(
        ReportInput::Text(cmp),
        &req,
        &meta,
        &HtmlStyle::default(),
        None,
        &mut second,
    )
    .unwrap();
    assert_eq!(first, second, "rendering is deterministic (INV-RPT-1)");
    unix_paths(&first)
}

fn check(dir: &Path, file: &str, bytes: &[u8]) {
    if file.ends_with(".html") {
        check_html(std::str::from_utf8(bytes).unwrap())
            .unwrap_or_else(|e| panic!("{}/{file}: {e}", dir.display()));
    }
    assert_golden(&dir.join(file), bytes);
}

#[test]
fn a7_3_interleaved_goldens_of_every_text_fixture() {
    let dirs = cases(&fixtures("text"));
    assert!(dirs.len() >= 35, "text report cases: {}", dirs.len());
    for dir in &dirs {
        let case: Case =
            serde_json::from_str(&std::fs::read_to_string(dir.join("case.json")).unwrap())
                .unwrap_or_else(|e| panic!("{}: {e}", dir.display()));
        let cmp = compare_case(&inputs(dir, &case));
        let mut defaults = ReportDefaults::new("interleaved");
        defaults.context_lines = case.context;
        defaults.width = case.width;
        let mut options = vec![format!("display-{}", case.display)];
        if case.ignore_unimportant {
            options.push("ignore-unimportant".into());
        }
        options.push(
            if case.line_numbers {
                "line-numbers"
            } else {
                "no-line-numbers"
            }
            .into(),
        );
        let wrap = format!("wrap-{}", case.wrap);
        let differs = cmp.stats().same_rows != cmp.stats().rows;
        let with = |extra: &[&str]| {
            let mut o = options.clone();
            o.extend(extra.iter().map(|s| (*s).to_owned()));
            o
        };
        let out = |palette: Option<&str>| {
            let mut o = vec![wrap.clone()];
            o.extend(palette.map(str::to_owned));
            o
        };
        check(
            dir,
            "expected.interleaved.txt",
            &render(&cmp, &defaults, &options, &out(None)),
        );
        let ansi = {
            let words = ReportWords {
                layout: Some("interleaved".to_owned()),
                options: options.clone(),
                output_options: out(None),
                title: None,
            };
            let mut req = ReportRequest::parse(ReportKind::Text, &words, &defaults).unwrap();
            req.apply_target(hikasa_report::TargetKind::Stdout { ansi: true });
            let hikasa_report::ReportBody::Text(opts) = &req.body else {
                unreachable!()
            };
            let meta = ReportMeta::for_text(&cmp, opts, None, "hikasa test");
            let mut bytes = Vec::new();
            render_report(
                ReportInput::Text(&cmp),
                &req,
                &meta,
                &HtmlStyle::default(),
                None,
                &mut bytes,
            )
            .unwrap();
            unix_paths(&bytes)
        };
        check(dir, "expected.interleaved.ansi.txt", &ansi);
        check(
            dir,
            "expected.interleaved.html",
            &render(&cmp, &defaults, &options, &out(Some("html-color"))),
        );
        check(
            dir,
            "expected.interleaved.mono.html",
            &render(&cmp, &defaults, &options, &out(Some("html-mono"))),
        );
        if differs {
            let left = with(&["strikeout-left-diffs"]);
            let right = with(&["strikeout-right-diffs"]);
            let both = with(&["strikeout-left-diffs", "strikeout-right-diffs"]);
            check(
                dir,
                "expected.interleaved.strike-left.html",
                &render(&cmp, &defaults, &left, &out(Some("html-color"))),
            );
            check(
                dir,
                "expected.interleaved.strike-right.html",
                &render(&cmp, &defaults, &right, &out(Some("html-color"))),
            );
            check(
                dir,
                "expected.interleaved.strike-both.txt",
                &render(&cmp, &defaults, &both, &out(None)),
            );
            check(
                dir,
                "expected.interleaved.strike-both.mono.html",
                &render(&cmp, &defaults, &both, &out(Some("html-mono"))),
            );
            let words = ReportWords {
                layout: Some("interleaved".to_owned()),
                options: both.clone(),
                output_options: out(None),
                title: None,
            };
            let mut req = ReportRequest::parse(ReportKind::Text, &words, &defaults).unwrap();
            req.apply_target(hikasa_report::TargetKind::Stdout { ansi: true });
            let hikasa_report::ReportBody::Text(opts) = &req.body else {
                unreachable!()
            };
            let meta = ReportMeta::for_text(&cmp, opts, None, "hikasa test");
            let mut bytes = Vec::new();
            render_report(
                ReportInput::Text(&cmp),
                &req,
                &meta,
                &HtmlStyle::default(),
                None,
                &mut bytes,
            )
            .unwrap();
            check(
                dir,
                "expected.interleaved.strike-both.ansi.txt",
                &unix_paths(&bytes),
            );
        }
    }
}

/// The inputs of the variants: important and unimportant changes, orphans of both sides, a long run of same lines
/// (a hidden run), a tab, a wide character, a control character and a bidirectional override.
const LEFT: &str = "alpha\nbeta  gamma\nsame 1\nsame 2\nsame 3\nsame 4\nsame 5\nsame 6\nsame 7\nleft only\nTAB\there\nwide 日本語 text that runs on and on past forty columns for wrapping\nesc \u{1b}[31m\u{202e}txt.exe\nend\n";
const RIGHT: &str = "alpha\nbeta gamma\nsame 1\nsame 2\nsame 3\nsame 4\nsame 5\nsame 6\nsame 7\nTAB\tthere\nright only\nwide 日本語 text that runs on and on past forty columns for wrapping too\nesc \u{1b}[32m\u{202e}txt.exe\nend\n";

fn variant_rules() -> TextRules {
    let mut rules = TextRules::default();
    // Embedded whitespace unimportant: `beta  gamma` / `beta gamma` is an unimportant change.
    rules.importance.embedded_whitespace = false;
    rules
}

#[test]
fn a7_3_interleaved_variants() {
    let dir = fixtures("interleaved").join("variants");
    let cmp = compare_bytes(
        ("left.txt", LEFT.as_bytes()),
        ("right.txt", RIGHT.as_bytes()),
        &variant_rules(),
    );
    let mut defaults = ReportDefaults::new("interleaved");
    defaults.width = 80;
    defaults.context_lines = 1;
    let s = |v: &[&str]| v.iter().map(|w| (*w).to_owned()).collect::<Vec<_>>();
    let variants: [(&str, &[&str], &[&str]); 16] = [
        ("display-all.txt", &["display-all"], &[]),
        ("display-mismatches.txt", &["display-mismatches"], &[]),
        ("display-context.txt", &["display-context"], &[]),
        ("display-none.txt", &["display-none"], &[]),
        ("display-none.html", &["display-none"], &["html-color"]),
        (
            "display-context.html",
            &["display-context"],
            &["html-color"],
        ),
        ("ignore-unimportant.txt", &["ignore-unimportant"], &[]),
        (
            "ignore-unimportant.html",
            &["ignore-unimportant", "strikeout-left-diffs"],
            &["html-color"],
        ),
        ("no-line-numbers.txt", &["no-line-numbers"], &[]),
        ("no-line-numbers.html", &["no-line-numbers"], &["html-mono"]),
        ("wrap-none-40.txt", &[], &["wrap-none"]),
        ("wrap-word-40.txt", &[], &["wrap-word"]),
        ("wrap-character-40.txt", &[], &["wrap-character"]),
        ("wrap-none-40.html", &[], &["wrap-none", "html-color"]),
        ("wrap-word-40.html", &[], &["wrap-word", "html-mono"]),
        (
            "strike-both-40.txt",
            &["strikeout-left-diffs", "strikeout-right-diffs"],
            &[],
        ),
    ];
    for (file, options, output) in variants {
        let mut d = defaults.clone();
        if file.contains("-40") {
            d.width = 40;
        }
        let bytes = render(&cmp, &d, &s(options), &s(output));
        check(&dir, &format!("expected.{file}"), &bytes);
    }
    // The variants' bytes say what the layout promises.
    let all = String::from_utf8(render(&cmp, &defaults, &s(&["display-all"]), &[])).unwrap();
    assert!(
        all.contains("\\u{1b}[31m\\u{202e}txt.exe"),
        "controls and bidi escaped: {all}"
    );
    assert!(!all.contains('\u{1b}') && !all.contains('\u{202e}'));
    let context =
        String::from_utf8(render(&cmp, &defaults, &s(&["display-context"]), &[])).unwrap();
    assert!(
        context.contains("… 5 lines hidden …"),
        "a hidden run: {context}"
    );
    let none = String::from_utf8(render(&cmp, &defaults, &s(&["display-none"]), &[])).unwrap();
    let body: Vec<&str> = none.split("\n\n").nth(1).unwrap().lines().collect();
    assert_eq!(
        body.len(),
        1,
        "display-none: the header and one hidden-rows line: {none}"
    );
    let hidden = format!("… {} lines hidden …", cmp.row_count());
    assert_eq!(body[0].trim_start(), hidden, "{none}");
}

/// The mark table of §1.7.5, row by row: same, changed important (left then right), changed unimportant (with and
/// without ignore-unimportant), orphans of each side and importance.
#[test]
fn a7_3_mark_table() {
    let mut rules = variant_rules();
    rules.importance.orphans_always_important = false;
    let cmp = compare_bytes(
        ("l.txt", b"same\nimportant\nbeta  gamma\nleft\n\nend\n"),
        ("r.txt", b"same\nIMPORTANT!\nbeta gamma\nend\nright\n"),
        &rules,
    );
    let defaults = ReportDefaults::new("interleaved");
    let marks = |iu: bool| -> Vec<(String, String)> {
        let options: Vec<String> = if iu {
            vec!["ignore-unimportant".into()]
        } else {
            vec![]
        };
        let out = String::from_utf8(render(&cmp, &defaults, &options, &[])).unwrap();
        let body = out.split("\n\n").nth(1).unwrap().to_owned();
        body.lines()
            .map(|line| {
                // The mark: columns 4 and 5 (a line without text loses its trailing spaces).
                let mark: String = format!("{:<6}", line).get(4..6).unwrap_or("  ").to_owned();
                (mark, line.get(7..).unwrap_or("").to_owned())
            })
            .collect()
    };
    let got = marks(false);
    let want: Vec<(&str, &str)> = vec![
        ("  ", "same"),
        ("-!", "important"),
        ("+!", "IMPORTANT!"),
        ("-~", "beta  gamma"),
        ("+~", "beta gamma"),
        ("-!", "left"),
        ("-~", ""),
        ("  ", "end"),
        ("+!", "right"),
    ];
    let got_ref: Vec<(&str, &str)> = got.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    assert_eq!(got_ref, want, "{got:#?}");
    let got = marks(true);
    let got_ref: Vec<(&str, &str)> = got.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let want: Vec<(&str, &str)> = vec![
        ("  ", "same"),
        ("-!", "important"),
        ("+!", "IMPORTANT!"),
        ("- ", "beta  gamma"),
        ("+ ", "beta gamma"),
        ("-!", "left"),
        ("- ", ""),
        ("  ", "end"),
        ("+!", "right"),
    ];
    assert_eq!(got_ref, want, "{got:#?}");
}

/// Pieces of a line broken by the characters-per-line limit keep the line's number (the second piece repeats it).
#[test]
fn a7_3_characters_per_line_pieces_keep_their_number() {
    let load = LoadOptions {
        chars_per_line_limit: Some(10),
        ..LoadOptions::default()
    };
    let cmp = compare_bytes_with(
        ("l.txt", b"0123456789abcdefghij\nx\n"),
        ("r.txt", b"0123456789abcdefghiJ\nx\n"),
        &TextRules::default(),
        &load,
    );
    let defaults = ReportDefaults::new("interleaved");
    let bytes = render(&cmp, &defaults, &[], &[]);
    let dir = fixtures("interleaved").join("pieces");
    check(&dir, "expected.cpl.txt", &bytes);
    let text = String::from_utf8(bytes).unwrap();
    let body: Vec<&str> = text.split("\n\n").nth(1).unwrap().lines().collect();
    assert_eq!(body[0], "1 1    0123456789", "{text}");
    assert_eq!(body[1], "1   -! abcdefghij", "{text}");
    assert_eq!(body[2], "  1 +! abcdefghiJ", "{text}");
    check(
        &dir,
        "expected.cpl.html",
        &render(&cmp, &defaults, &[], &["html-color".into()]),
    );
}
