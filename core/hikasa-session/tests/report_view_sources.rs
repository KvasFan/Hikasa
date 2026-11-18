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
