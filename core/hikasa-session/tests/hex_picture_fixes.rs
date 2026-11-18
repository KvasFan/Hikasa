//! W12c review fixes in the Service (`docs/design/m2-contracts.md` §1.12.33, *W12c review fixes*): a Picture format's
//! load conversion in the folder content test and `quick --rules` (F1, A12.25 rows with a fake runner), the
//! comparator's warnings and the defaults session (F6, N4, N5), the hex conversion's private output (F2), the dry run,
//! the targets and the refusals of `--diff-output` (F4, F7, F8), `hex dump --offset` (F9), the defaults session of the
//! subcommands (F10), the smaller items of F11 and the tests of the review's surviving mutations.

mod common;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::Write as _;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use common::{job, os, service, service_with, write};
use hikasa_core::{CancelToken, ContentKind, ErrorKind, JobKind, WarningCode};
use hikasa_folder::{DisplayOptions, FolderDisplayFilter, FolderMode, FolderRules, NullObserver};
use hikasa_formats::{ConvertError, ProcessRunner};
use hikasa_picture::{PngLevel, RgbaImage, encode_png};
use hikasa_session::api::{
    FolderCompareRequest, HexCompareRequest, HexDumpRequest, HexFindRequest, HexRuleWords,
    NoHexEvents, PictureCompareRequest, PictureRuleWords, QuickMethod, QuickRequest, ReportTarget,
    SessionRunRequest, diff_output_target,
};
use hikasa_session::sessions::{SessionRef, SessionRules, SessionType};
use hikasa_session::{ApiError, Service};

