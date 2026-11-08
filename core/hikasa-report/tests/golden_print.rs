//! M2 W7 acceptance A7.8 (D-210): HTML wrap and the print block. Without a wrap or print word every HTML byte is M1's:
//! every text and folder fixture rendered through [`render_report`] (with and without `wrap-character`) equals the M1
//! renderer's bytes, which `golden_text.rs` and `golden_folder.rs` pin to the M1 goldens. New goldens in
//! `tests/fixtures/print`: text side-by-side HTML × `wrap-none`, `wrap-word` (the interleaved ones are
//! `golden_interleaved.rs`'s variants); the print block × {portrait, landscape} × {colour, mono} for text
//! side-by-side, text summary and folder side-by-side; a screen palette with a different print palette. The printer
//! target makes the block a `print-portrait` word makes. `HIKASA_BLESS=1` rewrites the goldens.

mod common;

use common::{
    assert_golden, cases, check_html, compare_case, fixtures, folder_case, sibling_fixtures,
    unix_paths,
};
use hikasa_folder::FolderComparison;
use hikasa_report::{
    HtmlStyle, ReportBody, ReportDefaults, ReportInput, ReportKind, ReportMeta, ReportRequest,
    ReportWords, TargetKind, render_folder_report, render_report, render_text_report,
};
use hikasa_text::TextComparison;

fn words(layout: &str, output: &[&str]) -> ReportWords {
    ReportWords {
        layout: Some(layout.to_owned()),
        options: Vec::new(),
        output_options: output.iter().map(|s| (*s).to_owned()).collect(),
        title: None,
    }
}

fn request(kind: ReportKind, layout: &'static str, output: &[&str]) -> ReportRequest {
    let req = ReportRequest::parse(kind, &words(layout, output), &ReportDefaults::new(layout))
        .unwrap_or_else(|e| panic!("{layout} {output:?}: {e}"));
    assert!(
        req.unused.is_empty(),
        "{layout} {output:?}: {:?}",
        req.unused
    );
    req
}

/// Renders `req` twice (identical bytes, INV-RPT-1), checks the HTML (P-RPT-3).
fn render(input: ReportInput<'_>, req: &ReportRequest) -> Vec<u8> {
    let meta = match (input, &req.body) {
        (ReportInput::Text(cmp), ReportBody::Text(opts)) => {
            ReportMeta::for_text(cmp, opts, None, "hikasa test")
        }
        (ReportInput::Folder(cmp), ReportBody::Folder { options, .. }) => {
            ReportMeta::for_folder(cmp, options, None, "hikasa test")
        }
        _ => unreachable!(),
    };
    let mut first = Vec::new();
    let warnings =
        render_report(input, req, &meta, &HtmlStyle::default(), None, &mut first).unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let mut second = Vec::new();
    render_report(input, req, &meta, &HtmlStyle::default(), None, &mut second).unwrap();
    assert_eq!(first, second, "rendering is deterministic (INV-RPT-1)");
    check_html(std::str::from_utf8(&first).unwrap()).unwrap();
    unix_paths(&first)
}

/// The M1 renderer's bytes for `req`'s options.
fn m1(input: ReportInput<'_>, req: &ReportRequest) -> Vec<u8> {
    let mut out = Vec::new();
    match (input, &req.body) {
        (ReportInput::Text(cmp), ReportBody::Text(opts)) => {
            let meta = ReportMeta::for_text(cmp, opts, None, "hikasa test");
            render_text_report(cmp, opts, &meta, &mut out).unwrap();
        }
        (ReportInput::Folder(cmp), ReportBody::Folder { options, .. }) => {
            let meta = ReportMeta::for_folder(cmp, options, None, "hikasa test");
            render_folder_report(cmp, options, &meta, &mut out).unwrap();
        }
        _ => unreachable!(),
    }
    unix_paths(&out)
}

/// The folder holding a text case's inputs (its `source`, as `golden_text.rs` reads it).
fn text_inputs(dir: &std::path::Path) -> std::path::PathBuf {
    let case: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("case.json")).unwrap()).unwrap();
    match case["source"].as_str().map(|s| s.split_once('/')) {
        None => dir.to_path_buf(),
        Some(Some(("hikasa-text", name))) => sibling_fixtures("hikasa-text", "compare").join(name),
        Some(Some(("text", name))) => fixtures("text").join(name),
        Some(_) => panic!("{}: unknown source", dir.display()),
    }
}

/// The text fixture of the new goldens: changes spread over 30 lines, gap-free in display all.
fn text_cmp() -> TextComparison {
    compare_case(&fixtures("text").join("r01_hunks"))
}

/// The folder fixture of the new goldens: newer files on both sides.
fn folder_cmp() -> FolderComparison {
    folder_case(&sibling_fixtures("hikasa-folder", "trees").join("02_newer_files")).1
}

