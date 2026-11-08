//! M2 W7 properties (§6.7): P-IL (A7.4), P-XML (A7.5), P-FXML (A7.6), P-LINKS (A7.9, the href half), and the names
//! that are not Unicode in the folder XML (A7.6: on a Linux disk; on Windows a lone surrogate on disk; on macOS the
//! disk refuses such a name, which the test asserts before it stops).

mod common;

use std::collections::BTreeSet;

use common::inputs::{
    HOSTILE, NAME_CHARS, PLAIN, compare_memory, compare_on, compare_strings, compare_strings_cpl,
    folder_tree, materialize, text_pair,
};
use common::{config, xmlcheck};
use hikasa_core::Side;
use hikasa_folder::{
    DisplayOptions, FolderComparison, FolderDisplayFilter, FolderMode, FolderRules,
};
use hikasa_report::links::href_segments;
use hikasa_report::{
    HtmlStyle, ReportDefaults, ReportInput, ReportKind, ReportMeta, ReportRequest, ReportWords,
    folder_compare_json, href, render_report, summary_counts, text_compare_json,
};
use hikasa_text::{Importance, LineIdx, RowIdx, RowState, TextComparison, TextRules, ViewOptions};
use proptest::prelude::*;

/// Renders `words` of `kind` for `input`, with the header of the request.
fn render(
    input: ReportInput<'_>,
    kind: ReportKind,
    words: &ReportWords,
    defaults: &ReportDefaults,
) -> String {
    let req = ReportRequest::parse(kind, words, defaults).unwrap();
    let mut meta = match (input, &req.body) {
        (ReportInput::Text(cmp), hikasa_report::ReportBody::Text(opts)) => {
            ReportMeta::for_text(cmp, opts, None, "hikasa test")
        }
        (ReportInput::Folder(cmp), hikasa_report::ReportBody::Folder { options, .. }) => {
            ReportMeta::for_folder(cmp, options, None, "hikasa test")
        }
        _ => unreachable!(),
    };
    meta.title = req.title.clone();
    let mut out = Vec::new();
    render_report(input, &req, &meta, &HtmlStyle::default(), None, &mut out).unwrap();
    String::from_utf8(out).unwrap()
}

fn words(layout: &str, options: &[&str], output: &[&str], title: Option<String>) -> ReportWords {
    ReportWords {
        layout: Some(layout.to_owned()),
        options: options.iter().map(|s| (*s).to_owned()).collect(),
        output_options: output.iter().map(|s| (*s).to_owned()).collect(),
        title,
    }
}

/// The lines of a document.
fn doc_lines(cmp: &TextComparison, side: Side) -> Vec<String> {
    let doc = cmp.document(side);
    (0..doc.line_count())
        .map(|i| doc.line(LineIdx(i)).into_owned())
        .collect()
}

/// Number of decimal digits.
fn digits(n: u32) -> usize {
    n.checked_ilog10().map_or(1, |d| d as usize + 1)
}

/// The struck spans of an interleaved HTML document, per text line of the column: `(side, spans)`.
fn struck_spans(html: &str) -> Vec<(char, Vec<String>)> {
    let unescape = |s: &str| {
        s.replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&#39;", "'")
            .replace("&amp;", "&")
    };
    let mut out = Vec::new();
    for line in html
        .lines()
        .filter(|l| l.starts_with("<tr") && l.contains("<td class=\"t\">"))
    {
        let side = if line.contains(" r\">") {
            'r'
        } else if line.contains(" l\">") {
            'l'
        } else {
            ' '
        };
        let mut spans = Vec::new();
        let mut rest = line;
        while let Some(at) = rest.find("<del class=\"") {
            let after = &rest[at..];
            let open_end = after.find('>').unwrap() + 1;
            let close = after.find("</del>").unwrap();
            spans.push(unescape(&after[open_end..close]));
            rest = &after[close + 6..];
        }
        out.push((side, spans));
    }
    out
}

