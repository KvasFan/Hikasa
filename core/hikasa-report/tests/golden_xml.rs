//! M2 W7 acceptance A7.5 and A7.6 (D-198): the XML dumps' goldens. `expected.xml` for every text fixture (its display
//! and ignore-unimportant) and every folder fixture (its display options, flatten included), and the hostile text
//! fixture of `tests/fixtures/xml/hostile`: NUL, ESC, U+FFFE, U+FFFF, bidirectional controls, `]]>`, `&`, `<`, `"`,
//! U+FFFD from a lossy decode, emoji sequences, a CR in the title and in a name. Each document renders twice alike and
//! passes the structure checks; `xml_schema.rs` validates the same files against the XSD. The bytes are the same on
//! every platform (Windows host paths spelled as on Unix before the comparison). `HIKASA_BLESS=1` rewrites them.

mod common;

use std::path::Path;

use common::{
    assert_golden, cases, compare_bytes_with, compare_case, fixtures, folder_case,
    sibling_fixtures, unix_paths, xmlcheck,
};
use hikasa_folder::{DisplayOptions, FolderComparison, FolderDisplayFilter, FolderMode};
use hikasa_report::{
    HtmlStyle, ReportDefaults, ReportInput, ReportKind, ReportMeta, ReportRequest, ReportWords,
    render_report,
};
use hikasa_text::{LoadOptions, TextComparison, TextEncoding, TextRules};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[allow(dead_code)]
struct TextCase {
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

impl Default for TextCase {
    fn default() -> TextCase {
        TextCase {
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

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[allow(dead_code)]
struct FolderCase {
    description: String,
    source: String,
    filter: String,
    folders: FolderMode,
    ignore_unimportant: bool,
    suppress_filters: bool,
    columns: String,
    mono: bool,
    ansi: bool,
}

impl Default for FolderCase {
    fn default() -> FolderCase {
        FolderCase {
            description: String::new(),
            source: String::new(),
            filter: "all".into(),
            folders: FolderMode::CompareStructure,
            ignore_unimportant: false,
            suppress_filters: false,
            columns: "size,modified".into(),
            mono: false,
            ansi: false,
        }
    }
}

fn text_inputs(dir: &Path, case: &TextCase) -> std::path::PathBuf {
    match case.source.as_deref() {
        None => dir.to_path_buf(),
        Some(source) => match source.split_once('/') {
            Some(("hikasa-text", name)) => sibling_fixtures("hikasa-text", "compare").join(name),
            Some(("text", name)) => fixtures("text").join(name),
            _ => panic!("{}: unknown source {source}", dir.display()),
        },
    }
}

/// The XML dump of a text comparison (twice: identical bytes), checked.
pub fn text_xml(
    cmp: &TextComparison,
    options: &[String],
    defaults: &ReportDefaults,
    title: Option<&str>,
) -> Vec<u8> {
    let words = ReportWords {
        layout: Some("xml".to_owned()),
        options: options.to_vec(),
        output_options: Vec::new(),
        title: title.map(str::to_owned),
    };
    let req = ReportRequest::parse(ReportKind::Text, &words, defaults).unwrap();
    let hikasa_report::ReportBody::Text(opts) = &req.body else {
        unreachable!()
    };
    let mut meta = ReportMeta::for_text(cmp, opts, None, "hikasa test");
    meta.title = req.title.clone();
    let mut first = Vec::new();
    render_report(
        ReportInput::Text(cmp),
        &req,
        &meta,
        &HtmlStyle::default(),
        None,
        &mut first,
    )
    .unwrap();
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
    assert_eq!(first, second, "INV-RPT-1");
    xmlcheck::check(&first).unwrap_or_else(|e| panic!("{e}\n{}", String::from_utf8_lossy(&first)));
    unix_paths(&first)
}

/// The XML dump of a folder comparison with `display`, checked.
pub fn folder_xml(cmp: &FolderComparison, display: DisplayOptions) -> Vec<u8> {
    let mut defaults = ReportDefaults::new("xml");
    defaults.folder_display = display;
    let req = ReportRequest::parse(ReportKind::Folder, &ReportWords::default(), &defaults).unwrap();
    let hikasa_report::ReportBody::Folder { options, .. } = &req.body else {
        unreachable!()
    };
    let meta = ReportMeta::for_folder(cmp, options, None, "hikasa test");
    let mut first = Vec::new();
    render_report(
        ReportInput::Folder(cmp),
        &req,
        &meta,
        &HtmlStyle::default(),
        None,
        &mut first,
    )
    .unwrap();
    let mut second = Vec::new();
    render_report(
        ReportInput::Folder(cmp),
        &req,
        &meta,
        &HtmlStyle::default(),
        None,
        &mut second,
    )
    .unwrap();
    assert_eq!(first, second, "INV-RPT-1");
    xmlcheck::check(&first).unwrap_or_else(|e| panic!("{e}\n{}", String::from_utf8_lossy(&first)));
    unix_paths(&first)
}

#[test]
fn a7_5_text_xml_of_every_text_fixture() {
    let dirs = cases(&fixtures("text"));
    assert!(dirs.len() >= 35);
    for dir in &dirs {
        let case: TextCase =
            serde_json::from_str(&std::fs::read_to_string(dir.join("case.json")).unwrap()).unwrap();
        let cmp = compare_case(&text_inputs(dir, &case));
        let mut defaults = ReportDefaults::new("xml");
        defaults.context_lines = case.context;
        let mut options = vec![format!("display-{}", case.display)];
        if case.ignore_unimportant {
            options.push("ignore-unimportant".into());
        }
        assert_golden(
            &dir.join("expected.xml"),
            &text_xml(&cmp, &options, &defaults, None),
        );
    }
}

#[test]
fn a7_6_folder_xml_of_every_folder_fixture() {
    let dirs = cases(&fixtures("folder"));
    assert!(dirs.len() >= 25);
    let mut flat = false;
    for dir in &dirs {
        let case: FolderCase =
            serde_json::from_str(&std::fs::read_to_string(dir.join("case.json")).unwrap()).unwrap();
        let source = case.source.strip_prefix("hikasa-folder/").unwrap();
        let (_, cmp) = folder_case(&sibling_fixtures("hikasa-folder", "trees").join(source));
        flat |= cmp.rules().handling.flatten;
        let display = DisplayOptions {
            filter: FolderDisplayFilter::parse(&case.filter).unwrap(),
            folders: case.folders,
            ignore_unimportant: case.ignore_unimportant,
            suppress_filters: case.suppress_filters,
        };
        assert_golden(&dir.join("expected.xml"), &folder_xml(&cmp, display));
    }
    assert!(flat, "a flatten case");
}

/// The hostile inputs of A7.5: every character the escaping must handle, in lines, in the title and in a name.
pub const HOSTILE_LEFT: &[u8] = b"nul \x00 esc \x1b[1m end\n\
fffe \xef\xbf\xbe ffff \xef\xbf\xbf\n\
bidi \xe2\x80\xae txt.exe \xe2\x81\xa6x\xe2\x81\xa9\n\
cdata ]]> & < > \" '\n\
lossy \xff\xfe bytes\n\
emoji \xf0\x9f\x91\xa8\xe2\x80\x8d\xf0\x9f\x91\xa9\xe2\x80\x8d\xf0\x9f\x91\xa7 \xf0\x9f\x87\xa9\xf0\x9f\x87\xaa\n\
tab\there\n\
same\n";
pub const HOSTILE_RIGHT: &[u8] = b"nul \x00 esc \x1b[2m end\n\
fffe \xef\xbf\xbe ffff \xef\xbf\xbf!\n\
bidi \xe2\x80\xae exe.txt \xe2\x81\xa6y\xe2\x81\xa9\n\
cdata ]]> &amp; <b> \" '\n\
lossy \xfe\xff bytes\n\
emoji \xf0\x9f\x91\xa9\xe2\x80\x8d\xf0\x9f\x91\xa8 \xf0\x9f\x87\xa9\xf0\x9f\x87\xaa\n\
tab\tthere\n\
same\n\
right only \x01\x02\x0b\x0c\x7f\xc2\x85\n";

#[test]
fn a7_5_hostile_text_xml() {
    let mut load = LoadOptions::default();
    load.decode.override_encoding = Some(TextEncoding::UTF_8);
    let cmp = compare_bytes_with(
        ("h&'.txt", HOSTILE_LEFT),
        ("h.txt", HOSTILE_RIGHT),
        &TextRules::default(),
        &load,
    );
    let defaults = ReportDefaults::new("xml");
    let bytes = text_xml(
        &cmp,
        &[],
        &defaults,
        Some("title with CR\r, LF\n, NUL\u{0} & <tags> \"quoted\""),
    );
    let dir = fixtures("xml").join("hostile");
    assert_golden(&dir.join("expected.xml"), &bytes);
    let text = String::from_utf8(bytes.clone()).unwrap();
    assert!(text.contains("<c x=\"0\"/>") && text.contains("<c x=\"1b\"/>"));
    assert!(text.contains("<c x=\"fffe\"/>") && text.contains("<c x=\"ffff\"/>"));
    assert!(text.contains("]]&gt;"), "]]> never appears raw");
    assert!(text.contains("&#13;"), "the CR of the title");
    assert!(text.contains('\u{202e}'), "bidirectional controls are data");
    assert!(text.contains('\u{fffd}'), "a lossy decode");
    // The round trip: every line of both documents, and the title.
    let doc = roxmltree::Document::parse(&text).unwrap();
    for (left, raw) in [(true, HOSTILE_LEFT), (false, HOSTILE_RIGHT)] {
        let want: Vec<String> = String::from_utf8_lossy(raw)
            .lines()
            .map(str::to_owned)
            .collect();
        assert_eq!(xmlcheck::side_lines(&doc, left), want);
    }
    let title = doc
        .descendants()
        .find(|n| n.has_tag_name((xmlcheck::NAMESPACE, "title")))
        .map(|n| xmlcheck::line_text(n))
        .unwrap();
    assert_eq!(title, "title with CR\r, LF\n, NUL\u{0} & <tags> \"quoted\"");
}
