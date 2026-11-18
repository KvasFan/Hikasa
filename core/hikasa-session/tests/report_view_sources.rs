//! M2 W13c (D-366, §1.13.20): the report sources of views. `Service::text_report_source` — a live view without
//! edits reports what `text_compare` reports, byte for byte (`unsaved` false on both sides); with an unsaved edit,
//! once the recompare is installed, the report equals a fresh comparison of the saved text except the header's
//! *(unsaved edits)* mark on the edited side. `Service::folder_view_response` — the verdict, exit code and report
//! of a view's model equal `folder_run`'s.

mod common;

use std::path::Path;

use common::{job, os, service, write};
use hikasa_core::{JobKind, Pair, Side};
use hikasa_report::ReportWords;
use hikasa_session::Service;
use hikasa_session::api::{
    FolderCompareRequest, LiveOptions, LiveTextView, ReportFlags, ReportSource, ReportTarget,
    TextCompareRequest,
};
use hikasa_text::{EditRequest, GroupHint, TextPos};

/// The report of `source` with `layout` and `output` words, on the clipboard (its text: the report's bytes).
fn report(service: &Service, source: ReportSource<'_>, layout: &str, output: &[&str]) -> String {
    let flags = ReportFlags {
        words: ReportWords {
            layout: Some(layout.to_owned()),
            output_options: output.iter().map(|w| (*w).to_owned()).collect(),
            ..ReportWords::default()
        },
        ..ReportFlags::default()
    };
    let target = ReportTarget::Clipboard;
    let mut req = service
        .report_request(source.kind(), "side-by-side", &flags, &target)
        .unwrap();
    let control = job(service, JobKind::Report);
    let plan = service
        .report_plan(source, &mut req, &target, None, &control)
        .unwrap();
    let outcome = service
        .report_execute(source, &req, plan, None, &control)
        .unwrap();
    outcome.clipboard.expect("a clipboard payload").text
}

fn open(service: &Service, left: &Path, right: &Path) -> LiveTextView {
    let req = TextCompareRequest {
        left: os(left),
        right: os(right),
        ..TextCompareRequest::default()
    };
    service
        .text_compare_live(
            &req,
            LiveOptions::default(),
            &job(service, JobKind::TextCompare),
            None,
        )
        .unwrap()
}

#[test]
fn text_report_source_without_edits_equals_the_comparison() {
    let dir = tempfile::tempdir().unwrap();
    let service = service(dir.path());
    let left = write(dir.path(), "a.txt", b"one\ntwo\nthree\n");
    let right = write(dir.path(), "b.txt", b"one\nTWO\nthree\nfour\n");
    let view = open(&service, &left, &right);
    let source = service.text_report_source(&view);
    assert_eq!(source.unsaved, Pair::new(false, false));
    assert_eq!(source.verdict, view.response.verdict);
    assert_eq!(source.exit_code, view.response.exit_code);
    for (layout, output) in [
        ("side-by-side", &["text"][..]),
        ("side-by-side", &["html-color"][..]),
        ("summary", &["text"][..]),
        ("interleaved", &["html-mono"][..]),
    ] {
        assert_eq!(
            report(&service, ReportSource::Text(&source), layout, output),
            report(&service, ReportSource::Text(&view.response), layout, output),
            "{layout} {output:?}"
        );
    }
}

#[test]
fn text_report_source_marks_the_edited_side_after_the_install() {
    let dir = tempfile::tempdir().unwrap();
    let service = service(dir.path());
    let left = write(dir.path(), "a.txt", b"one\ntwo\nthree\n");
    let right = write(dir.path(), "b.txt", b"one\nTWO\nthree\nfour\n");
    let mut view = open(&service, &left, &right);
    let at = TextPos::new(1, 0);
    let out = view
        .docs
        .left
        .apply(
            &EditRequest::Replace {
                range: at..at,
                text: "X".into(),
                keep_eols: false,
            },
            GroupHint::None,
        )
        .unwrap();
    view.live.note_edit(Side::Left, &view.docs.left, &out);
    let recompare = view
        .live
        .recompare_job(Pair::new(&view.docs.left, &view.docs.right));
    let control = job(&service, JobKind::TextCompare);
    let done = recompare.run(None, &control.ctl()).unwrap();
    view.live
        .install(done, Pair::new(&view.docs.left, &view.docs.right));
    let source = service.text_report_source(&view);
    assert_eq!(source.unsaved, Pair::new(true, false));
    let edited = report(
        &service,
        ReportSource::Text(&source),
        "side-by-side",
        &["text"],
    );
    // The same text saved: a fresh comparison's report, with the mark after the left spec.
    std::fs::write(&left, b"one\nXtwo\nthree\n").unwrap();
    let saved = open(&service, &left, &right);
    let fresh = report(
        &service,
        ReportSource::Text(&saved.response),
        "side-by-side",
        &["text"],
    );
    let spec = left.display().to_string();
    let marked = fresh.replacen(&spec, &format!("{spec} (unsaved edits)"), 1);
    assert_ne!(marked, fresh, "the spec is in the header");
    assert_eq!(edited, marked);
    assert_eq!(source.verdict, saved.response.verdict);
}

