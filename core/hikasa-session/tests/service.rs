//! Service integration (§6.2.8): quick, text and folder compare on a temporary folder with `LocalVfs`, the
//! `FixedClock` and the fake conversion runner; `folder_prepare` + `folder_run` with a `ChannelObserver`; reports
//! and output files; format and vfs commands; the end-to-end `RulesComparator` injection.

mod common;

use std::ffi::OsString;
use std::path::Path;
use std::sync::Arc;

use common::{job, os, service, write};
use hikasa_core::{
    CompareVerdict, ContentKind, ContentMethod, ContentResult, ErrorKind, ExitCode, JobKind, Side,
    WarningCode,
};
use hikasa_folder::{
    ChannelObserver, DisplayOptions, FolderDisplayFilter, FolderMode, FolderRulesPatch,
    NullObserver, ScanEvent,
};
use hikasa_formats::{DetectReason, FormatKind};
use hikasa_report::{FolderLayout, FolderReportOptions, TextLayout, TextReportOptions};
use hikasa_session::api::{
    ApiOp, FolderCompareRequest, QuickMethod, QuickRequest, ReportSettings, TextCompareRequest,
};
use hikasa_session::{ApiError, Service};
use hikasa_text::{
    CompareObserver, TextDisplayFilter, TextDocument, TextRulesPatch, UnimportantItem, ViewOptions,
};

fn quick(
    service: &Service,
    left: &Path,
    right: &Path,
    method: QuickMethod,
) -> hikasa_session::api::QuickResponse {
    quick_with(service, left, right, method, |_| {})
}

fn quick_with(
    service: &Service,
    left: &Path,
    right: &Path,
    method: QuickMethod,
    edit: impl FnOnce(&mut QuickRequest),
) -> hikasa_session::api::QuickResponse {
    let mut req = QuickRequest {
        left: os(left),
        right: os(right),
        method,
        ..QuickRequest::default()
    };
    edit(&mut req);
    service
        .quick(&req, &job(service, JobKind::QuickCompare))
        .unwrap()
}

#[test]
fn quick_exit_codes_per_method() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let a = write(&w, "a.txt", b"alpha\nbeta\n");
    let same = write(&w, "same.txt", b"alpha\nbeta\n");
    let crlf = write(&w, "crlf.txt", b"alpha\r\nbeta\r\n");
    let ws = write(&w, "ws.txt", b"alpha  \nbeta\n");
    let other = write(&w, "other.txt", b"alpha\nbetx\n");
    let bin_a = write(&w, "a.bin", b"\x00\x01\x02binary");
    let bin_b = write(&w, "b.bin", b"\x00\x01\x03binary");

    let code = |left: &Path, right: &Path, method| quick(&service, left, right, method).exit_code;
    assert_eq!(code(&a, &same, QuickMethod::Rules), ExitCode::BinarySame);
    assert_eq!(code(&a, &crlf, QuickMethod::Rules), ExitCode::RulesSame);
    assert_eq!(
        code(&a, &other, QuickMethod::Rules),
        ExitCode::RulesDifferent
    );
    assert_eq!(
        code(&a, &ws, QuickMethod::Rules),
        ExitCode::RulesDifferent,
        "D-04: whitespace is important"
    );
    assert_eq!(
        code(&bin_a, &bin_b, QuickMethod::Rules),
        ExitCode::BinaryDifferent,
        "D-12"
    );
    let unimportant_ws = |iu: bool| {
        quick_with(&service, &a, &ws, QuickMethod::Rules, |r| {
            r.rules_patch.unimportant = vec![UnimportantItem::Whitespace];
            r.ignore_unimportant = iu;
        })
    };
    let resp = unimportant_ws(false);
    assert_eq!(resp.outcome.result, ContentResult::UnimportantOnly);
    assert_eq!(resp.exit_code, ExitCode::Similar);
    assert_eq!(
        unimportant_ws(true).exit_code,
        ExitCode::RulesSame,
        "--iu turns 12 into 2"
    );
    for method in [QuickMethod::Crc, QuickMethod::Binary] {
        assert_eq!(code(&a, &same, method), ExitCode::BinarySame);
        assert_eq!(code(&a, &crlf, method), ExitCode::BinaryDifferent);
        assert!(quick(&service, &a, &same, method).format.is_none());
    }
    assert_eq!(
        code(&a, &other, QuickMethod::Size),
        ExitCode::BinarySame,
        "same size"
    );
    assert_eq!(
        code(&a, &crlf, QuickMethod::Size),
        ExitCode::BinaryDifferent
    );
    let resp = quick(&service, &a, &other, QuickMethod::Rules);
    let format = resp.format.unwrap();
    assert_eq!(format.id, None, "*.txt is Everything Else");
    assert_eq!(format.reason, DetectReason::Fallback);
    assert_eq!(resp.outcome.method, ContentMethod::Rules);
}

