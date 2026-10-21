//! Properties of the reports (§6.2.7):
//!
//! - P-RPT-1: every patch reproduces the right side when applied to the left one. (a) The contract's form: LF
//!   documents with `compare_line_endings = true`, `ignore_unimportant = false`, any context, any algorithm,
//!   random importance rules; plain and annotated unified diffs are applied with `diffy` when the context is at
//!   least one line (`diffy` places a zero-context hunk by searching from the line before its new range and can
//!   remove the wrong one of two duplicate lines, so it is no oracle there), and every style (unified, annotated,
//!   context, normal; any context, zero included) with the position-exact appliers of `common::apply`. (b) The
//!   default rules (line endings not compared) and random LF / CRLF / lone-CR endings, missing final newlines
//!   and UTF-8 BOMs on either side: every style reproduces the right file byte for byte (after turning lone CRs
//!   into LF, which is what the patch writes, `patch_lossy_eol`), and `\ No newline at end of file` only
//!   follows the true last line of a side (wave-4 decision 1).
//! - P-RPT-2: rendering twice gives identical bytes, for every layout and format, text and folder.
//! - P-RPT-3: every HTML document is well-formed (tag balance and escaping, `common::check_html`).

mod common;

use common::{TreeFile, apply, check_html, compare_bytes, config, host, run_tree};
use hikasa_core::{CaseSensitivity, Pair, WarningCode};
use hikasa_folder::{DisplayOptions, FolderDisplayFilter, FolderMode, FolderRules};
use hikasa_report::{
    DisplayMode, FolderColumns, FolderLayout, FolderReportOptions, HtmlStyle, OutputFormat,
    PatchStyle, ReportMeta, TextLayout, TextReportOptions, Wrap, render_folder_report_with,
    render_text_report_with,
};
use hikasa_text::{AlgorithmChoice, TextComparison, TextRules};
use hikasa_vfs::{MemMeta, MemoryVfs};
use proptest::prelude::*;

/// Lines drawn from a small alphabet, so that the documents share many lines.
fn lines() -> impl Strategy<Value = Vec<String>> {
    prop::collection::vec(
        prop::sample::select(vec![
            "a", "b", "c", "d", "", "  a", "a  b", "x y", "B", "{", "}", "---", "+++", "@@ x",
            "\\ x", "*** y",
        ]),
        0..24,
    )
    .prop_map(|v| v.into_iter().map(str::to_owned).collect())
}

/// An LF document from lines, optionally without a final newline.
fn lf_text(lines: &[String], final_newline: bool) -> String {
    let mut s = lines.join("\n");
    if !lines.is_empty() && final_newline {
        s.push('\n');
    }
    s
}

/// Random rules with line endings compared.
fn rules() -> impl Strategy<Value = TextRules> {
    (
        prop::sample::select(AlgorithmChoice::ALL.to_vec()),
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
    )
        .prop_map(|(algorithm, ws, case, orphans, never)| {
            let mut r = TextRules::default();
            r.alignment.algorithm = algorithm;
            r.alignment.never_align_differences = never;
            r.importance.compare_line_endings = true;
            r.importance.leading_whitespace = !ws;
            r.importance.embedded_whitespace = !ws;
            r.importance.everything_else_case_sensitive = !case;
            r.importance.orphans_always_important = orphans;
            r
        })
}

/// Renders a text report and returns its warnings too.
fn render_with_warnings(
    cmp: &TextComparison,
    opts: &TextReportOptions,
) -> (Vec<u8>, Vec<hikasa_core::Warning>) {
    let meta = ReportMeta::for_text(cmp, opts, None, "hikasa test");
    let mut out = Vec::new();
    let warnings =
        render_text_report_with(cmp, opts, &meta, &HtmlStyle::default(), &mut out).unwrap();
    (out, warnings)
}

