//! M2 W7 acceptance A7.2 (D-196, D-209, D-210): [`ReportRequest`] and the vocabulary of §1.7.4. Every word in every
//! spelling, each slot's default, each conflict, wrong kind, unknown, `column-vcs`, unused words and their warning, the
//! layout × format and target matrices cell by cell, the scope with each layout; P-RPTREQ: `parse(words(r)) == r`,
//! `words(parse(w))` is a fixed point, and any permutation of a valid list parses to the same request.

mod common;

use hikasa_core::{Classify, ErrorKind, Side, WarningCode};
use hikasa_folder::select::Selection;
use hikasa_folder::{FolderDisplayFilter, FolderMode, NodeId};
use hikasa_report::{
    DisplayMode, FolderColumns, FolderLayout, Orientation, OutputFormat, PatchStyle, PrintBlock,
    ReportBody, ReportDefaults, ReportError, ReportKind, ReportRequest, ReportScope, ReportWords,
    StrikeOut, TargetKind, TextLayout, TextReportOptions, Wrap,
};
use proptest::prelude::*;

use common::config;

fn words(layout: &str, options: &[&str], output: &[&str]) -> ReportWords {
    ReportWords {
        layout: Some(layout.to_owned()),
        options: options.iter().map(|s| (*s).to_owned()).collect(),
        output_options: output.iter().map(|s| (*s).to_owned()).collect(),
        title: None,
    }
}

fn defaults() -> ReportDefaults {
    ReportDefaults::new("side-by-side")
}

fn text(layout: &str, options: &[&str], output: &[&str]) -> Result<ReportRequest, ReportError> {
    ReportRequest::parse(
        ReportKind::Text,
        &words(layout, options, output),
        &defaults(),
    )
}

fn folder(layout: &str, options: &[&str], output: &[&str]) -> Result<ReportRequest, ReportError> {
    ReportRequest::parse(
        ReportKind::Folder,
        &words(layout, options, output),
        &defaults(),
    )
}

fn text_opts(req: &ReportRequest) -> TextReportOptions {
    match &req.body {
        ReportBody::Text(opts) => *opts,
        other => panic!("not text: {other:?}"),
    }
}

fn folder_opts(req: &ReportRequest) -> hikasa_report::FolderReportOptions {
    match &req.body {
        ReportBody::Folder { options, .. } => *options,
        other => panic!("not folder: {other:?}"),
    }
}

#[test]
fn a7_2_layout_words_in_every_spelling() {
    let ctx = defaults().context_lines;
    let cases = [
        ("side-by-side", TextLayout::SideBySide),
        ("sbs", TextLayout::SideBySide),
        ("Side_By_Side", TextLayout::SideBySide),
        ("summary", TextLayout::Summary),
        ("interleaved", TextLayout::Interleaved),
        ("INTERLEAVED", TextLayout::Interleaved),
        (
            "patch",
            TextLayout::Patch(PatchStyle::Unified { lines: ctx }),
        ),
        ("statistics", TextLayout::Statistics),
        ("stats", TextLayout::Statistics),
        ("xml", TextLayout::Xml),
        ("unified", TextLayout::AnnotatedUnified { lines: ctx }),
        (
            "annotated-unified",
            TextLayout::AnnotatedUnified { lines: ctx },
        ),
    ];
    for (word, layout) in cases {
        assert_eq!(
            text_opts(&text(word, &[], &[]).unwrap()).layout,
            layout,
            "{word}"
        );
    }
    let cases = [
        ("side-by-side", FolderLayout::SideBySide),
        ("sbs", FolderLayout::SideBySide),
        ("summary", FolderLayout::Summary),
        ("xml", FolderLayout::Xml),
        ("XML", FolderLayout::Xml),
        ("list", FolderLayout::List),
        ("json", FolderLayout::Json),
    ];
    for (word, layout) in cases {
        assert_eq!(
            folder_opts(&folder(word, &[], &[]).unwrap()).layout,
            layout,
            "{word}"
        );
    }
    // Absent: the adapter's default.
    let d = ReportDefaults::new("interleaved");
    let req = ReportRequest::parse(ReportKind::Text, &ReportWords::default(), &d).unwrap();
    assert_eq!(text_opts(&req).layout, TextLayout::Interleaved);
    // The other kind's layout, and no layout at all.
    for (kind, word) in [
        (ReportKind::Text, "list"),
        (ReportKind::Text, "json"),
        (ReportKind::Folder, "interleaved"),
        (ReportKind::Folder, "patch"),
        (ReportKind::Folder, "statistics"),
        (ReportKind::Folder, "unified"),
        (ReportKind::Text, "tree"),
        (ReportKind::Folder, "tree"),
    ] {
        let err = ReportRequest::parse(kind, &words(word, &[], &[]), &defaults()).unwrap_err();
        assert!(
            matches!(err, ReportError::InvalidOption { .. }),
            "{word}: {err}"
        );
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
        assert!(err.to_string().contains(word), "{err}");
    }
}