#[test]
fn quick_errors_are_100() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let a = write(&w, "a.c", b"int a;\n");
    let run = |left: OsString, right: OsString| {
        service.quick(
            &QuickRequest {
                left,
                right,
                ..QuickRequest::default()
            },
            &job(&service, JobKind::QuickCompare),
        )
    };
    let missing = run(os(&a), os(&w.join("missing.c"))).unwrap_err();
    assert!(
        matches!(
            &missing,
            ApiError::LoadFailed {
                side: Side::Right,
                ..
            }
        ),
        "{missing:?}"
    );
    assert_eq!(missing.kind(), ErrorKind::NotFound);
    assert_eq!(missing.exit_code(ApiOp::Quick), ExitCode::UnknownError);
    let ftp = run(os(&a), OsString::from("ftp://host/x.c")).unwrap_err();
    assert_eq!(ftp.kind(), ErrorKind::Unsupported);
    assert_eq!(ftp.exit_code(ApiOp::Quick), ExitCode::UnknownError);
    let folder = run(os(&a), os(&w)).unwrap_err();
    assert!(matches!(folder, ApiError::InvalidArgument(_)));
    let relative = run(OsString::from("a.c"), OsString::from("a.c")).unwrap();
    assert_eq!(
        relative.exit_code,
        ExitCode::BinarySame,
        "relative specs use the captured cwd"
    );
    let bad_format = service
        .quick(
            &QuickRequest {
                left: os(&a),
                right: os(&a),
                format: Some("no-such-format".into()),
                ..QuickRequest::default()
            },
            &job(&service, JobKind::QuickCompare),
        )
        .unwrap_err();
    assert!(matches!(bad_format, ApiError::Format(_)));
    let bad_rules = service
        .quick(
            &QuickRequest {
                left: os(&a),
                right: os(&a),
                rules_patch: TextRulesPatch {
                    unimportant_text: vec![hikasa_text::TextPattern::regex("(")],
                    ..TextRulesPatch::default()
                },
                ..QuickRequest::default()
            },
            &job(&service, JobKind::QuickCompare),
        )
        .unwrap_err();
    assert!(matches!(bad_rules, ApiError::Text(_)), "{bad_rules:?}");
}