/// A PNG of `w` × `h` with the pixel function `f`.
fn png(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
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

fn solid(c: [u8; 4]) -> impl Fn(u32, u32) -> [u8; 4] {
    move |_, _| c
}

fn pic_req(l: &Path, r: &Path) -> PictureCompareRequest {
    PictureCompareRequest {
        left: os(l),
        right: os(r),
        ..PictureCompareRequest::default()
    }
}

fn hex_req(l: &Path, r: &Path) -> HexCompareRequest {
    HexCompareRequest {
        left: os(l),
        right: os(r),
        ..HexCompareRequest::default()
    }
}

/// The fake camera-RAW converter: the source holds `r,g,b` text, the target becomes a 2 × 2 PNG of that colour;
/// `fail` exits 1. It counts its runs.
#[derive(Default)]
struct RawRunner(std::sync::atomic::AtomicUsize);

impl ProcessRunner for RawRunner {
    fn run(
        &self,
        argv: &[OsString],
        _timeout: Option<Duration>,
        cancel: &CancelToken,
    ) -> Result<i32, ConvertError> {
        cancel.check()?;
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let text = String::from_utf8(std::fs::read(&argv[1])?).unwrap();
        if text.starts_with("fail") {
            return Ok(1);
        }
        let c: Vec<u8> = text
            .trim()
            .split(',')
            .map(|v| v.trim().parse().unwrap())
            .collect();
        std::fs::write(&argv[2], png(2, 2, solid([c[0], c[1], c[2], 255])))?;
        Ok(0)
    }
}

/// A Picture format of `*.fake` whose External load conversion is the fake runner's.
const FAKE_RAW: &str = r#"schema_version = 1
id = "fakeraw"
name = "Fake RAW"
kind = "picture"
mask = "*.fake"

[conversion]
method = "external"
load = "conv %s %t"
"#;

/// The results (and the content warnings) of a rules-based folder comparison of `l` and `r`, by path.
fn folder_results(svc: &Service, l: &Path, r: &Path) -> BTreeMap<String, (String, Vec<String>)> {
    let req = FolderCompareRequest {
        left: os(l),
        right: os(r),
        criteria: Some("rules-based".into()),
        base_rules: Some(FolderRules::default()),
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
        let content = &item["content"];
        out.insert(
            item["path"].as_str().unwrap().to_owned(),
            (
                content["result"]["result"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
                content["warnings"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .map(|w| w["code"].as_str().unwrap().to_owned())
                            .collect()
                    })
                    .unwrap_or_default(),
            ),
        );
    }
    out
}

/// `quick --rules` of `l` and `r` (`format` forced): the result and the warnings' codes.
fn quick(svc: &Service, l: &Path, r: &Path, format: Option<&str>) -> (String, Vec<WarningCode>) {
    let resp = svc
        .quick(
            &QuickRequest {
                left: os(l),
                right: os(r),
                method: QuickMethod::Rules,
                format: format.map(str::to_owned),
                ..QuickRequest::default()
            },
            &job(svc, JobKind::QuickCompare),
        )
        .unwrap();
    (
        resp.outcome.result.kind().as_str().to_owned(),
        resp.outcome.warnings.iter().map(|w| w.code).collect(),
    )
}

/// F1 (A12.25 rows): a Picture format with an External load conversion is converted before it decodes in the folder
/// content test and `quick --rules`, exactly as `picture compare` converts it (T11); without the conversion the same
/// files are no pictures. A failed conversion is the pair's error. N6: the side is reported `converted`.
#[test]
fn f1_picture_conversions_in_folders_quick_and_picture_compare() {
    let tmp = tempfile::tempdir().unwrap();
    write(
        &tmp.path().join("config"),
        "formats/fakeraw.toml",
        FAKE_RAW.as_bytes(),
    );
    let runner = Arc::new(RawRunner::default());
    let svc = service_with(
        tmp.path(),
        None,
        Arc::clone(&runner) as Arc<dyn ProcessRunner>,
    );
    let w = tmp.path().join("work");
    let (l, r) = (w.join("l"), w.join("r"));
    write(&l, "same.fake", b"1,2,3");
    write(&r, "same.fake", b"1, 2, 3 ");
    write(&l, "diff.fake", b"1,2,3");
    write(&r, "diff.fake", b"9,2,3");
    write(&l, "fail.fake", b"1,2,3");
    write(&r, "fail.fake", b"fail");
    write(&l, "twin.fake", b"4,5,6");
    write(&r, "twin.fake", b"4,5,6");
    // Named otherwise: only a forced format converts them.
    write(&l, "shot.bin", b"7,7,7");
    write(&r, "shot.bin", b"7, 7, 7");
    let got = folder_results(&svc, &l, &r);
    let want = [
        ("diff.fake", "important_differences"),
        ("fail.fake", "error"),
        ("same.fake", "rules_same"),
        ("twin.fake", "binary_same"),
    ];
    for (name, result) in want {
        assert_eq!(got[name].0, result, "{name}: {got:?}");
        let (q, _) = quick(&svc, &l.join(name), &r.join(name), None);
        assert_eq!(q, result, "quick --rules {name}");
        // T11: `picture compare` gives the same verdict.
        let run = svc.picture_compare(
            &pic_req(&l.join(name), &r.join(name)),
            &job(&svc, JobKind::PictureCompare),
        );
        match (result, run) {
            ("error", Err(e)) => assert_eq!(e.kind(), ErrorKind::InvalidData, "{e}"),
            (_, Ok(resp)) => {
                assert_eq!(resp.kind.as_str(), result, "picture compare {name}");
                // N6: both sides come from the conversion; the size is the input's.
                let json = serde_json::to_value(resp.json()).unwrap();
                assert_eq!(json["left"]["converted"], true, "{json}");
                assert_eq!(json["right"]["converted"], true);
                assert_eq!(
                    json["left"]["size"].as_u64(),
                    Some(std::fs::metadata(l.join(name)).unwrap().len())
                );
            }
            (_, other) => panic!("{name}: {other:?}"),
        }
    }
    // A forced Picture format converts both sides of `quick --rules`.
    assert_eq!(
        quick(
            &svc,
            &l.join("shot.bin"),
            &r.join("shot.bin"),
            Some("fakeraw")
        )
        .0,
        "rules_same"
    );
    // Identical inputs never reach the program.
    let before = runner.0.load(std::sync::atomic::Ordering::SeqCst);
    assert_eq!(
        quick(&svc, &l.join("twin.fake"), &r.join("twin.fake"), None).0,
        "binary_same"
    );
    assert_eq!(runner.0.load(std::sync::atomic::Ordering::SeqCst), before);
    // FF-002 off: the files are no pictures (Everything Else compares their text).
    let req = FolderCompareRequest {
        left: os(&l),
        right: os(&r),
        criteria: Some("rules-based".into()),
        base_rules: Some(FolderRules {
            format_overrides: BTreeMap::from([("fakeraw".to_owned(), false)]),
            ..FolderRules::default()
        }),
        ..FolderCompareRequest::default()
    };
    let resp = svc
        .folder_compare(&req, &job(&svc, JobKind::FolderCompare), &NullObserver)
        .unwrap();
    let json = serde_json::to_value(resp.json(&DisplayOptions {
        filter: FolderDisplayFilter::All,
        folders: FolderMode::CompareStructure,
        ignore_unimportant: false,
        suppress_filters: false,
    }))
    .unwrap();
    let same = json["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["path"] == "same.fake")
        .unwrap();
    assert_eq!(same["content"]["result"]["result"], "important_differences");
    // The converter detects with the comparison's overrides too: with Fake RAW off, a second Picture format of the
    // mask (without a conversion) is the pair's, and its raw bytes are no picture.
    write(
        &tmp.path().join("config"),
        "formats/plainfake.toml",
        b"schema_version = 1\nid = \"plainfake\"\nname = \"Plain Fake\"\nkind = \"picture\"\nmask = \"*.fake\"\n",
    );
    let svc = service_with(
        tmp.path(),
        None,
        Arc::clone(&runner) as Arc<dyn ProcessRunner>,
    );
    assert_eq!(folder_results(&svc, &l, &r)["same.fake"].0, "rules_same");
    let resp = svc
        .folder_compare(&req, &job(&svc, JobKind::FolderCompare), &NullObserver)
        .unwrap();
    let json = serde_json::to_value(resp.json(&DisplayOptions {
        filter: FolderDisplayFilter::All,
        folders: FolderMode::CompareStructure,
        ignore_unimportant: false,
        suppress_filters: false,
    }))
    .unwrap();
    let same = json["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["path"] == "same.fake")
        .unwrap();
    assert_eq!(
        same["content"]["result"]["result"], "binary_different",
        "{same}"
    );
    assert_eq!(
        same["content"]["warnings"][0]["code"],
        "fell_back_to_binary"
    );
}

/// F6, N4, N5: the folder content test and `quick --rules` take Auto Scale and the pixel limit of the defaults
/// session, and carry the comparison's warnings (`aspect_ratio_differs`).
#[test]
fn f6_defaults_session_and_the_comparison_warnings() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    let w = tmp.path().join("work");
    let (l, r) = (w.join("l"), w.join("r"));
    let red = solid([200, 0, 0, 255]);
    write(&l, "sq.png", &png(4, 4, &red));
    write(&r, "sq.png", &png(8, 8, &red));
    write(&l, "wide.png", &png(4, 4, &red));
    write(&r, "wide.png", &png(16, 4, &red));
    write(&l, "big.png", &png(1100, 1000, &red));
    write(
        &r,
        "big.png",
        &png(1100, 1000, |x, y| {
            if (x, y) == (5, 5) { [0; 4] } else { red(x, y) }
        }),
    );
    let before = folder_results(&svc, &l, &r);
    assert_eq!(before["sq.png"].0, "important_differences");
    assert_eq!(before["big.png"].0, "important_differences");
    let mut defaults = svc.session_defaults(SessionType::PictureCompare).unwrap();
    let SessionRules::Picture(rules) = &mut defaults.rules else {
        panic!("{:?}", defaults.rules);
    };
    rules.picture.auto_scale = true;
    rules.picture.max_megapixels = 1;
    svc.session_save_defaults(&defaults).unwrap();
    let after = folder_results(&svc, &l, &r);
    assert_eq!(after["sq.png"], ("rules_same".to_owned(), Vec::new()));
    assert_eq!(
        after["wide.png"],
        (
            "rules_same".to_owned(),
            vec!["aspect_ratio_differs".to_owned()]
        )
    );
    assert_eq!(after["big.png"].0, "error", "{after:?}");
    assert_eq!(
        quick(&svc, &l.join("wide.png"), &r.join("wide.png"), None),
        (
            "rules_same".to_owned(),
            vec![WarningCode::AspectRatioDiffers]
        )
    );
    assert_eq!(
        quick(&svc, &l.join("big.png"), &r.join("big.png"), None).0,
        "error"
    );
    // F10 (D-374): `picture compare` starts from the same defaults; a word typed wins.
    let words = svc.picture_rules_on_defaults(&PictureRuleWords::default());
    assert!(words.auto_scale);
    assert_eq!(words.max_megapixels.as_deref(), Some("1"));
    let typed = svc.picture_rules_on_defaults(&PictureRuleWords {
        max_megapixels: Some("2".into()),
        ..PictureRuleWords::default()
    });
    assert_eq!(typed.max_megapixels.as_deref(), Some("2"));
}

/// A GIF of a 1 × 1 screen with `frames` frames of one pixel.
fn gif(frames: usize) -> Vec<u8> {
    let mut g = b"GIF89a".to_vec();
    g.extend(1u16.to_le_bytes());
    g.extend(1u16.to_le_bytes());
    g.extend([0x80, 0, 0, 0, 0, 0, 255, 255, 255]);
    for _ in 0..frames {
        g.extend([0x2C, 0, 0, 0, 0, 1, 0, 1, 0, 0, 2, 2, 0x44, 0x01, 0]);
    }
    g.push(0x3B);
    g
}

/// N3, N7, N8, N9, N10, F4, F7, F8, F11 (the area's message), N43: `picture compare`'s warnings, image, errors and
/// refusals.
#[test]
fn picture_compare_outputs_errors_and_refusals() {
    let tmp = tempfile::tempdir().unwrap();
    write(
        &tmp.path().join("config"),
        "formats/hexkind.toml",
        b"schema_version = 1\nid = \"hexkind\"\nname = \"Hex Kind\"\nkind = \"hex\"\nmask = \"*.hk\"\n",
    );
    let svc = service(tmp.path());
    let w = tmp.path().join("work");
    let run =
        |req: &PictureCompareRequest| svc.picture_compare(req, &job(&svc, JobKind::PictureCompare));
    // N3: informational warnings with --verbose only.
    let anim = write(&w, "anim.gif", &gif(2));
    let codes = |verbose: bool| {
        let mut req = pic_req(&anim, &anim);
        req.verbose = verbose;
        run(&req)
            .unwrap()
            .warnings
            .iter()
            .map(|w| w.code)
            .collect::<Vec<_>>()
    };
    assert!(codes(false).is_empty(), "{:?}", codes(false));
    assert_eq!(
        codes(true),
        [WarningCode::FramesIgnored, WarningCode::FramesIgnored]
    );
    // N7: the image's checkerboard in the JSON: the flag, else a session's [view] value, else off.
    let a = write(&w, "a.png", &png(4, 3, solid([1, 2, 3, 128])));
    let b = write(&w, "b.png", &png(4, 3, solid([9, 2, 3, 128])));
    let out = w.join("out.png");
    let checkerboard = |flag: bool, view: Option<bool>| {
        let mut req = pic_req(&a, &b);
        req.diff_output = Some(os(&out));
        req.checkerboard = flag;
        req.view_checkerboard = view;
        let resp = run(&req).unwrap();
        serde_json::to_value(resp.json()).unwrap()["diff_output"]["checkerboard"]
            .as_bool()
            .unwrap()
    };
    assert!(checkerboard(true, None));
    assert!(!checkerboard(false, None));
    assert!(checkerboard(false, Some(true)));
    assert!(!checkerboard(false, Some(false)));
    // N8: the right side's decode error names the right spec.
    let good = write(&w, "good.png", &png(4, 3, solid([1, 2, 3, 255])));
    let broken = write(&w, "broken.png", &png(4, 3, solid([1, 2, 3, 255]))[..30]);
    let e = run(&pic_req(&good, &broken)).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::InvalidData);
    let text = e.to_string();
    assert!(
        text.contains("broken.png") && !text.contains("good.png"),
        "{text}"
    );
    // N9: a folder is refused before decoding.
    let e = run(&pic_req(&w, &good)).unwrap_err();
    assert!(
        matches!(&e, ApiError::InvalidArgument(m) if m.contains("use `folder compare`")),
        "{e:?}"
    );
    // N10: a format of another kind is refused.
    let mut req = pic_req(&good, &good);
    req.format = Some("hexkind".into());
    let e = run(&req).unwrap_err();
    assert!(
        matches!(&e, ApiError::InvalidArgument(m) if m.contains("format of the picture kind")),
        "{e:?}"
    );
    // F4: a dry run plans the image and writes nothing.
    let planned = w.join("planned.png");
    let mut req = pic_req(&a, &b);
    req.diff_output = Some(os(&planned));
    req.dry_run = true;
    let resp = run(&req).unwrap();
    assert!(!planned.exists());
    let plan = resp.planned.clone().unwrap();
    assert!(plan.file && plan.line.starts_with("write ") && plan.line.ends_with("planned.png"));
    assert_eq!(resp.written, None);
    let info = resp.diff_output.clone().unwrap();
    assert_eq!((info.width, info.height), (4, 3));
    req.diff_output = Some(os(&a));
    req.dry_run = false;
    // F8: an input is never the image's target (the same path, a link to it).
    let e = run(&req).unwrap_err();
    assert!(
        matches!(&e, ApiError::InvalidArgument(m) if m.contains("it is the left input")),
        "{e:?}"
    );
    req.diff_output = Some(os(&b));
    assert!(run(&req).unwrap_err().to_string().contains("right input"));
    #[cfg(unix)]
    {
        let link = w.join("link.png");
        std::os::unix::fs::symlink(&b, &link).unwrap();
        req.diff_output = Some(os(&link));
        assert!(run(&req).unwrap_err().to_string().contains("right input"));
        let hard = w.join("hard.png");
        std::fs::hard_link(&a, &hard).unwrap();
        req.diff_output = Some(os(&hard));
        assert!(run(&req).unwrap_err().to_string().contains("left input"));
    }
    // An existing target that is no input is replaced; the dry run says so.
    req.diff_output = Some(os(&out));
    req.dry_run = true;
    let line = run(&req).unwrap().planned.unwrap().line;
    assert!(line.starts_with("replace "), "{line}");
    // F11: the render limit names the compared area.
    let mut req = pic_req(&a, &b);
    req.geometry.offset = Some("1048576,1048576".into());
    req.diff_output = Some(os(&w.join("far.png")));
    let e = run(&req).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::TooLarge);
    assert!(e.to_string().contains("the compared area is"), "{e}");
    // N43: the theme's colours are opaque unless it says otherwise.
    let colors = svc.picture_colors();
    for c in [
        colors.same,
        colors.similar,
        colors.different,
        colors.background,
    ] {
        assert_eq!(c.a, 255, "{colors:?}");
    }
}