#[test]
fn a7_2_text_option_words_in_every_spelling() {
    let ctx = defaults().context_lines;
    let display = |w: &str| text_opts(&text("sbs", &[w], &[]).unwrap()).display;
    for (word, mode) in [
        ("display-all", DisplayMode::All),
        ("DISPLAY_ALL", DisplayMode::All),
        ("display-mismatches", DisplayMode::Mismatches),
        ("display-differences", DisplayMode::Mismatches),
        ("display-context", DisplayMode::Context { lines: ctx }),
        ("display-matches", DisplayMode::Matches),
        ("display-same", DisplayMode::Matches),
        ("display-none", DisplayMode::None),
        (" display_none ", DisplayMode::None),
    ] {
        assert_eq!(display(word), mode, "{word}");
    }
    let opts = |options: &[&str], output: &[&str]| {
        text_opts(&text("interleaved", options, output).unwrap())
    };
    assert!(opts(&["ignore-unimportant"], &[]).ignore_unimportant);
    assert!(opts(&["Ignore_Unimportant"], &[]).ignore_unimportant);
    assert!(opts(&["line-numbers"], &[]).line_numbers);
    assert!(!opts(&["no-line-numbers"], &[]).line_numbers);
    assert!(!opts(&["NO_LINE_NUMBERS"], &[]).line_numbers);
    assert_eq!(opts(&["strikeout-left-diffs"], &[]).strike, StrikeOut::LEFT);
    assert_eq!(
        opts(&["strikeout-right-diffs"], &[]).strike,
        StrikeOut::RIGHT
    );
    assert_eq!(
        opts(&["strikeout-left-diffs", "strikeout_right_diffs"], &[]).strike,
        StrikeOut::all()
    );
    let patch = |w: &str| text_opts(&text("patch", &[w], &[]).unwrap()).layout;
    assert_eq!(patch("patch-normal"), TextLayout::Patch(PatchStyle::Normal));
    assert_eq!(
        patch("patch-context"),
        TextLayout::Patch(PatchStyle::Context { lines: ctx })
    );
    assert_eq!(
        patch("PATCH_UNIFIED"),
        TextLayout::Patch(PatchStyle::Unified { lines: ctx })
    );
    for (word, wrap) in [
        ("wrap-none", Wrap::None),
        ("wrap-character", Wrap::Character),
        ("wrap-char", Wrap::Character),
        ("wrap-word", Wrap::Word),
        ("WRAP_WORD", Wrap::Word),
    ] {
        assert_eq!(opts(&[], &[word]).wrap, wrap, "{word}");
    }
    for (word, format) in [
        ("html-color", OutputFormat::Html { mono: false }),
        ("html-colour", OutputFormat::Html { mono: false }),
        ("html", OutputFormat::Html { mono: false }),
        ("HTML_MONO", OutputFormat::Html { mono: true }),
        ("text", OutputFormat::Text { ansi: false }),
        ("plain", OutputFormat::Text { ansi: false }),
        // `ansi` is plain text: colours come from the terminal, never from a word.
        ("ansi", OutputFormat::Text { ansi: false }),
    ] {
        assert_eq!(opts(&[], &[word]).format, format, "{word}");
    }
    let req = text("sbs", &[], &["html-custom:/a/B c.css"]).unwrap();
    assert_eq!(
        req.css.as_deref(),
        Some("/a/B c.css"),
        "the spec keeps its case"
    );
    assert_eq!(text_opts(&req).format, OutputFormat::Html { mono: false });
    let req = text("sbs", &[], &["HTML_CUSTOM:x.css"]).unwrap();
    assert_eq!(req.css.as_deref(), Some("x.css"));
    for (word, orientation) in [
        ("print-portrait", Orientation::Portrait),
        ("print-landscape", Orientation::Landscape),
        ("PRINT_LANDSCAPE", Orientation::Landscape),
    ] {
        let req = text("sbs", &[], &[word]).unwrap();
        assert_eq!(
            req.print,
            Some(PrintBlock {
                orientation,
                mono: false
            }),
            "{word}"
        );
    }
    for (word, mono) in [
        ("print-color", false),
        ("print-colour", false),
        ("print-mono", true),
    ] {
        let req = text("sbs", &[], &[word, "print-portrait"]).unwrap();
        assert_eq!(req.print_mono, mono, "{word}");
        assert_eq!(req.print.map(|p| p.mono), Some(mono));
        // Without an orientation there is no print block (it is the printer's palette).
        let req = text("sbs", &[], &[word]).unwrap();
        assert_eq!((req.print, req.print_mono), (None, mono));
    }
    // A list in one element, as a script line or the CLI gives it.
    let o = opts(
        &["display-context, line-numbers ,strikeout-left-diffs"],
        &["wrap-word,html-mono"],
    );
    assert_eq!(o.display, DisplayMode::Context { lines: ctx });
    assert_eq!(o.strike, StrikeOut::LEFT);
    assert_eq!(
        (o.wrap, o.format),
        (Wrap::Word, OutputFormat::Html { mono: true })
    );
}

