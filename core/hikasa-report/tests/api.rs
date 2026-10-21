//! Behaviour of the public API beyond the goldens: unsupported layout/format pairs, write errors (BrokenPipe stays
//! visible, D-35), rendering warnings (`patch_lossy_eol`), ANSI colours, display modes and ignore-unimportant in
//! side-by-side text, column widths, the JSON builders (§4.3.2, §4.3.3), custom CSS, chunked folder reading,
//! control characters in names, and the helper self-tests.

mod common;

use std::io::Write;

use common::{
    TreeFile, check_html, compare_bytes, compare_texts, folder_case, host, run_tree,
    sibling_fixtures, toml_subset,
};
use hikasa_core::{CaseSensitivity, Classify, ErrorKind, WarningCode};
use hikasa_folder::{DisplayOptions, FolderDisplayFilter, FolderMode, FolderRules};
use hikasa_report::json::{FOLDER_COMPARE_SCHEMA, TEXT_COMPARE_SCHEMA};
use hikasa_report::{
    DisplayMode, FolderColumns, FolderLayout, FolderReportOptions, HtmlStyle, OutputFormat,
    PatchStyle, ReportError, ReportMeta, TextLayout, TextReportOptions, Wrap, folder_compare_json,
    render_folder_report, render_folder_report_with, render_text_report, render_text_report_with,
    text_compare_json,
};
use hikasa_text::{
    DetectReason, FormatId, TextComparison, TextDisplayFilter, TextRules, ViewOptions,
};
use hikasa_vfs::{MemMeta, MemoryVfs};

fn opts(layout: TextLayout) -> TextReportOptions {
    TextReportOptions {
        layout,
        ..TextReportOptions::default()
    }
}

fn render(cmp: &TextComparison, o: &TextReportOptions) -> String {
    let meta = ReportMeta::for_text(cmp, o, None, "t");
    let mut out = Vec::new();
    render_text_report(cmp, o, &meta, &mut out).unwrap();
    String::from_utf8(out).unwrap()
}

/// The report body after the header (the first empty or `#` line).
fn body(report: &str) -> Vec<&str> {
    report
        .lines()
        .skip_while(|l| !l.is_empty() && *l != "#")
        .skip(1)
        .collect()
}

#[test]
fn unsupported_pairs_write_nothing() {
    let cmp = compare_texts("a\n", "b\n");
    for layout in [
        TextLayout::AnnotatedUnified { lines: 3 },
        TextLayout::Patch(PatchStyle::Unified { lines: 3 }),
        TextLayout::Patch(PatchStyle::Context { lines: 3 }),
        TextLayout::Patch(PatchStyle::Normal),
        TextLayout::Statistics,
    ] {
        let o = TextReportOptions {
            layout,
            format: OutputFormat::Html { mono: false },
            ..TextReportOptions::default()
        };
        let mut out = Vec::new();
        let err = render_text_report(
            &cmp,
            &o,
            &ReportMeta::for_text(&cmp, &o, None, ""),
            &mut out,
        )
        .unwrap_err();
        match err {
            ReportError::Unsupported { layout: l, format } => {
                assert_eq!(l, layout.name());
                assert_eq!(format, "html");
            }
            other => panic!("{other:?}"),
        }
        assert!(out.is_empty());
        assert_eq!(err_kind(&o, &cmp), ErrorKind::Unsupported);
    }
    let (_, fcmp) = folder_case(&sibling_fixtures("hikasa-folder", "trees").join("03_orphans"));
    for layout in [FolderLayout::List, FolderLayout::Json] {
        let o = FolderReportOptions {
            layout,
            format: OutputFormat::Html { mono: true },
            ..FolderReportOptions::default()
        };
        let mut out = Vec::new();
        let err = render_folder_report(
            &fcmp,
            &o,
            &ReportMeta::for_folder(&fcmp, &o, None, ""),
            &mut out,
        )
        .unwrap_err();
        assert!(
            matches!(err, ReportError::Unsupported { layout: l, format: "html" } if l == layout.name()),
            "{err:?}"
        );
        assert!(out.is_empty());
    }
}

fn err_kind(o: &TextReportOptions, cmp: &TextComparison) -> ErrorKind {
    let mut out = Vec::new();
    render_text_report(cmp, o, &ReportMeta::for_text(cmp, o, None, ""), &mut out)
        .unwrap_err()
        .kind()
}