/// F7: `--diff-output` names a file unless it is `-` or `clipboard://`.
#[test]
fn f7_diff_output_words() {
    assert_eq!(diff_output_target("-".as_ref()), ReportTarget::Stdout);
    assert_eq!(
        diff_output_target("clipboard://".as_ref()),
        ReportTarget::Clipboard
    );
    assert_eq!(
        diff_output_target("CLIPBOARD://".as_ref()),
        ReportTarget::Clipboard
    );
    for word in ["clipboard", "printer", "browser", "out.png"] {
        assert_eq!(
            diff_output_target(word.as_ref()),
            ReportTarget::File(word.into()),
            "{word}"
        );
    }
}

/// The fake Hex conversion of `*.up`: upper case.
const HEX_UPPER: &str = r#"schema_version = 1
id = "hexup"
name = "Hex Upper"
kind = "hex"
mask = "*.up"

[conversion]
method = "external"
load = "conv %s %t"
"#;

/// A runner that upper-cases the source and appends a byte to a watched file (an input changing during the job).
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

/// F2: a hex load conversion's output lives in a new private folder of the temp folder (0700), in a file created
/// exclusively (0600), both gone with the comparison. N13: the right input changing during the job is `stale` too.
#[test]
fn f2_converted_output_is_private_and_n13_the_right_side_is_watched() {
    let tmp = tempfile::tempdir().unwrap();
    write(
        &tmp.path().join("config"),
        "formats/hexup.toml",
        HEX_UPPER.as_bytes(),
    );
    let svc = service(tmp.path());
    let w = tmp.path().join("work");
    let a = write(&w, "a.up", b"abc");
    let b = write(&w, "b.up", b"abd");
    let temp = tmp.path().join("tmp");
    let resp = svc
        .hex_compare(&hex_req(&a, &b), &job(&svc, JobKind::HexCompare))
        .unwrap();
    assert_eq!(resp.kind, ContentKind::ImportantDifferences);
    let dirs: Vec<_> = std::fs::read_dir(&temp)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("hikasa-hex-")
        })
        .collect();
    assert_eq!(dirs.len(), 2, "{dirs:?}");
    for dir in &dirs {
        let files: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(files.len(), 1, "{files:?}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(dir), 0o700);
            assert_eq!(mode(&files[0]), 0o600);
        }
    }
    drop(resp);
    for dir in &dirs {
        assert!(!dir.exists(), "{dir:?}");
    }
    // N13.
    let watched = write(&w, "watched.up", b"abc");
    let other = write(&w, "other.up", b"abc");
    let svc = service_with(tmp.path(), None, Arc::new(Touching(watched.clone())));
    let resp = svc
        .hex_compare(&hex_req(&other, &watched), &job(&svc, JobKind::HexCompare))
        .unwrap();
    assert!(resp.stale);
    assert!(
        resp.warnings
            .iter()
            .any(|w| w.code == WarningCode::InputChanged)
    );
}