#[test]
fn quick_stdin_uses_the_name_hint_and_formats_mismatch() {
    let tmp = tempfile::tempdir().unwrap();
    let w = tmp.path().join("work");
    let c = write(&w, "x.c", b"int x; /* a */\n");
    let stdin = common::blob(b"int x; /* b */\n", Some("x.c"));
    let service = common::service_with(
        tmp.path(),
        Some(stdin),
        Arc::new(common::UpperRunner::default()),
    );
    let resp = quick_with(&service, Path::new("-"), &c, QuickMethod::Rules, |r| {
        r.left = OsString::from("-");
        r.rules_patch.unimportant = vec![UnimportantItem::Element("Comment".into())];
    });
    let format = resp.format.clone().unwrap();
    assert_eq!(
        format.id.as_ref().map(|i| i.as_str()),
        Some("c-cpp"),
        "D-15: the stdin name hint"
    );
    assert_eq!(resp.outcome.result, ContentResult::UnimportantOnly);
    assert_eq!(resp.left.canonical(), "-");

    let py = write(&w, "x.py", b"x = 1\n");
    let resp = quick(&service, &c, &py, QuickMethod::Rules);
    assert_eq!(
        resp.format
            .as_ref()
            .unwrap()
            .id
            .as_ref()
            .map(|i| i.as_str()),
        Some("c-cpp"),
        "the left name decides"
    );
    assert!(
        resp.outcome
            .warnings
            .iter()
            .any(|w| w.code == WarningCode::FormatMismatch),
        "{:?}",
        resp.outcome.warnings
    );
    let forced = quick_with(&service, &c, &py, QuickMethod::Rules, |r| {
        r.format = Some("python".into());
    });
    let format = forced.format.unwrap();
    assert!(matches!(format.reason, DetectReason::Forced { .. }));
    assert!(
        forced
            .outcome
            .warnings
            .iter()
            .all(|w| w.code != WarningCode::FormatMismatch)
    );
}

#[test]
fn quick_external_conversion_uses_the_runner() {
    let tmp = tempfile::tempdir().unwrap();
    write(
        &tmp.path().join("config"),
        "formats/upper.toml",
        common::EXTERNAL_FORMAT.as_bytes(),
    );
    let runner = Arc::new(common::UpperRunner::default());
    let service = common::service_with(tmp.path(), None, runner.clone());
    assert!(
        service.startup_warnings().is_empty(),
        "{:?}",
        service.startup_warnings()
    );
    let w = tmp.path().join("work");
    let lower = write(&w, "a.up", b"abc\n");
    let upper = write(&w, "b.up", b"ABC\n");
    let resp = quick(&service, &lower, &upper, QuickMethod::Rules);
    assert_eq!(
        resp.outcome.result,
        ContentResult::RulesSame,
        "{:?}",
        resp.outcome
    );
    assert_eq!(runner.calls.load(std::sync::atomic::Ordering::SeqCst), 2);
}

#[derive(Default)]
struct Recorder {
    loaded: Option<(u32, u32)>,
    phases: Vec<hikasa_core::Phase>,
}

impl CompareObserver for Recorder {
    fn loaded(&mut self, left: &Arc<TextDocument>, right: &Arc<TextDocument>) {
        self.loaded = Some((left.line_count(), right.line_count()));
    }
    fn phase(&mut self, phase: hikasa_core::Phase) {
        self.phases.push(phase);
    }
}

fn text_req(left: &Path, right: &Path) -> TextCompareRequest {
    TextCompareRequest {
        left: os(left),
        right: os(right),
        ..TextCompareRequest::default()
    }
}

