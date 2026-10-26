//! The panic boundary (§5.6; §6.2.8): a comparator that panics yields `ApiError::Internal` and exit code 100 —
//! through `quick --rules`, `text compare` and the injected `RulesComparator` of a folder comparison (whose panic
//! happens on a pool thread). The panicking piece is the injected conversion runner of an External format.

mod common;

use std::sync::Arc;

use common::{PanicRunner, job, os, write};
use hikasa_core::{ExitCode, JobKind};
use hikasa_folder::NullObserver;
use hikasa_session::ApiError;
use hikasa_session::api::{ApiOp, FolderCompareRequest, QuickRequest, TextCompareRequest};

fn setup() -> (tempfile::TempDir, hikasa_session::Service) {
    let tmp = tempfile::tempdir().unwrap();
    write(
        &tmp.path().join("config"),
        "formats/upper.toml",
        common::EXTERNAL_FORMAT.as_bytes(),
    );
    let w = tmp.path().join("work");
    write(&w, "l/a.up", b"abc\n");
    write(&w, "r/a.up", b"abd\n");
    let service = common::service_with(tmp.path(), None, Arc::new(PanicRunner));
    (tmp, service)
}

fn assert_internal(err: &ApiError, op: ApiOp) {
    assert!(
        matches!(err, ApiError::Internal(m) if m.starts_with("panic: ") && m.contains("exploded")),
        "{err:?}"
    );
    assert_eq!(err.exit_code(op), ExitCode::UnknownError);
}

#[test]
fn quick_rules_panic_is_internal() {
    let (tmp, service) = setup();
    let w = tmp.path().join("work");
    let err = service
        .quick(
            &QuickRequest {
                left: os(&w.join("l/a.up")),
                right: os(&w.join("r/a.up")),
                ..QuickRequest::default()
            },
            &job(&service, JobKind::QuickCompare),
        )
        .unwrap_err();
    assert_internal(&err, ApiOp::Quick);
}

#[test]
fn text_compare_panic_is_internal() {
    let (tmp, service) = setup();
    let w = tmp.path().join("work");
    let err = service
        .text_compare(
            &TextCompareRequest {
                left: os(&w.join("l/a.up")),
                right: os(&w.join("r/a.up")),
                ..TextCompareRequest::default()
            },
            &job(&service, JobKind::TextCompare),
            None,
        )
        .unwrap_err();
    assert_internal(&err, ApiOp::TextCompare);
}

#[test]
fn folder_rules_comparator_panic_is_internal() {
    let (tmp, service) = setup();
    let w = tmp.path().join("work");
    let req = FolderCompareRequest {
        left: os(&w.join("l")),
        right: os(&w.join("r")),
        criteria: Some("rules-based".into()),
        ..FolderCompareRequest::default()
    };
    let prepared = service.folder_prepare(&req).unwrap();
    let err = service
        .folder_run(
            &prepared,
            &job(&service, JobKind::FolderCompare),
            &NullObserver,
        )
        .unwrap_err();
    assert_internal(&err, ApiOp::FolderCompare);
    assert!(
        prepared.model().is_finished(),
        "the model is finished on unwinding (§1.6.8 item 20)"
    );
    // The service stays usable after a caught panic.
    let again = service.folder_compare(
        &FolderCompareRequest {
            criteria: Some("size".into()),
            ..req
        },
        &job(&service, JobKind::FolderCompare),
        &NullObserver,
    );
    assert!(again.is_ok(), "{again:?}");
}

#[test]
fn spawned_job_panic_is_internal() {
    let (tmp, service) = setup();
    let w = tmp.path().join("work");
    let req = QuickRequest {
        left: os(&w.join("l/a.up")),
        right: os(&w.join("r/a.up")),
        ..QuickRequest::default()
    };
    let handle = service.spawn(
        JobKind::QuickCompare,
        "quick".into(),
        move |service, job| service.quick(&req, job),
    );
    assert_internal(&handle.wait().unwrap_err(), ApiOp::Quick);
}