/// N16, N17, F9, F11 (`hex dump` of a missing file, `--block`'s word), F10 for hex, N46.
#[test]
fn hex_limits_dump_offsets_and_words() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    let w = tmp.path().join("work");
    // 25 sections: every 1,000th byte changed.
    let data: Vec<u8> = (0..30_000u32).map(|i| (i % 251) as u8).collect();
    let mut edited = data.clone();
    for k in 0..25 {
        edited[k * 1000 + 500] ^= 0xFF;
    }
    let a = write(&w, "a.bin", &data);
    let b = write(&w, "b.bin", &edited);
    let mut req = hex_req(&a, &b);
    req.rules.alignment = Some("none".into());
    let resp = svc
        .hex_compare(&req, &job(&svc, JobKind::HexCompare))
        .unwrap();
    let json = resp.json().unwrap();
    assert_eq!(json.stats.sections, 25);
    assert_eq!(json.sections_shown, 20);
    let lines = resp.human_lines().unwrap();
    assert!(
        lines.iter().any(|l| l.starts_with("... 5 more sections")),
        "{lines:?}"
    );
    // N17: --unlocked-max at most 1024 MiB.
    let check = |words: HexRuleWords| {
        let mut req = hex_req(&a, &b);
        req.rules = words;
        svc.check_hex_words(&req).map(|_| ())
    };
    assert!(
        check(HexRuleWords {
            unlocked_max_mib: Some("1024".into()),
            ..HexRuleWords::default()
        })
        .is_ok()
    );
    assert_eq!(
        check(HexRuleWords {
            unlocked_max_mib: Some("1025".into()),
            ..HexRuleWords::default()
        })
        .unwrap_err()
        .kind(),
        ErrorKind::InvalidInput
    );
    // F11: a size refused by its range names the word typed.
    let e = check(HexRuleWords {
        block: Some("3k".into()),
        ..HexRuleWords::default()
    })
    .unwrap_err();
    assert!(e.to_string().contains("\"3k\""), "{e}");
    // F9: an offset past the end, or before the start, is refused; the last byte and `end` are dumped.
    let dump = |offset: &str| {
        svc.hex_dump(
            &HexDumpRequest {
                spec: os(&a),
                offset: Some(offset.into()),
                length: Some("4".into()),
                ..HexDumpRequest::default()
            },
            &job(&svc, JobKind::HexCompare),
        )
    };
    assert_eq!(dump("29999").unwrap().json().offset, 29_999);
    assert_eq!(dump("end").unwrap().json().offset, 29_999);
    assert_eq!(dump("+16").unwrap().json().offset, 16);
    for bad in ["30000", "0x30D40", "18446744073709551615", "-1", "-5"] {
        let e = dump(bad).map(|_| ()).unwrap_err();
        assert_eq!(e.kind(), ErrorKind::InvalidInput, "{bad}: {e}");
    }
    assert!(
        dump("30000")
            .map(|_| ())
            .unwrap_err()
            .to_string()
            .contains("past the end")
    );
    for negative in ["-1", "-5"] {
        let e = dump(negative).map(|_| ()).unwrap_err().to_string();
        assert!(e.contains("from the start of the file"), "{negative}: {e}");
    }
    let empty = write(&w, "empty.bin", b"");
    let resp = svc
        .hex_dump(
            &HexDumpRequest {
                spec: os(&empty),
                offset: Some("0".into()),
                ..HexDumpRequest::default()
            },
            &job(&svc, JobKind::HexCompare),
        )
        .unwrap();
    assert!(resp.json().rows.is_empty());
    // F11: a dump has one input, not a left one.
    let e = svc
        .hex_dump(
            &HexDumpRequest {
                spec: os(&w.join("nonexist")),
                ..HexDumpRequest::default()
            },
            &job(&svc, JobKind::HexCompare),
        )
        .map(|_| ())
        .unwrap_err();
    assert_eq!(e.kind(), ErrorKind::NotFound);
    assert!(!e.to_string().contains("left input"), "{e}");
    // F10 for hex: the defaults session's rules under the words typed.
    let mut defaults = svc.session_defaults(SessionType::HexCompare).unwrap();
    let SessionRules::Hex(rules) = &mut defaults.rules else {
        panic!("{:?}", defaults.rules);
    };
    rules.hex.alignment = "complete".into();
    rules.hex.block = 4096;
    svc.session_save_defaults(&defaults).unwrap();
    let words = svc.hex_rules_on_defaults(&HexRuleWords {
        block: Some("16k".into()),
        ..HexRuleWords::default()
    });
    assert_eq!(words.alignment.as_deref(), Some("complete"));
    assert_eq!(words.block.as_deref(), Some("16k"));
    // N46: a hex session's [view] context reaches its run.
    write(
        &tmp.path().join("config"),
        "sessions/ctx.toml",
        format!(
            "type = \"hex-compare\"\n[specs]\nleft = {:?}\nright = {:?}\n[view]\ncontext = 2\n[display]\nfilter = \"all\"\n",
            a.display().to_string(),
            b.display().to_string()
        )
        .as_bytes(),
    );
    let SessionRunRequest::Hex(run) = svc
        .session_run_request(&SessionRef::parse("ctx").unwrap())
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(run.layout.context.as_deref(), Some("2"));
    assert_eq!(run.layout.display.as_deref(), Some("all"));
}