/// A writer that fails like a closed pipe.
struct BrokenPipe;

impl Write for BrokenPipe {
    fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
        Err(std::io::ErrorKind::BrokenPipe.into())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Err(std::io::ErrorKind::BrokenPipe.into())
    }
}

#[test]
fn write_errors_surface_as_io() {
    let cmp = compare_texts("a\nb\n", "a\nc\n");
    for layout in [
        TextLayout::AnnotatedUnified { lines: 3 },
        TextLayout::Patch(PatchStyle::Normal),
        TextLayout::Patch(PatchStyle::Context { lines: 1 }),
        TextLayout::SideBySide,
        TextLayout::Summary,
        TextLayout::Statistics,
    ] {
        let o = opts(layout);
        let err = render_text_report(
            &cmp,
            &o,
            &ReportMeta::for_text(&cmp, &o, None, ""),
            &mut BrokenPipe,
        )
        .unwrap_err();
        match err {
            ReportError::Io(e) => {
                assert_eq!(e.kind(), std::io::ErrorKind::BrokenPipe, "{layout:?}")
            }
            other => panic!("{layout:?}: {other:?}"),
        }
    }
    let (_, fcmp) = folder_case(&sibling_fixtures("hikasa-folder", "trees").join("03_orphans"));
    for layout in [
        FolderLayout::List,
        FolderLayout::SideBySide,
        FolderLayout::Summary,
        FolderLayout::Json,
    ] {
        let o = FolderReportOptions {
            layout,
            ..FolderReportOptions::default()
        };
        let err = render_folder_report(
            &fcmp,
            &o,
            &ReportMeta::for_folder(&fcmp, &o, None, ""),
            &mut BrokenPipe,
        )
        .unwrap_err();
        assert!(
            matches!(&err, ReportError::Io(e) if e.kind() == std::io::ErrorKind::BrokenPipe),
            "{layout:?}: {err:?}"
        );
    }
}

#[test]
fn lossy_endings_are_reported_by_patches_only() {
    let cmp = compare_bytes(
        ("a.txt", b"x\ry\r"),
        ("b.txt", b"x\rz\r"),
        &TextRules::default(),
    );
    for (layout, expected) in [
        (TextLayout::AnnotatedUnified { lines: 3 }, 2),
        (TextLayout::Patch(PatchStyle::Unified { lines: 0 }), 2),
        (TextLayout::Patch(PatchStyle::Context { lines: 3 }), 2),
        (TextLayout::Patch(PatchStyle::Normal), 2),
        (TextLayout::SideBySide, 0),
        (TextLayout::Summary, 0),
        (TextLayout::Statistics, 0),
    ] {
        let o = opts(layout);
        let mut out = Vec::new();
        let warnings = render_text_report_with(
            &cmp,
            &o,
            &ReportMeta::for_text(&cmp, &o, None, ""),
            &HtmlStyle::default(),
            &mut out,
        )
        .unwrap();
        assert_eq!(warnings.len(), expected, "{layout:?}");
        assert!(
            warnings
                .iter()
                .all(|w| w.code == WarningCode::PatchLossyEol)
        );
        let text = String::from_utf8(out).unwrap();
        assert_eq!(
            text.matches("# Warning: patch_lossy_eol").count(),
            expected,
            "{layout:?}: the header lists them"
        );
        if expected > 0 {
            assert!(
                !text.contains('\r'),
                "{layout:?}: lone CRs are written as LF"
            );
            assert_eq!(warnings[0].spec.as_deref(), Some(host("/l/a.txt").as_str()));
        }
    }
    // Zero context: only the changed line's ending is written; the left one is a CR, so it warns too.
    let crlf = compare_bytes(
        ("a.txt", b"x\r\ny\r\n"),
        ("b.txt", b"x\r\nz\r\n"),
        &TextRules::default(),
    );
    let o = opts(TextLayout::Patch(PatchStyle::Unified { lines: 1 }));
    let mut out = Vec::new();
    let w = render_text_report_with(
        &crlf,
        &o,
        &ReportMeta::for_text(&crlf, &o, None, ""),
        &HtmlStyle::default(),
        &mut out,
    )
    .unwrap();
    assert!(w.is_empty(), "CRLF is kept: {w:?}");
    assert!(String::from_utf8(out).unwrap().contains("-y\r\n+z\r\n"));
}