/// The patch layouts of P-RPT-1 with their exact appliers.
fn patch_layouts(context: u32) -> Vec<(TextLayout, apply::Applier)> {
    vec![
        (
            TextLayout::Patch(PatchStyle::Unified { lines: context }),
            apply::unified as apply::Applier,
        ),
        (
            TextLayout::AnnotatedUnified { lines: context },
            apply::unified,
        ),
        (
            TextLayout::Patch(PatchStyle::Context { lines: context }),
            apply::context,
        ),
        (TextLayout::Patch(PatchStyle::Normal), apply::normal),
    ]
}

/// Checks that every patch style turns `left` into `right` (both as the patch sees them), that the
/// `\ No newline at end of file` markers sit at the end of their side, and (unified, context ≥ 1) that `diffy`
/// agrees.
fn check_patches(
    cmp: &TextComparison,
    context: u32,
    left: &[u8],
    right: &[u8],
) -> Result<Vec<hikasa_core::Warning>, TestCaseError> {
    let mut all = Vec::new();
    for (layout, applier) in patch_layouts(context) {
        let opts = TextReportOptions {
            layout,
            ..TextReportOptions::default()
        };
        let (patch, warnings) = render_with_warnings(cmp, &opts);
        let shown = String::from_utf8_lossy(&patch).into_owned();
        let applied = applier(left, &patch)
            .map_err(|e| TestCaseError::fail(format!("{layout:?}: {e}\n{shown}")))?;
        prop_assert_eq!(
            String::from_utf8_lossy(&applied),
            String::from_utf8_lossy(right),
            "{:?}\n{}",
            layout,
            shown
        );
        apply::no_newline_markers_are_last(&patch)
            .map_err(|e| TestCaseError::fail(format!("{layout:?}: {e}\n{shown}")))?;
        let unified = matches!(
            layout,
            TextLayout::Patch(PatchStyle::Unified { .. }) | TextLayout::AnnotatedUnified { .. }
        );
        if unified && context >= 1 {
            let (l, r) = (
                std::str::from_utf8(left).unwrap(),
                std::str::from_utf8(right).unwrap(),
            );
            let parsed = diffy::Patch::from_str(&shown)
                .map_err(|e| TestCaseError::fail(format!("{e}\n{shown}")))?;
            let applied = diffy::apply(l, &parsed)
                .map_err(|e| TestCaseError::fail(format!("{e}\n{shown}")))?;
            prop_assert_eq!(&applied, r, "{}", shown);
        }
        all.extend(warnings);
    }
    Ok(all)
}

/// A line of the default-rules property: text from the shared alphabet and an ending (LF, CRLF or a lone CR).
fn any_ending_line() -> impl Strategy<Value = String> {
    (
        prop::sample::select(vec![
            "a", "b", "c", "", "  a", "a  b", "B", "{", "---", "+++", "@@ x", "\\ x", "*** y",
        ]),
        prop::sample::select(vec!["\n", "\n", "\r\n", "\r\n", "\r"]),
    )
        .prop_map(|(text, end)| format!("{text}{end}"))
}

/// A document of the default-rules property: lines, optionally without the last terminator and with a BOM.
fn any_ending_doc() -> impl Strategy<Value = Vec<u8>> {
    (
        prop::collection::vec(any_ending_line(), 0..16),
        any::<bool>(),
        prop::bool::weighted(0.15),
    )
        .prop_map(|(lines, final_newline, bom)| {
            let mut s = lines.concat();
            if !final_newline {
                while s.ends_with(['\n', '\r']) {
                    s.pop();
                }
            }
            let mut out = if bom {
                "\u{feff}".as_bytes().to_vec()
            } else {
                Vec::new()
            };
            out.extend_from_slice(s.as_bytes());
            out
        })
}

/// True when `bytes` holds a CR not followed by LF.
fn has_lone_cr(bytes: &[u8]) -> bool {
    bytes
        .iter()
        .enumerate()
        .any(|(k, &b)| b == b'\r' && bytes.get(k + 1) != Some(&b'\n'))
}