#[test]
fn text_compare_end_to_end() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let a = write(&w, "a.c", b"int a = 1;\nint b;\n/* note */\n");
    let b = write(&w, "b.c", b"int a = 2;\nint b;\n/* NOTE */\nint c;\n");
    let mut recorder = Recorder::default();
    let resp = service
        .text_compare(
            &text_req(&a, &b),
            &job(&service, JobKind::TextCompare),
            Some(&mut recorder),
        )
        .unwrap();
    assert_eq!(recorder.loaded, Some((3, 4)));
    assert!(!recorder.phases.is_empty());
    assert_eq!(resp.verdict, ContentResult::ImportantDifferences);
    assert_eq!(resp.exit_code, ExitCode::RulesDifferent);
    assert_eq!(resp.format.kind, FormatKind::Text);
    assert!(
        matches!(&resp.format.reason, DetectReason::Mask { id, mask } if id.as_str() == "c-cpp" && mask == "*.c")
    );
    assert_eq!(resp.detection.left, Some(resp.format.reason.clone()));
    assert_eq!(resp.detection.right, Some(resp.format.reason.clone()));
    assert!(!resp.comparison.bytes_identical());

    // JSON with the detection reasons (§4.3.2).
    let json = resp.json(&ViewOptions {
        filter: TextDisplayFilter::Differences,
        ignore_unimportant: false,
        expanded: vec![],
    });
    let value = serde_json::to_value(&json).unwrap();
    assert_eq!(value["format"]["reason"]["reason"], "mask");
    assert_eq!(value["format"]["reason"]["mask"], "*.c");
    assert_eq!(value["detection"]["right"]["id"], "c-cpp");
    assert_eq!(value["verdict"]["result"], "important_differences");

    // Reports use the injected clock and the optional title.
    let mut out = Vec::new();
    service
        .render_text(&resp, &TextReportOptions::default(), &mut out)
        .unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("2026-10-21T12:00:00+00:00"), "{text}");
    let mut out = Vec::new();
    let settings = ReportSettings {
        title: Some("My title".into()),
        ..ReportSettings::default()
    };
    let opts = TextReportOptions {
        layout: TextLayout::SideBySide,
        ..TextReportOptions::default()
    };
    let warnings = service
        .render_text_with(&resp, &opts, &settings, &mut out)
        .unwrap();
    assert!(warnings.is_empty());
    assert!(String::from_utf8(out).unwrap().contains("My title"));

    // Identical files: BinarySame (exit 1); line endings only: RulesSame (exit 2).
    let a2 = write(&w, "a2.c", b"int a = 1;\nint b;\n/* note */\n");
    let crlf = write(&w, "crlf.c", b"int a = 1;\r\nint b;\r\n/* note */\r\n");
    let same = service
        .text_compare(
            &text_req(&a, &a2),
            &job(&service, JobKind::TextCompare),
            None,
        )
        .unwrap();
    assert_eq!(same.exit_code, ExitCode::BinarySame);
    assert!(same.comparison.bytes_identical());
    let eol = service
        .text_compare(
            &text_req(&a, &crlf),
            &job(&service, JobKind::TextCompare),
            None,
        )
        .unwrap();
    assert_eq!(eol.exit_code, ExitCode::RulesSame);
}

#[test]
fn text_compare_rules_encodings_mismatch_and_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let a = write(&w, "a.c", b"int  a;\n");
    let b = write(&w, "b.py", b"int a;\n");
    let rules = write(
        &w,
        "rules.toml",
        b"schema_version = 1\nfuture_key = 1\n[importance]\nembedded_whitespace = false\n",
    );
    let mut req = text_req(&a, &b);
    req.rules_file = Some(os(&rules));
    let resp = service
        .text_compare(&req, &job(&service, JobKind::TextCompare), None)
        .unwrap();
    assert_eq!(resp.verdict, ContentResult::UnimportantOnly);
    assert_eq!(resp.exit_code, ExitCode::Similar);
    let codes: Vec<_> = resp.warnings.iter().map(|w| w.code).collect();
    assert!(codes.contains(&WarningCode::UnknownConfigKey), "{codes:?}");
    assert!(codes.contains(&WarningCode::FormatMismatch), "{codes:?}");
    let unknown_key = resp
        .warnings
        .iter()
        .find(|w| w.code == WarningCode::UnknownConfigKey)
        .unwrap();
    assert!(unknown_key.spec.as_deref().unwrap().ends_with("rules.toml"));
    assert!(
        matches!(resp.detection.right, Some(DetectReason::Mask { ref id, .. }) if id.as_str() == "python")
    );
    req.ignore_unimportant = true;
    let iu = service
        .text_compare(&req, &job(&service, JobKind::TextCompare), None)
        .unwrap();
    assert_eq!(iu.exit_code, ExitCode::RulesSame);

    // Per-pane encodings (TC-001): a Latin-1 file read as windows-1252 equals its UTF-8 twin.
    let latin = write(&w, "l.txt", b"caf\xe9\n");
    let utf8 = write(&w, "u.txt", "café\n".as_bytes());
    let mut req = text_req(&latin, &utf8);
    req.encodings.left = Some("windows-1252".into());
    let resp = service
        .text_compare(&req, &job(&service, JobKind::TextCompare), None)
        .unwrap();
    assert_eq!(resp.verdict, ContentResult::RulesSame);
    req.encodings.right = Some("no-such-encoding".into());
    assert!(matches!(
        service.text_compare(&req, &job(&service, JobKind::TextCompare), None),
        Err(ApiError::Format(_))
    ));

    // A missing right side is a LoadFailed of the right side (exit 100).
    let err = service
        .text_compare(
            &text_req(&a, &w.join("gone.c")),
            &job(&service, JobKind::TextCompare),
            None,
        )
        .unwrap_err();
    assert!(
        matches!(&err, ApiError::LoadFailed { side: Side::Right, spec, .. } if spec.ends_with("gone.c")),
        "{err:?}"
    );
    assert_eq!(err.exit_code(ApiOp::TextCompare), ExitCode::UnknownError);
    // A newer rules file is unsupported.
    let newer = write(&w, "newer.toml", b"schema_version = 99\n");
    let mut req = text_req(&a, &a);
    req.rules_file = Some(os(&newer));
    let err = service
        .text_compare(&req, &job(&service, JobKind::TextCompare), None)
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unsupported);
}

