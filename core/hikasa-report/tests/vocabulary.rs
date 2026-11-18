//! M2 W13c acceptance A13.16 (D-367): the report vocabulary of the dialogs. P-REPORTVOCAB — for each kind, every
//! listed layout and option word parses for its layout ([`ReportRequest::parse`]) and is used by it, every canonical
//! word the parser accepts for the kind (its tables, `request::layout_words` and `request::option_words`) is listed or
//! used by no layout of the kind, each choice's words exclude each other, defaults are listed words; random
//! selections of a layout's listed words parse with nothing unused.

mod common;

use hikasa_report::request::{layout_words, option_words};
use hikasa_report::vocabulary::{HTML_CUSTOM_WORD, LayoutEntry, ReportVocabulary, vocabulary};
use hikasa_report::{ReportDefaults, ReportError, ReportKind, ReportRequest, ReportWords};
use proptest::prelude::*;

use common::config;

const KINDS: [ReportKind; 5] = [
    ReportKind::Text,
    ReportKind::Folder,
    ReportKind::Table,
    ReportKind::Hex,
    ReportKind::Picture,
];

fn parse(kind: ReportKind, layout: &str, words: &[&str]) -> Result<ReportRequest, ReportError> {
    let words = ReportWords {
        layout: Some(layout.to_owned()),
        options: words.iter().map(|w| (*w).to_owned()).collect(),
        output_options: Vec::new(),
        title: None,
    };
    ReportRequest::parse(kind, &words, &ReportDefaults::new("side-by-side"))
}

/// A word as the parser would get it: `html-custom:` with a stylesheet path.
fn spelled(word: &str) -> String {
    if word == HTML_CUSTOM_WORD {
        format!("{word}/tmp/style.css")
    } else {
        word.to_owned()
    }
}

/// Every word a vocabulary lists for `layout` (the layout's options, its displays, its wrap, the outputs of its
/// forms, the print words of an HTML form, ignore-unimportant).
fn listed_for(vocab: &ReportVocabulary, layout: &LayoutEntry) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = layout
        .options
        .iter()
        .flat_map(|o| o.words.iter().copied())
        .collect();
    if layout.display {
        out.extend(vocab.displays.iter().map(|d| d.word));
    }
    if layout.wrap {
        out.extend(vocab.wrap.iter().flat_map(|w| w.words.iter().copied()));
    }
    if layout.forms.html {
        out.extend(
            vocab
                .outputs
                .iter()
                .map(|o| o.word)
                .filter(|w| *w != "text"),
        );
        out.extend(vocab.print.iter().flat_map(|p| p.words.iter().copied()));
    }
    if layout.forms.text {
        out.push("text");
    }
    if vocab.ignore_unimportant {
        out.push("ignore-unimportant");
    }
    out
}

#[test]
fn a13_16_every_listed_word_parses_for_its_layout_and_is_used() {
    for kind in KINDS {
        let vocab = vocabulary(kind);
        assert_eq!(vocab.kind, kind);
        assert!(!vocab.layouts.is_empty());
        for layout in vocab.layouts {
            let req = parse(kind, layout.word, &[])
                .unwrap_or_else(|e| panic!("{kind:?} layout {}: {e}", layout.word));
            assert_eq!(req.layout_word(), layout.word, "{kind:?}");
            for word in listed_for(vocab, layout) {
                let req = parse(kind, layout.word, &[&spelled(word)])
                    .unwrap_or_else(|e| panic!("{kind:?} {} {word}: {e}", layout.word));
                // `ignore-unimportant` and the print words are used by every layout that lists them.
                assert!(
                    req.unused.is_empty(),
                    "{kind:?} {}: {word} unused ({:?})",
                    layout.word,
                    req.unused
                );
            }
        }
    }
}

#[test]
fn a13_16_every_word_the_parser_accepts_is_listed() {
    for kind in KINDS {
        let vocab = vocabulary(kind);
        let layouts: Vec<&str> = vocab.layouts.iter().map(|l| l.word).collect();
        for word in layout_words(kind) {
            assert!(
                layouts.contains(word),
                "{kind:?}: the parser's layout {word} is not in the vocabulary"
            );
        }
        assert_eq!(
            layouts.len(),
            layout_words(kind).len(),
            "{kind:?}: a layout listed twice"
        );
        for word in option_words(kind) {
            let listed = vocab
                .layouts
                .iter()
                .any(|l| listed_for(vocab, l).contains(&word));
            if listed {
                continue;
            }
            // Not listed: the parser takes it, and no layout of the kind uses it (it would only be reported unused).
            for layout in vocab.layouts {
                match parse(kind, layout.word, &[&spelled(word)]) {
                    Ok(req) => assert!(
                        !req.unused.is_empty(),
                        "{kind:?}: {word} is used by {} but not listed",
                        layout.word
                    ),
                    Err(e) => panic!("{kind:?}: {word} with {}: {e}", layout.word),
                }
            }
        }
    }
}