#[test]
fn a7_2_folder_option_words_in_every_spelling() {
    let d = defaults();
    let display = |w: &str| folder_opts(&folder("list", &[w], &[]).unwrap()).display;
    for (word, filter) in [
        ("display-all", FolderDisplayFilter::All),
        ("display-mismatches", FolderDisplayFilter::Differences),
        ("display-differences", FolderDisplayFilter::Differences),
        ("display-matches", FolderDisplayFilter::Same),
        ("display-same", FolderDisplayFilter::Same),
        ("display-none", FolderDisplayFilter::None),
        ("display-no-orphans", FolderDisplayFilter::NoOrphans),
        (
            "display-mismatches-no-orphans",
            FolderDisplayFilter::DifferencesNoOrphans,
        ),
        (
            "display-differences-no-orphans",
            FolderDisplayFilter::DifferencesNoOrphans,
        ),
        ("display-orphans", FolderDisplayFilter::Orphans),
        ("display-left-newer", FolderDisplayFilter::LeftNewer),
        ("display-right-newer", FolderDisplayFilter::RightNewer),
        (
            "display-left-newer-orphans",
            FolderDisplayFilter::LeftNewerAndLeftOrphans,
        ),
        (
            "display-left-newer-and-left-orphans",
            FolderDisplayFilter::LeftNewerAndLeftOrphans,
        ),
        (
            "display-right-newer-orphans",
            FolderDisplayFilter::RightNewerAndRightOrphans,
        ),
        ("display-left-orphans", FolderDisplayFilter::LeftOrphans),
        ("DISPLAY_RIGHT_ORPHANS", FolderDisplayFilter::RightOrphans),
    ] {
        let got = display(word);
        assert_eq!(got.filter, filter, "{word}");
        assert_eq!(
            got.folders, d.folder_display.folders,
            "{word}: the folders stay the default's"
        );
    }
    // `display-context`: differences with the folder structure, whatever the default's folders.
    let mut only_files = defaults();
    only_files.folder_display.folders = FolderMode::OnlyFiles;
    let req = ReportRequest::parse(
        ReportKind::Folder,
        &words("list", &["display-context"], &[]),
        &only_files,
    )
    .unwrap();
    let got = folder_opts(&req).display;
    assert_eq!(
        (got.filter, got.folders),
        (
            FolderDisplayFilter::Differences,
            FolderMode::CompareStructure
        )
    );
    let cols = |w: &[&str]| folder_opts(&folder("sbs", w, &[]).unwrap()).columns;
    for (word, column) in [
        ("column-size", FolderColumns::SIZE),
        ("column-timestamp", FolderColumns::MODIFIED),
        ("column-modified", FolderColumns::MODIFIED),
        ("column-mtime", FolderColumns::MODIFIED),
        ("column-attributes", FolderColumns::ATTRIBUTES),
        ("column-crc", FolderColumns::CRC),
        ("column-owner", FolderColumns::OWNER),
        ("column-group", FolderColumns::GROUP),
        ("COLUMN_STATUS", FolderColumns::STATUS),
        ("column-none", FolderColumns::empty()),
    ] {
        assert_eq!(cols(&[word]), column, "{word}");
    }
    assert_eq!(
        cols(&["column-crc", "column-size", "column-crc"]),
        FolderColumns::CRC | FolderColumns::SIZE,
        "columns accumulate"
    );
    assert_eq!(
        cols(&["column-none", "column-none"]),
        FolderColumns::empty()
    );
    let req = folder("sbs", &["include-file-links"], &["html-mono"]).unwrap();
    let ReportBody::Folder {
        links: Some(links),
        options,
    } = &req.body
    else {
        panic!("links");
    };
    assert_eq!(options.format, OutputFormat::Html { mono: true });
    assert_eq!(links.text.layout, TextLayout::SideBySide);
    assert_eq!(links.text.display, DisplayMode::All);
    assert!(links.text.line_numbers);
    assert_eq!(links.text.wrap, Wrap::Character);
    assert_eq!(
        links.text.format,
        OutputFormat::Html { mono: true },
        "the folder report's palette"
    );
    assert!(
        folder_opts(&folder("list", &["ignore-unimportant"], &[]).unwrap())
            .display
            .ignore_unimportant
    );
}

