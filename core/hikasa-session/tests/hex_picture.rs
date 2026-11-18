//! W12c in the Service (§6.12 A12.17, A12.19, A12.22, A12.24, A12.25, A12.26): `hex_compare` and `picture_compare`
//! with every exit code, the conversion rows of D-331 with a fake converter, `stale`, stdin and archive members,
//! `hex dump` and `hex find`; the folder content test and `quick --rules` on pictures (D-344); the two session types
//! (round trip, unknown keys, `session run`, Swap Sides, `dirty`); the `--gui` requests' round trip (D-350); the
//! clipboard image of `picture_write`.

mod common;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::Write as _;
use std::sync::Arc;
use std::time::Duration;

use common::{job, os, service, service_with, write};
use hikasa_core::{CancelToken, ContentKind, ErrorKind, JobKind, Pair};
use hikasa_folder::{DisplayOptions, FolderDisplayFilter, FolderMode, FolderRules, NullObserver};
use hikasa_formats::{ConvertError, ProcessRunner};
use hikasa_picture::{PngLevel, RgbaImage, encode_png};
use hikasa_session::api::{
    FolderCompareRequest, GuiOpenRequest, HexCompareRequest, HexDumpRequest, HexFindRequest,
    HexLayoutWords, HexRuleWords, NoHexEvents, PictureCompareRequest, PictureGeometryWords,
    PictureRuleWords, QuickMethod, QuickRequest, ReportTarget, SessionRunRequest, ViewRequest,
};
use hikasa_session::sessions::{Session, SessionRef, SessionRules, SessionType, is_dirty};
use hikasa_session::{ApiError, Service};

// ---- inputs ---------------------------------------------------------------------------------------------------------

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

/// A PNG of `w` × `h` with the pixel function `f`.
pub fn png(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            rgba.extend_from_slice(&f(x, y));
        }
    }
    encode_png(
        &RgbaImage {
            width: w,
            height: h,
            rgba,
        },
        PngLevel::Fast,
    )
    .unwrap()
}

