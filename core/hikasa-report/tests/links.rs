//! M2 W7 acceptance A7.9 (D-200), the rendering half: folder side-by-side HTML with a scripted [`PairLinks`] (rows
//! written, skipped and failed) in colour and mono, and the XML `entry/file-report`; which rows link; the `href` table.
//! The names (`safe_component`, the numbering) are `hikasa-session`'s and tested there.

mod common;

use std::collections::BTreeMap;

use common::inputs::compare_memory;
use common::{assert_golden, check_html, fixtures, unix_paths, xmlcheck};
use hikasa_core::CaseSensitivity;
use hikasa_core::FileTime;
use hikasa_folder::{FolderRules, NodeId};
use hikasa_report::links::href_segments;
use hikasa_report::{
    FolderReportOptions, HtmlStyle, PairLink, PairLinks, ReportBody, ReportDefaults, ReportInput,
    ReportKind, ReportMeta, ReportRequest, ReportWords, href, linked_rows, render_report,
};
use hikasa_vfs::{MemMeta, MemoryVfs};

/// A script of links by row.
struct Script(BTreeMap<NodeId, PairLink>);

impl PairLinks for Script {
    fn link(&self, node: NodeId) -> Option<PairLink> {
        self.0.get(&node).cloned()
    }
}

fn meta(secs: i64) -> MemMeta {
    MemMeta {
        mtime: FileTime::from_unix(secs, 0),
        ..MemMeta::default()
    }
}

/// `/l` and `/r`: three differing pairs (one in a folder), a same pair, orphans, a pair that differs in time only.
fn tree() -> MemoryVfs {
    let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
    let file = |path: &str, bytes: &str, secs: i64| {
        vfs.insert_file(&common::host(path), bytes.as_bytes().to_vec(), meta(secs))
            .unwrap();
    };
    file("/l/a b#1.txt", "one", 1_000_000);
    file("/r/a b#1.txt", "two!", 1_000_000);
    file("/l/sub/né.c", "x", 2_000_000);
    file("/r/sub/né.c", "y", 1_000_000);
    file("/l/same.txt", "s", 1_000_000);
    file("/r/same.txt", "s", 1_000_000);
    file("/l/left-only.txt", "l", 1_000_000);
    file("/r/right-only.txt", "r", 1_000_000);
    file("/l/z&+.txt", "zz", 1_000_000);
    file("/r/z&+.txt", "z", 1_000_000);
    vfs
}

fn render(
    cmp: &hikasa_folder::FolderComparison,
    palette: &str,
    links: &Script,
) -> (Vec<u8>, Vec<u8>) {
    let defaults = ReportDefaults::new("side-by-side");
    let words = ReportWords {
        layout: Some("side-by-side".into()),
        options: vec!["include-file-links".into()],
        output_options: vec![palette.into()],
        title: None,
    };
    let req = ReportRequest::parse(ReportKind::Folder, &words, &defaults).unwrap();
    let ReportBody::Folder {
        options,
        links: Some(_),
    } = &req.body
    else {
        panic!("links")
    };
    let meta = ReportMeta::for_folder(cmp, options, None, "hikasa test");
    let mut html = Vec::new();
    render_report(
        ReportInput::Folder(cmp),
        &req,
        &meta,
        &HtmlStyle::default(),
        Some(links),
        &mut html,
    )
    .unwrap();
    let xml_words = ReportWords {
        layout: Some("xml".into()),
        ..ReportWords::default()
    };
    let xml_req = ReportRequest::parse(ReportKind::Folder, &xml_words, &defaults).unwrap();
    let mut xml = Vec::new();
    render_report(
        ReportInput::Folder(cmp),
        &xml_req,
        &meta,
        &HtmlStyle::default(),
        Some(links),
        &mut xml,
    )
    .unwrap();
    (unix_paths(&html), unix_paths(&xml))
}