#[test]
fn folder_view_response_equals_the_run() {
    let dir = tempfile::tempdir().unwrap();
    let service = service(dir.path());
    write(dir.path(), "l/same.txt", b"same\n");
    write(dir.path(), "r/same.txt", b"same\n");
    write(dir.path(), "l/diff.txt", b"one\n");
    write(dir.path(), "r/diff.txt", b"two\n");
    write(dir.path(), "l/only.txt", b"x\n");
    let req = FolderCompareRequest {
        left: os(&dir.path().join("l")),
        right: os(&dir.path().join("r")),
        ..FolderCompareRequest::default()
    };
    let control = job(&service, JobKind::FolderCompare);
    let prepared = service.folder_prepare(&req).unwrap();
    let run = service
        .folder_run(&prepared, &control, &hikasa_folder::NullObserver)
        .unwrap();
    for ignore in [false, true] {
        let view = service.folder_view_response(&prepared, ignore);
        assert_eq!(view.verdict, run.comparison.verdict(ignore));
        assert_eq!(view.ignore_unimportant, ignore);
    }
    // A tree whose only difference is unimportant: the verdict follows the flag given.
    write(dir.path(), "ul/a.c", b"int a; // one\n");
    write(dir.path(), "ur/a.c", b"int a; // two\n");
    let unimportant = FolderCompareRequest {
        left: os(&dir.path().join("ul")),
        right: os(&dir.path().join("ur")),
        criteria: Some("rules-based".into()),
        text_patch: hikasa_text::TextRulesPatch {
            unimportant: vec![hikasa_text::UnimportantItem::Element("Comment".into())],
            ..hikasa_text::TextRulesPatch::default()
        },
        ..FolderCompareRequest::default()
    };
    let up = service.folder_prepare(&unimportant).unwrap();
    service
        .folder_run(&up, &control, &hikasa_folder::NullObserver)
        .unwrap();
    let (strict, lenient) = (
        service.folder_view_response(&up, false),
        service.folder_view_response(&up, true),
    );
    assert_ne!(strict.verdict, lenient.verdict, "{strict:?} {lenient:?}");
    assert_eq!(strict.verdict, hikasa_core::CompareVerdict::UnimportantOnly);
    let view = service.folder_view_response(&prepared, run.ignore_unimportant);
    assert_eq!(view.exit_code, run.exit_code);
    assert_eq!(view.warnings, run.warnings);
    let source = |r| ReportSource::Folder {
        request: &req,
        prepared: &prepared,
        response: r,
    };
    for layout in ["side-by-side", "summary", "list", "json"] {
        let output: &[&str] = if layout == "side-by-side" {
            &["html-color"]
        } else {
            &[]
        };
        assert_eq!(
            report(&service, source(&view), layout, output),
            report(&service, source(&run), layout, output),
            "{layout}"
        );
    }
}

/// The comparison's warnings are in the source once (W13c review: mutation C5): a pair with a decoding warning
/// gives `text_report_source` the response's warnings, not the stale comparison's twice.
#[test]
fn text_report_source_keeps_each_warning_once() {
    let dir = tempfile::tempdir().unwrap();
    let service = service(dir.path());
    let left = write(dir.path(), "a.txt", b"one\ntw\xc3o\nthree\n");
    let right = write(dir.path(), "b.txt", b"one\ntwo\nthree\n");
    // Read as UTF-8: the malformed byte is a decoding warning of the comparison.
    let req = TextCompareRequest {
        left: os(&left),
        right: os(&right),
        encodings: Pair::new(Some("utf-8".to_owned()), None),
        ..TextCompareRequest::default()
    };
    let view = service
        .text_compare_live(
            &req,
            LiveOptions::default(),
            &job(&service, JobKind::TextCompare),
            None,
        )
        .unwrap();
    assert!(
        !view.response.comparison.warnings().is_empty(),
        "the fixture has a comparison warning: {:?}",
        view.response.warnings
    );
    let source = service.text_report_source(&view);
    assert_eq!(source.warnings, view.response.warnings);
}

/// A folder view's model warnings are in its response (W13c review: mutation C4): an unreadable folder's warning
/// is in `folder_view_response` as in the run's.
#[cfg(unix)]
#[test]
fn folder_view_response_keeps_the_models_warnings() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let service = service(dir.path());
    write(dir.path(), "l/locked/a.txt", b"a\n");
    write(dir.path(), "r/locked/a.txt", b"a\n");
    write(dir.path(), "l/b.txt", b"b\n");
    write(dir.path(), "r/b.txt", b"b\n");
    let locked = dir.path().join("l/locked");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read_dir(&locked).is_ok() {
        // Root reads it anyway: nothing to show.
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        return;
    }
    let req = FolderCompareRequest {
        left: os(&dir.path().join("l")),
        right: os(&dir.path().join("r")),
        ..FolderCompareRequest::default()
    };
    let control = job(&service, JobKind::FolderCompare);
    let prepared = service.folder_prepare(&req).unwrap();
    let run = service
        .folder_run(&prepared, &control, &hikasa_folder::NullObserver)
        .unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        !run.comparison.read().warnings().is_empty(),
        "{:?}",
        run.warnings
    );
    let view = service.folder_view_response(&prepared, run.ignore_unimportant);
    assert_eq!(view.warnings, run.warnings);
}
