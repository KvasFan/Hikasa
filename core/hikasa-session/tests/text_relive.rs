//! A11.10 (D-304; contract §1.11.13): `Service::text_relive` of a live view's edited documents with new rules, a
//! format per side, anchors and ignored lines equals a fresh comparison of the same texts with the same request
//! (`hikasa.text_compare.v1` documents equal, apart from the file names); the documents are not touched (their
//! undo logs, marks and bookmarks stay); an anchor beyond a document is refused; the *identical* verdict follows
//! the files.

#![allow(clippy::single_range_in_vec_init)] // one-range line sets are meant as such

mod common;

use std::path::Path;

use common::{job, os, service, write};
use hikasa_core::{JobKind, Pair};
use hikasa_session::Service;
use hikasa_session::api::{ApiError, LiveOptions, LiveTextView, TextCompareRequest};
use hikasa_text::{
    Anchor, AnchorSide, EditRequest, GroupHint, IgnoredLines, LineIdx, RangeSet, TextPos,
    UnimportantItem, ViewOptions,
};

const LEFT: &str = "int a = 1; // one\nint b = 2;\n/* c */ int c = 3;\nreturn a;\nint d;\nint e;\n";
const RIGHT: &str = "int a = 1; // uno\nint b = 2;\nint c = 3;\nreturn b;\nint d;\nint f;\n";

fn open(svc: &Service, root: &Path) -> (LiveTextView, TextCompareRequest) {
    let a = write(root, "w/a.c", LEFT.as_bytes());
    let b = write(root, "w/b.c", RIGHT.as_bytes());
    let req = TextCompareRequest {
        left: os(&a),
        right: os(&b),
        ..TextCompareRequest::default()
    };
    let view = svc
        .text_compare_live(
            &req,
            LiveOptions::default(),
            &job(svc, JobKind::TextCompare),
            None,
        )
        .unwrap();
    (view, req)
}

fn insert(doc: &mut hikasa_text::EditableDocument, line: u32, byte: u32, text: &str) {
    doc.apply(
        &EditRequest::Replace {
            range: TextPos::new(line, byte)..TextPos::new(line, byte),
            text: text.into(),
            keep_eols: false,
        },
        GroupHint::None,
    )
    .unwrap();
}

/// The JSON document without the specs, sizes and times of the files (the fresh comparison reads other files).
fn json_without_specs(r: &hikasa_session::api::TextCompareResponse) -> serde_json::Value {
    let mut v = serde_json::to_value(r.json(&ViewOptions::default())).unwrap();
    fn strip(v: &mut serde_json::Value) {
        match v {
            serde_json::Value::Object(map) => {
                for key in ["left", "right", "spec", "path"] {
                    if map.get(key).is_some_and(serde_json::Value::is_string) {
                        map.remove(key);
                    }
                }
                map.values_mut().for_each(strip);
            }
            serde_json::Value::Array(items) => items.iter_mut().for_each(strip),
            _ => {}
        }
    }
    strip(&mut v);
    // The files' own facts: the relive's documents were read from the view's files (their size and time).
    for side in ["left", "right"] {
        if let Some(file) = v.get_mut(side).and_then(serde_json::Value::as_object_mut) {
            file.remove("mtime");
            file.remove("size");
        }
    }
    v
}

