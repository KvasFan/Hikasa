//! Jobs (§1.8.3, §5.1, §5.6; §6.2.8 "Jobs"): `Service::spawn` returns results, drops progress when the receiver
//! is slow, turns a panic into `ApiError::Internal`, cancels through `JobHandle::cancel`; `run_guarded`.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use hikasa_core::{ExitCode, JobKind, Phase, Progress, Unit};
use hikasa_session::api::ApiOp;
use hikasa_session::{ApiError, run_guarded};

#[test]
fn spawn_returns_the_result_on_its_own_named_thread() {
    let tmp = tempfile::tempdir().unwrap();
    let service = common::service(tmp.path());
    let handle = service.spawn(JobKind::TextCompare, "names".into(), |_service, job| {
        let thread = std::thread::current().name().unwrap_or("").to_owned();
        Ok((job.id, job.kind, thread))
    });
    assert_eq!(handle.kind(), JobKind::TextCompare);
    assert_eq!(handle.title(), "names");
    let id = handle.id();
    let (job_id, kind, thread) = handle.wait().unwrap();
    assert_eq!(job_id, id);
    assert_eq!(kind, JobKind::TextCompare);
    // Wave-4 review (consumer finding "a job pins a worker"): the body runs on its own `hikasa-job-<id>`
    // thread and enters the pool only through the engines, so long jobs never starve a small one.
    assert_eq!(thread, format!("hikasa-job-{id}"));
    // Job ids increase.
    let next = service.spawn(JobKind::Report, "n".into(), |_, job| Ok(job.id));
    assert!(next.wait().unwrap() > id);
}

#[test]
fn try_result_yields_at_most_once() {
    let tmp = tempfile::tempdir().unwrap();
    let service = common::service(tmp.path());
    let (go_tx, go_rx) = crossbeam_channel::bounded::<()>(1);
    let handle = service.spawn(JobKind::Report, "once".into(), move |_, _| {
        go_rx
            .recv()
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        Ok(42)
    });
    assert!(handle.try_result().is_none(), "not finished yet");
    go_tx.send(()).unwrap();
    let mut result = None;
    for _ in 0..500 {
        if let Some(r) = handle.try_result() {
            result = Some(r);
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(result.unwrap().unwrap(), 42);
    assert!(handle.try_result().is_none(), "Some at most once");
    assert!(handle.is_finished());
    // Waiting after the result was taken is a caller error, not a fake internal error (wave-4 review).
    let err = handle.wait().unwrap_err();
    assert!(matches!(err, ApiError::InvalidArgument(_)), "{err:?}");
    assert!(err.to_string().contains("already taken"), "{err}");
}

#[test]
fn slow_receivers_lose_progress_but_never_block_the_job() {
    let tmp = tempfile::tempdir().unwrap();
    let service = common::service(tmp.path());
    let handle = service.spawn(JobKind::FolderCompare, "noisy".into(), |_, job| {
        let ctl = job.ctl();
        for k in 0..1000u64 {
            // Completions always pass the throttle (§5.4), so all 1000 reach the lossy channel.
            ctl.progress_now(Progress {
                phase: Phase::Scanning,
                done: k,
                total: Some(k),
                unit: Unit::Folders,
                current: None,
            });
        }
        Ok(())
    });
    let progress = handle.progress().clone();
    handle.wait().unwrap();
    let received: Vec<Progress> = progress.try_iter().collect();
    assert!(!received.is_empty());
    assert!(received.len() <= 64, "{} reports kept", received.len());
    assert_eq!(
        received[0].done, 0,
        "the first reports are kept, later ones dropped"
    );
}

#[test]
fn panics_become_internal_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let service = common::service(tmp.path());
    let handle = service.spawn(
        JobKind::Report,
        "boom".into(),
        |_, _| -> Result<(), ApiError> { panic!("kaboom {}", 7) },
    );
    let err = handle.wait().unwrap_err();
    assert!(
        matches!(&err, ApiError::Internal(m) if m == "panic: kaboom 7"),
        "{err:?}"
    );
    assert_eq!(err.exit_code(ApiOp::Other), ExitCode::UnknownError);
    // A panic on a rayon worker inside the job propagates to the job and is caught there.
    let handle = service.spawn(JobKind::Report, "nested".into(), |service, _| {
        service.pool().join(|| panic!("worker panic"), || 1);
        Ok(())
    });
    assert!(matches!(handle.wait(), Err(ApiError::Internal(m)) if m.contains("worker panic")));
    // run_guarded directly.
    let caught = run_guarded(|| -> Result<(), ApiError> { panic!("{}", String::from("owned")) });
    assert!(matches!(caught, Err(ApiError::Internal(m)) if m == "panic: owned"));
    assert_eq!(run_guarded(|| Ok(5)).unwrap(), 5);
}

#[test]
fn cancel_through_the_handle() {
    let tmp = tempfile::tempdir().unwrap();
    let service = common::service(tmp.path());
    let started = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&started);
    let handle = service.spawn(JobKind::FolderCompare, "endless".into(), move |_, job| {
        flag.store(true, Ordering::SeqCst);
        loop {
            job.cancel.check().map_err(|_| ApiError::Cancelled)?;
            std::thread::yield_now();
        }
        #[allow(unreachable_code)]
        Ok(())
    });
    while !started.load(Ordering::SeqCst) {
        std::thread::yield_now();
    }
    handle.cancel();
    assert!(matches!(handle.wait(), Err(ApiError::Cancelled)));
}