proptest! {
    #![proptest_config(config("tests/proptest-regressions/props.txt"))]

    /// P-RPT-1 (a): LF documents, line endings compared (the strategy is kept as it was, so the committed seeds
    /// replay the cases they were found with).
    #[test]
    fn p_rpt_1_patches_reproduce_the_right_side(
        left in lines(),
        right in lines(),
        left_nl in any::<bool>(),
        right_nl in any::<bool>(),
        context in 0u32..5,
        rules in rules(),
    ) {
        let (l, r) = (lf_text(&left, left_nl), lf_text(&right, right_nl));
        let cmp = compare_bytes(("a.txt", l.as_bytes()), ("b.txt", r.as_bytes()), &rules);
        let warnings = check_patches(&cmp, context, l.as_bytes(), r.as_bytes())?;
        prop_assert!(warnings.is_empty(), "{:?}", warnings);
    }

    /// P-RPT-1 (b): the default rules (line endings not compared, or compared) with LF, CRLF and lone-CR endings,
    /// missing final newlines and BOMs: every style is exact, a lone CR being written as LF and warned.
    #[test]
    fn p_rpt_1_default_rules_any_line_endings(
        left in any_ending_doc(),
        right in any_ending_doc(),
        context in 0u32..4,
        algorithm in prop::sample::select(AlgorithmChoice::ALL.to_vec()),
        compare_line_endings in prop::bool::weighted(0.25),
    ) {
        let mut rules = TextRules::default();
        rules.alignment.algorithm = algorithm;
        rules.importance.compare_line_endings = compare_line_endings;
        let cmp = compare_bytes(("a.txt", &left), ("b.txt", &right), &rules);
        let lossy = has_lone_cr(&left) || has_lone_cr(&right);
        let (l, r) = if lossy {
            (apply::lone_cr_to_lf(&left), apply::lone_cr_to_lf(&right))
        } else {
            (left.clone(), right.clone())
        };
        // A file holding nothing but a BOM has no line to write it with: the patch says so instead.
        let bom_only = |b: &[u8]| b == "\u{feff}".as_bytes();
        if bom_only(&left) != bom_only(&right) {
            let opts = TextReportOptions::default();
            let (_, warnings) = render_with_warnings(&cmp, &opts);
            prop_assert!(warnings.iter().any(|w| w.code == WarningCode::PatchEncoding), "{:?}", warnings);
            return Ok(());
        }
        let warnings = check_patches(&cmp, context, &l, &r)?;
        if !lossy {
            prop_assert!(
                !warnings.iter().any(|w| w.code == WarningCode::PatchLossyEol),
                "{:?}",
                warnings
            );
        }
        prop_assert!(!warnings.iter().any(|w| w.code == WarningCode::PatchEncoding), "{:?}", warnings);
    }

    /// P-RPT-2 and P-RPT-3 for text reports: every layout and format renders identically twice (also from a
    /// second, independent comparison of the same inputs), and every HTML document is well-formed, for texts with
    /// markup, entities, quotes, control characters, tabs, wide characters and every line ending.
    #[test]
    fn p_rpt_2_3_text_reports(
        left in prop::collection::vec(any_line(), 0..12),
        right in prop::collection::vec(any_line(), 0..12),
        context in 0u32..4,
        width in 10u16..120,
        iu in any::<bool>(),
        numbers in any::<bool>(),
        wrap in prop::sample::select(vec![Wrap::None, Wrap::Character, Wrap::Word]),
        display in prop::sample::select(vec![
            DisplayMode::All, DisplayMode::Mismatches, DisplayMode::Context { lines: 1 }, DisplayMode::Matches,
        ]),
        rules in rules(),
        title in any_line(),
        css in any_line(),
    ) {
        let (l, r) = (left.concat(), right.concat());
        let cmp = compare_bytes(("a.txt", l.as_bytes()), ("b.txt", r.as_bytes()), &rules);
        let again = compare_bytes(("a.txt", l.as_bytes()), ("b.txt", r.as_bytes()), &rules);
        let layouts = [
            TextLayout::AnnotatedUnified { lines: context },
            TextLayout::Patch(PatchStyle::Unified { lines: context }),
            TextLayout::Patch(PatchStyle::Context { lines: context }),
            TextLayout::Patch(PatchStyle::Normal),
            TextLayout::SideBySide,
            TextLayout::Summary,
            TextLayout::Statistics,
        ];
        let style = HtmlStyle::custom(css);
        for layout in layouts {
            for format in [
                OutputFormat::Text { ansi: false },
                OutputFormat::Text { ansi: true },
                OutputFormat::Html { mono: false },
                OutputFormat::Html { mono: true },
            ] {
                let html = matches!(format, OutputFormat::Html { .. });
                if html && !matches!(layout, TextLayout::SideBySide | TextLayout::Summary) {
                    continue;
                }
                let opts = TextReportOptions {
                    layout, display, ignore_unimportant: iu, line_numbers: numbers, width, wrap, format,
                };
                let mut meta = ReportMeta::for_text(&cmp, &opts, Some(zoned()), "hikasa test");
                meta.title = Some(title.clone());
                meta.errors = Pair::new(Some(title.clone()), None);
                let mut first = Vec::new();
                render_text_report_with(&cmp, &opts, &meta, &style, &mut first).unwrap();
                let mut second = Vec::new();
                render_text_report_with(&cmp, &opts, &meta, &style, &mut second).unwrap();
                prop_assert_eq!(&first, &second);
                let mut third = Vec::new();
                render_text_report_with(&again, &opts, &meta, &style, &mut third).unwrap();
                prop_assert_eq!(&first, &third);
                if html {
                    let text = String::from_utf8(first).unwrap();
                    prop_assert!(check_html(&text).is_ok(), "{:?}\n{}", check_html(&text), text);
                }
            }
        }
    }

    /// P-RPT-2 and P-RPT-3 for folder reports on random tree pairs whose names carry markup, entities, quotes,
    /// spaces and non-ASCII text.
    #[test]
    fn p_rpt_2_3_folder_reports(
        left in prop::collection::btree_map(path(), content(), 0..10),
        right in prop::collection::btree_map(path(), content(), 0..10),
        preset in prop::sample::select(FolderDisplayFilter::PRESETS.to_vec()),
        folders in prop::sample::select(FolderMode::ALL.to_vec()),
        iu in any::<bool>(),
        suppress in any::<bool>(),
        columns in 0u16..128,
    ) {
        let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
        vfs.insert_dir(&host("/l"), MemMeta::default()).unwrap();
        vfs.insert_dir(&host("/r"), MemMeta::default()).unwrap();
        for (base, files) in [("/l", &left), ("/r", &right)] {
            for (p, c) in files {
                // A path that is a file on one insert and a folder on another is skipped.
                let _ = vfs.insert_file(&host(&format!("{base}/{p}")), c.as_bytes().to_vec(), MemMeta::default());
            }
        }
        let tree = TreeFile::default();
        let cmp = run_tree(&tree, &vfs, &FolderRules::default()).unwrap();
        for layout in [FolderLayout::List, FolderLayout::SideBySide, FolderLayout::Summary, FolderLayout::Json] {
            for format in [
                OutputFormat::Text { ansi: false },
                OutputFormat::Text { ansi: true },
                OutputFormat::Html { mono: false },
                OutputFormat::Html { mono: true },
            ] {
                let html = matches!(format, OutputFormat::Html { .. });
                if html && !matches!(layout, FolderLayout::SideBySide | FolderLayout::Summary) {
                    continue;
                }
                let opts = FolderReportOptions {
                    layout,
                    display: DisplayOptions { filter: preset, folders, ignore_unimportant: iu, suppress_filters: suppress },
                    columns: FolderColumns::from_bits_truncate(columns),
                    format,
                };
                let meta = ReportMeta::for_folder(&cmp, &opts, Some(zoned()), "hikasa test");
                let style = HtmlStyle::default();
                let mut first = Vec::new();
                render_folder_report_with(&cmp, &opts, &meta, &style, &mut first).unwrap();
                let mut second = Vec::new();
                render_folder_report_with(&cmp, &opts, &meta, &style, &mut second).unwrap();
                prop_assert_eq!(&first, &second);
                if html {
                    let text = String::from_utf8(first).unwrap();
                    prop_assert!(check_html(&text).is_ok(), "{:?}\n{}", check_html(&text), text);
                } else if layout == FolderLayout::Json {
                    let v: serde_json::Value = serde_json::from_slice(&first).unwrap();
                    prop_assert!(v["items"].is_array());
                }
            }
        }
    }
}

