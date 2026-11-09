//! `Service::text_convert` in one call (M2 W10b, contract §1.10.15, §1.10.23): the request's answers decide the
//! questions (open ones refuse the request, nothing written), standard output goes to the caller's sink, a cancel
//! leaves the files not reached `not_run` (exit 100). The CLI's runs are in `cli/hikasa-cli/tests/cli_text_convert.rs`.

mod common;

use common::{job, os, service, write};
use hikasa_core::{ExitCode, JobKind};
use hikasa_session::ApiError;
use hikasa_session::api::{ConvertTarget, SaveAnswers, TextConvertRequest};
use hikasa_vfs::ops::ItemStatus;

#[test]
fn the_one_call_answers_by_the_request_and_refuses_open_questions() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let a = write(&w, "a.txt", b"a \n");
    let out = write(&w, "out.txt", b"old\n");
    let mut req = TextConvertRequest {
        files: vec![os(&a)],
        trim_trailing: true,
        target: ConvertTarget::Output(os(&out)),
        ..TextConvertRequest::default()
    };
    let job = job(&service, JobKind::TextConvert);
    // `replace_existing` is open: refused, nothing written.
    let err = service
        .text_convert(&req, &mut Vec::new(), &job)
        .unwrap_err();
    assert!(matches!(err, ApiError::InvalidArgument(_)), "{err:?}");
    assert!(err.to_string().contains("replace_existing 1"), "{err}");
    assert_eq!(std::fs::read(&out).unwrap(), b"old\n");
    // `--yes` answers it.
    req.answers = SaveAnswers {
        yes: true,
        ..SaveAnswers::default()
    };
    let outcome = service.text_convert(&req, &mut Vec::new(), &job).unwrap();
    assert_eq!(outcome.exit_code, ExitCode::Success);
    assert_eq!(outcome.dto.files[0].status, ItemStatus::Done);
    assert_eq!(std::fs::read(&out).unwrap(), b"a\n");
    // Standard output: the caller's sink.
    req.target = ConvertTarget::Stdout;
    let mut sink = Vec::new();
    let outcome = service.text_convert(&req, &mut sink, &job).unwrap();
    assert_eq!(sink, b"a\n");
    assert_eq!(outcome.dto.target, "stdout");
    assert!(outcome.dto.plan.is_empty());
    assert_eq!(std::fs::read(&a).unwrap(), b"a \n");
    // A dry run writes nothing, also to the sink.
    req.dry_run = true;
    let mut sink = Vec::new();
    let outcome = service.text_convert(&req, &mut sink, &job).unwrap();
    assert!(sink.is_empty());
    assert_eq!(outcome.dto.files[0].status, ItemStatus::Planned);
    assert_eq!(
        outcome.human_lines(),
        vec![format!(
            "would convert 1 lines: {}",
            service.parse_spec(a.as_os_str()).unwrap().canonical()
        )]
    );
}

#[test]
fn a_cancelled_conversion_is_exit_100() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let a = write(&w, "a.txt", b"a \n");
    let b = write(&w, "b.txt", b"b \n");
    let req = TextConvertRequest {
        files: vec![os(&a), os(&b)],
        trim_trailing: true,
        target: ConvertTarget::InPlace,
        ..TextConvertRequest::default()
    };
    let job = job(&service, JobKind::TextConvert);
    let plan = service.text_convert_plan(&req, &job).unwrap();
    job.cancel.cancel();
    let outcome = service
        .text_convert_execute(plan, &mut Vec::new(), &job)
        .unwrap();
    assert_eq!(outcome.exit_code, ExitCode::UnknownError);
    assert!(
        outcome
            .dto
            .files
            .iter()
            .all(|f| f.status == ItemStatus::NotRun),
        "{:?}",
        outcome.dto.files
    );
    assert_eq!(outcome.dto.summary.not_run, 2);
    assert_eq!(std::fs::read(&a).unwrap(), b"a \n");
    // A cancel before the plan is `Cancelled`.
    assert!(matches!(
        service.text_convert_plan(&req, &job),
        Err(ApiError::Cancelled)
    ));
}

/// W10b note 64: standard output holds the converted bytes (up to `STDOUT_HELD`) until the source's check at the end
/// of its read, so a file that changed between the plan and the execution writes nothing to the sink (exit 100).
#[test]
fn standard_output_holds_the_bytes_until_the_source_is_checked() {
    let tmp = tempfile::tempdir().unwrap();
    let service = service(tmp.path());
    let w = tmp.path().join("work");
    let a = write(&w, "a.txt", b"one \ntwo\nthree\n");
    let req = TextConvertRequest {
        files: vec![os(&a)],
        trim_trailing: true,
        target: ConvertTarget::Stdout,
        ..TextConvertRequest::default()
    };
    let job = job(&service, JobKind::TextConvert);
    let plan = service.text_convert_plan(&req, &job).unwrap();
    // The same length, other bytes: only the CRC at the end of the read finds it.
    std::fs::write(&a, b"one \ntwo\nthreE\n").unwrap();
    let mut sink = Vec::new();
    let outcome = service.text_convert_execute(plan, &mut sink, &job).unwrap();
    assert_eq!(outcome.exit_code, ExitCode::UnknownError);
    assert_eq!(outcome.dto.files[0].status, ItemStatus::Failed);
    assert!(sink.is_empty(), "{:?}", String::from_utf8_lossy(&sink));
    // Another length: the same.
    std::fs::write(&a, b"one \ntwo\nthree\n").unwrap();
    let plan = service.text_convert_plan(&req, &job).unwrap();
    std::fs::write(&a, b"other\n").unwrap();
    let mut sink = Vec::new();
    let outcome = service.text_convert_execute(plan, &mut sink, &job).unwrap();
    assert_eq!(outcome.exit_code, ExitCode::UnknownError);
    assert!(sink.is_empty());
    // Unchanged: the converted bytes.
    std::fs::write(&a, b"one \ntwo\nthree\n").unwrap();
    let plan = service.text_convert_plan(&req, &job).unwrap();
    let mut sink = Vec::new();
    let outcome = service.text_convert_execute(plan, &mut sink, &job).unwrap();
    assert_eq!(outcome.exit_code, ExitCode::Success);
    assert_eq!(sink, b"one\ntwo\nthree\n");
}