#[test]
fn ansi_only_when_asked() {
    let cmp = compare_texts("same\nold\n", "same\nnew\nadded\n");
    for layout in [
        TextLayout::AnnotatedUnified { lines: 3 },
        TextLayout::Patch(PatchStyle::Context { lines: 3 }),
        TextLayout::Patch(PatchStyle::Normal),
        TextLayout::SideBySide,
        TextLayout::Summary,
    ] {
        let mut o = opts(layout);
        assert!(!render(&cmp, &o).contains('\x1b'), "{layout:?}");
        o.format = OutputFormat::Text { ansi: true };
        let colored = render(&cmp, &o);
        assert!(colored.contains("\x1b[31m"), "{layout:?}: red important");
        assert!(colored.contains("\x1b[0m"), "{layout:?}: reset");
    }
    let mut o = opts(TextLayout::AnnotatedUnified { lines: 3 });
    o.format = OutputFormat::Text { ansi: true };
    let out = render(&cmp, &o);
    assert!(out.contains("\x1b[31m-old\x1b[0m\n"), "{out:?}");
    assert!(out.contains("\x1b[31m+new\x1b[0m\n"));
    assert!(out.contains("\x1b[36m@@ -1,2 +1,3 @@ important\x1b[0m\n"));
    // Statistics never colours.
    let mut o = opts(TextLayout::Statistics);
    o.format = OutputFormat::Text { ansi: true };
    assert!(!render(&cmp, &o).contains('\x1b'));
    // The folder list colours the markers (FC-031).
    let (_, fcmp) = folder_case(&sibling_fixtures("hikasa-folder", "trees").join("03_orphans"));
    let o = FolderReportOptions {
        format: OutputFormat::Text { ansi: true },
        ..FolderReportOptions::default()
    };
    let mut out = Vec::new();
    render_folder_report(
        &fcmp,
        &o,
        &ReportMeta::for_folder(&fcmp, &o, None, ""),
        &mut out,
    )
    .unwrap();
    let out = String::from_utf8(out).unwrap();
    assert!(out.contains("\x1b[35m-  left_dir/\x1b[0m\n"), "{out:?}");
    assert!(out.contains("=  both.txt\n"));
}

#[test]
fn side_by_side_display_modes() {
    let left: String = (1..=20).map(|i| format!("line {i}\n")).collect();
    let right = left.replace("line 10\n", "line ten\n");
    let cmp = compare_texts(&left, &right);
    let mut o = opts(TextLayout::SideBySide);
    o.width = 40;
    let all = render(&cmp, &o);
    assert_eq!(body(&all).len(), 21, "title row + 20 rows");
    o.display = DisplayMode::Mismatches;
    let diff = body(&render(&cmp, &o)).join("\n");
    assert!(
        diff.contains("      … 9 lines hidden …\n10 line 10"),
        "{diff}"
    );
    assert!(diff.ends_with("      … 10 lines hidden …"), "{diff}");
    o.display = DisplayMode::Context { lines: 2 };
    let ctx = body(&render(&cmp, &o)).join("\n");
    assert!(ctx.contains("      … 7 lines hidden …"), "{ctx}");
    assert!(ctx.contains(" 8 line 8"), "{ctx}");
    o.display = DisplayMode::Matches;
    let same = body(&render(&cmp, &o)).join("\n");
    assert!(same.contains("      … 1 line hidden …"), "{same}");
    assert!(!same.contains('|'), "{same}");
}

#[test]
fn ignore_unimportant_in_side_by_side() {
    let mut rules = TextRules::default();
    rules.importance.embedded_whitespace = false;
    let cmp = compare_bytes(
        ("a.txt", b"a b\nvalue one\n"),
        ("b.txt", b"a  b\nvalue two\n"),
        &rules,
    );
    let mut o = opts(TextLayout::SideBySide);
    o.width = 30;
    let rows = render(&cmp, &o);
    assert!(body(&rows)[1].contains(" ~ "), "{rows}");
    assert!(body(&rows)[2].contains(" | "), "{rows}");
    o.ignore_unimportant = true;
    let rows = render(&cmp, &o);
    assert!(!body(&rows)[1].contains('~'), "{rows}");
    o.display = DisplayMode::Mismatches;
    let rows = render(&cmp, &o);
    assert!(
        body(&rows).join("\n").contains("… 1 line hidden …"),
        "{rows}"
    );
    // HTML: the unimportant row looks same (no class, no spans).
    o.format = OutputFormat::Html { mono: false };
    o.display = DisplayMode::All;
    let html = render(&cmp, &o);
    assert!(
        html.contains("<tr><td class=\"n\">1</td><td class=\"t l\">a b</td>"),
        "{html}"
    );
    o.ignore_unimportant = false;
    let html = render(&cmp, &o);
    assert!(
        html.contains("<tr class=\"unimp\"><td class=\"n\">1</td>"),
        "{html}"
    );
}

