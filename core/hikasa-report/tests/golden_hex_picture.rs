//! Hex and picture reports and JSON (§1.12.24, §1.12.25; §6.12 A12.23): a golden file for every hex layout (summary,
//! side-by-side with and without addresses and under each display filter, interleaved; text, ANSI and HTML) and
//! picture layout (summary as text and HTML; side-by-side HTML scaled and full-size); the HTML is well-formed and
//! self-contained; the JSON documents and the CLI's JSON goldens validate against `docs/schemas/` (closed schemas;
//! planted violations are refused). `HIKASA_BLESS=1` rewrites the goldens.

mod common;

use std::path::Path;
use std::sync::Arc;

use common::jsonschema::{closed, validate};
use common::{assert_golden, check_html, fixtures};
use hikasa_core::{ContentKind, Ctl, Pair, Warning, WarningCode};
use hikasa_hex::{HexAlignOptions, HexDocument, HexLayout, HexLoad, align};
use hikasa_picture::{
    DiffMode, Geometry, Orientation, Picture, PictureColors, PictureOptions, PixelProbe, ToneMap,
    compare_pictures,
};
use hikasa_report::json::{
    HexDumpJson, HexLayoutJson, HexSideJson, ValuesBlockJson, hex_compare_json, hex_dump_rows,
    picture_compare_json,
};
use hikasa_report::{
    DiffOutput, HexSide, HexSource, HtmlStyle, PictureSide, PictureSource, ReportDefaults,
    ReportInput, ReportKind, ReportMeta, ReportRequest, ReportWords, TargetKind, render_report,
};
use serde_json::Value;

/// Seeded xorshift bytes.
fn bytes(seed: u64, n: usize) -> Vec<u8> {
    let mut x = seed | 1;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

/// The hex case: 200 bytes; the right side with a changed run, an insertion and a changed byte.
struct HexCase {
    docs: Pair<HexDocument>,
    alignment: hikasa_hex::HexAlignment,
    options: HexAlignOptions,
    sides: Pair<HexSide>,
    warnings: Vec<Warning>,
}

fn hex_case() -> HexCase {
    let left = bytes(5, 200);
    let mut right = left.clone();
    right[20..24].copy_from_slice(b"WXYZ");
    right.splice(100..100, b"+inserted+".iter().copied());
    right[190] ^= 0x20;
    let docs = Pair::new(
        HexDocument::from_bytes("left.bin", left.into()),
        HexDocument::from_bytes("right.bin", right.into()),
    );
    let options = HexAlignOptions::default();
    let alignment = align(&docs.left, &docs.right, &options, &Ctl::detached()).unwrap();
    let side = |spec: &str, size: u64| HexSide {
        spec: spec.to_owned(),
        size,
        mtime: Some(jiff::Timestamp::from_second(1_700_000_000).unwrap()),
        format: None,
        converted: false,
        load: HexLoad::Memory,
    };
    HexCase {
        sides: Pair::new(side("/l/left.bin", 200), side("/r/right.bin", 210)),
        warnings: vec![Warning {
            code: WarningCode::InputChanged,
            message: "/r/right.bin: changed during the comparison".into(),
            spec: Some("/r/right.bin".into()),
        }],
        docs,
        alignment,
        options,
    }
}

impl HexCase {
    fn source(&self, layout: HexLayout) -> HexSource<'_> {
        HexSource {
            documents: Pair::new(&self.docs.left, &self.docs.right),
            alignment: &self.alignment,
            options: &self.options,
            sides: Pair::new(&self.sides.left, &self.sides.right),
            kind: ContentKind::BinaryDifferent,
            stale: true,
            layout,
            max_sections: 20,
            preview: 6,
            warnings: &self.warnings,
        }
    }
}

fn meta() -> ReportMeta {
    ReportMeta {
        title: None,
        generated_at: None,
        generator: "hikasa test".into(),
        mode: None,
        filters: None,
        errors: Pair::new(None, None),
        folder_link: None,
    }
}

/// Renders `input` with `words` (layout, options, output options).
fn render(
    kind: ReportKind,
    input: ReportInput<'_>,
    layout: &str,
    options: &[&str],
    output: &[&str],
    target: TargetKind,
) -> Vec<u8> {
    let words = ReportWords {
        layout: Some(layout.to_owned()),
        options: options.iter().map(|w| (*w).to_owned()).collect(),
        output_options: output.iter().map(|w| (*w).to_owned()).collect(),
        title: None,
    };
    let mut req = ReportRequest::parse(kind, &words, &ReportDefaults::new("side-by-side")).unwrap();
    req.check_target(target).unwrap();
    req.apply_target(target);
    let mut out = Vec::new();
    render_report(input, &req, &meta(), &HtmlStyle::default(), None, &mut out).unwrap();
    out
}