#[test]
fn a13_16_choices_exclude_each_other_and_defaults_are_listed() {
    for kind in KINDS {
        let vocab = vocabulary(kind);
        for layout in vocab.layouts {
            let mut choices: Vec<_> = layout.options.to_vec();
            choices.extend(vocab.print.iter().copied().filter(|_| layout.forms.html));
            choices.extend(vocab.wrap.filter(|_| layout.wrap));
            for option in &choices {
                if let Some(default) = option.default {
                    assert!(
                        option.words.contains(&default),
                        "{kind:?} {}",
                        option.caption
                    );
                }
                if option.is_check() {
                    continue;
                }
                for (i, a) in option.words.iter().enumerate() {
                    for b in &option.words[i + 1..] {
                        assert!(
                            matches!(
                                parse(kind, layout.word, &[a, b]),
                                Err(ReportError::Conflict { .. })
                            ),
                            "{kind:?} {}: {a} and {b} do not exclude each other",
                            layout.word
                        );
                    }
                }
            }
            if layout.display {
                let displays: Vec<&str> = vocab.displays.iter().map(|d| d.word).collect();
                assert!(matches!(
                    parse(kind, layout.word, &[displays[0], displays[1]]),
                    Err(ReportError::Conflict { .. })
                ));
            }
        }
    }
}

#[test]
fn a13_16_a_word_outside_the_tables_is_refused() {
    // The parser's tables are the gate: every canonical word of a kind parses for some layout, a word of another
    // kind does not.
    for kind in KINDS {
        for word in option_words(kind) {
            let any = layout_words(kind)
                .iter()
                .any(|layout| parse(kind, layout, &[&spelled(word)]).is_ok());
            assert!(any, "{kind:?}: {word}");
        }
    }
    assert!(parse(ReportKind::Picture, "summary", &["context-3"]).is_err());
    assert!(parse(ReportKind::Hex, "summary", &["context-65"]).is_err());
    assert!(parse(ReportKind::Text, "list", &[]).is_err());
}

/// A random selection of a layout's words: one word of each choice or none, each check on or off, a display, an
/// output of its forms.
fn selection(
    vocab: &'static ReportVocabulary,
    layout: &'static LayoutEntry,
    picks: &[u8],
) -> Vec<String> {
    let mut picks = picks.iter().copied().cycle();
    let mut next = |n: usize| usize::from(picks.next().unwrap_or(0)) % n;
    let mut out = Vec::new();
    let mut choices: Vec<_> = layout.options.to_vec();
    choices.extend(vocab.wrap.filter(|_| layout.wrap));
    if layout.forms.html {
        choices.extend(vocab.print.iter().copied());
    }
    let mut columns = 0;
    for option in &choices {
        let pick = next(option.words.len() + 1);
        if pick == option.words.len() {
            continue;
        }
        let word = option.words[pick];
        // `column-none` excludes the other columns (the dialog's own rule is the parser's message).
        if word == "column-none" && columns > 0 {
            continue;
        }
        if word.starts_with("column-") {
            if out.iter().any(|w: &String| w == "column-none") {
                continue;
            }
            columns += 1;
        }
        out.push(word.to_owned());
    }
    if layout.display && !vocab.displays.is_empty() {
        out.push(vocab.displays[next(vocab.displays.len())].word.to_owned());
    }
    let outputs: Vec<&str> = vocab
        .outputs
        .iter()
        .map(|o| o.word)
        .filter(|w| {
            if *w == "text" {
                layout.forms.text
            } else {
                layout.forms.html
            }
        })
        .collect();
    if !outputs.is_empty() && next(2) == 0 {
        out.push(spelled(outputs[next(outputs.len())]));
    }
    if vocab.ignore_unimportant && next(2) == 0 {
        out.push("ignore-unimportant".to_owned());
    }
    out
}

proptest! {
    #![proptest_config(config("tests/proptest-regressions/vocabulary.txt"))]

    /// P-REPORTVOCAB: any selection the dialog can make from a layout's listed words parses, with nothing unused.
    #[test]
    fn p_reportvocab(kind in 0usize..5, layout in 0usize..8, picks in proptest::collection::vec(any::<u8>(), 1..24)) {
        let kind = KINDS[kind];
        let vocab = vocabulary(kind);
        let layout = &vocab.layouts[layout % vocab.layouts.len()];
        let mut words = selection(vocab, layout, &picks);
        // `html-custom:` takes the rest of a list: last.
        words.sort_by_key(|w| w.starts_with(HTML_CUSTOM_WORD));
        let refs: Vec<&str> = words.iter().map(String::as_str).collect();
        let req = parse(kind, layout.word, &refs);
        prop_assert!(req.is_ok(), "{:?} {} {:?}: {:?}", kind, layout.word, words, req.err());
        let req = req.unwrap();
        prop_assert!(req.unused.is_empty(), "{:?} {} {:?}: unused {:?}", kind, layout.word, words, req.unused);
    }
}