#[test]
fn a7_9_linked_rows_and_their_rendering() {
    let vfs = tree();
    let cmp = compare_memory(&vfs, &FolderRules::default());
    let options = FolderReportOptions {
        layout: hikasa_report::FolderLayout::SideBySide,
        ..FolderReportOptions::default()
    };
    let rows = linked_rows(&cmp, &options, None);
    let read = cmp.read();
    let paths: Vec<String> = rows
        .iter()
        .map(|id| read.path(*id).as_str().to_owned())
        .collect();
    assert_eq!(
        paths,
        ["sub/né.c", "a b#1.txt", "z&+.txt"],
        "file pairs that differ, in report order"
    );
    let list = FolderReportOptions::default();
    assert!(
        linked_rows(&cmp, &list, None).is_empty(),
        "only side-by-side links"
    );
    drop(read);
    let script = Script(BTreeMap::from([
        (
            rows[0],
            PairLink::Href(href(&["report_files", "1-n\u{e9}.c.html"])),
        ),
        (rows[1], PairLink::Skipped("too_large".into())),
        (rows[2], PairLink::Failed("io: <disk> & \"full\"".into())),
    ]));
    let dir = fixtures("links").join("three_rows");
    for (palette, file) in [
        ("html-color", "expected.sbs.html"),
        ("html-mono", "expected.sbs.mono.html"),
    ] {
        let (html, xml) = render(&cmp, palette, &script);
        let text = std::str::from_utf8(&html).unwrap();
        check_html(text).unwrap();
        assert_eq!(
            text.matches("<a href=\"report_files/1-n%C3%A9.c.html\">")
                .count(),
            2,
            "both side names link"
        );
        assert_eq!(
            text.matches("title=\"no file report: too_large\"").count(),
            2
        );
        assert_eq!(
            text.matches("title=\"no file report: io: &lt;disk&gt; &amp; &quot;full&quot;\"")
                .count(),
            2
        );
        assert_eq!(text.matches("<a href=").count(), 2, "nothing else links");
        assert_golden(&dir.join(file), &html);
        xmlcheck::check(&xml).unwrap();
        assert_golden(&dir.join("expected.xml"), &xml);
        let doc = String::from_utf8(xml).unwrap();
        assert!(
            doc.contains(
                "<file-report status=\"written\" href=\"report_files/1-n%C3%A9.c.html\"/>"
            )
        );
        assert!(doc.contains("<file-report status=\"skipped\" reason=\"too_large\"/>"));
        assert!(doc.contains(
            "<file-report status=\"failed\" reason=\"io: &lt;disk&gt; &amp; &quot;full&quot;\"/>"
        ));
    }
    // Without links, the report is M1's.
    let (plain, _) = render(&cmp, "html-color", &Script(BTreeMap::new()));
    assert!(!String::from_utf8(plain).unwrap().contains("<a "));
}

/// A7.9: the `href` table: ASCII, space, `#`, `%`, `?`, `&`, `+`, non-ASCII (its UTF-8 bytes), bytes that are not UTF-8,
/// `..`, a segment of dots, Windows spellings. Each decodes back to its bytes.
#[test]
fn a7_9_href_table() {
    let table: [(&[u8], &str); 12] = [
        (b"report.html", "report.html"),
        (b"a b", "a%20b"),
        (b"r#1", "r%231"),
        (b"100%", "100%25"),
        (b"q?x=1&y=2+3", "q%3Fx%3D1%26y%3D2%2B3"),
        ("é日".as_bytes(), "%C3%A9%E6%97%A5"),
        (b"bad\xff\xfe", "bad%FF%FE"),
        (b"..", ".."),
        (b"...", "..."),
        (b"C:\\x\\r.html", "C%3A%5Cx%5Cr.html"),
        (b"\\\\?\\C:", "%5C%5C%3F%5CC%3A"),
        (b"~_-.", "~_-."),
    ];
    for (bytes, want) in table {
        let link = href(&[bytes]);
        assert_eq!(link, want, "{bytes:?}");
        assert_eq!(href_segments(&link).unwrap(), [bytes.to_vec()]);
    }
    assert_eq!(href(&["..", "a b.html"]), "../a%20b.html");
    assert_eq!(href(&["f_files", "01-x.html"]), "f_files/01-x.html");
}
