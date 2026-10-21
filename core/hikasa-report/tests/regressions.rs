//! Regression tests of the wave-4 review findings that are not about patches (those are in `tests/apply.rs` and
//! `tests/props.rs`): bidirectional controls and unambiguous name escaping (decision 5), display widths of emoji
//! sequences, source line numbers under a characters-per-line limit and blank sizes of entries whose metadata is
//! unknown (decision 6), and the JSON shapes (decision 4).

mod common;

use std::sync::Arc;

use common::{TreeFile, compare_bytes, host, mspec, run_tree};
use hikasa_core::{CaseSensitivity, Ctl, ErrorKind, FileTime};
use hikasa_folder::{
    DisplayOptions, FolderComparison, FolderDisplayFilter, FolderMode, FolderRules,
};
use hikasa_report::{
    FolderColumns, FolderLayout, FolderReportOptions, OutputFormat, ReportMeta, TextLayout,
    TextReportOptions, render_folder_report, render_text_report, text_compare_json,
};
use hikasa_text::{
    CompareInput, CompiledFormat, LoadOptions, TextComparison, TextDisplayFilter, TextRules,
    ViewOptions, compare, load_document,
};
use hikasa_vfs::{Fault, MemMeta, MemoryVfs, VfsOp};
use unicode_width::UnicodeWidthStr;

/// A file time for the trees.
fn meta() -> MemMeta {
    MemMeta {
        mtime: Some(FileTime::parse("2026-01-01T00:00:00Z").unwrap()),
        ..MemMeta::default()
    }
}

/// Scans `/l` and `/r` holding `files` (path, content) on both sides.
fn tree(files: &[(&str, &[u8])], rules: &FolderRules, faults: &[Fault]) -> FolderComparison {
    let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
    vfs.insert_dir(&host("/l"), meta()).unwrap();
    vfs.insert_dir(&host("/r"), meta()).unwrap();
    for (path, content) in files {
        for base in ["/l", "/r"] {
            vfs.insert_file(&host(&format!("{base}/{path}")), content.to_vec(), meta())
                .unwrap();
        }
    }
    for f in faults {
        vfs.inject(f.clone());
    }
    run_tree(&TreeFile::default(), &vfs, rules).unwrap()
}

/// Renders a folder report as text.
fn folder_text(cmp: &FolderComparison, opts: &FolderReportOptions) -> String {
    let meta = ReportMeta::for_folder(cmp, opts, None, "hikasa test");
    let mut out = Vec::new();
    render_folder_report(cmp, opts, &meta, &mut out).unwrap();
    String::from_utf8(out).unwrap()
}

/// Renders a text report.
fn text(cmp: &TextComparison, opts: &TextReportOptions) -> String {
    let meta = ReportMeta::for_text(cmp, opts, None, "hikasa test");
    let mut out = Vec::new();
    render_text_report(cmp, opts, &meta, &mut out).unwrap();
    String::from_utf8(out).unwrap()
}

/// The table rows of a side-by-side report (after the header's blank line and the title row).
fn table(report: &str) -> Vec<&str> {
    report
        .lines()
        .skip_while(|l| !l.is_empty())
        .skip(2)
        .collect()
}

fn layout(layout: FolderLayout) -> FolderReportOptions {
    FolderReportOptions {
        layout,
        ..FolderReportOptions::default()
    }
}

#[test]
fn bidi_controls_are_escaped_in_lists_and_side_by_side() {
    let cmp = tree(
        &[("\u{202e}txt.exe", b"x"), ("\u{2067}a\u{2069}.c", b"y")],
        &FolderRules::default(),
        &[],
    );
    for l in [FolderLayout::List, FolderLayout::SideBySide] {
        let out = folder_text(&cmp, &layout(l));
        for c in ['\u{202e}', '\u{2067}', '\u{2069}'] {
            assert!(!out.contains(c), "{l:?}: {out}");
        }
        assert!(out.contains("\\u{202e}txt.exe"), "{l:?}: {out}");
        assert!(out.contains("\\u{2067}a\\u{2069}.c"), "{l:?}: {out}");
    }
    let html = folder_text(
        &cmp,
        &FolderReportOptions {
            format: OutputFormat::Html { mono: false },
            ..layout(FolderLayout::SideBySide)
        },
    );
    assert!(
        !html.contains('\u{202e}') && html.contains("\\u{202e}txt.exe"),
        "{html}"
    );
    // Text side-by-side (plain and HTML).
    let cmp = compare_bytes(
        ("a.txt", "\u{202e}reversed\u{202c} bidi\n".as_bytes()),
        ("b.txt", "\u{2066}isolated\u{2069}\n".as_bytes()),
        &TextRules::default(),
    );
    for format in [
        OutputFormat::Text { ansi: false },
        OutputFormat::Text { ansi: true },
        OutputFormat::Html { mono: false },
    ] {
        let opts = TextReportOptions {
            layout: TextLayout::SideBySide,
            format,
            ..TextReportOptions::default()
        };
        let out = text(&cmp, &opts);
        for c in ['\u{202e}', '\u{202c}', '\u{2066}', '\u{2069}'] {
            assert!(!out.contains(c), "{format:?}: {out}");
        }
        assert!(
            out.contains("\\u{202e}reversed\\u{202c} bidi"),
            "{format:?}: {out}"
        );
    }
}

