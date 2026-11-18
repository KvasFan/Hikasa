//! A10.21's reports (TC-024, D-282, contract §1.10.17): an ignored row counts as same in every report — the
//! side-by-side marker, the interleaved mark, the HTML row (marked `ign`), the XML row (`ignored="true"`), the
//! patches (a hunk of ignored rows only is left out; a hunk with another difference keeps every row exactly) —
//! and the JSON document carries `ignored`, the rows' `ignored: true`, `stats.ignored_rows` and the sections'
//! `ignored`.

#![allow(clippy::single_range_in_vec_init)] // one-range line sets are meant as such

use std::sync::Arc;

use hikasa_core::{ContentResult, Ctl};
use hikasa_report::{
    OutputFormat, PatchStyle, ReportMeta, TextLayout, TextReportOptions, render_text_report,
    text_compare_json,
};
use hikasa_text::{
    CompareInput, CompiledFormat, IgnoredLines, RangeSet, TextComparison, TextDocument, TextRules,
    ViewOptions, compare,
};

const LEFT: &str = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\n";
const RIGHT: &str = "a\nB\nc\nd\ne\nf\ng\nh\ni\nj\nK\nl\n";

/// The pair with line 1 of both sides (`b` → `B`) ignored; line 10 (`k` → `K`) still differs.
fn cmp(ignored: bool) -> TextComparison {
    let cmp = compare(
        CompareInput {
            left: Arc::new(TextDocument::from_text(LEFT)),
            right: Arc::new(TextDocument::from_text(RIGHT)),
            format: Arc::new(CompiledFormat::everything_else()),
            rules: TextRules::default(),
            bytes_identical: false,
            force_tokenize: false,
            anchors: Vec::new(),
            right_format: None,
            ignored: if ignored {
                IgnoredLines {
                    left: RangeSet::from_ranges(vec![1..2]),
                    right: RangeSet::from_ranges(vec![1..2]),
                }
            } else {
                IgnoredLines::default()
            },
        },
        None,
        &Ctl::detached(),
        None,
    )
    .unwrap();
    cmp.check_invariants().unwrap();
    cmp
}

fn render(cmp: &TextComparison, layout: TextLayout, format: OutputFormat) -> String {
    let o = TextReportOptions {
        layout,
        format,
        ..TextReportOptions::default()
    };
    let meta = ReportMeta::for_text(cmp, &o, None, "t");
    let mut out = Vec::new();
    render_text_report(cmp, &o, &meta, &mut out).unwrap();
    String::from_utf8(out).unwrap()
}

const TEXT: OutputFormat = OutputFormat::Text { ansi: false };

#[test]
fn side_by_side_and_interleaved_show_ignored_rows_as_same() {
    let (with, without) = (cmp(true), cmp(false));
    let sbs = |c: &TextComparison| render(c, TextLayout::SideBySide, TEXT);
    let line = |report: &str, needle: &str| -> String {
        report
            .lines()
            .find(|l| l.contains(needle))
            .unwrap()
            .to_owned()
    };
    let marked = |l: String| l.contains(['<', '>', '|', '~']);
    assert!(marked(line(&sbs(&without), " b")));
    assert!(!marked(line(&sbs(&with), " b")));
    assert!(!marked(line(&sbs(&with), " B")));
    assert!(marked(line(&sbs(&with), " k")));
    let il = render(&with, TextLayout::Interleaved, TEXT);
    let marks: Vec<&str> = il.lines().filter(|l| l.contains('!')).collect();
    assert!(
        marks.iter().all(|l| !l.ends_with('b') && !l.ends_with('B')),
        "{il}"
    );
}

#[test]
fn html_and_xml_mark_ignored_rows() {
    let html = render(
        &cmp(true),
        TextLayout::SideBySide,
        OutputFormat::Html { mono: false },
    );
    let c = cmp(true);
    let n = c.stats().ignored_rows as usize;
    assert!(n >= 1);
    assert_eq!(html.matches("class=\"ign\"").count(), n, "{html}");
    assert!(!html.contains(">B<") || html.matches("imp").count() > 0);
    let xml = render(&c, TextLayout::Xml, TEXT);
    assert_eq!(xml.matches("ignored=\"true\"").count(), n, "{xml}");
}

#[test]
fn patches_leave_out_hunks_of_ignored_rows_only() {
    let unified = TextLayout::Patch(PatchStyle::Unified { lines: 1 });
    let with = render(&cmp(true), unified, TEXT);
    let without = render(&cmp(false), unified, TEXT);
    assert_eq!(without.matches("@@ -").count(), 2);
    assert_eq!(with.matches("@@ -").count(), 1, "{with}");
    assert!(with.contains("-k\n+K\n") && !with.contains("-b\n"));
    // With enough context both changes share a hunk, written exactly, ignored row included.
    let wide = render(
        &cmp(true),
        TextLayout::Patch(PatchStyle::Unified { lines: 5 }),
        TEXT,
    );
    assert_eq!(wide.matches("@@ -").count(), 1);
    assert!(
        wide.contains("-b\n+B\n") && wide.contains("-k\n+K\n"),
        "{wide}"
    );
}

#[test]
fn json_carries_the_marks() {
    let c = cmp(true);
    let v = serde_json::to_value(text_compare_json(&c, &ViewOptions::default())).unwrap();
    assert_eq!(
        v["ignored"],
        serde_json::json!({"left": [[2, 3]], "right": [[2, 3]]})
    );
    let n = c.stats().ignored_rows;
    let plain = cmp(false);
    assert!(n >= 1);
    assert_eq!(v["stats"]["ignored_rows"], n);
    assert_eq!(
        v["stats"]["important_rows"],
        plain.stats().important_rows - n
    );
    let rows: Vec<&serde_json::Value> = v["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i.get("ignored").is_some())
        .collect();
    assert_eq!(rows.len() as u64, n);
    assert!(rows.iter().all(|r| r["ignored"] == true));
    assert!(
        rows.iter()
            .all(|r| r["left"]["text"] == "b" || r["right"]["text"] == "B")
    );
    assert_eq!(v["sections"][0]["ignored"], true);
    assert!(v["sections"][1].get("ignored").is_none());
    assert_eq!(v["verdict"]["result"], "important_differences");
    assert!(v.get("right_format").is_none());
    // Everything ignored: rules same.
    let all = compare(
        CompareInput {
            left: Arc::new(TextDocument::from_text(LEFT)),
            right: Arc::new(TextDocument::from_text(RIGHT)),
            format: Arc::new(CompiledFormat::everything_else()),
            rules: TextRules::default(),
            bytes_identical: false,
            force_tokenize: false,
            anchors: Vec::new(),
            right_format: Some(Arc::new(CompiledFormat::everything_else())),
            ignored: IgnoredLines {
                left: RangeSet::from_ranges(vec![0..12]),
                right: RangeSet::from_ranges(vec![0..12]),
            },
        },
        None,
        &Ctl::detached(),
        None,
    )
    .unwrap();
    assert_eq!(all.verdict(false), ContentResult::RulesSame);
    let v = serde_json::to_value(text_compare_json(&all, &ViewOptions::default())).unwrap();
    assert_eq!(v["verdict"]["result"], "rules_same");
    assert_eq!(v["right_format"]["name"], v["format"]["name"]);
}
