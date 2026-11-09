//! A10.20 and A10.21 through the service (D-281, D-282; contract §1.10.16, §1.10.17, §1.10.23): a format per pane
//! from the request and from `[specs] left_format`/`right_format` (and `--format` beside one refused), JSON
//! `right_format`; ignored lines from the request (a line beyond a document refused), the verdict and exit code of
//! a pair whose only differences are ignored, the session round trip of `ignored_left`/`ignored_right` with
//! `ignored_check_left`/`ignored_check_right`, and a changed file dropping its marks with `ignored_lines_dropped`.

#![allow(clippy::single_range_in_vec_init)] // one-range line sets are meant as such

mod common;

use std::path::Path;

use common::{job, os, service, write};
use hikasa_core::{ContentResult, ExitCode, JobKind, Pair, Side, WarningCode};
use hikasa_session::Service;
use hikasa_session::api::{
    ApiError, SessionRunRequest, TextCompareRequest, ViewRequest, ignored_words, parse_ignored,
    parse_ignored_word,
};
use hikasa_session::ipc::{OpenOptions, ViewKind};
use hikasa_session::sessions::{Session, SessionRef, SessionType};
use hikasa_text::{IgnoredLines, RangeSet, ViewOptions, ignored_check};
use toml::Table;

const C_TEXT: &str = "int a = 1; // one\nint b = 2;\n/* c */ int c = 3;\nreturn a;\n";

fn compare(
    svc: &Service,
    req: &TextCompareRequest,
) -> Result<hikasa_session::api::TextCompareResponse, ApiError> {
    svc.text_compare(req, &job(svc, JobKind::TextCompare), None)
}

fn pair(root: &Path) -> TextCompareRequest {
    let a = write(root, "w/a.c", C_TEXT.as_bytes());
    let b = write(root, "w/b.txt", format!("{C_TEXT}extra\n").as_bytes());
    TextCompareRequest {
        left: os(&a),
        right: os(&b),
        ..TextCompareRequest::default()
    }
}

fn comments_unimportant(req: &mut TextCompareRequest) {
    req.rules_patch.unimportant = vec![hikasa_text::UnimportantItem::Element("Comment".into())];
}

#[test]
fn a_format_per_pane_from_the_request() {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(dir.path());
    let mut req = pair(dir.path());
    comments_unimportant(&mut req);
    // D-15: the left name's format for both.
    let one = compare(&svc, &req).unwrap();
    assert!(one.right_format.is_none() && one.comparison.right_format().is_none());
    let mismatch = |r: &hikasa_session::api::TextCompareResponse| {
        r.warnings
            .iter()
            .any(|w| w.code == hikasa_core::WarningCode::FormatMismatch)
    };
    assert!(mismatch(&one), "{:?}", one.warnings);
    // The right pane as plain text: every comment line differs.
    req.formats = Pair::new(None, Some("Everything Else".into()));
    let two = compare(&svc, &req).unwrap();
    let rf = two.right_format.as_ref().unwrap();
    assert_eq!(rf.name, "Everything Else");
    // A pane with its own format: no `format_mismatch` (W10b note 10).
    assert!(!mismatch(&two), "{:?}", two.warnings);
    assert_eq!(two.format.name, one.format.name);
    let differing = |r: &hikasa_session::api::TextCompareResponse| {
        let s = r.comparison.stats();
        s.rows - s.same_rows
    };
    assert!(differing(&two) > differing(&one), "{}", differing(&two));
    let json = serde_json::to_value(two.json(&ViewOptions::default())).unwrap();
    assert_eq!(json["right_format"]["name"], "Everything Else");
    assert_eq!(json["right_format"]["reason"]["reason"], "fallback");
    assert!(
        serde_json::to_value(one.json(&ViewOptions::default()))
            .unwrap()
            .get("right_format")
            .is_none()
    );
    // A pane's format by id; the other keeps D-15's.
    req.formats = Pair::new(Some("java".into()), None);
    let three = compare(&svc, &req).unwrap();
    assert_eq!(three.comparison.format().id().unwrap().as_str(), "java");
    assert_eq!(three.right_format.as_ref().unwrap().name, one.format.name);
    // Both the same: one format, no `right_format`.
    req.formats = Pair::new(Some("c-cpp".into()), Some("c-cpp".into()));
    assert!(compare(&svc, &req).unwrap().right_format.is_none());
    // `--format` beside a pane's own is refused (exit 100).
    req.format = Some("c-cpp".into());
    let err = compare(&svc, &req).err().unwrap();
    assert!(matches!(err, ApiError::InvalidArgument(_)), "{err:?}");
    assert_eq!(
        err.exit_code(hikasa_session::api::ApiOp::TextCompare)
            .code(),
        100
    );
    // An unknown pane format is an error too.
    req.format = None;
    req.formats = Pair::new(None, Some("no such format".into()));
    assert!(compare(&svc, &req).is_err());
}