/// A 24-bit BMP of `w` × `h` (opaque pixels of `f`).
pub fn bmp(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
    let row = (w * 3).div_ceil(4) * 4;
    let size = 54 + row * h;
    let mut out = Vec::new();
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&(h as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(&[0; 24]);
    for y in (0..h).rev() {
        let mut line = Vec::new();
        for x in 0..w {
            let [r, g, b, _] = f(x, y);
            line.extend_from_slice(&[b, g, r]);
        }
        line.resize(row as usize, 0);
        out.extend_from_slice(&line);
    }
    out
}

fn gradient(x: u32, y: u32) -> [u8; 4] {
    [(x * 20) as u8, (y * 30) as u8, 77, 255]
}

fn hex_req(l: &std::path::Path, r: &std::path::Path) -> HexCompareRequest {
    HexCompareRequest {
        left: os(l),
        right: os(r),
        ..HexCompareRequest::default()
    }
}

fn pic_req(l: &std::path::Path, r: &std::path::Path) -> PictureCompareRequest {
    PictureCompareRequest {
        left: os(l),
        right: os(r),
        ..PictureCompareRequest::default()
    }
}

/// The Hex format `*.up` with an External load conversion run by the fake runner (upper case).
const HEX_UPPER: &str = r#"schema_version = 1
id = "hexup"
name = "Hex Upper"
kind = "hex"
mask = "*.up"

[conversion]
method = "external"
load = "conv %s %t"
"#;

// ---- hex ------------------------------------------------------------------------------------------------------------

/// A12.19: every row of D-331's table, with the fake converter for the conversion rows; the errors of exit 100.
#[test]
fn a12_19_hex_exit_codes_of_every_verdict() {
    let tmp = tempfile::tempdir().unwrap();
    write(
        &tmp.path().join("config"),
        "formats/hexup.toml",
        HEX_UPPER.as_bytes(),
    );
    let svc = service_with(tmp.path(), None, Arc::new(common::UpperRunner::default()));
    let w = tmp.path().join("work");
    let data = bytes(7, 50_000);
    let mut changed = data.clone();
    changed[20_000] ^= 0xFF;
    let a = write(&w, "a.bin", &data);
    let b = write(&w, "b.bin", &data);
    let c = write(&w, "c.bin", &changed);
    let run = |req: &HexCompareRequest| svc.hex_compare(req, &job(&svc, JobKind::HexCompare));
    for mode in ["none", "fast", "complete"] {
        let mut req = hex_req(&a, &b);
        req.rules.alignment = Some(mode.into());
        let resp = run(&req).unwrap();
        assert_eq!(
            (resp.kind, resp.exit_code.code()),
            (ContentKind::BinarySame, 1),
            "{mode}"
        );
        req.right = os(&c);
        let resp = run(&req).unwrap();
        assert_eq!(
            (resp.kind, resp.exit_code.code()),
            (ContentKind::BinaryDifferent, 11),
            "{mode}"
        );
        assert_eq!(resp.alignment.stats.differing_bytes, 1);
    }
    // The conversion rows: raw identical; outputs identical (case only); outputs different.
    let up = |name: &str, text: &[u8]| write(&w, name, text);
    let (u1, u2, u3, u4) = (
        up("one.up", b"abc"),
        up("two.up", b"abc"),
        up("three.up", b"ABC"),
        up("four.up", b"abd"),
    );
    let resp = run(&hex_req(&u1, &u2)).unwrap();
    assert_eq!(
        (resp.kind, resp.exit_code.code()),
        (ContentKind::BinarySame, 1)
    );
    assert!(resp.opened.sides.left.converted);
    let resp = run(&hex_req(&u1, &u3)).unwrap();
    assert_eq!(
        (resp.kind, resp.exit_code.code()),
        (ContentKind::RulesSame, 2)
    );
    let resp = run(&hex_req(&u1, &u4)).unwrap();
    assert_eq!(
        (resp.kind, resp.exit_code.code()),
        (ContentKind::ImportantDifferences, 13)
    );
    assert_eq!(resp.opened.sides.left.load.as_str(), "temp_cache");
    // Errors: a missing file, a bad word (before anything is read: the missing file is not reached), a folder.
    let err = run(&hex_req(&a, &w.join("missing"))).unwrap_err();
    assert_eq!(
        (
            err.kind(),
            err.exit_code(hikasa_session::api::ApiOp::HexCompare).code()
        ),
        (ErrorKind::NotFound, 100)
    );
    let mut bad = hex_req(&a, &w.join("missing"));
    bad.rules.alignment = Some("slow".into());
    assert_eq!(run(&bad).unwrap_err().kind(), ErrorKind::InvalidInput);
    let mut bad = hex_req(&a, &b);
    bad.rules.block = Some("1000".into());
    assert_eq!(run(&bad).unwrap_err().kind(), ErrorKind::InvalidInput);
    let mut bad = hex_req(&a, &b);
    bad.format = Some("hexup".into());
    bad.formats.left = Some("hexup".into());
    assert_eq!(run(&bad).unwrap_err().kind(), ErrorKind::InvalidInput);
    let mut bad = hex_req(&a, &b);
    bad.format = Some("c-cpp".into());
    assert_eq!(run(&bad).unwrap_err().kind(), ErrorKind::InvalidInput);
    let err = run(&hex_req(&a, &w)).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
    // Sizes as words: 8k and 16KiB.
    let mut words = hex_req(&a, &c);
    words.rules.block = Some("8k".into());
    words.rules.window = Some("16KiB".into());
    words.rules.alignment = Some("complete".into());
    assert_eq!(run(&words).unwrap().options.window, 16 << 10);
}

/// A runner that upper-cases its input and appends a byte to the original file: the input changes during the job.
struct Touching(std::path::PathBuf);

impl ProcessRunner for Touching {
    fn run(
        &self,
        argv: &[OsString],
        _timeout: Option<Duration>,
        _cancel: &CancelToken,
    ) -> Result<i32, ConvertError> {
        let source = std::fs::read(&argv[1])?;
        std::fs::write(&argv[2], source.to_ascii_uppercase())?;
        let mut f = std::fs::OpenOptions::new().append(true).open(&self.0)?;
        f.write_all(b"!")?;
        Ok(0)
    }
}

/// A12.19: `stale` and `input_changed` when the fake converter changes an input during the job; stdin and an
/// archive member as inputs; sections, `max_sections`, the preview.
#[test]
fn a12_19_stale_stdin_and_archive_members() {
    let tmp = tempfile::tempdir().unwrap();
    write(
        &tmp.path().join("config"),
        "formats/hexup.toml",
        HEX_UPPER.as_bytes(),
    );
    let w = tmp.path().join("work");
    let watched = write(&w, "watched.up", b"abc");
    let other = write(&w, "other.up", b"abc");
    let svc = service_with(tmp.path(), None, Arc::new(Touching(watched.clone())));
    let resp = svc
        .hex_compare(&hex_req(&watched, &other), &job(&svc, JobKind::HexCompare))
        .unwrap();
    assert!(resp.stale);
    assert!(
        resp.warnings
            .iter()
            .any(|w| w.code == hikasa_core::WarningCode::InputChanged),
        "{:?}",
        resp.warnings
    );
    assert!(resp.json().unwrap().stale);
    // Standard input and a zip member.
    let data = bytes(3, 4000);
    let stdin = common::blob(&data, Some("in.bin"));
    let svc = service_with(
        tmp.path(),
        Some(stdin),
        Arc::new(common::UpperRunner::default()),
    );
    let file = write(&w, "file.bin", &data);
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    zip.start_file("inner.bin", zip::write::SimpleFileOptions::default())
        .unwrap();
    let mut patched = data.clone();
    patched[10] = patched[10].wrapping_add(1);
    patched[3000] = patched[3000].wrapping_add(1);
    zip.write_all(&patched).unwrap();
    let archive = write(&w, "a.zip", &zip.finish().unwrap().into_inner());
    let req = HexCompareRequest {
        left: "-".into(),
        right: os(&file),
        ..HexCompareRequest::default()
    };
    let resp = svc
        .hex_compare(&req, &job(&svc, JobKind::HexCompare))
        .unwrap();
    assert_eq!(resp.kind, ContentKind::BinarySame);
    assert_eq!(resp.opened.sides.left.load.as_str(), "memory");
    let mut req = hex_req(&file, &archive.join("inner.bin"));
    req.max_sections = Some("1".into());
    req.preview = Some("2".into());
    let resp = svc
        .hex_compare(&req, &job(&svc, JobKind::HexCompare))
        .unwrap();
    assert_eq!(resp.kind, ContentKind::BinaryDifferent);
    assert_eq!(resp.opened.sides.right.load.as_str(), "temp_cache");
    let json = resp.json().unwrap();
    assert_eq!(json.stats.sections, 2);
    assert_eq!(json.sections_shown, 1);
    assert_eq!(json.sections[0].preview.left.len(), 2);
    let lines = resp.human_lines().unwrap();
    assert!(
        lines[0].starts_with("0x0000000A L 1  R 1  changed"),
        "{lines:?}"
    );
    assert!(lines[1].starts_with("... 1 more section"), "{lines:?}");
}

/// A12.21: `hex dump` with values at the last bytes; `hex find` over a walk with masks, limits and the counts.
#[test]
fn a12_21_dump_and_find() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    let w = tmp.path().join("work");
    let f = write(&w, "d.bin", &(0u8..40).collect::<Vec<_>>());
    let resp = svc
        .hex_dump(
            &HexDumpRequest {
                spec: os(&f),
                offset: Some("0x24".into()),
                length: Some("16".into()),
                values: true,
                layout: HexLayoutWords {
                    bytes_per_line: Some("2".into()),
                    ..HexLayoutWords::default()
                },
                ..HexDumpRequest::default()
            },
            &job(&svc, JobKind::HexCompare),
        )
        .unwrap();
    assert_eq!((resp.offset, resp.bytes.len()), (36, 4));
    let json = resp.json();
    assert_eq!(json.rows.len(), 2);
    assert_eq!(json.rows[1].hex, "2627");
    let values = json.values.unwrap();
    assert_eq!(values.little.u16, Some(0x2524));
    assert_eq!(values.big.u32, Some(0x2425_2627));
    assert_eq!(values.little.u64, None);
    // Find: a hit in two files (one inside a folder the walk reaches recursively), a masked file, the limits.
    write(&w, "t/a.bin", b"xxDEADxxDEAD");
    write(&w, "t/sub/b.bin", b"DEAD");
    write(&w, "t/c.txt", b"DEAD");
    let find = |req: &HexFindRequest| {
        svc.hex_find(req, &mut NoHexEvents, &job(&svc, JobKind::HexFind))
            .unwrap()
    };
    let mut req = HexFindRequest {
        pattern: "de ad".into(),
        roots: vec![os(&w.join("t"))],
        hex: true,
        recursive: true,
        mask: Some("*.bin".into()),
        keep_hits: true,
        ..HexFindRequest::default()
    };
    let out = find(&req);
    assert!(
        out.json.files.is_empty(),
        "DE AD are not the ASCII text DEAD"
    );
    req.pattern = "44 45 41 44".into();
    let out = find(&req);
    let files: Vec<(&str, Vec<u64>)> = out
        .json
        .files
        .iter()
        .map(|f| (f.spec.rsplit(['/', '\\']).next().unwrap(), f.hits.clone()))
        .collect();
    assert_eq!(files, [("a.bin", vec![2, 8]), ("b.bin", vec![0])]);
    assert_eq!(out.json.counts.files_searched, 2);
    req.max_matches = Some(2);
    let out = find(&req);
    assert!(out.json.limit_reached);
    assert_eq!(out.json.counts.hits, 2);
    // Text, case folded by default.
    let text = HexFindRequest {
        pattern: "dead".into(),
        roots: vec![os(&w.join("t"))],
        recursive: true,
        ..HexFindRequest::default()
    };
    assert_eq!(find(&text).json.counts.hits, 4);
    // A refused pattern is exit 100 before anything is read.
    let bad = HexFindRequest {
        pattern: "zz".into(),
        hex: true,
        roots: vec![os(&w.join("missing"))],
        ..HexFindRequest::default()
    };
    let err = svc
        .hex_find(&bad, &mut NoHexEvents, &job(&svc, JobKind::HexFind))
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
}

