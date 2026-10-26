//! The RPT-004 header (§3.4, §6.2.7): title (default or given), the generation time injected by the caller
//! (D-24; RFC 3339 with the zone's offset, omitted when `None`), the generator, the specs of both sides with
//! encoding and line endings, the mode and rules lines of `ReportMeta::for_text` / `for_folder` (fixed wording),
//! per-side errors and warnings — in every layout that has a header, as text, as `# ` lines in patches, and as
//! the HTML `meta` table.

mod common;

use common::{compare_bytes, compare_texts, folder_case, host, sibling_fixtures};
use hikasa_core::Pair;
use hikasa_folder::{DisplayOptions, FolderDisplayFilter, FolderMode};
use hikasa_report::{
    DisplayMode, FolderColumns, FolderLayout, FolderReportOptions, OutputFormat, PatchStyle,
    ReportMeta, TextLayout, TextReportOptions, render_folder_report, render_text_report,
};
use hikasa_text::{TextComparison, TextRules};

/// 2026-10-21T12:00:00Z seen from UTC+02:00.
fn zoned() -> jiff::Zoned {
    "2026-10-21T12:00:00Z"
        .parse::<jiff::Timestamp>()
        .unwrap()
        .to_zoned(jiff::tz::TimeZone::fixed(jiff::tz::offset(2)))
}

fn text(cmp: &TextComparison, opts: &TextReportOptions, meta: &ReportMeta) -> String {
    let mut out = Vec::new();
    render_text_report(cmp, opts, meta, &mut out).unwrap();
    String::from_utf8(out).unwrap()
}

fn opts(layout: TextLayout, format: OutputFormat) -> TextReportOptions {
    TextReportOptions {
        layout,
        format,
        ..TextReportOptions::default()
    }
}

#[test]
fn for_text_wording() {
    let cmp = compare_texts("a\nb\n", "a\nc\n");
    let o = opts(TextLayout::SideBySide, OutputFormat::Text { ansi: false });
    let meta = ReportMeta::for_text(&cmp, &o, Some(zoned()), "hikasa 0.1.0");
    assert_eq!(meta.title, None);
    assert_eq!(meta.generator, "hikasa 0.1.0");
    assert_eq!(meta.generated_at, Some(zoned()));
    assert_eq!(
        meta.mode.as_deref(),
        Some("display: all; ignore unimportant: no")
    );
    assert_eq!(
        meta.filters.as_deref(),
        Some(
            "format: Everything Else; algorithm: standard (auto); skew: 300; closeness: 500‰; never align: no; \
             unimportant: none; line endings: ignored; orphan lines: always important"
        )
    );
    assert_eq!(meta.errors, Pair::new(None, None));
    // The mode follows the layout: patches use their own context; normal diffs show the differences.
    let unified = ReportMeta::for_text(
        &cmp,
        &opts(
            TextLayout::AnnotatedUnified { lines: 5 },
            OutputFormat::Text { ansi: false },
        ),
        None,
        "",
    );
    assert_eq!(
        unified.mode.as_deref(),
        Some("display: context (5 lines); ignore unimportant: no")
    );
    let mut o2 = opts(
        TextLayout::Patch(PatchStyle::Normal),
        OutputFormat::Text { ansi: false },
    );
    o2.ignore_unimportant = true;
    assert_eq!(
        ReportMeta::for_text(&cmp, &o2, None, "").mode.as_deref(),
        Some("display: differences; ignore unimportant: yes")
    );
    let mut o3 = opts(TextLayout::Summary, OutputFormat::Text { ansi: false });
    o3.display = DisplayMode::Matches;
    assert_eq!(
        ReportMeta::for_text(&cmp, &o3, None, "").mode.as_deref(),
        Some("display: same; ignore unimportant: no")
    );
}

#[test]
fn rules_wording_lists_the_unimportant_items() {
    let mut rules = TextRules::default();
    rules.importance.leading_whitespace = false;
    rules.importance.everything_else_case_sensitive = false;
    rules.importance.compare_line_endings = true;
    rules.importance.orphans_always_important = false;
    rules.importance.elements.insert("Comment".into(), false);
    rules.importance.elements.insert("String".into(), true);
    rules
        .importance
        .unimportant
        .push(hikasa_text::TextPattern::regex("\\$Id[^$]*\\$"));
    let mut plain = hikasa_text::TextPattern::plain("TODO");
    plain.match_case = false;
    rules.importance.unimportant.push(plain);
    rules.alignment.algorithm = hikasa_text::AlgorithmChoice::Histogram;
    rules.alignment.closeness = false;
    rules.alignment.never_align_differences = true;
    let cmp = compare_bytes(("a.txt", b"x\n"), ("b.txt", b"y\n"), &rules);
    let meta = ReportMeta::for_text(&cmp, &TextReportOptions::default(), None, "");
    assert_eq!(
        meta.filters.as_deref(),
        Some(
            "format: Everything Else; algorithm: histogram (histogram); skew: 300; closeness: off; never align: yes; \
             unimportant: leading whitespace, case, element Comment, regex \"\\\\$Id[^$]*\\\\$\", text \"TODO\" (any case); \
             line endings: compared; orphan lines: important unless trivial"
        )
    );
}