#[test]
fn name_escaping_is_unambiguous() {
    let cmp = tree(
        &[("a\nb.txt", b"1"), ("a\\nb.txt", b"2")],
        &FolderRules::default(),
        &[],
    );
    let list = folder_text(&cmp, &layout(FolderLayout::List));
    let lines: Vec<&str> = list.lines().filter(|l| !l.starts_with('#')).collect();
    assert_eq!(lines, ["=  a\\nb.txt", "=  a\\\\nb.txt"], "{list}");
    let sbs = folder_text(&cmp, &layout(FolderLayout::SideBySide));
    let rows = table(&sbs);
    assert_eq!(rows.len(), 2, "{sbs}");
    assert!(
        rows[0].starts_with("a\\nb.txt ") && rows[1].starts_with("a\\\\nb.txt "),
        "{sbs}"
    );
}

#[test]
fn emoji_sequences_keep_columns_aligned() {
    let family = "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}";
    let names = [
        "plain.txt".to_owned(),
        format!("{family}.txt"),
        "\u{263a}\u{fe0f}.txt".to_owned(),
    ];
    let files: Vec<(&str, &[u8])> = names
        .iter()
        .map(|n| (n.as_str(), b"x".as_slice()))
        .collect();
    let cmp = tree(&files, &FolderRules::default(), &[]);
    let sbs = folder_text(&cmp, &layout(FolderLayout::SideBySide));
    let widths: Vec<usize> = table(&sbs)
        .iter()
        .map(|l| UnicodeWidthStr::width(l.split("  =  ").next().unwrap()))
        .collect();
    assert_eq!(widths.len(), 3, "{sbs}");
    assert!(widths.iter().all(|w| *w == widths[0]), "{widths:?}\n{sbs}");
    // Text side-by-side: the marker column is the same on every row.
    let cmp = compare_bytes(
        (
            "a.txt",
            format!("{family} x\n\u{263a}\u{fe0f}\u{263a}\u{fe0f}\nab\n").as_bytes(),
        ),
        (
            "b.txt",
            format!("{family} y\n\u{263a}\u{fe0f}\u{263a}\u{fe0f}!\nac\n").as_bytes(),
        ),
        &TextRules::default(),
    );
    let opts = TextReportOptions {
        layout: TextLayout::SideBySide,
        width: 30,
        line_numbers: false,
        ..TextReportOptions::default()
    };
    let out = text(&cmp, &opts);
    let columns: Vec<usize> = table(&out)
        .iter()
        .filter_map(|l| l.find(" | ").map(|k| UnicodeWidthStr::width(&l[..k])))
        .collect();
    assert_eq!(columns.len(), 3, "{out}");
    assert!(
        columns.iter().all(|c| *c == columns[0]),
        "{columns:?}\n{out}"
    );
}

#[test]
fn characters_per_line_pieces_show_their_source_line() {
    let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
    vfs.insert_file(
        &host("/l/a.txt"),
        b"abcdefghij\nx\ny\n".to_vec(),
        MemMeta::default(),
    )
    .unwrap();
    vfs.insert_file(
        &host("/r/a.txt"),
        b"abcdefghij\nx\nZ\n".to_vec(),
        MemMeta::default(),
    )
    .unwrap();
    let format = Arc::new(CompiledFormat::everything_else());
    let load = LoadOptions {
        chars_per_line_limit: Some(4),
        ..LoadOptions::default()
    };
    let doc = |p: &str| {
        let (d, _) =
            load_document(&vfs, &mspec(p), &format, &load, None, &Ctl::detached()).unwrap();
        Arc::new(d)
    };
    let cmp = compare(
        CompareInput {
            left: doc("/l/a.txt"),
            right: doc("/r/a.txt"),
            format: format.clone(),
            rules: TextRules::default(),
            bytes_identical: false,
            force_tokenize: false,
        },
        None,
        &Ctl::detached(),
        None,
    )
    .unwrap();
    let opts = TextReportOptions {
        layout: TextLayout::SideBySide,
        width: 40,
        ..TextReportOptions::default()
    };
    let rows = text(&cmp, &opts);
    let numbers: Vec<&str> = table(&rows)
        .iter()
        .map(|l| l.split_whitespace().next().unwrap())
        .collect();
    // "abcdefghij" is three pieces of source line 1; x is line 2; y (deleted) and Z (inserted) are line 3, not
    // the fifth piece.
    assert_eq!(numbers, ["1", "1", "1", "2", "3", ">"], "{rows}");
    assert!(table(&rows)[5].ends_with("> 3 Z"), "{rows}");
    let html = text(
        &cmp,
        &TextReportOptions {
            format: OutputFormat::Html { mono: false },
            ..opts
        },
    );
    assert!(
        html.contains("<td class=\"n\">3</td><td class=\"t l\">y</td>"),
        "{html}"
    );
    assert!(!html.contains("<td class=\"n\">5</td>"), "{html}");
}