// ---- pictures -----------------------------------------------------------------------------------------------------

/// A12.22: every row of D-339's table; probes, geometry words, the refusals; `--diff-output` to a file, stdout and the
/// clipboard (A12.17: the adapter places the exact RGBA of the engine's render).
#[test]
fn a12_22_picture_exit_codes_and_outputs() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    let w = tmp.path().join("work");
    let a = write(&w, "a.png", &png(8, 6, gradient));
    let same = write(&w, "same.png", &png(8, 6, gradient));
    let as_bmp = write(&w, "a.bmp", &bmp(8, 6, gradient));
    let near = write(
        &w,
        "near.png",
        &png(8, 6, |x, y| {
            let mut p = gradient(x, y);
            if (x, y) == (2, 3) {
                p[2] += 3;
            }
            p
        }),
    );
    let run =
        |req: &PictureCompareRequest| svc.picture_compare(req, &job(&svc, JobKind::PictureCompare));
    let code = |req: &PictureCompareRequest| {
        let r = run(req).unwrap();
        (r.kind, r.exit_code.code())
    };
    assert_eq!(code(&pic_req(&a, &same)), (ContentKind::BinarySame, 1));
    assert_eq!(code(&pic_req(&a, &as_bmp)), (ContentKind::RulesSame, 2));
    assert_eq!(
        code(&pic_req(&a, &near)),
        (ContentKind::ImportantDifferences, 13)
    );
    let mut tol = pic_req(&a, &near);
    tol.rules.tolerance = Some("3".into());
    assert_eq!(code(&tol), (ContentKind::UnimportantOnly, 12));
    tol.rules.ignore_unimportant = true;
    assert_eq!(code(&tol), (ContentKind::UnimportantOnly, 2));
    // Probes and geometry.
    let mut req = pic_req(&a, &near);
    req.probes = vec!["2,3".into(), "100,100".into()];
    req.geometry = PictureGeometryWords {
        left_orientation: Some("cw,ccw".into()),
        right_orientation: None,
        offset: Some("0,0".into()),
    };
    let resp = run(&req).unwrap();
    assert_eq!(resp.probes[0].distance, 3);
    assert_eq!(resp.probes[1].class, hikasa_picture::PixelClass::Outside);
    let json = resp.json();
    assert_eq!(json.stats.different, 1);
    assert_eq!(json.probes.len(), 2);
    // The refusals of §1.12.23 (before anything is read: the specs do not exist).
    let missing = w.join("nope.png");
    let refused = |f: &dyn Fn(&mut PictureCompareRequest)| {
        let mut req = pic_req(&missing, &missing);
        f(&mut req);
        run(&req).unwrap_err()
    };
    assert_eq!(
        refused(&|r| r.metadata = true).kind(),
        ErrorKind::Unsupported
    );
    assert_eq!(
        refused(&|r| r.checkerboard = true).kind(),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        refused(&|r| r.diff_pane = Some("left".into())).kind(),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        refused(&|r| r.rules.blend = Some("30".into())).kind(),
        ErrorKind::InvalidInput
    );
    for (field, word) in [
        ("tolerance", "256"),
        ("offset", "1,2,3"),
        ("orientation", "up"),
    ] {
        let err = refused(&|r| match field {
            "tolerance" => r.rules.tolerance = Some(word.into()),
            "offset" => r.geometry.offset = Some(word.into()),
            _ => r.geometry.left_orientation = Some(word.into()),
        });
        assert_eq!(err.kind(), ErrorKind::InvalidInput, "{field}");
    }
    assert_eq!(
        run(&pic_req(&a, &missing)).unwrap_err().kind(),
        ErrorKind::NotFound
    );
    let corrupt = write(&w, "corrupt.png", &png(8, 6, gradient)[..40]);
    assert_eq!(
        run(&pic_req(&a, &corrupt)).unwrap_err().kind(),
        ErrorKind::InvalidData
    );
    let dds = write(&w, "x.dds", b"DDS |not decoded");
    assert_eq!(
        run(&pic_req(&dds, &dds)).unwrap_err().kind(),
        ErrorKind::Unsupported
    );
    // --diff-output: a file (decoded equals the engine's render), stdout bytes, the clipboard image.
    let out = w.join("diff.png");
    let mut req = pic_req(&a, &near);
    req.diff_output = Some(os(&out));
    req.checkerboard = true;
    let resp = run(&req).unwrap();
    let render = resp
        .comparison
        .render(
            hikasa_picture::Pane::Difference,
            resp.mode,
            &resp.colors,
            true,
            &hikasa_core::Ctl::detached(),
        )
        .unwrap();
    let written = hikasa_picture::decode_bytes(
        &std::fs::read(&out).unwrap(),
        Some("diff.png"),
        &hikasa_picture::DecodeOptions::default(),
        &hikasa_core::Ctl::detached(),
    )
    .unwrap();
    assert_eq!(&*written.rgba, &render.rgba[..]);
    assert_eq!(resp.json().diff_output.unwrap().width, 8);
    req.diff_output = Some("-".into());
    let resp = run(&req).unwrap();
    assert!(
        resp.written
            .unwrap()
            .stdout
            .unwrap()
            .starts_with(b"\x89PNG")
    );
    req.diff_output = Some("clipboard://".into());
    req.diff_pane = Some("right".into());
    let resp = run(&req).unwrap();
    let image = resp.written.clone().unwrap().clipboard.unwrap();
    let right = resp
        .comparison
        .render(
            hikasa_picture::Pane::Right,
            resp.mode,
            &resp.colors,
            true,
            &hikasa_core::Ctl::detached(),
        )
        .unwrap();
    assert_eq!((image.width, image.height, image.rgba), (8, 6, right.rgba));
    // `picture_write` refuses the printer and the browser.
    let err = svc
        .picture_write(
            &resp.comparison,
            hikasa_picture::Pane::Left,
            resp.mode,
            false,
            &ReportTarget::Printer,
            &job(&svc, JobKind::PictureCompare),
        )
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
}