#[test]
fn a_relive_equals_a_fresh_comparison_of_the_edited_texts() {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(dir.path());
    let (mut view, mut req) = open(&svc, dir.path());
    insert(&mut view.docs.left, 3, 7, "+1");
    insert(&mut view.docs.right, 5, 0, "/* new */\n");
    view.docs.left.toggle_bookmark(2, LineIdx(4));
    let steps = (view.docs.left.undo_steps(), view.docs.right.undo_steps());
    // New rules, a format per side, an anchor and ignored lines.
    req.rules_patch.unimportant = vec![UnimportantItem::Element("Comment".into())];
    req.formats = Pair::new(None, Some("Everything Else".into()));
    req.anchors = vec![Anchor::Pair {
        left: 4..5,
        right: 4..5,
    }];
    req.ignored = IgnoredLines {
        left: RangeSet::from_ranges(vec![0..1]),
        right: RangeSet::new(),
    };
    let relived = svc
        .text_relive_full(
            Pair::new(&view.docs.left, &view.docs.right),
            &req,
            &job(&svc, JobKind::TextCompare),
            None,
        )
        .unwrap();
    // The documents are as they were.
    assert_eq!(
        (view.docs.left.undo_steps(), view.docs.right.undo_steps()),
        steps
    );
    assert_eq!(view.docs.left.bookmark(2), Some(LineIdx(4)));
    // A fresh comparison of the same texts under the same names and the same request.
    let fresh_dir = tempfile::tempdir().unwrap();
    let a = write(
        fresh_dir.path(),
        "w/a.c",
        view.docs.left.document().rope().to_string().as_bytes(),
    );
    let b = write(
        fresh_dir.path(),
        "w/b.c",
        view.docs.right.document().rope().to_string().as_bytes(),
    );
    let fresh_req = TextCompareRequest {
        left: os(&a),
        right: os(&b),
        ..req.clone()
    };
    let fresh = svc
        .text_compare(&fresh_req, &job(&svc, JobKind::TextCompare), None)
        .unwrap();
    assert_eq!(
        json_without_specs(&relived.response),
        json_without_specs(&fresh)
    );
    assert_eq!(
        relived
            .response
            .right_format
            .as_ref()
            .map(|f| f.name.as_str()),
        Some("Everything Else")
    );
    // `text_relive` is the same comparison.
    let live = svc
        .text_relive(
            Pair::new(&view.docs.left, &view.docs.right),
            &req,
            &job(&svc, JobKind::TextCompare),
            None,
        )
        .unwrap();
    assert_eq!(
        live.current().stats(),
        relived.live.current().stats(),
        "both entry points compare alike"
    );
    live.check_invariants().unwrap();
}

#[test]
fn identical_files_stay_identical_until_an_edit() {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(dir.path());
    let a = write(dir.path(), "w/a.c", LEFT.as_bytes());
    let b = write(dir.path(), "w/b.c", LEFT.as_bytes());
    let req = TextCompareRequest {
        left: os(&a),
        right: os(&b),
        ..TextCompareRequest::default()
    };
    let jc = || job(&svc, JobKind::TextCompare);
    let mut view = svc
        .text_compare_live(&req, LiveOptions::default(), &jc(), None)
        .unwrap();
    let opened = view.response.verdict;
    let again = svc
        .text_relive_full(
            Pair::new(&view.docs.left, &view.docs.right),
            &req,
            &jc(),
            None,
        )
        .unwrap();
    assert_eq!(again.response.verdict, opened);
    insert(&mut view.docs.left, 0, 0, " ");
    view.docs.left.undo();
    // Undone: unmodified again, the same file.
    let undone = svc
        .text_relive_full(
            Pair::new(&view.docs.left, &view.docs.right),
            &req,
            &jc(),
            None,
        )
        .unwrap();
    assert_eq!(undone.response.verdict, opened);
    insert(&mut view.docs.left, 0, 0, "x");
    let edited = svc
        .text_relive_full(
            Pair::new(&view.docs.left, &view.docs.right),
            &req,
            &jc(),
            None,
        )
        .unwrap();
    assert_ne!(edited.response.verdict, opened);
}

#[test]
fn anchors_beyond_the_documents_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let svc = service(dir.path());
    let (view, mut req) = open(&svc, dir.path());
    req.anchors = vec![Anchor::Isolate {
        side: AnchorSide::Right,
        lines: 3..40,
    }];
    let err = svc
        .text_relive(
            Pair::new(&view.docs.left, &view.docs.right),
            &req,
            &job(&svc, JobKind::TextCompare),
            None,
        )
        .unwrap_err();
    assert!(matches!(err, ApiError::InvalidArgument(_)), "{err}");
}