/// N20: a file that cannot be read is skipped as `error` with its error, a link to a folder as `filtered`.
#[cfg(unix)]
#[test]
fn n20_find_skips_unreadable_files_and_folder_links() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    let w = tmp.path().join("work");
    let root = w.join("root");
    write(&root, "hit.bin", b"xxneedlexx");
    let locked = write(&root, "locked.bin", b"needle");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read(&locked).is_ok() {
        // Running as root: permissions do not stop reads.
        return;
    }
    std::fs::create_dir_all(w.join("elsewhere")).unwrap();
    std::os::unix::fs::symlink(w.join("elsewhere"), root.join("dirlink")).unwrap();
    let req = HexFindRequest {
        pattern: "needle".into(),
        roots: vec![os(&root)],
        recursive: true,
        ..HexFindRequest::default()
    };
    let outcome = svc
        .hex_find(&req, &mut NoHexEvents, &job(&svc, JobKind::HexFind))
        .unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o600)).unwrap();
    let json = serde_json::to_value(&outcome.json).unwrap();
    let reasons: BTreeMap<String, (String, bool)> = json["skipped"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            (
                Path::new(s["spec"].as_str().unwrap())
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                (
                    s["reason"].as_str().unwrap().to_owned(),
                    !s["error"].is_null(),
                ),
            )
        })
        .collect();
    assert_eq!(reasons["locked.bin"], ("error".to_owned(), true), "{json}");
    assert_eq!(reasons["dirlink"], ("filtered".to_owned(), false), "{json}");
    assert_eq!(json["counts"]["files_matched"], 1);
}

/// N11: `session edit --set view.checkerboard=…` takes a boolean only.
#[test]
fn n11_session_edit_checks_the_checkerboard() {
    let tmp = tempfile::tempdir().unwrap();
    let svc = service(tmp.path());
    write(
        &tmp.path().join("config"),
        "sessions/p.toml",
        b"type = \"picture-compare\"\n[specs]\nleft = \"a.png\"\nright = \"b.png\"\n",
    );
    let reference = SessionRef::parse("p").unwrap();
    let edit = |value: &str| {
        svc.session_edit(
            &hikasa_session::api::SessionTarget::Named(reference.clone()),
            &hikasa_session::api::SessionEdit {
                set: vec![("view.checkerboard".to_owned(), value.to_owned())],
                ..hikasa_session::api::SessionEdit::default()
            },
            false,
        )
    };
    assert!(edit("false").is_ok());
    for bad in ["\"yes\"", "1"] {
        assert_eq!(
            edit(bad).unwrap_err().kind(),
            ErrorKind::InvalidInput,
            "{bad}"
        );
    }
}