/// The stylesheet of an HTML report: the text of its `<style>` element.
fn style_of(html: &[u8]) -> String {
    let html = std::str::from_utf8(html).unwrap();
    let start = html.find("<style>").unwrap() + "<style>".len();
    let end = html[start..].find("</style>").unwrap() + start;
    html[start..end].to_owned()
}

#[test]
fn a7_8_without_wrap_or_print_words_every_html_byte_is_m1s() {
    let text = cases(&fixtures("text"));
    assert!(text.len() >= 35, "text fixtures: {}", text.len());
    for dir in &text {
        let cmp = compare_case(&text_inputs(dir));
        for layout in ["side-by-side", "summary"] {
            for palette in ["html-color", "html-mono"] {
                let plain = request(ReportKind::Text, layout, &[palette]);
                let expected = m1(ReportInput::Text(&cmp), &plain);
                assert_eq!(
                    render(ReportInput::Text(&cmp), &plain),
                    expected,
                    "{}: {layout} {palette}",
                    dir.display()
                );
                if layout == "side-by-side" {
                    let character = request(ReportKind::Text, layout, &[palette, "wrap-character"]);
                    assert_eq!(
                        render(ReportInput::Text(&cmp), &character),
                        expected,
                        "{}: wrap-character is M1's rule",
                        dir.display()
                    );
                }
            }
        }
    }
    let folders = cases(&fixtures("folder"));
    assert!(folders.len() >= 25, "folder fixtures: {}", folders.len());
    for dir in &folders {
        let case = std::fs::read_to_string(dir.join("case.json")).unwrap();
        let case: serde_json::Value = serde_json::from_str(&case).unwrap();
        let source = case["source"]
            .as_str()
            .and_then(|s| s.strip_prefix("hikasa-folder/"))
            .unwrap_or_else(|| panic!("{}: source", dir.display()));
        let (_, cmp) = folder_case(&sibling_fixtures("hikasa-folder", "trees").join(source));
        for layout in ["side-by-side", "summary"] {
            for palette in ["html-color", "html-mono"] {
                let plain = request(ReportKind::Folder, layout, &[palette]);
                assert_eq!(
                    render(ReportInput::Folder(&cmp), &plain),
                    m1(ReportInput::Folder(&cmp), &plain),
                    "{}: {layout} {palette}",
                    dir.display()
                );
            }
        }
    }
}

#[test]
fn a7_8_text_side_by_side_wrap_goldens() {
    let dir = fixtures("print");
    let cmp = text_cmp();
    let plain = style_of(&render(
        ReportInput::Text(&cmp),
        &request(ReportKind::Text, "side-by-side", &["html-color"]),
    ));
    for (file, output, rule) in [
        (
            "text.sbs.wrap-none.html",
            &["wrap-none", "html-color"][..],
            "table.sbs td.t { white-space: pre; overflow-wrap: normal; }\n",
        ),
        (
            "text.sbs.wrap-word.html",
            &["wrap-word", "html-color"][..],
            "table.sbs td.t { overflow-wrap: break-word; }\n",
        ),
        (
            "text.sbs.wrap-word.mono.html",
            &["wrap-word", "html-mono"][..],
            "table.sbs td.t { overflow-wrap: break-word; }\n",
        ),
    ] {
        let bytes = render(
            ReportInput::Text(&cmp),
            &request(ReportKind::Text, "side-by-side", output),
        );
        let css = style_of(&bytes);
        assert!(css.contains(rule), "{file}: {rule}");
        assert!(!css.contains("@media print"), "{file}: no print block");
        if output.contains(&"html-color") {
            // The wrap rules follow the layout's own and change nothing else.
            assert_eq!(
                css.replacen(rule, "", 1).replacen(
                    "table.sbs { width: auto; table-layout: auto; }\n",
                    "",
                    1
                ),
                plain,
                "{file}"
            );
        }
        assert_golden(&dir.join(file), &bytes);
    }
}

/// The print goldens: (file stem, kind, layout).
const PRINTED: [(&str, ReportKind, &str); 3] = [
    ("text.sbs", ReportKind::Text, "side-by-side"),
    ("text.summary", ReportKind::Text, "summary"),
    ("folder.sbs", ReportKind::Folder, "side-by-side"),
];