#[test]
fn ignored_words_are_parsed_by_the_core() {
    assert_eq!(parse_ignored_word("left:3-5").unwrap(), (Side::Left, 2..5));
    assert_eq!(
        parse_ignored_word(" RIGHT : 7 ").unwrap(),
        (Side::Right, 6..7)
    );
    for bad in ["3-5", "left:", "left:0", "left:5-3", "centre:1", "left:x"] {
        assert!(
            matches!(parse_ignored_word(bad), Err(ApiError::InvalidArgument(_))),
            "{bad}"
        );
    }
    let ignored = parse_ignored(&["left:1".into(), "left:2-3".into(), "right:9".into()]).unwrap();
    assert_eq!(ignored.left.ranges(), &[0..3]);
    assert_eq!(ignored.right.ranges(), &[8..9]);
    assert_eq!(ignored_words(&ignored.left), ["1-3"]);
    assert_eq!(
        ignored_words(&RangeSet::from_ranges(vec![4..5, 7..9])),
        ["5", "8-9"]
    );
}

#[test]
fn only_ignored_differences_are_rules_same() {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(dir.path());
    let mut req = pair(dir.path());
    let plain = compare(&svc, &req).unwrap();
    assert_eq!(plain.verdict, ContentResult::ImportantDifferences);
    // The right file's extra line 5 is the only difference.
    req.ignored = parse_ignored(&["right:5".into()]).unwrap();
    let got = compare(&svc, &req).unwrap();
    assert_eq!(got.verdict, ContentResult::RulesSame);
    assert_eq!(
        got.exit_code,
        ExitCode::for_content(&ContentResult::RulesSame, false)
    );
    assert_eq!(got.comparison.stats().ignored_rows, 1);
    let check = got.ignored_check.right.unwrap();
    assert_eq!(
        Some(check),
        ignored_check(got.comparison.document(Side::Right), &req.ignored.right)
    );
    assert!(got.ignored_check.left.is_none());
    // A line beyond the document is refused (exit 100).
    req.ignored = parse_ignored(&["right:6".into()]).unwrap();
    let err = compare(&svc, &req).err().unwrap();
    assert!(matches!(err, ApiError::InvalidArgument(_)), "{err:?}");
}

fn session_ref(name: &str) -> SessionRef {
    SessionRef::parse(name).unwrap()
}

