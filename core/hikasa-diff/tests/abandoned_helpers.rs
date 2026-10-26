//! The cap on abandoned helper threads (D-54 as amended in wave 2). imara-diff cannot be interrupted, so a
//! cancelled call abandons its helper thread, which keeps computing until its call ends. At most two abandoned
//! helpers may run: a further helper call waits for one of them to end instead of starting a third, and it stays
//! cancellable within 100 ms while it waits.
//!
//! This test has its own binary because the count is process-wide. Its input keeps imara-diff busy for seconds
//! (release) to minutes (debug), so the abandoned helpers outlive the test; they end with the process.

#[path = "../benches/support/inputs.rs"]
mod inputs;

use std::thread;
use std::time::{Duration, Instant};

use hikasa_core::CancelToken;
use hikasa_diff::testing::abandoned_helpers;
use hikasa_diff::{Algorithm, AlignInput, AlignOptions, DiffError, TokenId, TrivialSet, align};
use inputs::XorShift;

const LATENCY: Duration = Duration::from_millis(100);

/// Live threads named `hikasa-imara` (Linux only: elsewhere, e.g. a Windows build run under a compatibility
/// layer on Linux, `/proc/self/task` may resolve to a host directory whose thread names are not ours).
fn imara_threads() -> Option<usize> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let tasks = std::fs::read_dir("/proc/self/task").ok()?;
    Some(
        tasks
            .flatten()
            .filter(|task| {
                std::fs::read_to_string(task.path().join("comm"))
                    .is_ok_and(|name| name.trim_end() == "hikasa-imara")
            })
            .count(),
    )
}

#[test]
fn at_most_two_abandoned_helpers_keep_running() {
    // Histogram on 200,000 random tokens per side over 50 symbols: imara-diff's quadratic worst case.
    let mut rng = XorShift(0x5EED_00AB);
    let mut side = || -> Vec<TokenId> {
        (0..200_000)
            .map(|_| TokenId(rng.below(50) as u32))
            .collect()
    };
    let (left, right) = (side(), side());
    let trivial = TrivialSet::default();
    let input = AlignInput {
        left: &left,
        right: &right,
        left_weights: &[],
        right_weights: &[],
        trivial: &trivial,
        distinct: 50,
    };
    let opts = AlignOptions {
        closeness: None,
        ..AlignOptions::with_algorithm(Algorithm::Histogram)
    };
    // Starts a call, cancels it after `after` (while imara-diff runs, or while the call waits for a slot), and
    // returns the cancellation latency.
    let cancel_after = |after: Duration, check: &dyn Fn()| -> Duration {
        let cancel = CancelToken::new();
        thread::scope(|scope| {
            let worker = scope.spawn(|| align(&input, &opts, None, None, &cancel).map(|_| ()));
            thread::sleep(after);
            assert!(
                !worker.is_finished(),
                "the call finished before it was cancelled"
            );
            check();
            let start = Instant::now();
            cancel.cancel();
            let result = worker.join().unwrap();
            let latency = start.elapsed();
            assert_eq!(result, Err(DiffError::Cancelled));
            latency
        })
    };
    assert_eq!(abandoned_helpers(), 0);
    for abandoned in 1..=2 {
        let latency = cancel_after(Duration::from_millis(300), &|| {});
        assert!(latency <= LATENCY, "helper call {abandoned}: {latency:?}");
        assert_eq!(abandoned_helpers(), abandoned);
    }
    // A third call waits for a slot: no third helper starts, and cancelling the wait is immediate.
    let latency = cancel_after(Duration::from_millis(300), &|| {
        assert_eq!(abandoned_helpers(), 2);
        if let Some(threads) = imara_threads() {
            assert_eq!(threads, 2, "a third helper thread started");
        }
    });
    assert!(latency <= LATENCY, "waiting call: {latency:?}");
    assert_eq!(abandoned_helpers(), 2);
}