#[test]
fn entries_whose_metadata_is_unknown_show_no_size() {
    // A folder hidden by a name mask (name-only, D-45) shown with Suppress Filters: no size (not its 0 total).
    let mut rules = FolderRules::default();
    rules.filters.names.exclude_folders = vec!["obj".into()];
    rules.filters.names.exclude_files = vec!["*.o".into()];
    let cmp = tree(
        &[
            ("obj/big.bin", &[0; 1000]),
            ("keep.c", &[0; 1000]),
            ("skip.o", &[0; 1000]),
        ],
        &rules,
        &[],
    );
    let opts = FolderReportOptions {
        layout: FolderLayout::SideBySide,
        display: DisplayOptions {
            filter: FolderDisplayFilter::All,
            folders: FolderMode::CompareStructure,
            ignore_unimportant: false,
            suppress_filters: true,
        },
        columns: FolderColumns::SIZE,
        format: OutputFormat::Text { ansi: false },
    };
    let out = folder_text(&cmp, &opts);
    let rows = table(&out);
    let row = |name: &str| {
        *rows
            .iter()
            .find(|l| l.starts_with(name))
            .unwrap_or_else(|| panic!("{name}: {out}"))
    };
    assert_eq!(
        row("obj/").split_whitespace().collect::<Vec<_>>(),
        ["obj/", ".", "obj/"],
        "{out}"
    );
    assert_eq!(
        row("skip.o").split_whitespace().collect::<Vec<_>>(),
        ["skip.o", ".", "skip.o"],
        "{out}"
    );
    assert_eq!(
        row("keep.c").split_whitespace().collect::<Vec<_>>(),
        ["keep.c", "1000", "=", "keep.c", "1000"],
        "{out}"
    );
    // A stat failure on one side, a listing failure on the other.
    let faults = [
        Fault {
            path: host("/r/c.txt"),
            op: VfsOp::Stat,
            kind: ErrorKind::Io,
        },
        Fault {
            path: host("/l/sub"),
            op: VfsOp::List,
            kind: ErrorKind::PermissionDenied,
        },
    ];
    let cmp = tree(
        &[("c.txt", b"12"), ("sub/x.txt", b"123")],
        &FolderRules::default(),
        &faults,
    );
    let out = folder_text(
        &cmp,
        &FolderReportOptions {
            columns: FolderColumns::SIZE,
            ..layout(FolderLayout::SideBySide)
        },
    );
    let rows = table(&out);
    let row = |name: &str| {
        *rows
            .iter()
            .find(|l| l.starts_with(name))
            .unwrap_or_else(|| panic!("{name}: {out}"))
    };
    assert_eq!(
        row("c.txt").split_whitespace().collect::<Vec<_>>(),
        ["c.txt", "2", "E", "c.txt"],
        "{out}"
    );
    assert_eq!(
        row("sub/").split_whitespace().collect::<Vec<_>>(),
        ["sub/", "E", "sub/", "3"],
        "{out}"
    );
}

#[test]
fn text_json_display_toggles_are_arrays() {
    let cmp = compare_bytes(
        ("a.txt", b"a\nb\n"),
        ("b.txt", b"a\nc\n"),
        &TextRules::default(),
    );
    for filter in [
        TextDisplayFilter::All,
        TextDisplayFilter::Differences,
        TextDisplayFilter::Same,
        TextDisplayFilter::None,
        TextDisplayFilter::Context { lines: 1 },
        TextDisplayFilter::Toggles { bits: 2 },
    ] {
        let view = ViewOptions {
            filter,
            ..ViewOptions::default()
        };
        let v = serde_json::to_value(text_compare_json(&cmp, &view)).unwrap();
        assert!(
            v["display"]["toggles"].is_array(),
            "{filter:?}: {}",
            v["display"]
        );
    }
}

#[test]
fn the_folder_json_report_starts_with_its_schema() {
    let cmp = tree(&[("a.txt", b"x")], &FolderRules::default(), &[]);
    let out = folder_text(&cmp, &layout(FolderLayout::Json));
    assert!(
        out.starts_with("{\n  \"schema\": \"hikasa.folder_compare.v1\",\n  \"left\": "),
        "{out}"
    );
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["items"].as_array().unwrap().len(), 1);
}

#[test]
fn the_title_reaches_the_header() {
    let cmp = compare_bytes(("a.txt", b"a\n"), ("b.txt", b"b\n"), &TextRules::default());
    let opts = TextReportOptions::default();
    let meta = ReportMeta::for_text(&cmp, &opts, None, "hikasa test").with_title("Nightly <diff>");
    let mut out = Vec::new();
    render_text_report(&cmp, &opts, &meta, &mut out).unwrap();
    assert!(
        String::from_utf8(out)
            .unwrap()
            .starts_with("# Nightly <diff>\n")
    );
}