#[test]
fn side_by_side_lines_fit_the_width() {
    let long = "word ".repeat(40);
    let left = format!("{long}\n\tтабуляция и 日本語のテキスト\nshort\n");
    let right = format!("{long}!\n\tтабуляция и 日本語のテキスト?\n");
    let cmp = compare_texts(&left, &right);
    for width in [24u16, 40, 77, 160] {
        for wrap in [Wrap::None, Wrap::Character, Wrap::Word] {
            for line_numbers in [true, false] {
                let o = TextReportOptions {
                    width,
                    wrap,
                    line_numbers,
                    ..opts(TextLayout::SideBySide)
                };
                let out = render(&cmp, &o);
                for line in body(&out) {
                    let w: usize = line.chars().map(unicode_width_of).sum();
                    assert!(w <= usize::from(width), "{width} {wrap:?}: {line:?}");
                }
            }
        }
    }
}

/// Display width of one character (the reports' rule: unicode-width, controls never reach the output).
fn unicode_width_of(c: char) -> usize {
    // Wide ranges used by the test input: CJK and kana.
    if ('\u{3040}'..='\u{30ff}').contains(&c) || ('\u{4e00}'..='\u{9fff}').contains(&c) {
        2
    } else {
        1
    }
}

#[test]
fn text_json_document() {
    let cmp = compare_texts(
        "alpha\nbeta one\ngamma\ndelta\nepsilon\n",
        "alpha\nbeta two\ngamma\ndelta\nepsilon\nzeta\n",
    );
    let view = ViewOptions {
        filter: TextDisplayFilter::Differences,
        ignore_unimportant: false,
        expanded: std::iter::once(3..4).collect(),
    };
    let reason = DetectReason::Mask {
        id: FormatId::new("c-cpp").unwrap(),
        mask: "*.c".into(),
    };
    let doc = text_compare_json(&cmp, &view).with_detection(Some(&reason), Some(&reason), None);
    let v = serde_json::to_value(&doc).unwrap();
    assert_eq!(v["format"]["reason"]["reason"], "mask");
    assert_eq!(v["detection"]["left"]["mask"], "*.c");
    assert_eq!(v["detection"]["right"], serde_json::Value::Null);
    assert_eq!(v["display"]["mode"], "mismatches");
    assert_eq!(v["verdict"]["result"], "important_differences");
    let items = v["items"].as_array().unwrap();
    let kinds: Vec<&str> = items.iter().map(|i| i["type"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["gap", "row", "gap", "row", "gap", "row"]);
    assert_eq!(items[0]["rows"], serde_json::json!([0, 1]));
    assert_eq!(
        items[1]["left"],
        serde_json::json!({"line": 2, "text": "beta one"})
    );
    assert_eq!(items[1]["inline"]["left"][0]["important"], true);
    assert_eq!(items[1]["inline"]["whole_line"], false);
    assert_eq!(items[3]["state"], "same", "the expanded row");
    assert_eq!(items[3]["inline"], serde_json::Value::Null);
    assert_eq!(items[5]["left"], serde_json::Value::Null);
    assert_eq!(items[5]["right"]["line"], 6);
    assert_eq!(v["sections"][1]["left_lines"], serde_json::json!([6, 6]));
    assert_eq!(v["sections"][1]["right_lines"], serde_json::json!([6, 7]));
    let keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(keys.len(), 12, "{keys:?}");
    assert_eq!(TEXT_COMPARE_SCHEMA, "hikasa.text_compare.v1");
}

#[test]
fn folder_json_document() {
    let (_, cmp) =
        folder_case(&sibling_fixtures("hikasa-folder", "trees").join("28_content_rules"));
    let display = DisplayOptions {
        filter: FolderDisplayFilter::Differences,
        folders: FolderMode::CompareStructure,
        ignore_unimportant: false,
        suppress_filters: false,
    };
    let v = serde_json::to_value(folder_compare_json(&cmp, &display)).unwrap();
    assert_eq!(v["verdict"], "differences");
    assert_eq!(v["complete"], true);
    assert_eq!(v["case"], "sensitive");
    assert_eq!(v["display"]["filter"], "differences");
    let items = v["items"].as_array().unwrap();
    let paths: Vec<&str> = items.iter().map(|i| i["path"].as_str().unwrap()).collect();
    assert_eq!(
        paths,
        [
            "sub",
            "sub/only_unimportant.txt",
            "broken.txt",
            "important.txt",
            "spaces.txt"
        ]
    );
    assert_eq!(items[0]["kind"], "folder");
    assert_eq!(items[0]["aggregate"], serde_json::json!(["different"]));
    assert_eq!(items[0]["depth"], 0);
    assert_eq!(items[1]["depth"], 1);
    assert_eq!(items[1]["aggregate"], serde_json::Value::Null);
    assert_eq!(items[1]["content"]["result"]["result"], "unimportant_only");
    assert_eq!(items[2]["content"]["result"]["result"], "error");
    assert_eq!(
        items[2]["content"]["result"]["error"]["message"],
        "scripted rules failure"
    );
    assert_eq!(items[3]["diffs"], serde_json::json!(["content"]));
    assert_eq!(items[3]["left"]["size"], 3);
    assert_eq!(
        items[0]["left"]["size"], 4,
        "a folder's size is its subtree total (FC-040)"
    );
    // Under ignore-unimportant the unimportant-only pairs are same and leave the Differences view.
    let iu = DisplayOptions {
        ignore_unimportant: true,
        ..display
    };
    let v = serde_json::to_value(folder_compare_json(&cmp, &iu)).unwrap();
    let paths: Vec<&str> = v["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, ["broken.txt", "important.txt"]);
    assert_eq!(FOLDER_COMPARE_SCHEMA, "hikasa.folder_compare.v1");
}

#[test]
fn custom_css_follows_the_built_in_stylesheet() {
    let cmp = compare_texts("a\n", "b\n");
    let style = HtmlStyle::custom("span.imp { color: green }\n");
    for layout in [TextLayout::SideBySide, TextLayout::Summary] {
        let o = TextReportOptions {
            layout,
            format: OutputFormat::Html { mono: true },
            ..TextReportOptions::default()
        };
        let mut out = Vec::new();
        render_text_report_with(
            &cmp,
            &o,
            &ReportMeta::for_text(&cmp, &o, None, ""),
            &style,
            &mut out,
        )
        .unwrap();
        let html = String::from_utf8(out).unwrap();
        let builtin = html.find("table.sbs").unwrap();
        let custom = html.find("span.imp { color: green }").unwrap();
        assert!(builtin < custom && custom < html.find("</style>").unwrap());
        check_html(&html).unwrap();
    }
    let (_, fcmp) = folder_case(&sibling_fixtures("hikasa-folder", "trees").join("03_orphans"));
    let o = FolderReportOptions {
        layout: FolderLayout::SideBySide,
        format: OutputFormat::Html { mono: false },
        ..FolderReportOptions::default()
    };
    let mut out = Vec::new();
    render_folder_report_with(
        &fcmp,
        &o,
        &ReportMeta::for_folder(&fcmp, &o, None, ""),
        &style,
        &mut out,
    )
    .unwrap();
    assert!(
        String::from_utf8(out)
            .unwrap()
            .contains("span.imp { color: green }\n</style>")
    );
}

#[test]
fn large_folder_reports_read_in_chunks() {
    let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
    vfs.insert_dir(&host("/l"), MemMeta::default()).unwrap();
    vfs.insert_dir(&host("/r"), MemMeta::default()).unwrap();
    for i in 0..5000 {
        vfs.insert_file(
            &host(&format!("/l/f{i:05}")),
            b"x".to_vec(),
            MemMeta::default(),
        )
        .unwrap();
        vfs.insert_file(
            &host(&format!("/r/f{i:05}")),
            b"y".to_vec(),
            MemMeta::default(),
        )
        .unwrap();
    }
    let cmp = run_tree(&TreeFile::default(), &vfs, &FolderRules::default()).unwrap();
    let o = FolderReportOptions::default();
    let mut out = Vec::new();
    render_folder_report(
        &cmp,
        &o,
        &ReportMeta::for_folder(&cmp, &o, None, ""),
        &mut out,
    )
    .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert_eq!(text.lines().count(), 5001);
    assert!(text.lines().nth(4999).unwrap().ends_with("f04999"));
    let json = folder_compare_json(&cmp, &o.display);
    assert_eq!(json.items.len(), 5000);
}

#[test]
fn control_characters_in_names_are_escaped() {
    let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
    vfs.insert_dir(&host("/l"), MemMeta::default()).unwrap();
    vfs.insert_dir(&host("/r"), MemMeta::default()).unwrap();
    vfs.insert_file(&host("/l/a\nb\tc\u{1b}"), b"x".to_vec(), MemMeta::default())
        .unwrap();
    let cmp = run_tree(&TreeFile::default(), &vfs, &FolderRules::default()).unwrap();
    for layout in [FolderLayout::List, FolderLayout::SideBySide] {
        let o = FolderReportOptions {
            layout,
            ..FolderReportOptions::default()
        };
        let mut out = Vec::new();
        render_folder_report(
            &cmp,
            &o,
            &ReportMeta::for_folder(&cmp, &o, None, ""),
            &mut out,
        )
        .unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("a\\nb\\tc\\u{1b}"), "{text}");
        assert!(!text.contains('\u{1b}'));
    }
    // JSON keeps the name as it is (escaped by JSON itself).
    let json = serde_json::to_string(&folder_compare_json(
        &cmp,
        &FolderReportOptions::default().display,
    ))
    .unwrap();
    assert!(json.contains("\"name\":\"a\\nb\\tc\\u001b\""), "{json}");
}

#[test]
fn vocabulary_parsers_reject_with_invalid_option() {
    let err = DisplayMode::parse("sometimes", 3).unwrap_err();
    assert!(matches!(
        err,
        ReportError::InvalidOption {
            option: "display mode",
            ..
        }
    ));
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
    assert!(FolderColumns::parse_list("size,revision").is_err());
    assert!(FolderLayout::parse("tree").is_err());
    assert!(OutputFormat::parse("pdf").is_err());
    assert!(Wrap::parse("never").is_err());
    assert!(PatchStyle::parse("ed", 3).is_err());
}

#[test]
fn the_html_checker_rejects_broken_documents() {
    let ok = "<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\"><style>a > b { }</style></head><body><p class=\"x\">a &amp; b</p></body></html>\n";
    assert_eq!(check_html(ok), Ok(()));
    for bad in [
        "<html></html>",
        "<!DOCTYPE html>\n<html><p></html>",
        "<!DOCTYPE html>\n<html>a < b</html>",
        "<!DOCTYPE html>\n<html>a & b</html>",
        "<!DOCTYPE html>\n<html><p class=x></p></html>",
        "<!DOCTYPE html>\n<html><script></script></html>",
        "<!DOCTYPE html>\n<html>\u{1}</html>",
        "<!DOCTYPE html>\n<html><style></p></style></html>",
    ] {
        assert!(check_html(bad).is_err(), "{bad}");
    }
}

#[test]
fn the_toml_reader_handles_the_fixture_subset() {
    let v = toml_subset(
        "description = \"a # b\" # comment\n[options]\nrules_limit = 16\ncapabilities = [\"read\", \"list\"]\n\
         [[left]]\npath = \"caf\\u00e9.txt\"\ncontent = \"line\\r\\n\"\n[[left]]\npath = \"x\"\nkind_unknown = true\n",
    );
    assert_eq!(v["description"], "a # b");
    assert_eq!(v["options"]["rules_limit"], 16);
    assert_eq!(v["options"]["capabilities"][1], "list");
    assert_eq!(v["left"][0]["path"], "café.txt");
    assert_eq!(v["left"][0]["content"], "line\r\n");
    assert_eq!(v["left"][1]["kind_unknown"], true);
}

#[test]
fn tiny_widths_never_underflow() {
    let cmp = compare_bytes(
        ("日本語.txt", "日本\tx\n".as_bytes()),
        ("中文.txt", "中文\ty\n".as_bytes()),
        &TextRules::default(),
    );
    for width in [0u16, 1, 5, 7, 9, 11] {
        for wrap in [Wrap::None, Wrap::Character, Wrap::Word] {
            for line_numbers in [true, false] {
                let o = TextReportOptions {
                    width,
                    wrap,
                    line_numbers,
                    format: OutputFormat::Text { ansi: true },
                    ..opts(TextLayout::SideBySide)
                };
                let out = render(&cmp, &o);
                assert!(out.contains('日') || out.contains('\x1b'), "{out}");
            }
        }
    }
}