/// A golden case: file name, layout, option words, output words, target.
type Case = (
    &'static str,
    &'static str,
    &'static [&'static str],
    &'static [&'static str],
    TargetKind,
);

const FILE: TargetKind = TargetKind::File;
const TERMINAL: TargetKind = TargetKind::Stdout { ansi: true };

fn self_contained(name: &str, html: &str) {
    check_html(html).unwrap_or_else(|e| panic!("{name}: {e}"));
    for external in ["http:", "https:", "src=\"/", "url(", "@import"] {
        assert!(!html.contains(external), "{name}: {external}");
    }
}

/// A12.23: every hex layout and form.
#[test]
fn hex_report_goldens() {
    let case = hex_case();
    let layout = HexLayout {
        bytes_per_line: 16,
        group: 4,
        ..HexLayout::default()
    };
    let src = case.source(layout);
    let dir = fixtures("hex");
    let cases: &[Case] = &[
        ("summary.txt", "summary", &[], &[], FILE),
        ("summary.html", "summary", &[], &["html-color"], FILE),
        ("sbs.txt", "side-by-side", &[], &[], FILE),
        ("sbs_ansi.txt", "side-by-side", &[], &[], TERMINAL),
        (
            "sbs_no_addresses.txt",
            "side-by-side",
            &["no-line-numbers"],
            &[],
            FILE,
        ),
        (
            "sbs_differences.txt",
            "side-by-side",
            &["display-differences", "context-1"],
            &[],
            FILE,
        ),
        ("sbs_same.txt", "side-by-side", &["display-same"], &[], FILE),
        ("sbs_none.txt", "side-by-side", &["display-none"], &[], FILE),
        (
            "sbs.html",
            "side-by-side",
            &["display-differences"],
            &["html-color"],
            FILE,
        ),
        ("sbs_mono.html", "side-by-side", &[], &["html-mono"], FILE),
        ("interleaved.txt", "interleaved", &[], &[], FILE),
        (
            "interleaved.html",
            "interleaved",
            &["no-line-numbers"],
            &["html-color"],
            FILE,
        ),
    ];
    for (name, layout, options, output, target) in cases {
        let bytes = render(
            ReportKind::Hex,
            ReportInput::Hex(&src),
            layout,
            options,
            output,
            *target,
        );
        let text = String::from_utf8(bytes.clone()).unwrap();
        if name.ends_with(".html") {
            self_contained(name, &text);
        }
        assert_golden(&dir.join(name), &bytes);
        // Deterministic bytes.
        let again = render(
            ReportKind::Hex,
            ReportInput::Hex(&src),
            layout,
            options,
            output,
            *target,
        );
        assert_eq!(bytes, again, "{name}");
    }
    // Decimal addresses and Latin-1 characters reach the rows.
    let decimal = case.source(HexLayout {
        bytes_per_line: 7,
        address: hikasa_hex::Radix::Decimal,
        chars: hikasa_hex::CharSet::Latin1,
        ..HexLayout::default()
    });
    let text = render(
        ReportKind::Hex,
        ReportInput::Hex(&decimal),
        "side-by-side",
        &[],
        &[],
        FILE,
    );
    assert_golden(&dir.join("sbs_decimal.txt"), &text);
    // Words of another kind are refused.
    for word in ["full-size", "cells-all", "patch-unified", "context-65"] {
        let words = ReportWords {
            layout: Some("summary".into()),
            options: vec![word.into()],
            ..ReportWords::default()
        };
        assert!(
            ReportRequest::parse(ReportKind::Hex, &words, &ReportDefaults::new("summary")).is_err(),
            "{word}"
        );
    }
}

/// The picture case: an 8 × 6 gradient and a 9 × 6 copy with differences and a transparent corner.
fn pictures() -> Pair<Arc<Picture>> {
    let make = |w: u32, f: &dyn Fn(u32, u32) -> [u8; 4]| {
        let mut rgba = Vec::new();
        for y in 0..6 {
            for x in 0..w {
                rgba.extend_from_slice(&f(x, y));
            }
        }
        Arc::new(Picture::from_rgba(w, 6, rgba).unwrap())
    };
    let base = |x: u32, y: u32| [(x * 30) as u8, (y * 40) as u8, 90, 255];
    Pair::new(
        make(8, &base),
        make(9, &|x, y| {
            let mut p = base(x, y);
            if x == 3 {
                p[0] += 3;
            }
            if (x, y) == (6, 2) {
                p = [255, 0, 0, 255];
            }
            if (x, y) == (0, 0) {
                p[3] = 0;
            }
            p
        }),
    )
}