#[test]
fn text_headers_in_every_layout() {
    let cmp = compare_bytes(
        ("a.txt", b"one\ntwo\n"),
        ("b.txt", "\u{feff}one\r\nthree\r\n".as_bytes()),
        &TextRules::default(),
    );
    let layouts = [
        (TextLayout::SideBySide, ""),
        (TextLayout::Summary, ""),
        (TextLayout::AnnotatedUnified { lines: 3 }, "# "),
        (TextLayout::Patch(PatchStyle::Unified { lines: 3 }), "# "),
        (TextLayout::Patch(PatchStyle::Context { lines: 3 }), "# "),
        (TextLayout::Patch(PatchStyle::Normal), "# "),
    ];
    for (layout, prefix) in layouts {
        let o = opts(layout, OutputFormat::Text { ansi: false });
        let mut meta = ReportMeta::for_text(&cmp, &o, Some(zoned()), "hikasa 0.1.0");
        meta.errors = Pair::new(Some("left side: cannot read".into()), None);
        let out = text(&cmp, &o, &meta);
        let head: Vec<&str> = out.lines().take(8).collect();
        assert_eq!(head[0], format!("{prefix}Text Compare"), "{layout:?}");
        assert_eq!(
            head[1],
            format!("{prefix}Generated: 2026-10-21T14:00:00+02:00 by hikasa 0.1.0")
        );
        assert_eq!(
            head[2],
            format!("{prefix}Left: {} (UTF-8, lf)", host("/l/a.txt"))
        );
        assert_eq!(
            head[3],
            format!("{prefix}Right: {} (UTF-8 BOM, cr_lf)", host("/r/b.txt"))
        );
        assert!(
            head[4].starts_with(&format!("{prefix}Mode: display: ")),
            "{layout:?}"
        );
        assert!(head[5].starts_with(&format!("{prefix}Rules: format: Everything Else")));
        assert_eq!(
            head[6],
            format!("{prefix}Left error: left side: cannot read")
        );
        assert_eq!(
            head[7],
            prefix.trim_end(),
            "{layout:?}: blank line ends the header"
        );
        // Without a time the line is omitted (golden tests).
        meta.generated_at = None;
        let out = text(&cmp, &o, &meta);
        assert!(!out.contains("Generated"), "{layout:?}");
        // A given title replaces the default; control characters are escaped.
        meta.title = Some("Nightly\nreport".into());
        let out = text(&cmp, &o, &meta);
        assert!(
            out.starts_with(&format!("{prefix}Nightly\\nreport\n")),
            "{layout:?}"
        );
    }
    // Statistics CSV: no header; the errors go to the Errors column.
    let o = opts(TextLayout::Statistics, OutputFormat::Text { ansi: false });
    let mut meta = ReportMeta::for_text(&cmp, &o, Some(zoned()), "hikasa 0.1.0");
    meta.errors = Pair::new(Some("x, \"y\"".into()), Some("z".into()));
    let csv = text(&cmp, &o, &meta);
    assert!(csv.starts_with("SourceFile,CompareFile,"));
    assert!(
        csv.ends_with(",\"left: x, \"\"y\"\"; right: z\"\r\n"),
        "{csv}"
    );
}

#[test]
fn text_headers_carry_the_comparison_warnings() {
    // A binary pair: both sides warn binary_content.
    let cmp = compare_bytes(
        ("a.bin", b"\x89PNG\r\n\x1a\n1"),
        ("b.bin", b"\x89PNG\r\n\x1a\n2"),
        &TextRules::default(),
    );
    let o = opts(TextLayout::Summary, OutputFormat::Text { ansi: false });
    let out = text(&cmp, &o, &ReportMeta::for_text(&cmp, &o, None, ""));
    assert_eq!(
        out.lines()
            .filter(|l| l.starts_with("Warning: binary_content: "))
            .count(),
        2,
        "{out}"
    );
    assert!(out.contains(", binary)"), "{out}");
}

