//! The global-pool trap of hikasa-session (§5.1, T7): this test binary builds the rayon global pool with
//! recognisable thread names, then runs the service's parallel operations and records the threads that run the
//! injected callbacks (conversion runner, progress sink, scan observer, comparison observer). No callback may run
//! on a global-pool thread; the service's own `hikasa-worker-N` threads must appear (the recording works).
//! (rayon starts the global pool's threads when it is built, so a panicking `start_handler` cannot see later
//! implicit use; the formats crate uses the same technique, §1.4.8 item 12.)

mod common;

use std::sync::Arc;

use common::{UpperRunner, os, write};
use hikasa_core::{JobKind, Progress, ProgressSink};
use hikasa_folder::{ScanEvent, ScanObserver};
use hikasa_session::api::{
    FolderCompareRequest, FormatTokensRequest, QuickMethod, QuickRequest, TextCompareRequest,
};
use parking_lot::Mutex;

#[derive(Default)]
struct Threads(Mutex<Vec<String>>);

impl Threads {
    fn record(&self) {
        let name = std::thread::current()
            .name()
            .unwrap_or("<unnamed>")
            .to_owned();
        self.0.lock().push(name);
    }
}

impl ProgressSink for Threads {
    fn report(&self, _progress: Progress) {
        self.record();
    }
}

impl ScanObserver for Threads {
    fn on_event(&self, _event: ScanEvent) {
        self.record();
    }
}

#[test]
fn no_work_runs_on_the_rayon_global_pool() {
    rayon::ThreadPoolBuilder::new()
        .num_threads(2)
        .thread_name(|i| format!("global-trap-{i}"))
        .build_global()
        .unwrap();
    let tmp = tempfile::tempdir().unwrap();
    write(
        &tmp.path().join("config"),
        "formats/upper.toml",
        common::EXTERNAL_FORMAT.as_bytes(),
    );
    let runner = Arc::new(UpperRunner::default());
    let service = common::service_with(tmp.path(), None, runner.clone());
    let threads = Arc::new(Threads::default());
    let sink: Arc<dyn ProgressSink> = threads.clone();
    let w = tmp.path().join("work");
    // Large enough for parallel key building and loading on the pool.
    let text: String = (0..60_000)
        .map(|k| format!("int value_{k} = {k}; /* c */\n"))
        .collect();
    write(&w, "l/big.c", text.as_bytes());
    write(&w, "r/big.c", text.replace("value_7", "VALUE_7").as_bytes());
    for k in 0..40 {
        write(&w, &format!("l/f{k}.up"), format!("abc {k}\n").as_bytes());
        write(&w, &format!("r/f{k}.up"), format!("ABC {k}\n").as_bytes());
    }

    let resp = service
        .text_compare(
            &TextCompareRequest {
                left: os(&w.join("l/big.c")),
                right: os(&w.join("r/big.c")),
                ..TextCompareRequest::default()
            },
            &service.job(JobKind::TextCompare, sink.clone()),
            None,
        )
        .unwrap();
    assert_eq!(resp.exit_code, hikasa_core::ExitCode::RulesDifferent);
    let folder = service
        .folder_compare(
            &FolderCompareRequest {
                left: os(&w.join("l")),
                right: os(&w.join("r")),
                criteria: Some("size rules-based".into()),
                ..FolderCompareRequest::default()
            },
            &service.job(JobKind::FolderCompare, sink.clone()),
            &*threads,
        )
        .unwrap();
    assert!(folder.comparison.is_complete());
    service
        .format_tokens(
            &FormatTokensRequest {
                file: os(&w.join("l/big.c")),
                lines: Some(1..=3),
                ..FormatTokensRequest::default()
            },
            &service.job(JobKind::FormatTokens, sink.clone()),
        )
        .unwrap();
    service
        .quick(
            &QuickRequest {
                left: os(&w.join("l/big.c")),
                right: os(&w.join("r/big.c")),
                method: QuickMethod::Crc,
                ..QuickRequest::default()
            },
            &service.job(JobKind::QuickCompare, sink.clone()),
        )
        .unwrap();
    let handle = service.spawn(JobKind::TextCompare, "job".into(), {
        let w = w.clone();
        move |service, job| {
            service.text_compare(
                &TextCompareRequest {
                    left: os(&w.join("l/f1.up")),
                    right: os(&w.join("r/f1.up")),
                    ..TextCompareRequest::default()
                },
                job,
                None,
            )
        }
    });
    handle.wait().unwrap();

    let mut seen: Vec<String> = threads.0.lock().clone();
    seen.extend(runner.threads.lock().iter().cloned());
    assert!(
        seen.iter().all(|name| !name.starts_with("global-trap-")),
        "work ran on the global pool: {seen:?}"
    );
    assert!(
        seen.iter().any(|name| name.starts_with("hikasa-worker-")),
        "the recording sees the service pool: {seen:?}"
    );
}