fn picture_source<'a>(
    cmp: &'a hikasa_picture::PictureComparison,
    sides: &'a Pair<PictureSide>,
    colors: &'a PictureColors,
    probes: &'a [PixelProbe],
    out: Option<&'a DiffOutput>,
    warnings: &'a [Warning],
) -> PictureSource<'a> {
    PictureSource {
        comparison: cmp,
        sides: Pair::new(&sides.left, &sides.right),
        kind: ContentKind::ImportantDifferences,
        ignore_unimportant: false,
        mode: DiffMode::Tolerance,
        tone_map: ToneMap::Clamp,
        colors,
        probes,
        diff_output: out,
        warnings,
    }
}

fn picture_sides() -> Pair<PictureSide> {
    let side = |spec: &str| PictureSide {
        spec: spec.to_owned(),
        size: 120,
        mtime: None,
        format: None,
    };
    Pair::new(side("/l/a.png"), side("/r/b.png"))
}

/// A12.23: the picture summary (text, HTML) and the side-by-side HTML (scaled, full-size), images embedded.
#[test]
fn picture_report_goldens() {
    let p = pictures();
    let options = PictureOptions {
        tolerance: 4,
        geometry: Geometry {
            orientation: Pair::new(Orientation::Identity, Orientation::Identity),
            auto_scale: false,
            offset: (0, 0),
        },
    };
    let cmp = compare_pictures(
        Arc::clone(&p.left),
        Arc::clone(&p.right),
        &options,
        &Ctl::detached(),
    )
    .unwrap();
    let sides = picture_sides();
    let colors = PictureColors::default();
    let probes = vec![cmp.probe(3, 1), cmp.probe(8, 0)];
    let src = picture_source(&cmp, &sides, &colors, &probes, None, &[]);
    let dir = fixtures("picture");
    let cases: &[Case] = &[
        ("summary.txt", "summary", &[], &[], FILE),
        (
            "summary_ansi.txt",
            "summary",
            &["ignore-unimportant"],
            &[],
            TERMINAL,
        ),
        ("summary.html", "summary", &[], &["html-color"], FILE),
        (
            "sbs.html",
            "side-by-side",
            &["checkerboard"],
            &["html-color"],
            FILE,
        ),
        (
            "sbs_full_range.html",
            "side-by-side",
            &["full-size", "mode-range"],
            &["html-mono"],
            FILE,
        ),
    ];
    for (name, layout, opts, output, target) in cases {
        let bytes = render(
            ReportKind::Picture,
            ReportInput::Picture(&src),
            layout,
            opts,
            output,
            *target,
        );
        let text = String::from_utf8(bytes.clone()).unwrap();
        if name.ends_with(".html") {
            self_contained(name, &text);
        }
        if layout == &"side-by-side" {
            assert_eq!(
                text.matches("src=\"data:image/png;base64,").count(),
                3,
                "{name}"
            );
        }
        assert_golden(&dir.join(name), &bytes);
    }
    // A side-by-side report has no text form.
    let words = ReportWords {
        layout: Some("side-by-side".into()),
        output_options: vec!["text".into()],
        ..ReportWords::default()
    };
    assert!(
        ReportRequest::parse(ReportKind::Picture, &words, &ReportDefaults::new("summary")).is_err()
    );
    // Scaled down: a picture wider than 1,024 pixels is embedded at a level that fits, full-size as it is.
    let wide = Arc::new(Picture::from_rgba(2100, 2, vec![128; 2100 * 2 * 4]).unwrap());
    let cmp = compare_pictures(
        Arc::clone(&wide),
        wide,
        &PictureOptions::default(),
        &Ctl::detached(),
    )
    .unwrap();
    let src = picture_source(&cmp, &sides, &colors, &[], None, &[]);
    let html = String::from_utf8(render(
        ReportKind::Picture,
        ReportInput::Picture(&src),
        "side-by-side",
        &[],
        &["html-color"],
        FILE,
    ))
    .unwrap();
    assert!(html.contains("width=\"525\" height=\"1\""), "scaled by 4");
    let html = String::from_utf8(render(
        ReportKind::Picture,
        ReportInput::Picture(&src),
        "side-by-side",
        &["full-size"],
        &["html-color"],
        FILE,
    ))
    .unwrap();
    assert!(html.contains("width=\"2100\" height=\"2\""));
}