#[test]
fn a7_2_each_slot_has_its_default() {
    let mut d = ReportDefaults::new("interleaved");
    d.text_display = DisplayMode::Mismatches;
    d.line_numbers = false;
    d.width = 99;
    d.context_lines = 7;
    let req = ReportRequest::parse(ReportKind::Text, &ReportWords::default(), &d).unwrap();
    let o = text_opts(&req);
    assert_eq!(o.layout, TextLayout::Interleaved);
    assert_eq!(o.display, DisplayMode::Mismatches);
    assert!(!o.ignore_unimportant);
    assert!(!o.line_numbers);
    assert_eq!(o.width, 99);
    assert_eq!(o.strike, StrikeOut::empty());
    assert_eq!(o.wrap, Wrap::Character);
    assert_eq!(o.format, OutputFormat::Text { ansi: false });
    assert_eq!(
        (req.css.clone(), req.print, req.print_mono),
        (None, None, false)
    );
    assert_eq!(req.scope, ReportScope::All);
    assert!(req.unused.is_empty());
    let req = ReportRequest::parse(ReportKind::Text, &words("patch", &[], &[]), &d).unwrap();
    assert_eq!(
        text_opts(&req).layout,
        TextLayout::Patch(PatchStyle::Unified { lines: 7 })
    );
    let req = ReportRequest::parse(ReportKind::Text, &words("unified", &[], &[]), &d).unwrap();
    assert_eq!(
        text_opts(&req).layout,
        TextLayout::AnnotatedUnified { lines: 7 }
    );
    let req = ReportRequest::parse(
        ReportKind::Text,
        &words("sbs", &["display-context"], &[]),
        &d,
    )
    .unwrap();
    assert_eq!(text_opts(&req).display, DisplayMode::Context { lines: 7 });
    let req = folder("sbs", &[], &[]).unwrap();
    let o = folder_opts(&req);
    assert_eq!(o.columns, FolderColumns::SIZE | FolderColumns::MODIFIED);
    assert_eq!(o.display, defaults().folder_display);
    assert!(matches!(req.body, ReportBody::Folder { links: None, .. }));
}

#[test]
fn a7_2_conflicts_whatever_the_order() {
    let pairs: [(&str, &[&str], &[&str], &str); 9] = [
        ("sbs", &["display-all", "display-context"], &[], "display"),
        (
            "sbs",
            &["line-numbers", "no-line-numbers"],
            &[],
            "line numbers",
        ),
        (
            "patch",
            &["patch-normal", "patch-unified"],
            &[],
            "patch style",
        ),
        ("sbs", &[], &["html-color", "html-mono"], "palette"),
        ("sbs", &[], &["html-mono", "html-custom:x.css"], "palette"),
        (
            "sbs",
            &[],
            &["html-custom:a.css", "html-custom:b.css"],
            "palette",
        ),
        ("sbs", &[], &["text", "html-color"], "palette"),
        (
            "sbs",
            &[],
            &["print-portrait", "print-landscape"],
            "orientation",
        ),
        ("sbs", &[], &["wrap-none", "wrap-word"], "wrap"),
    ];
    for (layout, options, output, slot) in pairs {
        for reversed in [false, true] {
            let (mut o, mut p) = (options.to_vec(), output.to_vec());
            if reversed {
                o.reverse();
                p.reverse();
            }
            let err = text(layout, &o, &p).unwrap_err();
            let ReportError::Conflict { slot: got, .. } = &err else {
                panic!("{options:?} {output:?}: {err}");
            };
            assert_eq!(*got, slot);
            assert_eq!(err.kind(), ErrorKind::InvalidInput);
        }
    }
    assert!(text("sbs", &[], &["print-color", "print-mono"]).is_err());
    // A word of a flag (`--html-mono`) against a word of a list (`html-color`): two different words of one slot.
    let err = text("sbs", &[], &["html-color", "html_mono"]).unwrap_err();
    assert_eq!(
        err.to_string(),
        "conflicting report options html-color and html_mono (both set the palette)"
    );
    let err = folder("list", &["display-orphans", "display-same"], &[]).unwrap_err();
    assert!(matches!(
        err,
        ReportError::Conflict {
            slot: "display",
            ..
        }
    ));
    let err = folder("sbs", &["column-size", "column-none"], &[]).unwrap_err();
    assert!(
        matches!(
            err,
            ReportError::Conflict {
                slot: "columns",
                ..
            }
        ),
        "{err}"
    );
    let err = folder("sbs", &["column-none", "column-crc"], &[]).unwrap_err();
    assert!(matches!(
        err,
        ReportError::Conflict {
            slot: "columns",
            ..
        }
    ));
    // The same word twice, or two spellings of one value, are one.
    assert!(text("sbs", &["display-all", "DISPLAY_ALL"], &[]).is_ok());
    assert!(text("sbs", &["display-mismatches", "display-differences"], &[]).is_ok());
    assert!(text("sbs", &[], &["html-color", "html-colour"]).is_ok());
}

#[test]
fn a7_2_wrong_kind_unknown_and_unsupported() {
    for word in [
        "column-crc",
        "include-file-links",
        "display-orphans",
        "display-left-newer",
    ] {
        let err = text("sbs", &[word], &[]).unwrap_err();
        assert!(
            matches!(&err, ReportError::InvalidOption { option: "text report option", value, .. } if value == word),
            "{word}: {err}"
        );
    }
    for word in [
        "patch-normal",
        "wrap-none",
        "strikeout-left-diffs",
        "line-numbers",
        "no-line-numbers",
    ] {
        let err = folder("sbs", &[word], &[]).unwrap_err();
        assert!(
            matches!(&err, ReportError::InvalidOption { option: "folder report option", value, .. } if value == word),
            "{word}: {err}"
        );
    }
    for word in [
        "bogus",
        "display-bogus",
        "column-bogus",
        "wrap-line",
        "patch-ed",
        "html-custom:",
        "column-",
    ] {
        let err = text("sbs", &[word], &[]).unwrap_err();
        assert!(
            matches!(err, ReportError::InvalidOption { .. }),
            "{word}: {err}"
        );
        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }
    for word in ["column-vcs", "column-revision", "COLUMN_VERSION"] {
        let err = folder("sbs", &[word], &[]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Unsupported, "{word}");
        assert_eq!(
            err.to_string(),
            "the version control columns (column-vcs, column-revision, column-version) are not available"
        );
    }
}