fn folder_tree(root: &Path) {
    write(root, "l/same.txt", b"same\n");
    write(root, "r/same.txt", b"same\n");
    write(root, "l/eol.txt", b"a\nb\n");
    write(root, "r/eol.txt", b"a\r\nb\r\n");
    write(root, "l/ws.c", b"int a;\n");
    write(root, "r/ws.c", b"int  a;\n");
    write(root, "l/diff.txt", b"one\n");
    write(root, "r/diff.txt", b"two\n");
    write(root, "l/sub/deep.py", b"x = 1  # c\n");
    write(root, "r/sub/deep.py", b"x = 1  # changed comment\n");
    write(root, "l/bin.dat", b"\x00\x01\x02");
    write(root, "r/bin.dat", b"\x00\x01\x03");
    write(root, "l/only-left.txt", b"l\n");
    write(root, "r/only-right.txt", b"r\n");
}

#[test]
fn folder_prepare_and_run_with_a_channel_observer() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    folder_tree(&w);
    let req = FolderCompareRequest {
        left: os(&w.join("l")),
        right: os(&w.join("r")),
        criteria: Some("size rules-based".into()),
        ..FolderCompareRequest::default()
    };
    let prepared = service.folder_prepare(&req).unwrap();
    assert!(
        prepared.text_rules().is_some(),
        "content method rules builds the text rules"
    );
    assert_eq!(
        prepared.rules().criteria.content.method,
        ContentMethod::Rules
    );
    let model = Arc::clone(prepared.model());
    assert!(!model.is_finished());
    assert_eq!(model.generation(), 0);
    let (observer, events) = ChannelObserver::bounded(4);
    let consumer = std::thread::spawn(move || events.iter().collect::<Vec<ScanEvent>>());
    let resp = service
        .folder_run(&prepared, &job(&service, JobKind::FolderCompare), &observer)
        .unwrap();
    drop(observer);
    let events = consumer.join().unwrap();
    assert_eq!(events.first(), Some(&ScanEvent::Started));
    assert_eq!(events.last(), Some(&ScanEvent::Finished { complete: true }));
    assert!(model.is_finished());
    assert!(model.generation() > 0);
    assert!(resp.comparison.is_complete());
    assert_eq!(resp.verdict, CompareVerdict::Differences);
    assert_eq!(resp.exit_code, ExitCode::RulesDifferent);
    resp.comparison.check_invariants().unwrap();
    // A prepared comparison is scanned once.
    let again = service.folder_run(
        &prepared,
        &job(&service, JobKind::FolderCompare),
        &NullObserver,
    );
    assert!(matches!(again, Err(ApiError::Folder(_))));
    // The JSON of §4.3.3 and the list report (with and without the summary line).
    let display = DisplayOptions {
        filter: FolderDisplayFilter::All,
        folders: FolderMode::CompareStructure,
        ignore_unimportant: false,
        suppress_filters: false,
    };
    let json = serde_json::to_value(resp.json(&display)).unwrap();
    assert_eq!(json["verdict"], "differences");
    let opts = FolderReportOptions {
        layout: FolderLayout::List,
        display,
        ..FolderReportOptions::default()
    };
    let mut full = Vec::new();
    service.render_folder(&resp, &opts, &mut full).unwrap();
    let full = String::from_utf8(full).unwrap();
    let last = full.lines().last().unwrap();
    assert!(last.starts_with("# "), "{full}");
    let mut quiet = Vec::new();
    let settings = ReportSettings {
        list_summary: false,
        ..ReportSettings::default()
    };
    service
        .render_folder_with(&resp, &opts, &settings, &mut quiet)
        .unwrap();
    let quiet = String::from_utf8(quiet).unwrap();
    assert_eq!(
        quiet,
        full.lines()
            .filter(|l| !l.starts_with("# "))
            .map(|l| format!("{l}\n"))
            .collect::<String>()
    );
}