fn schema(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/schemas")
        .join(name);
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn assert_valid(schema_name: &str, what: &str, doc: &Value) {
    let errors = validate(&schema(schema_name), doc);
    assert!(errors.is_empty(), "{what}:\n{}", errors.join("\n"));
}

/// The four schemas are closed; generated documents and the CLI's goldens validate; planted violations are refused.
#[test]
fn hex_and_picture_json_validate() {
    for name in [
        "hex_compare.v1.json",
        "hex_dump.v1.json",
        "hex_find.v1.json",
        "picture_compare.v1.json",
    ] {
        let errors = closed(&schema(name), &[]);
        assert!(errors.is_empty(), "{name}: {}", errors.join("\n"));
    }
    let case = hex_case();
    let doc = serde_json::to_value(hex_compare_json(&case.source(HexLayout::default())).unwrap())
        .unwrap();
    assert_golden(
        &fixtures("json").join("hex_compare.json"),
        serde_json::to_string_pretty(&doc).unwrap().as_bytes(),
    );
    assert_valid("hex_compare.v1.json", "generated hex", &doc);
    let mut planted = doc.clone();
    planted["sections"][0]["colour"] = Value::from("red");
    assert!(!validate(&schema("hex_compare.v1.json"), &planted).is_empty());
    let mut planted = doc.clone();
    planted["stats"]["sections"] = Value::from("3");
    assert!(!validate(&schema("hex_compare.v1.json"), &planted).is_empty());
    // A dump with values (NaN, infinities and 64-bit strings).
    let layout = HexLayout::default();
    let raw = [0x00, 0x00, 0xc0, 0x7f, 0x00, 0x00, 0x80, 0xff];
    let values = hikasa_hex::values::from_bytes(&raw);
    let dump = HexDumpJson {
        side: HexSideJson::new(&case.sides.left),
        layout: HexLayoutJson::new(&layout),
        offset: 0,
        length: 8,
        rows: hex_dump_rows(&layout, 0, &raw),
        values: Some(ValuesBlockJson::new(0, &values)),
    };
    let doc = serde_json::to_value(&dump).unwrap();
    assert_eq!(doc["values"]["little"]["f32"], "NaN");
    assert_valid("hex_dump.v1.json", "generated dump", &doc);
    // A picture comparison with a diff output and warnings.
    let p = pictures();
    let options = PictureOptions {
        tolerance: 2,
        geometry: Geometry {
            orientation: Pair::new(Orientation::Cw90, Orientation::Cw90),
            auto_scale: true,
            offset: (1, -1),
        },
    };
    let cmp = compare_pictures(
        Arc::clone(&p.left),
        Arc::clone(&p.right),
        &options,
        &Ctl::detached(),
    )
    .unwrap();
    let sides = picture_sides();
    let colors = PictureColors::default();
    let probes = vec![cmp.probe(0, 0)];
    let out = DiffOutput {
        target: "/o/d.png".into(),
        pane: hikasa_picture::Pane::Difference,
        mode: DiffMode::Range,
        width: 9,
        height: 6,
        checkerboard: true,
    };
    let warnings = vec![Warning {
        code: WarningCode::AspectRatioDiffers,
        message: "aspect".into(),
        spec: None,
    }];
    let src = picture_source(&cmp, &sides, &colors, &probes, Some(&out), &warnings);
    let doc = serde_json::to_value(picture_compare_json(&src)).unwrap();
    assert_golden(
        &fixtures("json").join("picture_compare.json"),
        serde_json::to_string_pretty(&doc).unwrap().as_bytes(),
    );
    assert_valid("picture_compare.v1.json", "generated picture", &doc);
    let mut planted = doc.clone();
    planted["geometry"]["scaled"] = Value::from("middle");
    assert!(!validate(&schema("picture_compare.v1.json"), &planted).is_empty());
    // The CLI's goldens: the `result` of each envelope.
    for (suite, prefix) in [("hex", "hikasa.hex_"), ("picture", "hikasa.picture_")] {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../cli/hikasa-cli/tests/goldens")
            .join(suite);
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for path in entries.map(|e| e.unwrap().path()) {
            if path.extension().is_none_or(|e| e != "json") {
                continue;
            }
            let doc: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            let id = doc["schema"].as_str().unwrap_or_default();
            if !id.starts_with(prefix) || doc["result"].is_null() {
                continue;
            }
            let file = format!("{}.json", id.trim_start_matches("hikasa."));
            assert_valid(&file, &path.display().to_string(), &doc["result"]);
        }
    }
}