/// The number columns and the mark of each line of the interleaved column (display all, no wrap): a same row once
/// with both numbers and `  `; a difference as its left line (`-`) then its right line (`+`), each with its own number
/// only, the importance `!` important, `~` unimportant, ` ` for an unimportant one under ignore-unimportant.
fn expected_gutters(cmp: &TextComparison, iu: bool) -> Vec<(String, String, String)> {
    let number = |side: Side, line: Option<LineIdx>| {
        line.map(|l| cmp.document(side).physical_line(l).to_string())
            .unwrap_or_default()
    };
    let mut out = Vec::new();
    for i in 0..cmp.row_count() {
        let row = cmp.row(RowIdx(i)).unwrap();
        if row.state == RowState::Same {
            out.push((
                number(Side::Left, row.left),
                number(Side::Right, row.right),
                "  ".to_owned(),
            ));
            continue;
        }
        let importance = match row.importance {
            Importance::Unimportant if iu => ' ',
            Importance::Important => '!',
            _ => '~',
        };
        if row.left.is_some() {
            out.push((
                number(Side::Left, row.left),
                String::new(),
                format!("-{importance}"),
            ));
        }
        if row.right.is_some() {
            out.push((
                String::new(),
                number(Side::Right, row.right),
                format!("+{importance}"),
            ));
        }
    }
    out
}

/// What a strike-out of `strike` must strike, per line of the column (display all).
fn expected_spans(cmp: &TextComparison, iu: bool, strike: [bool; 2]) -> Vec<(char, Vec<String>)> {
    let mut out = Vec::new();
    for i in 0..cmp.row_count() {
        let idx = RowIdx(i);
        let row = cmp.row(idx).unwrap();
        let unimportant_as_same = iu && row.importance == Importance::Unimportant;
        if row.state == RowState::Same {
            out.push((' ', Vec::new()));
            continue;
        }
        let inline = (row.state == RowState::Changed && !unimportant_as_same)
            .then(|| cmp.inline(idx))
            .flatten();
        for (at, line) in [(0usize, row.left), (1usize, row.right)] {
            let Some(line) = line else { continue };
            let text = cmp
                .document(if at == 0 { Side::Left } else { Side::Right })
                .line(line)
                .into_owned();
            let mut spans = Vec::new();
            if strike[at] {
                if row.state == RowState::Changed {
                    if let Some(inl) = &inline {
                        let ranges = if at == 0 { &inl.left } else { &inl.right };
                        for r in ranges.iter().filter(|r| !iu || r.important) {
                            spans.push(
                                text[r.range.start as usize..r.range.end as usize].to_owned(),
                            );
                        }
                    }
                } else if !unimportant_as_same && !text.is_empty() {
                    spans.push(text.clone());
                }
            }
            out.push((if at == 0 { 'l' } else { 'r' }, spans));
        }
    }
    out
}