#[test]
fn folder_exit_codes() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    write(&w, "same/l/a.txt", b"x\n");
    write(&w, "same/r/a.txt", b"x\n");
    write(&w, "ws/l/a.c", b"int a;\n");
    write(&w, "ws/r/a.c", b"int  a;\n");
    let run = |dir: &str, edit: &dyn Fn(&mut FolderCompareRequest)| {
        let mut req = FolderCompareRequest {
            left: os(&w.join(dir).join("l")),
            right: os(&w.join(dir).join("r")),
            ..FolderCompareRequest::default()
        };
        edit(&mut req);
        service.folder_compare(&req, &job(&service, JobKind::FolderCompare), &NullObserver)
    };
    let rules_ws = |r: &mut FolderCompareRequest| {
        r.criteria = Some("rules-based".into());
        r.text_patch.unimportant = vec![UnimportantItem::Whitespace];
    };
    assert_eq!(run("same", &|_| {}).unwrap().exit_code, ExitCode::RulesSame);
    let similar = run("ws", &rules_ws).unwrap();
    assert_eq!(similar.verdict, CompareVerdict::UnimportantOnly);
    assert_eq!(similar.exit_code, ExitCode::Similar);
    let iu = run("ws", &|r| {
        rules_ws(r);
        r.ignore_unimportant = true;
    })
    .unwrap();
    assert_eq!(iu.exit_code, ExitCode::RulesSame);
    // The criteria word `ignore-unimportant` sets it too.
    let word = run("ws", &|r| {
        r.criteria = Some("rules-based ignore-unimportant".into());
        r.text_patch.unimportant = vec![UnimportantItem::Whitespace];
    })
    .unwrap();
    assert_eq!(word.exit_code, ExitCode::RulesSame);
    // A base that is not loadable: 107.
    let err = service
        .folder_compare(
            &FolderCompareRequest {
                left: os(&w.join("same/l")),
                right: os(&w.join("nowhere")),
                ..FolderCompareRequest::default()
            },
            &job(&service, JobKind::FolderCompare),
            &NullObserver,
        )
        .unwrap_err();
    assert_eq!(
        err.exit_code(ApiOp::FolderCompare),
        ExitCode::LoadFailed,
        "{err:?}"
    );
    // Bad criteria and filters are argument errors (100).
    let err = run("same", &|r| r.criteria = Some("sizes".into())).unwrap_err();
    assert_eq!(err.exit_code(ApiOp::FolderCompare), ExitCode::UnknownError);
    let err = run("same", &|r| {
        r.patch = FolderRulesPatch {
            name_filter: Some("[".into()),
            ..FolderRulesPatch::default()
        }
    })
    .unwrap_err();
    assert_eq!(err.exit_code(ApiOp::FolderCompare), ExitCode::UnknownError);
    // A cancelled scan after the bases are loaded is incomplete: verdict Incomplete, exit 100.
    let req = FolderCompareRequest {
        left: os(&w.join("same/l")),
        right: os(&w.join("same/r")),
        ..FolderCompareRequest::default()
    };
    let prepared = service.folder_prepare(&req).unwrap();
    let control = job(&service, JobKind::FolderCompare);
    struct CancelOnStart(hikasa_core::CancelToken);
    impl hikasa_folder::ScanObserver for CancelOnStart {
        fn on_event(&self, event: ScanEvent) {
            if event == ScanEvent::Started {
                self.0.cancel();
            }
        }
    }
    let resp = service
        .folder_run(&prepared, &control, &CancelOnStart(control.cancel.clone()))
        .unwrap();
    assert!(!resp.comparison.is_complete());
    assert_eq!(resp.verdict, CompareVerdict::Incomplete);
    assert_eq!(resp.exit_code, ExitCode::UnknownError);
    // A folder rules file with an unknown key warns and names the file.
    let rules = write(&w, "folder.toml", b"schema_version = 1\nwhat = 1\n");
    let resp = run("same", &|r| r.rules_file = Some(os(&rules))).unwrap();
    assert!(resp.warnings.iter().any(|w| {
        w.code == WarningCode::UnknownConfigKey
            && w.spec
                .as_deref()
                .is_some_and(|s| s.ends_with("folder.toml"))
    }));
}

