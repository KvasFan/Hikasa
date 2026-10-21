//! Global-pool trap (§5.1, T7): this test binary configures the rayon global pool with recognisable thread
//! names, then runs `compare` with explicit pools on inputs large enough for parallel key building, line
//! weights, the pattern runtime-limit pass and row classification.
//!
//! Two traps: (1) every thread of the global pool is kept busy for the whole test, so any implicit use of the
//! global pool — by any pass, whether or not it reports progress (the line-weights pass does not) — blocks, and
//! a watchdog fails the test; (2) a progress sink records the thread of every report (key building and
//! classification report from the worker that did the work), and a report from a global-pool thread fails the
//! test.

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{doc, format, rules_with};
use hikasa_core::{CancelToken, Ctl, Progress, ProgressSink};
use hikasa_text::{AlgorithmChoice, CompareInput, TextPattern, TextRules, compare};

#[derive(Default)]
struct Threads(Mutex<Vec<String>>);

impl ProgressSink for Threads {
    fn report(&self, _progress: Progress) {
        let name = std::thread::current().name().unwrap_or("").to_owned();
        self.0.lock().unwrap().push(name);
    }
}

/// Global-pool threads.
const GLOBAL_THREADS: usize = 2;

#[test]
fn compare_uses_only_the_pool_it_is_given() {
    rayon::ThreadPoolBuilder::new()
        .num_threads(GLOBAL_THREADS)
        .thread_name(|i| format!("global-trap-{i}"))
        .build_global()
        .unwrap();
    // Trap 1: occupy every global-pool thread until the test ends.
    let release = Arc::new(AtomicBool::new(false));
    let (busy_tx, busy_rx) = mpsc::channel();
    for _ in 0..GLOBAL_THREADS {
        let release = Arc::clone(&release);
        let busy = busy_tx.clone();
        rayon::spawn(move || {
            busy.send(()).unwrap();
            while !release.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(1));
            }
        });
    }
    for _ in 0..GLOBAL_THREADS {
        busy_rx.recv().unwrap();
    }

    let left: String = (0..40_000)
        .map(|k| format!("procedure P{k}; begin x := {k}; end; {{ c }}\n"))
        .collect();
    let right: String = (0..40_000)
        .map(|k| {
            if k % 97 == 0 {
                format!("procedure P{k}; begin x := {}; end;  {{ C }}\n", k + 1)
            } else {
                format!("procedure P{k}; begin x := {k}; end; {{ c }}\n")
            }
        })
        .collect();
    let pool = Arc::new(
        rayon::ThreadPoolBuilder::new()
            .num_threads(4)
            .thread_name(|i| format!("explicit-{i}"))
            .build()
            .unwrap(),
    );
    let mut ws_off = rules_with(AlgorithmChoice::Standard);
    ws_off.importance.trailing_whitespace = false;
    // A fancy-regex pattern (backreference) runs the runtime-limit pass over every line.
    let mut fancy = rules_with(AlgorithmChoice::Standard);
    fancy
        .importance
        .unimportant
        .push(TextPattern::regex(r"(zz)\1"));
    let cases = [
        ("pascal", rules_with(AlgorithmChoice::Standard)),
        ("pascal", ws_off),
        ("pascal", fancy),
        ("ee", TextRules::default()),
    ];
    for (fmt, rules) in cases {
        let input = CompareInput {
            left: doc(&left),
            right: doc(&right),
            format: format(fmt),
            rules,
            bytes_identical: false,
            force_tokenize: false,
        };
        let pool = Arc::clone(&pool);
        let (done_tx, done_rx) = mpsc::channel();
        std::thread::spawn(move || {
            let threads = Threads::default();
            let cancel = CancelToken::never();
            let ctl = Ctl {
                cancel: &cancel,
                progress: &threads,
            };
            let changed = compare(input, Some(&pool), &ctl, None)
                .unwrap()
                .stats()
                .changed_rows;
            let _ = done_tx.send((changed, threads.0.into_inner().unwrap()));
        });
        let Ok((changed, names)) = done_rx.recv_timeout(Duration::from_secs(300)) else {
            release.store(true, Ordering::Relaxed);
            panic!("{fmt}: compare blocked on the busy global pool (an implicit global-pool use)");
        };
        assert!(changed > 0);
        assert!(
            names.iter().all(|n| !n.starts_with("global-trap")),
            "{fmt}: {names:?}"
        );
        assert!(
            names.iter().any(|n| n.starts_with("explicit-")),
            "{fmt}: the explicit pool did the work"
        );
    }
    release.store(true, Ordering::Relaxed);
}