proptest! {
    #![proptest_config(config("tests/proptest-regressions/props_reports.txt"))]

    /// P-IL (A7.4): in display all without wrap, the interleaved text with its gutter removed, keeping the lines whose
    /// mark is ` ` or starts with `-`, is the left document's lines in order; with ` ` or `+`, the right's. Every struck
    /// span of the HTML is an inline range of its side or an orphan line, and nothing else is struck.
    #[test]
    fn p_il_interleaved_lines_are_both_documents((left, right) in text_pair(PLAIN), iu in any::<bool>(), strike in 1u8..4) {
        let cmp = compare_strings(&left, &right, &TextRules::default());
        let mut defaults = ReportDefaults::new("interleaved");
        defaults.width = 400;
        let mut options = vec!["display-all"];
        if iu {
            options.push("ignore-unimportant");
        }
        let text = render(ReportInput::Text(&cmp), ReportKind::Text, &words("interleaved", &options, &["wrap-none"], None), &defaults);
        let n = digits(cmp.document(Side::Left).line_count().max(cmp.document(Side::Right).line_count()));
        let gutter = 2 * n + 5;
        let body = text.split_once("\n\n").unwrap().1;
        let (mut l, mut r) = (Vec::new(), Vec::new());
        let mut gutters = Vec::new();
        for line in body.lines() {
            let padded = format!("{line:<gutter$}");
            // `<left number> <right number> <mark> `: the numbers right-aligned in n columns, the mark two characters.
            prop_assert!(padded.is_char_boundary(gutter), "{line:?}");
            let head = &padded[..gutter];
            prop_assert_eq!([&head[n..=n], &head[2 * n + 1..2 * n + 2], &head[2 * n + 4..]], [" "; 3], "{:?}", line);
            gutters.push((head[..n].trim_start().to_owned(), head[n + 1..2 * n + 1].trim_start().to_owned(), head[2 * n + 2..2 * n + 4].to_owned()));
            let mark: Vec<char> = padded[2 * n + 2..].chars().take(1).collect();
            let rest = padded.get(gutter..).unwrap_or("").to_owned();
            match mark[0] {
                ' ' => {
                    l.push(rest.clone());
                    r.push(rest);
                }
                '-' => l.push(rest),
                '+' => r.push(rest),
                other => prop_assert!(false, "mark {other:?} in {line:?}"),
            }
        }
        prop_assert_eq!(l, doc_lines(&cmp, Side::Left), "{}", text);
        prop_assert_eq!(r, doc_lines(&cmp, Side::Right), "{}", text);
        prop_assert_eq!(gutters, expected_gutters(&cmp, iu), "{}", text);
        let strike_words: Vec<&str> = [(1, "strikeout-left-diffs"), (2, "strikeout-right-diffs")]
            .iter()
            .filter(|(bit, _)| strike & bit != 0)
            .map(|(_, w)| *w)
            .collect();
        let mut options = options.clone();
        options.extend(strike_words);
        let html = render(ReportInput::Text(&cmp), ReportKind::Text, &words("interleaved", &options, &["html-color"], None), &defaults);
        prop_assert_eq!(struck_spans(&html), expected_spans(&cmp, iu, [strike & 1 != 0, strike & 2 != 0]), "{}", html);
    }

    /// P-XML (A7.5): generated content with every character the escaping must handle, under a random
    /// characters-per-line limit: the document is well-formed and passes the structure checks; reading it back (`l`
    /// and `r` texts, `c` as their characters, `&#13;` as CR, each side's pieces joined by its `*-piece`) rebuilds each
    /// document exactly in display all, and the title; `summary` equals `summary_counts`, which equals the text
    /// summary's numbers; the `verdict` is `hikasa.text_compare.v1`'s.
    #[test]
    fn p_xml_text_dump_round_trips(
        (left, right) in text_pair(HOSTILE),
        cpl in proptest::option::weighted(0.6, 1u32..8),
        name in proptest::collection::vec(proptest::sample::select(NAME_CHARS), 0..6),
        iu in any::<bool>(),
        title in proptest::collection::vec(proptest::sample::select(&['T', ' ', '\r', '\n', '&', '<', '>', '"', '\u{0}', '\u{1b}', '\u{ffff}', '\u{202e}'][..]), 0..10),
    ) {
        let name = format!("n{}.txt", name.into_iter().collect::<String>());
        let cmp = compare_strings_cpl(&name, &left, &right, &TextRules::default(), cpl);
        let title: String = title.into_iter().collect();
        let mut options = vec!["display-all"];
        if iu {
            options.push("ignore-unimportant");
        }
        let defaults = ReportDefaults::new("xml");
        let xml = render(ReportInput::Text(&cmp), ReportKind::Text, &words("xml", &options, &[], (!title.is_empty()).then(|| title.clone())), &defaults);
        xmlcheck::check(xml.as_bytes()).map_err(|e| TestCaseError::fail(format!("{e}\n{xml}")))?;
        let doc = roxmltree::Document::parse(&xml).unwrap();
        for (lines, source) in [(xmlcheck::side_lines(&doc, true), &left), (xmlcheck::side_lines(&doc, false), &right)] {
            let mut rebuilt = lines.join("\n");
            if source.ends_with('\n') {
                rebuilt.push('\n');
            }
            prop_assert_eq!(&rebuilt, source, "{}", xml);
        }
        // Each row's `d` and `u` are its side's inline differences, by importance, in order (display all: one `row` per
        // comparison row).
        let rows: Vec<_> = doc.descendants().filter(|n| n.has_tag_name((xmlcheck::NAMESPACE, "row"))).collect();
        prop_assert_eq!(rows.len() as u32, cmp.row_count());
        for (i, element) in rows.iter().enumerate() {
            let idx = RowIdx(i as u32);
            let row = cmp.row(idx).unwrap();
            let inline = (row.state == RowState::Changed).then(|| cmp.inline(idx)).flatten();
            for (at, (name, line)) in [("l", row.left), ("r", row.right)].into_iter().enumerate() {
                let got: Vec<(String, String)> = element
                    .children()
                    .filter(|c| c.has_tag_name((xmlcheck::NAMESPACE, name)))
                    .flat_map(|l| l.children().filter(roxmltree::Node::is_element).filter(|c| !c.has_tag_name((xmlcheck::NAMESPACE, "c"))).collect::<Vec<_>>())
                    .map(|c| (c.tag_name().name().to_owned(), xmlcheck::line_text(c)))
                    .collect();
                let mut want = Vec::new();
                if let (Some(line), Some(inl)) = (line, &inline) {
                    let text = cmp.document(if at == 0 { Side::Left } else { Side::Right }).line(line).into_owned();
                    let mut from = 0usize;
                    for r in if at == 0 { &inl.left } else { &inl.right } {
                        let (start, end) = (r.range.start as usize, (r.range.end as usize).min(text.len()));
                        if start >= end || start < from {
                            continue;
                        }
                        want.push(((if r.important { "d" } else { "u" }).to_owned(), text.get(start..end).unwrap_or_default().to_owned()));
                        from = end;
                    }
                }
                prop_assert_eq!(got, want, "row {} side {}\n{}", i, name, xml);
            }
        }
        // Every attribute reads back to one string: the spec with its backslashes and controls.
        let spec = doc.descendants().find(|n| n.has_tag_name((xmlcheck::NAMESPACE, "side"))).unwrap().attribute("spec").unwrap();
        prop_assert_eq!(xmlcheck::unescape(spec).map_err(TestCaseError::fail)?, cmp.document(Side::Left).origin().display(), "{}", xml);
        let read_title = doc.descendants().find(|n| n.has_tag_name((xmlcheck::NAMESPACE, "title"))).map(xmlcheck::line_text).unwrap();
        let want_title = if title.is_empty() { "Text Compare".to_owned() } else { title.clone() };
        prop_assert_eq!(read_title, want_title);
        let counts: Vec<(String, String, u64)> = summary_counts(ReportInput::Text(&cmp), iu)
            .into_iter()
            .map(|c| (c.group.to_owned(), c.name.to_owned(), c.value))
            .collect();
        prop_assert_eq!(xmlcheck::counts(&doc), counts.clone());
        // The text summary shows the same numbers.
        let summary = render(ReportInput::Text(&cmp), ReportKind::Text, &words("summary", &options, &[], None), &defaults);
        let number = |label: &str| -> u64 {
            summary.lines().find_map(|l| l.strip_prefix(&format!("{label}:"))).unwrap().trim().parse().unwrap()
        };
        let value = |g: &str, n: &str| counts.iter().find(|c| c.0 == g && c.1 == n).unwrap().2;
        prop_assert_eq!(number("Rows"), value("rows", "total"));
        prop_assert_eq!(number("Changed rows"), value("rows", "changed"));
        prop_assert_eq!(number("Left-only rows"), value("rows", "left-only"));
        prop_assert_eq!(number("Unimportant sections"), value("sections", "unimportant"));
        prop_assert_eq!(number("Right lines"), value("lines", "right"));
        let important = format!(
            "added {}, deleted {}, changed {}, original {}",
            value("important-lines", "added"),
            value("important-lines", "deleted"),
            value("important-lines", "changed"),
            value("important-lines", "original")
        );
        prop_assert!(summary.contains(&format!("Important lines:     {important}")) || summary.lines().any(|l| l.starts_with("Important lines:") && l.ends_with(&important)), "{}", summary);
        let json = serde_json::to_value(text_compare_json(&cmp, &ViewOptions { filter: hikasa_text::TextDisplayFilter::All, ignore_unimportant: iu, expanded: Vec::new() })).unwrap();
        let verdict = doc.descendants().find(|n| n.has_tag_name((xmlcheck::NAMESPACE, "summary"))).unwrap().attribute("verdict").unwrap().to_owned();
        prop_assert_eq!(json["verdict"]["result"].as_str().unwrap(), verdict);
    }

    /// P-FXML (A7.6): random trees on `MemoryVfs`, with names holding spaces, controls, markup and backslashes, compared
    /// by size and time or by the rules-based content test (so unimportant differences occur): the set of `(path,
    /// state)` of the `entry` and `folder` elements equals the JSON report's items for the same display (D-29), with
    /// `unimportant` exactly where the category is different and would be same under ignore-unimportant; every
    /// element's parent is its tree parent's element (`tree` for a top-level row, for one whose folder is not shown, and
    /// for every row of a flat tree); paths and names read back to the tree's; `summary` equals `summary_counts`. A name
    /// with controls is escaped in its attributes.
    #[test]
    fn p_fxml_folder_dump_is_the_json_items(tree in folder_tree(), filter in 0usize..5, iu in any::<bool>(), flat in proptest::bool::weighted(0.2), by_rules in any::<bool>()) {
        let vfs = materialize(&tree);
        let mut rules = FolderRules::default();
        rules.handling.flatten = flat;
        if by_rules {
            rules.criteria.content.method = hikasa_core::ContentMethod::Rules;
        }
        let cmp = compare_memory(&vfs, &rules);
        let filters = [FolderDisplayFilter::All, FolderDisplayFilter::Differences, FolderDisplayFilter::Orphans, FolderDisplayFilter::NoOrphans, FolderDisplayFilter::Same];
        let display = DisplayOptions { filter: filters[filter], folders: FolderMode::CompareStructure, ignore_unimportant: iu, suppress_filters: false };
        let (xml, mut items) = folder_dump(&cmp, display);
        // `unimportant`: different, and same under ignore-unimportant (the marker `~`), which a display without
        // ignore-unimportant shows apart.
        if !iu {
            let all = DisplayOptions { filter: FolderDisplayFilter::All, ignore_unimportant: true, ..display };
            let same_if_ignored: BTreeSet<String> = folder_compare_json(&cmp, &all)
                .items
                .iter()
                .filter(|item| item.category == "same")
                .map(|item| item.path.clone())
                .collect();
            items = items
                .into_iter()
                .map(|(path, state)| {
                    let unimportant = state == "different" && same_if_ignored.contains(&path);
                    (path, if unimportant { "unimportant".to_owned() } else { state })
                })
                .collect();
        }
        let doc = roxmltree::Document::parse(&xml).unwrap();
        let elements: Vec<roxmltree::Node<'_, '_>> = doc
            .descendants()
            .filter(|n| n.has_tag_name((xmlcheck::NAMESPACE, "entry")) || n.has_tag_name((xmlcheck::NAMESPACE, "folder")))
            .collect();
        let path_of = |n: &roxmltree::Node<'_, '_>| xmlcheck::unescape(n.attribute("path").unwrap()).unwrap();
        let shown_folders: BTreeSet<String> = elements
            .iter()
            .filter(|n| n.has_tag_name((xmlcheck::NAMESPACE, "folder")))
            .map(path_of)
            .collect();
        let mut got = BTreeSet::new();
        for n in &elements {
            // The path and the name read back to the tree's own (`\\`, `\u{…}`).
            let path = path_of(n);
            let name = xmlcheck::unescape(n.attribute("name").unwrap()).unwrap();
            prop_assert_eq!(path.rsplit('/').next(), Some(name.as_str()), "{}", xml);
            // The parent element is the tree parent's, or `tree`.
            let parent = path.rsplit_once('/').map(|(up, _)| up.to_owned()).filter(|up| !flat && shown_folders.contains(up));
            let element = n.parent_element().unwrap();
            let got_parent = element.has_tag_name((xmlcheck::NAMESPACE, "folder")).then(|| path_of(&element));
            prop_assert!(got_parent == parent && (parent.is_some() || element.has_tag_name((xmlcheck::NAMESPACE, "tree"))), "{} below {:?}, not {:?}\n{}", path, got_parent, parent, xml);
            got.insert((path, n.attribute("state").unwrap().to_owned()));
        }
        prop_assert_eq!(got.len(), elements.len());
        prop_assert_eq!(got, items, "{}", xml);
        let counts: Vec<(String, String, u64)> = summary_counts(ReportInput::Folder(&cmp), iu)
            .into_iter()
            .map(|c| (c.group.to_owned(), c.name.to_owned(), c.value))
            .collect();
        prop_assert_eq!(xmlcheck::counts(&doc), counts);
        prop_assert!(!xml.contains('\u{1}') && !xml.contains('\t'), "controls are escaped in attributes");
    }

    /// P-LINKS (A7.9, the href half): every href is relative, is exactly its segments with the RFC 3986 unreserved
    /// bytes (`A–Z a–z 0–9 - . _ ~`) kept and every other byte `%XX` in upper-case hex, joined by `/`, and decodes back
    /// to its segments' bytes.
    #[test]
    fn p_links_hrefs_decode_back(segments in proptest::collection::vec(proptest::collection::vec(prop_oneof![any::<u8>(), proptest::sample::select(&b"~-._:/%aZ09"[..])], 1..12), 1..4)) {
        let link = href(&segments);
        let want: Vec<String> = segments
            .iter()
            .map(|segment| {
                segment
                    .iter()
                    .map(|&b| match b {
                        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => char::from(b).to_string(),
                        _ => format!("%{b:02X}"),
                    })
                    .collect()
            })
            .collect();
        prop_assert_eq!(&link, &want.join("/"));
        prop_assert!(link.bytes().all(|b| b.is_ascii_alphanumeric() || b"-._~%/".contains(&b)), "{}", link);
        prop_assert!(!link.starts_with('/') && !link.contains('\\') && !link.contains(':'));
        prop_assert_eq!(href_segments(&link).unwrap(), segments);
    }
}