#[test]
fn a7_2_unused_words_and_their_warning() {
    let req = text(
        "sbs",
        &["strikeout-left-diffs", "display-all"],
        &["print-mono"],
    )
    .unwrap();
    assert_eq!(req.unused, ["strikeout-left-diffs"]);
    let w = req.unused_warning().unwrap();
    assert_eq!(w.code, WarningCode::ReportOptionUnused);
    assert_eq!(w.message, "side-by-side does not use: strikeout-left-diffs");
    let req = text(
        "patch",
        &["display_mismatches", "line-numbers"],
        &["wrap-word", "print-landscape"],
    )
    .unwrap();
    assert_eq!(
        req.unused,
        [
            "display-mismatches",
            "line-numbers",
            "print-landscape",
            "wrap-word"
        ]
    );
    assert_eq!(
        req.unused_warning().unwrap().message,
        "patch does not use: display-mismatches, line-numbers, print-landscape, wrap-word"
    );
    let req = text("summary", &["display-context"], &[]).unwrap();
    assert_eq!(req.unused, ["display-context"]);
    let req = folder("list", &["column-crc", "include-file-links"], &[]).unwrap();
    assert_eq!(req.unused, ["column-crc", "include-file-links"]);
    assert!(matches!(req.body, ReportBody::Folder { links: None, .. }));
    let req = folder("summary", &["display-orphans"], &[]).unwrap();
    assert_eq!(req.unused, ["display-orphans"]);
    assert!(
        folder("sbs", &["column-crc"], &["html-color"])
            .unwrap()
            .unused_warning()
            .is_none()
    );
    // The palette is the exception: an HTML word with a layout that has none is refused, as in M1.
    for layout in ["patch", "statistics", "unified", "xml"] {
        let err = text(layout, &[], &["html-color"]).unwrap_err();
        assert!(
            matches!(err, ReportError::Unsupported { format: "html", .. }),
            "{layout}: {err}"
        );
    }
    for layout in ["list", "json", "xml"] {
        let err = folder(layout, &[], &["html-mono"]).unwrap_err();
        assert!(
            matches!(err, ReportError::Unsupported { format: "html", .. }),
            "{layout}: {err}"
        );
    }
    // XML and JSON have no text form: `text` is refused there too.
    assert!(matches!(
        text("xml", &[], &["text"]),
        Err(ReportError::Unsupported { format: "text", .. })
    ));
    assert!(matches!(
        folder("json", &[], &["text"]),
        Err(ReportError::Unsupported { format: "text", .. })
    ));
    assert!(text("statistics", &[], &["text"]).is_ok());
}

/// A wrap word a layout does not use is reported and changes no byte (D-210: the rule is side-by-side's and
/// interleaved's): the summary HTML with `wrap-none` or `wrap-word` equals the summary HTML without it.
#[test]
fn a7_2_an_unused_wrap_word_changes_no_byte() {
    let cmp =
        common::inputs::compare_strings("a\nb\n", "a\nc\n", &hikasa_text::TextRules::default());
    let render = |output: &[&str]| {
        let req = text("summary", &[], output).unwrap();
        let ReportBody::Text(opts) = &req.body else {
            unreachable!()
        };
        let meta = hikasa_report::ReportMeta::for_text(&cmp, opts, None, "hikasa test");
        let mut out = Vec::new();
        let warnings = hikasa_report::render_report(
            hikasa_report::ReportInput::Text(&cmp),
            &req,
            &meta,
            &hikasa_report::HtmlStyle::default(),
            None,
            &mut out,
        )
        .unwrap();
        (out, warnings)
    };
    let (plain, warnings) = render(&["html-color"]);
    assert!(warnings.is_empty(), "{warnings:?}");
    for wrap in ["wrap-none", "wrap-word"] {
        let (bytes, warnings) = render(&["html-color", wrap]);
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            String::from_utf8(plain.clone()).unwrap(),
            "{wrap}"
        );
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(warnings[0].code, WarningCode::ReportOptionUnused);
        assert_eq!(warnings[0].message, format!("summary does not use: {wrap}"));
    }
}