#[test]
fn a_cancelled_text_compare_job_returns_cancelled() {
    let tmp = tempfile::tempdir().unwrap();
    let service = common::service(tmp.path());
    let work = tmp.path().join("work");
    let big: String = (0..200_000).map(|k| format!("line {k}\n")).collect();
    common::write(&work, "a.txt", big.as_bytes());
    common::write(&work, "b.txt", big.replace("line 1", "LINE 1").as_bytes());
    let req = hikasa_session::api::TextCompareRequest {
        left: work.join("a.txt").into_os_string(),
        right: work.join("b.txt").into_os_string(),
        ..Default::default()
    };
    let handle = service.spawn(
        JobKind::TextCompare,
        "cancelled".into(),
        move |service, job| {
            job.cancel.cancel();
            service.text_compare(&req, job, None)
        },
    );
    assert!(matches!(handle.wait(), Err(ApiError::Cancelled)));
}

#[test]
fn shared_types_are_send_and_sync() {
    fn send_sync<T: Send + Sync>() {}
    fn send<T: Send>() {}
    send_sync::<hikasa_session::Service>();
    send_sync::<hikasa_session::JobControl>();
    send::<hikasa_session::JobHandle<u32>>();
    send_sync::<hikasa_session::api::PreparedFolderCompare>();
    send_sync::<hikasa_session::api::TextCompareResponse>();
    send_sync::<hikasa_session::api::FolderCompareResponse>();
    send_sync::<hikasa_session::ApiError>();
    send_sync::<hikasa_session::ipc::ViewWaiter>();
    send_sync::<hikasa_session::ipc::ServerHandle>();
    send::<hikasa_session::ipc::IpcClient>();
    send::<hikasa_session::ipc::IpcServer>();
}

#[test]
fn direct_job_controls_throttle_progress() {
    let tmp = tempfile::tempdir().unwrap();
    let service = common::service(tmp.path());
    let seen = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = Arc::clone(&seen);
    let sink: Arc<dyn hikasa_core::ProgressSink> = Arc::new(move |_p: Progress| {
        counter.fetch_add(1, Ordering::SeqCst);
    });
    let job = service.job(JobKind::VfsRead, sink);
    let ctl = job.ctl();
    for k in 0..10_000u64 {
        ctl.progress(Phase::Reading, || Progress {
            phase: Phase::Reading,
            done: k,
            total: None,
            unit: Unit::Bytes,
            current: None,
        });
    }
    // The first report of the phase passes; the rest fall inside the 50 ms throttle (§5.4).
    assert!(
        seen.load(Ordering::SeqCst) < 100,
        "{}",
        seen.load(Ordering::SeqCst)
    );
    assert!(seen.load(Ordering::SeqCst) >= 1);
    assert!(!job.cancel.is_cancelled());
    let other = service.job(JobKind::VfsRead, Arc::new(hikasa_core::NullProgress));
    assert!(other.id > job.id);
}