/// End-to-end injection (§6.2.8): `folder compare --content rules` gives, for every pair, the same result as
/// `quick` (Rules) on that pair, with the same text rules.
#[test]
fn rules_comparator_injection_agrees_with_quick() {
    let tmp = tempfile::tempdir().unwrap();
    write(
        &tmp.path().join("config"),
        "formats/upper.toml",
        common::EXTERNAL_FORMAT.as_bytes(),
    );
    let service = common::service_with(tmp.path(), None, Arc::new(common::UpperRunner::default()));
    let w = tmp.path().join("work");
    folder_tree(&w);
    write(&w, "l/conv.up", b"abc\n");
    write(&w, "r/conv.up", b"ABC\n");
    write(&w, "l/conv2.up", b"abc\n");
    write(&w, "r/conv2.up", b"abd\n");
    for unimportant in [
        vec![],
        vec![UnimportantItem::Whitespace],
        vec![UnimportantItem::Element("Comment".into())],
    ] {
        let text_patch = TextRulesPatch {
            unimportant: unimportant.clone(),
            ..TextRulesPatch::default()
        };
        let resp = service
            .folder_compare(
                &FolderCompareRequest {
                    left: os(&w.join("l")),
                    right: os(&w.join("r")),
                    criteria: Some("rules-based".into()),
                    text_patch: text_patch.clone(),
                    ..FolderCompareRequest::default()
                },
                &job(&service, JobKind::FolderCompare),
                &NullObserver,
            )
            .unwrap();
        let read = resp.comparison.read();
        let mut compared = 0;
        for (id, _) in read.preorder() {
            let Some(outcome) = read.content_outcome(id) else {
                continue;
            };
            let path = read.path(id);
            let quick = quick_with(
                &service,
                &w.join("l").join(path.as_str()),
                &w.join("r").join(path.as_str()),
                QuickMethod::Rules,
                |r| {
                    r.rules_patch = text_patch.clone();
                },
            );
            assert_eq!(
                outcome.result.kind(),
                quick.outcome.result.kind(),
                "{path} with {unimportant:?}: folder {:?} vs quick {:?}",
                outcome,
                quick.outcome
            );
            compared += 1;
        }
        assert_eq!(compared, 8, "every two-sided file pair was content-tested");
    }
}