/// The view's request with marks becomes a session with `[specs] ignored_*` and their checks; running it applies
/// the marks; once the file changes under them they are dropped with `ignored_lines_dropped`.
#[test]
fn ignored_lines_round_trip_through_a_session() {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(dir.path());
    let base = pair(dir.path());
    let request = svc
        .open_view_request(
            ViewKind::TextCompare,
            &[
                base.left.to_string_lossy().into_owned(),
                base.right.to_string_lossy().into_owned(),
            ],
            &OpenOptions::default(),
        )
        .unwrap();
    let ViewRequest::Text {
        request: mut text,
        view,
    } = request
    else {
        panic!("a text view")
    };
    text.ignored = IgnoredLines {
        left: RangeSet::new(),
        right: RangeSet::from_ranges(vec![4..5]),
    };
    text.formats = Pair::new(None, Some("everything-else".into()));
    let response = compare(&svc, &text).unwrap();
    text.ignored_check = response.ignored_check;
    let (session, warnings) = svc
        .session_for_view(
            &ViewRequest::Text {
                request: text.clone(),
                view,
            },
            Table::new(),
            Table::new(),
            None,
        )
        .unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(session.specs.ignored_right, ["5"]);
    assert!(session.specs.ignored_left.is_empty());
    assert_eq!(
        session.specs.ignored_check_right,
        format!("{:08x}", response.ignored_check.right.unwrap())
    );
    assert_eq!(session.specs.right_format, "everything-else");
    svc.session_save(&session_ref("marks"), &session, false)
        .unwrap();
    // The file holds the keys.
    let saved = std::fs::read_to_string(dir.path().join("config/sessions/marks.toml"))
        .or_else(|_| std::fs::read_to_string(svc.session_store().path(&session_ref("marks"))))
        .unwrap();
    assert!(saved.contains("ignored_right = [\"5\"]"), "{saved}");
    assert!(saved.contains("ignored_check_right = "), "{saved}");
    assert!(
        saved.contains("right_format = \"everything-else\""),
        "{saved}"
    );
    assert!(!saved.contains("ignored_left"), "{saved}");

    let run = |svc: &Service| match svc.session_run_request(&session_ref("marks")).unwrap() {
        SessionRunRequest::Text(req) => compare(svc, &req).unwrap(),
        _ => panic!("a text session"),
    };
    let got = run(&svc);
    assert_eq!(got.verdict, ContentResult::RulesSame);
    assert!(got.right_format.is_some());
    assert!(
        got.warnings
            .iter()
            .all(|w| w.code != WarningCode::IgnoredLinesDropped)
    );
    // A change of line endings keeps the marks (the check covers the text only).
    write(
        dir.path(),
        "w/b.txt",
        format!("{C_TEXT}extra\n").replace('\n', "\r\n").as_bytes(),
    );
    let got = run(&svc);
    assert_eq!(got.comparison.stats().ignored_rows, 1);
    // The marked line changed: the right side's marks are dropped with the warning.
    write(dir.path(), "w/b.txt", format!("{C_TEXT}other\n").as_bytes());
    let got = run(&svc);
    let dropped: Vec<_> = got
        .warnings
        .iter()
        .filter(|w| w.code == WarningCode::IgnoredLinesDropped)
        .collect();
    assert_eq!(dropped.len(), 1, "{:?}", got.warnings);
    assert!(
        dropped[0]
            .spec
            .as_deref()
            .is_some_and(|s| s.ends_with("b.txt"))
    );
    assert_eq!(got.comparison.stats().ignored_rows, 0);
    assert_eq!(got.verdict, ContentResult::ImportantDifferences);
    // The file shorter than the marks: dropped too, never an error.
    write(dir.path(), "w/b.txt", C_TEXT.as_bytes());
    let got = run(&svc);
    assert!(
        got.warnings
            .iter()
            .any(|w| w.code == WarningCode::IgnoredLinesDropped)
    );
    // Swap Sides exchanges the keys.
    let mut swapped = session.clone();
    swapped.swap_sides();
    assert_eq!(swapped.specs.ignored_left, ["5"]);
    assert_eq!(swapped.specs.left_format, "everything-else");
    assert!(swapped.specs.ignored_check_right.is_empty());
}

/// `[specs] left_format` through `session run`, and `format` beside it: the side's own wins, the other side takes
/// `format`.
#[test]
fn session_pane_formats() {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(dir.path());
    let base = pair(dir.path());
    let mut session = Session::builtin(SessionType::TextCompare);
    session.specs.left = base.left.to_string_lossy().into_owned();
    session.specs.right = base.right.to_string_lossy().into_owned();
    session.specs.format = "c-cpp".into();
    session.specs.left_format = "java".into();
    svc.session_save(&session_ref("fmt"), &session, false)
        .unwrap();
    let SessionRunRequest::Text(req) = svc.session_run_request(&session_ref("fmt")).unwrap() else {
        panic!("a text session")
    };
    assert_eq!(req.format, None);
    assert_eq!(
        req.formats,
        Pair::new(Some("java".into()), Some("c-cpp".into()))
    );
    let got = compare(&svc, &req).unwrap();
    assert_eq!(got.comparison.format().id().unwrap().as_str(), "java");
    assert_eq!(
        got.comparison
            .right_format()
            .unwrap()
            .id()
            .unwrap()
            .as_str(),
        "c-cpp"
    );
    // `session edit --set` reaches the keys.
    let edited = svc
        .session_edit(
            &hikasa_session::api::SessionTarget::Named(session_ref("fmt")),
            &hikasa_session::api::SessionEdit {
                set: vec![("specs.right_format".into(), "\"python\"".into())],
                ..Default::default()
            },
            false,
        )
        .map(|_| ());
    if let Err(e) = edited {
        panic!("{e:?}");
    }
    let loaded = svc.session_load(&session_ref("fmt")).unwrap().value;
    assert_eq!(loaded.specs.right_format, "python");
}