/// A12.25: the folder content test and `quick --rules` (D-344): PNG ↔ BMP of the same pixels, a re-compressed JPEG
/// within and above the defaults session's tolerance, a DDS pair (bytes), a corrupt PNG (error), a hex-kind pair
/// (bytes); FF-002 overrides disable the Picture format (bytes again).
#[test]
fn a12_25_folder_content_test_and_quick_rules() {
    let tmp = tempfile::tempdir().unwrap();
    write(
        &tmp.path().join("config"),
        "formats/hexkind.toml",
        b"schema_version = 1\nid = \"hexkind\"\nname = \"Hex Kind\"\nkind = \"hex\"\nmask = \"*.hk\"\n",
    );
    let svc = service(tmp.path());
    let w = tmp.path().join("work");
    let jpegs = common::fixtures("pictures");
    let q95 = std::fs::read(jpegs.join("photo-q95.jpg")).unwrap();
    let q70 = std::fs::read(jpegs.join("photo-q70.jpg")).unwrap();
    write(&w, "l/pic.png", &png(8, 6, gradient));
    write(&w, "r/pic.png", &bmp(8, 6, gradient));
    write(&w, "l/photo.jpg", &q95);
    write(&w, "r/photo.jpg", &q70);
    write(&w, "l/tex.dds", b"DDS |one");
    write(&w, "r/tex.dds", b"DDS |two");
    write(&w, "l/bad.png", &png(8, 6, gradient));
    write(&w, "r/bad.png", &png(8, 6, gradient)[..30]);
    write(&w, "l/raw.hk", b"abc");
    write(&w, "r/raw.hk", b"ABC");
    let results = |svc: &Service, overrides: BTreeMap<String, bool>| {
        let req = FolderCompareRequest {
            left: os(&w.join("l")),
            right: os(&w.join("r")),
            criteria: Some("rules-based".into()),
            base_rules: Some(FolderRules {
                format_overrides: overrides,
                ..FolderRules::default()
            }),
            ..FolderCompareRequest::default()
        };
        let resp = svc
            .folder_compare(&req, &job(svc, JobKind::FolderCompare), &NullObserver)
            .unwrap();
        let display = DisplayOptions {
            filter: FolderDisplayFilter::All,
            folders: FolderMode::CompareStructure,
            ignore_unimportant: false,
            suppress_filters: false,
        };
        let json = serde_json::to_value(resp.json(&display)).unwrap();
        let mut out = BTreeMap::new();
        for item in json["items"].as_array().unwrap() {
            out.insert(
                item["path"].as_str().unwrap().to_owned(),
                item["content"]["result"]["result"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
            );
        }
        out
    };
    let quick = |svc: &Service, name: &str| {
        let resp = svc
            .quick(
                &QuickRequest {
                    left: os(&w.join("l").join(name)),
                    right: os(&w.join("r").join(name)),
                    method: QuickMethod::Rules,
                    ..QuickRequest::default()
                },
                &job(svc, JobKind::QuickCompare),
            )
            .unwrap();
        resp.outcome.result.kind().as_str().to_owned()
    };
    let got = results(&svc, BTreeMap::new());
    let want = [
        ("bad.png", "error"),
        ("photo.jpg", "important_differences"),
        ("pic.png", "rules_same"),
        ("raw.hk", "binary_different"),
        ("tex.dds", "binary_different"),
    ];
    for (name, result) in want {
        assert_eq!(got[name], result, "{name}: {got:?}");
        assert_eq!(quick(&svc, name), result, "quick --rules {name}");
    }
    // The defaults session's tolerance changes the result: within it, unimportant only.
    let mut defaults = svc.session_defaults(SessionType::PictureCompare).unwrap();
    let SessionRules::Picture(rules) = &mut defaults.rules else {
        panic!("{:?}", defaults.rules);
    };
    rules.picture.tolerance = 60;
    svc.session_save_defaults(&defaults).unwrap();
    assert_eq!(
        results(&svc, BTreeMap::new())["photo.jpg"],
        "unimportant_only"
    );
    assert_eq!(quick(&svc, "photo.jpg"), "unimportant_only");
    // FF-002: the Picture format off: bytes again.
    let off = results(&svc, BTreeMap::from([("pictures".to_owned(), false)]));
    assert_eq!(off["pic.png"], "binary_different");
    assert_eq!(off["photo.jpg"], "binary_different");
}

// ---- sessions (A12.24) --------------------------------------------------------------------------------------------

/// A12.24: both types keep their keys (a pre-W12 opaque file loads with its unknown keys kept); `session run` equals the
/// subcommand's request; `session edit --set` validates through the parsers; Swap Sides swaps the `_left`/`_right`
/// keys and negates `offset`; geometry is not `dirty`.
#[test]
fn a12_24_sessions() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    let w = tmp.path().join("work");
    let a = write(&w, "a.png", &png(4, 4, gradient));
    let b = write(&w, "b.png", &png(4, 4, gradient));
    // A file written before W12: an opaque table with keys of its own.
    write(
        &tmp.path().join("config"),
        "sessions/old.toml",
        format!(
            "type = \"picture-compare\"\n[specs]\nleft = {:?}\nright = {:?}\n[rules]\nlegacy = 1\n[rules.picture]\ntolerance = 9\nfuture_key = \"kept\"\n[view]\norientation_left = \"cw\"\noffset = [3, -4]\n",
            a.display().to_string(),
            b.display().to_string()
        )
        .as_bytes(),
    );
    let reference = SessionRef::parse("old").unwrap();
    let loaded = svc.session_load(&reference).unwrap().value;
    let SessionRules::Picture(rules) = &loaded.rules else {
        panic!("{:?}", loaded.rules);
    };
    assert_eq!(rules.picture.tolerance, 9);
    // A save keeps the unknown keys and writes only what changed.
    svc.session_save(&reference, &loaded, false).unwrap();
    let text = std::fs::read_to_string(tmp.path().join("config/sessions/old.toml")).unwrap();
    assert!(
        text.contains("legacy = 1") && text.contains("future_key = \"kept\""),
        "{text}"
    );
    assert!(!text.contains("tone_map"), "{text}");
    // session run = the subcommand's request.
    let SessionRunRequest::Picture(run) = svc.session_run_request(&reference).unwrap() else {
        panic!()
    };
    assert_eq!(run.rules.tolerance.as_deref(), Some("9"));
    assert_eq!(run.geometry.left_orientation.as_deref(), Some("cw"));
    assert_eq!(run.geometry.offset.as_deref(), Some("3,-4"));
    let direct = PictureCompareRequest {
        rules: PictureRuleWords {
            tolerance: Some("9".into()),
            ..run.rules.clone()
        },
        geometry: run.geometry.clone(),
        ..pic_req(&a, &b)
    };
    let from_session = svc
        .picture_compare(&run, &job(&svc, JobKind::PictureCompare))
        .unwrap();
    let from_flags = svc
        .picture_compare(&direct, &job(&svc, JobKind::PictureCompare))
        .unwrap();
    assert_eq!(
        serde_json::to_value(from_session.json()).unwrap(),
        serde_json::to_value(from_flags.json()).unwrap()
    );
    // edit --set validates through the parsers.
    let edit = |key: &str, value: &str| {
        svc.session_edit(
            &hikasa_session::api::SessionTarget::Named(reference.clone()),
            &hikasa_session::api::SessionEdit {
                set: vec![(key.to_owned(), value.to_owned())],
                ..hikasa_session::api::SessionEdit::default()
            },
            false,
        )
    };
    assert!(edit("rules.picture.mode", "\"range\"").is_ok());
    assert_eq!(
        edit("rules.picture.mode", "\"loud\"").unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        edit("rules.picture.tolerance", "300").unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        edit("view.orientation_right", "\"sideways\"")
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidInput
    );
    // Swap Sides: specs, orientations and the offset.
    let mut swapped = svc.session_load(&reference).unwrap().value;
    swapped.swap_sides();
    assert_eq!(
        swapped
            .view
            .get("orientation_right")
            .and_then(|v| v.as_str()),
        Some("cw")
    );
    assert!(swapped.view.get("orientation_left").is_none());
    assert_eq!(
        swapped.view.get("offset").unwrap().as_array().unwrap(),
        &vec![toml::Value::Integer(-3), toml::Value::Integer(4)]
    );
    assert_eq!(swapped.specs.left, b.display().to_string());
    // Geometry and layout are view state: not dirty.
    let before = svc.session_load(&reference).unwrap().value;
    let mut after = before.clone();
    after.view.insert(
        "offset".into(),
        toml::Value::Array(vec![1.into(), 1.into()]),
    );
    assert!(!is_dirty(&before, &after));
    let SessionRules::Picture(r) = &mut after.rules else {
        panic!()
    };
    r.picture.auto_scale = true;
    assert!(is_dirty(&before, &after));
    // A hex session: the rules as numbers, [view] and [display] as words; run = the subcommand's request.
    let mut hex = Session::builtin(SessionType::HexCompare);
    hex.specs.left = a.display().to_string();
    hex.specs.right = b.display().to_string();
    let SessionRules::Hex(rules) = &mut hex.rules else {
        panic!()
    };
    rules.hex.alignment = "none".into();
    rules.hex.block = 4096;
    hex.view
        .insert("bytes_per_line".into(), toml::Value::Integer(8));
    hex.display.insert("filter".into(), "all".into());
    let href = SessionRef::parse("h").unwrap();
    svc.session_save(&href, &hex, false).unwrap();
    let SessionRunRequest::Hex(run) = svc.session_run_request(&href).unwrap() else {
        panic!()
    };
    assert_eq!(
        (run.rules.alignment.as_deref(), run.rules.block.as_deref()),
        (Some("none"), Some("4096"))
    );
    assert_eq!(run.layout.bytes_per_line.as_deref(), Some("8"));
    assert_eq!(run.layout.display.as_deref(), Some("all"));
    let edit_hex = |key: &str, value: &str| {
        svc.session_edit(
            &hikasa_session::api::SessionTarget::Named(href.clone()),
            &hikasa_session::api::SessionEdit {
                set: vec![(key.to_owned(), value.to_owned())],
                ..hikasa_session::api::SessionEdit::default()
            },
            false,
        )
    };
    assert!(edit_hex("rules.hex.alignment", "\"complete\"").is_ok());
    assert_eq!(
        edit_hex("rules.hex.block", "1000").unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    assert_eq!(
        edit_hex("view.group", "3").unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
}

// ---- IPC (A12.26) -------------------------------------------------------------------------------------------------

/// A12.26: `for_hex`/`for_picture` → `open_view_request` round trip for every word; the protocol's additive fields
/// are absent from the wire when unused.
#[test]
fn a12_26_gui_requests_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    // Absolute specs of the platform (IPC refuses relative ones).
    let (l, r) = if cfg!(windows) {
        ("C:\\data\\a.bin", "C:\\data\\b.bin")
    } else {
        ("/data/a.bin", "/data/b.bin")
    };
    let hex = HexCompareRequest {
        left: l.into(),
        right: r.into(),
        rules: HexRuleWords {
            alignment: Some("complete".into()),
            block: Some("4k".into()),
            skew: Some("100".into()),
            window: Some("8k".into()),
            budget_mib: Some("5".into()),
            unlocked_max_mib: Some("1".into()),
        },
        layout: HexLayoutWords {
            bytes_per_line: Some("8".into()),
            group: Some("2".into()),
            chars: Some("latin1".into()),
            address: Some("decimal".into()),
            context: Some("3".into()),
            display: Some("all".into()),
        },
        formats: Pair::new(None, Some("x".into())),
        ..HexCompareRequest::default()
    };
    let open = GuiOpenRequest::for_hex(&hex, false);
    let specs: Vec<String> = vec![l.into(), r.into()];
    let back = svc
        .open_view_request(open.kind.unwrap(), &specs, &open.options)
        .unwrap();
    let ViewRequest::Hex {
        request,
        layout,
        filter,
    } = back
    else {
        panic!("{back:?}")
    };
    assert_eq!(request.rules, hex.rules);
    assert_eq!(request.layout, hex.layout);
    assert_eq!(request.formats, hex.formats);
    assert_eq!((layout.bytes_per_line, layout.group), (8, 2));
    assert_eq!(filter, Some(hikasa_hex::DisplayFilter::All));
    let picture = PictureCompareRequest {
        left: l.into(),
        right: r.into(),
        rules: PictureRuleWords {
            mode: Some("blend".into()),
            tolerance: Some("4".into()),
            blend: Some("30".into()),
            auto_scale: true,
            tone_map: Some("reinhard".into()),
            max_megapixels: Some("10".into()),
            ignore_unimportant: true,
        },
        geometry: PictureGeometryWords {
            left_orientation: Some("cw".into()),
            right_orientation: Some("flip-h,180".into()),
            offset: Some("-2,5".into()),
        },
        ..PictureCompareRequest::default()
    };
    let open = GuiOpenRequest::for_picture(&picture, true);
    let back = svc
        .open_view_request(open.kind.unwrap(), &specs, &open.options)
        .unwrap();
    let ViewRequest::Picture {
        request,
        geometry,
        mode,
    } = back
    else {
        panic!("{back:?}")
    };
    assert_eq!(request.rules, picture.rules);
    assert_eq!(request.geometry, picture.geometry);
    assert_eq!(geometry.offset, (-2, 5));
    assert!(geometry.auto_scale);
    assert_eq!(mode, hikasa_picture::DiffMode::Blend { percent: 30 });
    // A bad word is refused by the check (the CLI's precheck, D-350).
    let mut bad = picture.clone();
    bad.geometry.offset = Some("x".into());
    assert!(matches!(
        svc.check_picture_words(&bad),
        Err(ApiError::InvalidArgument(_))
    ));
    // The additive fields are left out of the wire when unused.
    let wire = serde_json::to_value(hikasa_session::ipc::OpenOptions::default()).unwrap();
    assert!(wire.get("hex").is_none() && wire.get("picture").is_none());
}