/// The view helpers give listed words only: every display a view's filter gives is one of the kind's displays, every
/// option word one of a layout's.
#[test]
fn a13_16_view_words_are_listed() {
    use hikasa_folder::{DisplayOptions, FolderDisplayFilter, FolderMode};
    use hikasa_report::vocabulary::{
        ViewWords, folder_view_words, hex_view_words, picture_view_words, table_view_words,
        text_view_words,
    };
    use hikasa_text::TextDisplayFilter;
    fn check(kind: ReportKind, words: &ViewWords) {
        let vocab = vocabulary(kind);
        if let Some(d) = words.display {
            assert!(vocab.displays.iter().any(|e| e.word == d), "{kind:?} {d}");
        }
        for w in words.on.iter().chain(&words.off) {
            assert!(
                vocab
                    .layouts
                    .iter()
                    .any(|l| l.options.iter().any(|o| o.words.contains(w))),
                "{kind:?} {w}"
            );
        }
        if let Some(f) = words.folders {
            assert!(vocab.folders.is_some_and(|o| o.words.contains(&f)), "{f}");
        }
    }
    for filter in [
        TextDisplayFilter::All,
        TextDisplayFilter::Differences,
        TextDisplayFilter::Same,
        TextDisplayFilter::None,
        TextDisplayFilter::Context { lines: 5 },
    ] {
        let w = text_view_words(filter);
        assert!(w.display.is_some());
        check(ReportKind::Text, &w);
    }
    assert_eq!(
        text_view_words(TextDisplayFilter::Context { lines: 5 }).context,
        Some(5)
    );
    for filter in FolderDisplayFilter::PRESETS {
        for folders in [
            FolderMode::CompareStructure,
            FolderMode::AlwaysShowFolders,
            FolderMode::OnlyFiles,
        ] {
            let display = DisplayOptions {
                filter,
                folders,
                ignore_unimportant: false,
                suppress_filters: false,
            };
            let w = folder_view_words(&display);
            assert!(w.display.is_some(), "{filter:?}");
            check(ReportKind::Folder, &w);
        }
    }
    for filter in [
        hikasa_table::RowFilter::ALL,
        hikasa_table::RowFilter::DIFFERENCES,
        hikasa_table::RowFilter::SAME,
        hikasa_table::RowFilter::NONE,
    ] {
        let w = table_view_words(filter);
        assert!(w.display.is_some());
        check(ReportKind::Table, &w);
    }
    for filter in [
        Some(hikasa_hex::DisplayFilter::All),
        Some(hikasa_hex::DisplayFilter::Differences),
        Some(hikasa_hex::DisplayFilter::Same),
        None,
    ] {
        for (context, addresses) in [(0, true), (64, false)] {
            check(ReportKind::Hex, &hex_view_words(filter, context, addresses));
        }
    }
    for mode in [
        hikasa_picture::DiffMode::Tolerance,
        hikasa_picture::DiffMode::Range,
        hikasa_picture::DiffMode::Blend { percent: 30 },
    ] {
        for board in [false, true] {
            check(ReportKind::Picture, &picture_view_words(mode, board));
        }
    }
}

/// The layout flags are the parser's (W13c review: mutation C3): `scope` is true exactly for the layouts whose request
/// takes a selection (`check_scope`), `display` exactly for those that use a display word.
#[test]
fn a13_16_scope_and_display_flags_are_the_parsers() {
    use hikasa_core::Side;
    use hikasa_report::{LineScope, ReportScope};
    for kind in KINDS {
        let vocab = vocabulary(kind);
        for layout in vocab.layouts {
            let mut req = parse(kind, layout.word, &[]).unwrap();
            req.scope = match kind {
                ReportKind::Text => ReportScope::TextLines(LineScope {
                    side: Side::Left,
                    first: 1,
                    last: 2,
                }),
                ReportKind::Folder => ReportScope::FolderRows(Default::default()),
                _ => ReportScope::All,
            };
            let scoped = req.scope != ReportScope::All && req.check_scope().is_ok();
            assert_eq!(layout.scope, scoped, "{kind:?} {}: scope", layout.word);
            if let Some(display) = vocab.displays.first() {
                let req = parse(kind, layout.word, &[display.word]).unwrap();
                assert_eq!(
                    layout.display,
                    req.unused.is_empty(),
                    "{kind:?} {}: display ({:?})",
                    layout.word,
                    req.unused
                );
            }
        }
    }
}

/// A text view's display filter is the report display that shows the same rows (W13c review: mutation C6): All →
/// `display-all`, Differences → `display-mismatches`, Same → `display-matches`, None → `display-none`, Context →
/// `display-context` with its lines.
#[test]
fn a13_16_text_view_words_name_the_views_display() {
    use hikasa_report::vocabulary::text_view_words;
    use hikasa_text::TextDisplayFilter;
    for (filter, word) in [
        (TextDisplayFilter::All, "display-all"),
        (TextDisplayFilter::Differences, "display-mismatches"),
        (TextDisplayFilter::Same, "display-matches"),
        (TextDisplayFilter::None, "display-none"),
        (TextDisplayFilter::Context { lines: 4 }, "display-context"),
    ] {
        assert_eq!(text_view_words(filter).display, Some(word), "{filter:?}");
    }
}