/// The layout × format matrix of §1.7.4, cell by cell: which palettes each layout takes.
#[test]
fn a7_2_layout_format_matrix() {
    // (layout, text, html)
    let text_cells = [
        ("unified", true, false),
        ("side-by-side", true, true),
        ("summary", true, true),
        ("interleaved", true, true),
        ("patch", true, false),
        ("statistics", true, false),
        ("xml", false, false),
    ];
    for (layout, plain, html) in text_cells {
        assert_eq!(text(layout, &[], &["text"]).is_ok(), plain, "{layout} text");
        for word in ["html-color", "html-mono", "html-custom:x.css"] {
            assert_eq!(text(layout, &[], &[word]).is_ok(), html, "{layout} {word}");
        }
    }
    let folder_cells = [
        ("list", true, false),
        ("side-by-side", true, true),
        ("summary", true, true),
        ("json", false, false),
        ("xml", false, false),
    ];
    for (layout, plain, html) in folder_cells {
        assert_eq!(
            folder(layout, &[], &["text"]).is_ok(),
            plain,
            "{layout} text"
        );
        for word in ["html-color", "html-mono", "html-custom:x.css"] {
            assert_eq!(
                folder(layout, &[], &[word]).is_ok(),
                html,
                "{layout} {word}"
            );
        }
    }
    // ANSI only on a terminal's stdout, for the layouts with an ANSI form.
    for (layout, ansi) in [
        ("unified", true),
        ("side-by-side", true),
        ("summary", true),
        ("interleaved", true),
        ("patch", true),
        ("statistics", false),
        ("xml", false),
    ] {
        let mut req = text(layout, &[], &[]).unwrap();
        req.apply_target(TargetKind::Stdout { ansi: true });
        assert_eq!(
            req.format() == (OutputFormat::Text { ansi: true }),
            ansi,
            "{layout}"
        );
        for target in [
            TargetKind::File,
            TargetKind::Clipboard,
            TargetKind::Stdout { ansi: false },
        ] {
            let mut req = text(layout, &[], &[]).unwrap();
            req.apply_target(target);
            assert_eq!(
                req.format(),
                OutputFormat::Text { ansi: false },
                "{layout} {target:?}"
            );
        }
    }
    let mut req = text("sbs", &[], &["html-color"]).unwrap();
    req.apply_target(TargetKind::Stdout { ansi: true });
    assert_eq!(
        req.format(),
        OutputFormat::Html { mono: false },
        "HTML stays HTML on a terminal"
    );
}

/// The target matrix of §1.7.4, cell by cell.
#[test]
fn a7_2_target_matrix() {
    let targets = [
        TargetKind::Stdout { ansi: false },
        TargetKind::Stdout { ansi: true },
        TargetKind::File,
        TargetKind::Clipboard,
        TargetKind::Printer,
        TargetKind::Browser,
    ];
    let html_form = |kind: ReportKind, layout: &str| match kind {
        ReportKind::Text => matches!(layout, "side-by-side" | "summary" | "interleaved"),
        ReportKind::Folder => matches!(layout, "side-by-side" | "summary"),
    };
    for (kind, layouts) in [
        (
            ReportKind::Text,
            &[
                "unified",
                "side-by-side",
                "summary",
                "interleaved",
                "patch",
                "statistics",
                "xml",
            ][..],
        ),
        (
            ReportKind::Folder,
            &["list", "side-by-side", "summary", "json", "xml"][..],
        ),
    ] {
        for layout in layouts {
            let req = ReportRequest::parse(kind, &words(layout, &[], &[]), &defaults()).unwrap();
            for target in targets {
                let ok = req.check_target(target).is_ok();
                let want = match target {
                    TargetKind::Printer | TargetKind::Browser => html_form(kind, layout),
                    _ => true,
                };
                assert_eq!(ok, want, "{kind:?} {layout} {target:?}");
                if !ok {
                    let err = req.check_target(target).unwrap_err();
                    assert_eq!(err.kind(), ErrorKind::Unsupported);
                }
            }
        }
    }
    // Links: folder side-by-side HTML on a file, or the printer's and the browser's HTML.
    let linked = folder("sbs", &["include-file-links"], &["html-color"]).unwrap();
    for (target, ok) in [
        (TargetKind::File, true),
        (TargetKind::Printer, true),
        (TargetKind::Browser, true),
        (TargetKind::Clipboard, false),
        (TargetKind::Stdout { ansi: false }, false),
        (TargetKind::Stdout { ansi: true }, false),
    ] {
        assert_eq!(linked.check_target(target).is_ok(), ok, "{target:?}");
        if !ok {
            assert_eq!(
                linked.check_target(target).unwrap_err().to_string(),
                "file reports need a folder on disk"
            );
        }
    }
    let plain = folder("sbs", &["include-file-links"], &[]).unwrap();
    let err = plain.check_target(TargetKind::File).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unsupported);
    assert!(
        err.to_string()
            .starts_with("file reports need side-by-side HTML")
    );
    assert!(plain.check_target(TargetKind::Printer).is_ok());
    // The printer: the print palette and a portrait print block; html-color and html-mono are unused there.
    let mut req = text("sbs", &[], &["html-color", "print-mono"]).unwrap();
    req.apply_target(TargetKind::Printer);
    assert_eq!(req.format(), OutputFormat::Html { mono: true });
    assert_eq!(
        req.print,
        Some(PrintBlock {
            orientation: Orientation::Portrait,
            mono: true
        })
    );
    assert_eq!(req.unused, ["html-color"]);
    let mut req = text("interleaved", &[], &["print-landscape"]).unwrap();
    req.apply_target(TargetKind::Printer);
    assert_eq!(
        req.print,
        Some(PrintBlock {
            orientation: Orientation::Landscape,
            mono: false
        })
    );
    let mut req = text("summary", &[], &["html-mono"]).unwrap();
    req.apply_target(TargetKind::Browser);
    assert_eq!(req.format(), OutputFormat::Html { mono: false });
    assert_eq!(req.print, None, "the browser has no print block");
    let mut linked = folder("sbs", &["include-file-links"], &["print-mono"]).unwrap();
    linked.apply_target(TargetKind::Printer);
    let ReportBody::Folder {
        links: Some(links), ..
    } = &linked.body
    else {
        panic!()
    };
    assert_eq!(
        links.text.format,
        OutputFormat::Html { mono: true },
        "pair reports share the palette"
    );
}