/// A line (with its terminator) of arbitrary content: markup, entities, quotes, controls, tabs, wide characters.
fn any_line() -> impl Strategy<Value = String> {
    (
        prop::collection::vec(
            prop::sample::select(vec![
                "a", "<", ">", "&", "\"", "'", "\t", " ", "\u{1}", "\u{1b}", "\u{7f}", "\u{85}",
                "日", "e\u{301}", "😀", "&amp;", "</style>", "<script>", "]]>", "--", "x",
            ]),
            0..12,
        ),
        prop::sample::select(vec!["\n", "\r\n", "\r", ""]),
    )
        .prop_map(|(parts, end)| {
            let mut s = parts.concat();
            s.push_str(end);
            s
        })
}

/// A relative path of one or two components.
fn path() -> impl Strategy<Value = String> {
    let name = prop::sample::select(vec![
        "a", "b", "<x>", "a&b", "q\"t", "it's", "日本", "sp ace", "Z", "x.txt",
    ]);
    (name.clone(), prop::option::of(name)).prop_map(|(a, b)| match b {
        Some(b) => format!("{a}/{b}"),
        None => a.to_owned(),
    })
}

/// File content.
fn content() -> impl Strategy<Value = String> {
    prop::sample::select(vec!["", "x", "y", "xy\n"]).prop_map(str::to_owned)
}