#[test]
fn html_header_table() {
    let cmp = compare_texts("a\n", "b\n");
    for layout in [TextLayout::SideBySide, TextLayout::Summary] {
        let o = opts(layout, OutputFormat::Html { mono: false });
        let mut meta = ReportMeta::for_text(&cmp, &o, Some(zoned()), "hikasa <0.1>");
        meta.title = Some("A & B".into());
        meta.errors = Pair::new(None, Some("gone".into()));
        let html = text(&cmp, &o, &meta);
        assert!(html.contains("<title>A &amp; B</title>"));
        assert!(html.contains("<meta name=\"generator\" content=\"hikasa &lt;0.1&gt;\">"));
        assert!(html.contains("<h1>A &amp; B</h1>"));
        assert!(html.contains(
            "<tr><th>Generated</th><td>2026-10-21T14:00:00+02:00 by hikasa &lt;0.1&gt;</td></tr>"
        ));
        assert!(html.contains(&format!(
            "<tr><th>Left</th><td>{} (UTF-8, lf)</td></tr>",
            host("/l/left.txt")
        )));
        assert!(html.contains("<tr class=\"error\"><th>Right error</th><td>gone</td></tr>"));
        assert!(
            html.contains("<tr><th>Mode</th><td>display: all; ignore unimportant: no</td></tr>")
        );
        assert!(html.contains("<tr><th>Rules</th><td>format: Everything Else;"));
    }
}

fn folder_opts(layout: FolderLayout, format: OutputFormat) -> FolderReportOptions {
    FolderReportOptions {
        layout,
        display: DisplayOptions {
            filter: FolderDisplayFilter::Differences,
            folders: FolderMode::OnlyFiles,
            ignore_unimportant: true,
            suppress_filters: false,
        },
        columns: FolderColumns::default(),
        format,
    }
}

#[test]
fn folder_headers() {
    let (_, cmp) =
        folder_case(&sibling_fixtures("hikasa-folder", "trees").join("28_content_rules"));
    let o = folder_opts(FolderLayout::SideBySide, OutputFormat::Text { ansi: false });
    let meta = ReportMeta::for_folder(&cmp, &o, Some(zoned()), "hikasa 0.1.0");
    assert_eq!(
        meta.mode.as_deref(),
        Some(
            "display: differences; folders: only files; ignore unimportant: yes; suppress filters: no"
        )
    );
    assert_eq!(
        meta.filters.as_deref(),
        Some("size, timestamps (2 s), content: rules; filters: none")
    );
    for layout in [FolderLayout::SideBySide, FolderLayout::Summary] {
        let o = folder_opts(layout, OutputFormat::Text { ansi: false });
        let mut out = Vec::new();
        render_folder_report(&cmp, &o, &meta, &mut out).unwrap();
        let out = String::from_utf8(out).unwrap();
        let head: Vec<&str> = out.lines().take(7).collect();
        assert_eq!(
            head,
            [
                "Folder Compare",
                "Generated: 2026-10-21T14:00:00+02:00 by hikasa 0.1.0",
                format!("Left: {}", host("/l")).as_str(),
                format!("Right: {}", host("/r")).as_str(),
                "Mode: display: differences; folders: only files; ignore unimportant: yes; suppress filters: no",
                "Rules: size, timestamps (2 s), content: rules; filters: none",
                "Warning: rules_size_limit: compared by bytes",
            ],
            "{layout:?}"
        );
    }
    // The list and the JSON document have no header.
    for layout in [FolderLayout::List, FolderLayout::Json] {
        let o = folder_opts(layout, OutputFormat::Text { ansi: false });
        let mut out = Vec::new();
        render_folder_report(&cmp, &o, &meta, &mut out).unwrap();
        assert!(!String::from_utf8(out).unwrap().contains("Folder Compare"));
    }
}

#[test]
fn incomplete_folder_comparisons_say_so() {
    let (_, cmp) =
        folder_case(&sibling_fixtures("hikasa-folder", "trees").join("32_cancel_partway"));
    assert!(!cmp.is_complete());
    let o = folder_opts(FolderLayout::Summary, OutputFormat::Html { mono: true });
    let mut meta = ReportMeta::for_folder(&cmp, &o, None, "");
    meta.errors = Pair::new(Some("left: <cancelled>".into()), None);
    let mut out = Vec::new();
    render_folder_report(&cmp, &o, &meta, &mut out).unwrap();
    let html = String::from_utf8(out).unwrap();
    assert!(
        html.contains("<tr><th>Status</th><td>incomplete (the comparison was cancelled)</td></tr>")
    );
    assert!(
        html.contains(
            "<tr class=\"error\"><th>Left error</th><td>left: &lt;cancelled&gt;</td></tr>"
        )
    );
    assert!(html.contains("<tr><th>Complete</th><td>no</td></tr>"));
    assert!(html.contains("<tr><th>Verdict</th><td>incomplete</td></tr>"));
}