#[test]
fn a7_2_scope_with_each_layout() {
    let lines = ReportScope::TextLines(hikasa_report::LineScope {
        side: Side::Left,
        first: 2,
        last: 5,
    });
    for (layout, ok) in [
        ("side-by-side", true),
        ("interleaved", true),
        ("xml", true),
        ("summary", false),
        ("statistics", false),
        ("patch", false),
        ("unified", false),
    ] {
        let mut req = text(layout, &[], &[]).unwrap();
        req.scope = lines.clone();
        assert_eq!(req.check_scope().is_ok(), ok, "{layout}");
        assert_eq!(req.check_target(TargetKind::File).is_ok(), ok, "{layout}");
        if !ok {
            assert_eq!(
                req.check_scope().unwrap_err().kind(),
                ErrorKind::InvalidInput
            );
        }
    }
    let mut selection = Selection::default();
    selection.insert(NodeId(1), hikasa_folder::select::SideSet::all());
    for (layout, ok) in [
        ("list", true),
        ("side-by-side", true),
        ("xml", true),
        ("summary", false),
        ("json", false),
    ] {
        let mut req = folder(layout, &[], &[]).unwrap();
        req.scope = ReportScope::FolderRows(selection.clone());
        assert_eq!(req.check_scope().is_ok(), ok, "{layout}");
    }
    // A scope of the other kind, an empty range.
    let mut req = folder("list", &[], &[]).unwrap();
    req.scope = lines;
    assert_eq!(
        req.check_scope().unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    let mut req = text("sbs", &[], &[]).unwrap();
    req.scope = ReportScope::TextLines(hikasa_report::LineScope {
        side: Side::Right,
        first: 5,
        last: 2,
    });
    assert!(req.check_scope().is_err());
    req.scope = ReportScope::TextLines(hikasa_report::LineScope {
        side: Side::Right,
        first: 0,
        last: 2,
    });
    assert!(req.check_scope().is_err());
}

#[test]
fn a7_2_ignore_unimportant_and_canonical_words() {
    assert!(
        text("patch", &["ignore-unimportant"], &[])
            .unwrap()
            .ignore_unimportant()
    );
    assert!(!text("patch", &[], &[]).unwrap().ignore_unimportant());
    assert!(
        folder("json", &["ignore_unimportant"], &[])
            .unwrap()
            .ignore_unimportant()
    );
    let req = text(
        "Interleaved",
        &[
            "wrap_word",
            "strikeout-right-diffs",
            "Display_Context",
            "ignore-unimportant",
            "no-line-numbers",
        ],
        &["print-landscape", "HTML_MONO", "print-mono"],
    )
    .unwrap();
    let w = req.words();
    assert_eq!(w.layout.as_deref(), Some("interleaved"));
    assert_eq!(
        w.options,
        [
            "display-context",
            "ignore-unimportant",
            "no-line-numbers",
            "strikeout-right-diffs"
        ]
    );
    assert_eq!(
        w.output_options,
        ["html-mono", "print-mono", "print-landscape", "wrap-word"]
    );
    let req = folder(
        "sbs",
        &["column-crc", "column-size", "include-file-links"],
        &["html-custom:s.css"],
    )
    .unwrap();
    let w = req.words();
    assert_eq!(
        w.options,
        [
            "display-all",
            "column-size",
            "column-crc",
            "include-file-links"
        ]
    );
    assert_eq!(w.output_options, ["html-custom:s.css"]);
}

// ---- P-RPTREQ ----------------------------------------------------------------------------------------------------

/// The words a random request picks from, per slot: one word of a slot at most (two would conflict), except the
/// strike-outs and the columns.
const TEXT_SLOTS: [&[&str]; 9] = [
    &[
        "display-all",
        "display-mismatches",
        "display-context",
        "display-matches",
        "display-none",
        "display_differences",
    ],
    &["ignore-unimportant"],
    &["line-numbers", "no-line-numbers"],
    &["strikeout-left-diffs", "strikeout-right-diffs"],
    &["patch-normal", "patch-context", "patch-unified"],
    &["html-color", "html-mono", "html-custom:my.css", "text"],
    &["print-color", "print-mono"],
    &["print-portrait", "print-landscape"],
    &["wrap-none", "wrap-character", "wrap-word"],
];
const TEXT_LAYOUTS: [&str; 7] = [
    "side-by-side",
    "summary",
    "interleaved",
    "patch",
    "statistics",
    "xml",
    "unified",
];
const FOLDER_SLOTS: [&[&str]; 8] = [
    &[
        "display-all",
        "display-mismatches",
        "display-context",
        "display-orphans",
        "display-left-newer-orphans",
        "display-none",
    ],
    &["ignore-unimportant"],
    &[
        "column-size",
        "column-timestamp",
        "column-crc",
        "column-status",
        "column-owner",
    ],
    &["include-file-links"],
    &["html-color", "html-mono", "html-custom:my.css", "text"],
    &["print-color", "print-mono"],
    &["print-portrait", "print-landscape"],
    &["column-none"],
];
const FOLDER_LAYOUTS: [&str; 5] = ["side-by-side", "summary", "list", "json", "xml"];

/// A random word list: a layout and, per slot, nothing, one word, or (strike, columns) several.
fn word_lists() -> impl Strategy<Value = (ReportKind, ReportWords, Vec<usize>)> {
    (
        any::<bool>(),
        0usize..7,
        proptest::collection::vec(any::<u8>(), 9),
        any::<u64>(),
    )
        .prop_map(|(folder_kind, layout, picks, seed)| {
            let (kind, slots, layouts): (ReportKind, &[&[&str]], &[&str]) = if folder_kind {
                (ReportKind::Folder, &FOLDER_SLOTS, &FOLDER_LAYOUTS)
            } else {
                (ReportKind::Text, &TEXT_SLOTS, &TEXT_LAYOUTS)
            };
            let mut list = Vec::new();
            for (slot, pick) in slots.iter().zip(&picks) {
                let pick = usize::from(*pick);
                if pick % 3 == 0 {
                    continue;
                }
                let many = slot.len() > 1
                    && (slot[0].starts_with("strikeout") || slot[0].starts_with("column-size"));
                if many {
                    for (i, word) in slot.iter().enumerate() {
                        if (pick >> i) & 1 == 1 {
                            list.push((*word).to_owned());
                        }
                    }
                } else {
                    list.push(slot[pick % slot.len()].to_owned());
                }
            }
            // `column-none` with a column conflicts: keep one of them.
            if list.iter().any(|w| w == "column-none")
                && list
                    .iter()
                    .any(|w| w.starts_with("column-") && w != "column-none")
            {
                list.retain(|w| w != "column-none");
            }
            let words = ReportWords {
                layout: Some(layouts[layout % layouts.len()].to_owned()),
                options: list,
                output_options: Vec::new(),
                title: (seed % 3 == 0).then(|| "T, itle".to_owned()),
            };
            // A permutation of the list, from the seed.
            let mut order: Vec<usize> = (0..words.options.len()).collect();
            let mut s = seed;
            for i in (1..order.len()).rev() {
                s = s
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                order.swap(i, (s >> 33) as usize % (i + 1));
            }
            (kind, words, order)
        })
}

proptest! {
    #![proptest_config(config("tests/proptest-regressions/request.txt"))]

    /// P-RPTREQ.
    #[test]
    fn p_rptreq_words_round_trip((kind, words, order) in word_lists()) {
        let d = defaults();
        let Ok(req) = ReportRequest::parse(kind, &words, &d) else {
            // A palette word the layout refuses: refused whatever the order.
            let permuted = ReportWords { options: order.iter().map(|i| words.options[*i].clone()).collect(), ..words.clone() };
            prop_assert!(ReportRequest::parse(kind, &permuted, &d).is_err());
            return Ok(());
        };
        let canonical = req.words();
        let again = ReportRequest::parse(kind, &canonical, &d).unwrap();
        prop_assert_eq!(&again, &req, "parse(words(r)) == r: {:?}", canonical);
        prop_assert_eq!(again.words(), canonical.clone(), "words(parse(w)) is a fixed point");
        let permuted = ReportWords { options: order.iter().map(|i| words.options[*i].clone()).collect(), ..words.clone() };
        let shuffled = ReportRequest::parse(kind, &permuted, &d).unwrap();
        prop_assert_eq!(&shuffled, &req, "a permutation parses to the same request");
        // Words split between the two lists, or joined into one element, mean the same.
        // (`html-custom:` takes the rest of a list, so it goes last.)
        let mut list: Vec<String> = words.options.clone();
        list.sort_by_key(|w| w.starts_with("html-custom:"));
        let joined = ReportWords { options: vec![list.join(",")], ..words.clone() };
        prop_assert_eq!(&ReportRequest::parse(kind, &joined, &d).unwrap(), &req);
    }
}