/// A fixed generation time (the clock is injected, D-24).
fn zoned() -> jiff::Zoned {
    "2026-10-21T12:00:00Z"
        .parse::<jiff::Timestamp>()
        .unwrap()
        .to_zoned(jiff::tz::TimeZone::fixed(jiff::tz::offset(2)))
}

/// The two cases the review found (shrunk), where `diffy` removes the wrong one of two duplicate lines from a
/// correct zero-context patch: the exact appliers (and GNU patch, `tests/apply.rs`) reproduce the right side.
#[test]
fn p_rpt_1_regressions_zero_context_duplicate_lines() {
    let doc = |lines: &[&str], nl: bool| {
        let lines: Vec<String> = lines.iter().map(|s| (*s).to_owned()).collect();
        lf_text(&lines, nl)
    };
    let cases = [
        (
            doc(&["*** y", "B", "*** y", "c", "a"], false),
            &["a", "*** y", "B", "c"][..],
            AlgorithmChoice::Standard,
        ),
        (
            doc(&["a", "*** y", "a  b", "*** y"], true),
            &["*** y", "a  b"][..],
            AlgorithmChoice::Auto,
        ),
        (
            doc(&["a", "*** y", "a  b", "*** y"], false),
            &["*** y", "a  b"][..],
            AlgorithmChoice::Auto,
        ),
    ];
    for (left, right, algorithm) in cases {
        for right_nl in [false, true] {
            let right = doc(right, right_nl);
            let mut rules = TextRules::default();
            rules.alignment.algorithm = algorithm;
            rules.importance.compare_line_endings = true;
            rules.importance.orphans_always_important = false;
            let cmp = compare_bytes(
                ("a.txt", left.as_bytes()),
                ("b.txt", right.as_bytes()),
                &rules,
            );
            for context in 0..3 {
                check_patches(&cmp, context, left.as_bytes(), right.as_bytes())
                    .unwrap_or_else(|e| panic!("{left:?} -> {right:?}, context {context}: {e}"));
            }
        }
    }
}