#[test]
fn write_output_is_atomic() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let target = w.join("report.txt");
    let spec = service
        .write_output(target.as_os_str(), b"first\n")
        .unwrap();
    assert_eq!(spec.canonical(), target.to_string_lossy());
    service
        .write_output(OsString::from("report.txt").as_os_str(), b"second\n")
        .unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"second\n");
    let err = service
        .write_output(w.join("no/such/dir/r.txt").as_os_str(), b"x")
        .unwrap_err();
    assert!(matches!(err, ApiError::Vfs(_)), "{err:?}");
    assert!(!w.join("no").exists());
    assert!(matches!(
        service.write_output(OsString::from("-").as_os_str(), b"x"),
        Err(ApiError::Vfs(_))
    ));
}

#[test]
fn spec_context_and_startup_facts() {
    let tmp = tempfile::tempdir().unwrap();
    write(
        &tmp.path().join("config"),
        "options.toml",
        b"[text_editing]\nnumber_of_context_lines = 5\nx = 1\n[mystery]\n",
    );
    write(&tmp.path().join("config"), "formats/broken.toml", b"[[[");
    let service = service(tmp.path());
    assert_eq!(service.options().context_lines, 5);
    assert_eq!(service.ignored_option_keys(), ["text_editing.x"]);
    let codes: Vec<_> = service.startup_warnings().iter().map(|w| w.code).collect();
    assert_eq!(
        codes,
        vec![
            WarningCode::UnknownConfigKey,
            WarningCode::UserFormatSkipped
        ]
    );
    assert_eq!(service.pool().current_num_threads(), 4);
    assert_eq!(service.stream_options().threads, 4);
    assert_eq!(
        service.generator(),
        format!("hikasa {}", hikasa_session::HIKASA_VERSION)
    );
    let ctx = service.spec_context();
    assert_eq!(ctx.cwd.as_deref(), Some(tmp.path().join("work").as_path()));
    assert_eq!(
        service
            .parse_spec(OsString::from("x").as_os_str())
            .unwrap()
            .canonical(),
        tmp.path().join("work").join("x").to_string_lossy()
    );
    assert!(
        service
            .parse_spec(OsString::from("bogus://x").as_os_str())
            .is_err()
    );
    assert_eq!(service.config().dir, tmp.path().join("config"));
    // A newer options.toml stops the service (D-31).
    let newer = tempfile::tempdir().unwrap();
    write(
        &newer.path().join("config"),
        "options.toml",
        b"schema_version = 7\n",
    );
    let init = hikasa_session::ServiceInit {
        config_dir: Some(newer.path().join("config")),
        ..hikasa_session::ServiceInit::new(
            common::facts(newer.path()),
            common::fixed_clock(),
            Arc::new(common::UpperRunner::default()),
        )
    };
    let err = Service::new(init).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unsupported);
    // A single thread is fine; stream threads are min(4, pool size).
    let single = hikasa_session::ServiceInit {
        config_dir: Some(tmp.path().join("config")),
        threads: std::num::NonZeroUsize::new(1),
        ..hikasa_session::ServiceInit::new(
            common::facts(tmp.path()),
            common::fixed_clock(),
            Arc::new(common::UpperRunner::default()),
        )
    };
    let single = Service::new(single).unwrap();
    assert_eq!(single.stream_options().threads, 1);
}

#[test]
fn content_kinds_cover_quick_results() {
    // The quick response's exit code is ExitCode::for_content of its result (§2.21).
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let a = write(&w, "a.txt", b"x\n");
    let b = write(&w, "b.txt", b"y\n");
    let resp = quick(&service, &a, &b, QuickMethod::Rules);
    assert_eq!(
        resp.outcome.result.kind(),
        ContentKind::ImportantDifferences
    );
    assert_eq!(
        resp.exit_code,
        ExitCode::for_content(&resp.outcome.result, false)
    );
}