#[test]
fn a7_8_print_block_goldens() {
    let dir = fixtures("print");
    let text = text_cmp();
    let folder = folder_cmp();
    for (stem, kind, layout) in PRINTED {
        let input = match kind {
            ReportKind::Text => ReportInput::Text(&text),
            _ => ReportInput::Folder(&folder),
        };
        for orientation in ["portrait", "landscape"] {
            for (palette, screen, print) in [
                ("color", "html-color", "print-color"),
                ("mono", "html-mono", "print-mono"),
            ] {
                let file = format!("{stem}.{orientation}.{palette}.html");
                let word = format!("print-{orientation}");
                let bytes = render(input, &request(kind, layout, &[&word, screen, print]));
                let css = style_of(&bytes);
                let page = format!("@page {{ size: {orientation}; margin: 12mm; }}\n");
                assert!(css.contains(&page), "{file}");
                for rule in [
                    "@media print {\n",
                    "print-color-adjust: exact;",
                    "thead { display: table-header-group; }",
                    "tr { break-inside: avoid; page-break-inside: avoid; }",
                ] {
                    assert!(css.contains(rule), "{file}: {rule}");
                }
                assert!(!css.contains("@media screen"), "{file}: one palette");
                // The block is appended: the stylesheet before it is the one without the block.
                let plain = style_of(&render(input, &request(kind, layout, &[screen])));
                assert!(css.starts_with(&plain), "{file}: the block is appended");
                assert_golden(&dir.join(&file), &bytes);
            }
        }
    }
}

#[test]
fn a7_8_a_print_palette_other_than_the_screens() {
    let dir = fixtures("print");
    let text = text_cmp();
    let folder = folder_cmp();
    for (file, kind, layout, screen, print) in [
        (
            "text.sbs.screen-color.print-mono.html",
            ReportKind::Text,
            "side-by-side",
            "html-color",
            "print-mono",
        ),
        (
            "text.summary.screen-mono.print-color.html",
            ReportKind::Text,
            "summary",
            "html-mono",
            "print-color",
        ),
        (
            "folder.sbs.screen-color.print-mono.html",
            ReportKind::Folder,
            "side-by-side",
            "html-color",
            "print-mono",
        ),
    ] {
        let input = match kind {
            ReportKind::Text => ReportInput::Text(&text),
            _ => ReportInput::Folder(&folder),
        };
        let bytes = render(
            input,
            &request(kind, layout, &["print-landscape", screen, print]),
        );
        let css = style_of(&bytes);
        // Each palette is scoped to its medium: the screen's under `@media screen`, the print one's under
        // `@media print`, then the block.
        let screen_at = css
            .find("@media screen {\n")
            .unwrap_or_else(|| panic!("{file}"));
        let print_at = css[screen_at..].find("}\n@media print {\n").unwrap() + screen_at;
        let screen_css = style_of(&render(input, &request(kind, layout, &[screen])));
        let other = if screen == "html-color" {
            "html-mono"
        } else {
            "html-color"
        };
        let print_css = style_of(&render(input, &request(kind, layout, &[other])));
        // The rules before the palette are shared by the two stylesheets.
        let shared = screen_css
            .bytes()
            .zip(print_css.bytes())
            .take_while(|(a, b)| a == b)
            .count();
        let shared = screen_css[..shared].rfind('\n').map_or(0, |n| n + 1);
        assert_eq!(
            &css[..screen_at],
            &screen_css[..shared],
            "{file}: shared rules"
        );
        assert_eq!(
            &css[screen_at + "@media screen {\n".len()..print_at],
            &screen_css[shared..],
            "{file}: the screen palette"
        );
        let after = &css[print_at + "}\n@media print {\n".len()..];
        assert!(
            after.starts_with(&print_css[shared..]),
            "{file}: the print palette"
        );
        assert!(
            after.contains("@page { size: landscape; margin: 12mm; }\n"),
            "{file}: the block"
        );
        assert_golden(&dir.join(file), &bytes);
    }
}

#[test]
fn a7_8_print_mono_alone_adds_no_block_and_the_printer_target_does() {
    let text = text_cmp();
    let input = ReportInput::Text(&text);
    // `print-mono` without an orientation or the printer target: no block, M1's bytes.
    let alone = request(
        ReportKind::Text,
        "side-by-side",
        &["html-color", "print-mono"],
    );
    assert!(alone.print.is_none() && alone.print_mono);
    let plain = request(ReportKind::Text, "side-by-side", &["html-color"]);
    assert_eq!(render(input, &alone), m1(input, &plain));
    // The printer target: the print palette and a portrait block, as `print-portrait` with the same palettes.
    let mut printed = request(ReportKind::Text, "side-by-side", &["print-mono"]);
    printed.check_target(TargetKind::Printer).unwrap();
    printed.apply_target(TargetKind::Printer);
    let golden = std::fs::read(fixtures("print").join("text.sbs.portrait.mono.html")).unwrap();
    assert_eq!(render(input, &printed), golden);
    // The browser target: the print palette, no block.
    let mut browsed = request(ReportKind::Text, "side-by-side", &["print-mono"]);
    browsed.apply_target(TargetKind::Browser);
    assert!(browsed.print.is_none());
    let mono = request(ReportKind::Text, "side-by-side", &["html-mono"]);
    assert_eq!(render(input, &browsed), m1(input, &mono));
}