/// The folder XML of `display` (checked), and the `(path, state)` the JSON report holds for it.
fn folder_dump(
    cmp: &FolderComparison,
    display: DisplayOptions,
) -> (String, BTreeSet<(String, String)>) {
    let mut defaults = ReportDefaults::new("xml");
    defaults.folder_display = display;
    let xml = render(
        ReportInput::Folder(cmp),
        ReportKind::Folder,
        &ReportWords::default(),
        &defaults,
    );
    xmlcheck::check(xml.as_bytes()).unwrap_or_else(|e| panic!("{e}\n{xml}"));
    let json = folder_compare_json(cmp, &display);
    let items = json
        .items
        .iter()
        .map(|item| {
            let state = match item.category {
                "left_orphan" => "left-only".to_owned(),
                "right_orphan" => "right-only".to_owned(),
                other => other.replace('_', "-"),
            };
            (item.path.clone(), state)
        })
        .collect();
    (xml, items)
}

/// A7.6: a name that is not Unicode keeps a lossy `name` (U+FFFD) and its exact OS bytes in `raw-name`, in the XML; the
/// side-by-side HTML shows the lossy name. On a Linux disk (bytes); on Windows a lone surrogate; macOS refuses such a
/// name (EILSEQ, 92), which is asserted before the test stops.
#[test]
fn a7_6_names_that_are_not_unicode() {
    use hikasa_vfs::{LocalOptions, LocalVfs};
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    for side in ["l", "r"] {
        std::fs::create_dir_all(root.join(side)).unwrap();
    }
    let (name, raw): (std::ffi::OsString, Vec<u8>) = {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let bytes = b"bad\xff.txt".to_vec();
            (std::ffi::OsStr::from_bytes(&bytes).to_owned(), bytes)
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStringExt;
            let wide: Vec<u16> = "bad"
                .encode_utf16()
                .chain([0xD800])
                .chain(".txt".encode_utf16())
                .collect();
            let name = std::ffi::OsString::from_wide(&wide);
            let raw = name.as_encoded_bytes().to_vec();
            (name, raw)
        }
    };
    match std::fs::write(root.join("l").join(&name), b"x") {
        Ok(()) => {}
        #[cfg(target_vendor = "apple")]
        Err(e) if e.raw_os_error() == Some(92) => {
            eprintln!(
                "a7_6_names_that_are_not_unicode: the file system refuses a name that is not UTF-8 (EILSEQ), as APFS does"
            );
            return;
        }
        // Wine keeps names as UTF-8 on the Unix side and cannot hold a lone surrogate (`ERROR_FILE_NOT_FOUND`); the
        // disk part runs on Windows itself, where `WINEPREFIX` is not set.
        #[cfg(windows)]
        Err(e) if std::env::var_os("WINEPREFIX").is_some() => {
            eprintln!(
                "a7_6_names_that_are_not_unicode: wine refuses a name with a lone surrogate ({e}); the disk part runs on Windows"
            );
            return;
        }
        Err(e) => panic!("{e}"),
    }
    std::fs::write(root.join("r").join(&name), b"y").unwrap();
    // A name with a control character (Unix only: Windows refuses U+0001 in a name).
    #[cfg(unix)]
    std::fs::write(root.join("l").join("c\u{1}d.txt"), b"z").unwrap();
    let vfs = LocalVfs::new(LocalOptions::default());
    let spec =
        |p: std::path::PathBuf| hikasa_core::Spec::Local(hikasa_core::LocalPath::new(p).unwrap());
    let cmp = compare_on(
        &vfs,
        spec(root.join("l")),
        spec(root.join("r")),
        &FolderRules::default(),
    );
    let display = DisplayOptions {
        filter: FolderDisplayFilter::All,
        folders: FolderMode::CompareStructure,
        ignore_unimportant: false,
        suppress_filters: false,
    };
    let (xml, _) = folder_dump(&cmp, display);
    let doc = roxmltree::Document::parse(&xml).unwrap();
    let left = doc
        .descendants()
        .find(|n| {
            n.has_tag_name((xmlcheck::NAMESPACE, "left"))
                && n.attribute("name").is_some_and(|v| v.starts_with("bad"))
        })
        .unwrap_or_else(|| panic!("{xml}"));
    assert_eq!(left.attribute("name"), Some("bad\u{fffd}.txt"), "{xml}");
    let hex: String = raw.iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(left.attribute("raw-name"), Some(hex.as_str()), "{xml}");
    let right = left.next_sibling_element().unwrap();
    assert_eq!(right.attribute("raw-name"), Some(hex.as_str()));
    #[cfg(unix)]
    {
        let ctl_node = doc
            .descendants()
            .find(|n| {
                n.has_tag_name((xmlcheck::NAMESPACE, "left"))
                    && n.attribute("name") == Some("c\\u{1}d.txt")
            })
            .unwrap_or_else(|| panic!("{xml}"));
        assert_eq!(
            ctl_node.attribute("raw-name"),
            Some("6301642e747874"),
            "an escaped name carries its bytes"
        );
    }
    let mut defaults = ReportDefaults::new("side-by-side");
    defaults.folder_display = display;
    let html = render(
        ReportInput::Folder(&cmp),
        ReportKind::Folder,
        &words("side-by-side", &[], &["html-color"], None),
        &defaults,
    );
    assert!(html.contains("bad\u{fffd}.txt"), "{html}");
}
