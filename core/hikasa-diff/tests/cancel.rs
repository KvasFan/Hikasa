//! Cancellation latency (§5.3, gate G4, diff part; D-54): a large imara-diff call runs on a helper thread and
//! `align` returns `Err(Cancelled)` within 100 ms of `cancel()`, without waiting for the call; long Standard and
//! closeness work observes cancellation at its check points too.
//!
//! The inputs are slow on purpose (imara-diff on random repetitive data is roughly quadratic: seconds per call),
//! so each test cancels well before the work could finish. Scheduling noise on a loaded machine only delays the
//! measured return, so each measurement takes the best of three attempts. Abandoned helper threads end with this
//! test process; once two of them run, further helper calls wait for a slot (still cancellable within 100 ms,
//! D-54), so the later attempts measure that wait. `tests/abandoned_helpers.rs` checks the cap itself.

#[path = "../benches/support/inputs.rs"]
mod inputs;

use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use hikasa_core::CancelToken;
use hikasa_diff::testing::{HELPER_THRESHOLD_TOKENS, align_with_helper_threshold};
use hikasa_diff::{
    Algorithm, AlignInput, AlignOptions, Closeness, DiffError, PairScorer, TokenId, TrivialSet,
    align,
};
use inputs::{XorShift, pathological};

const LATENCY: Duration = Duration::from_millis(100);

/// Random sequences of `n` tokens over `alphabet` symbols.
fn random(n: usize, alphabet: u64, seed: u64) -> (Vec<TokenId>, Vec<TokenId>) {
    let mut rng = XorShift(seed);
    let left = (0..n)
        .map(|_| TokenId(rng.below(alphabet) as u32))
        .collect();
    let right = (0..n)
        .map(|_| TokenId(rng.below(alphabet) as u32))
        .collect();
    (left, right)
}

/// Runs `job` on a thread, cancels after `after`, and returns the latency from `cancel()` to the job's return
/// (best of three attempts). Every attempt must end in `Err(Cancelled)`.
fn cancel_latency(
    after: Duration,
    job: impl Fn(&CancelToken) -> Result<(), DiffError> + Sync,
) -> Duration {
    (0..3)
        .map(|_| {
            let cancel = CancelToken::new();
            thread::scope(|scope| {
                let worker = scope.spawn(|| job(&cancel));
                thread::sleep(after);
                assert!(
                    !worker.is_finished(),
                    "the work finished before it was cancelled"
                );
                let start = Instant::now();
                cancel.cancel();
                let result = worker.join().unwrap();
                let latency = start.elapsed();
                assert_eq!(result, Err(DiffError::Cancelled));
                latency
            })
        })
        .min()
        .unwrap()
}

/// G4 (diff): a call on 300,000 tokens per side (above any helper threshold) is abandoned within 100 ms.
#[test]
fn a_large_imara_call_is_abandoned_within_100_ms() {
    let (left, right) = random(300_000, 1000, 11);
    let trivial = TrivialSet::default();
    let input = AlignInput {
        left: &left,
        right: &right,
        left_weights: &[],
        right_weights: &[],
        trivial: &trivial,
        distinct: 1000,
    };
    for algorithm in [Algorithm::Myers, Algorithm::Histogram] {
        let opts = AlignOptions {
            closeness: None,
            ..AlignOptions::with_algorithm(algorithm)
        };
        let latency = cancel_latency(Duration::from_millis(300), |cancel| {
            align(&input, &opts, None, None, cancel).map(|_| ())
        });
        assert!(
            latency <= LATENCY,
            "{algorithm}: cancellation took {latency:?}"
        );
    }
}

/// The test-only option of §6.2.3: with the threshold lowered, a mid-sized call also runs on the helper
/// thread and is abandoned within 100 ms; with the threshold above the input size, the same call would run
/// inline (and could not be interrupted), which is why the production threshold is low.
#[test]
fn a_lowered_threshold_moves_mid_sized_calls_to_the_helper() {
    const { assert!(HELPER_THRESHOLD_TOKENS <= 4096) };
    let (left, right) = random(60_000, 4, 12);
    let trivial = TrivialSet::default();
    let input = AlignInput {
        left: &left,
        right: &right,
        left_weights: &[],
        right_weights: &[],
        trivial: &trivial,
        distinct: 4,
    };
    let opts = AlignOptions {
        closeness: None,
        ..AlignOptions::with_algorithm(Algorithm::Myers)
    };
    let latency = cancel_latency(Duration::from_millis(150), |cancel| {
        align_with_helper_threshold(&input, &opts, None, None, cancel, 1000).map(|_| ())
    });
    assert!(latency <= LATENCY, "cancellation took {latency:?}");
}

/// Standard's own loops check cancellation every 65,536 operations.
#[test]
fn standard_observes_cancellation_within_100_ms() {
    let input = pathological(10_000, 100, 13);
    let opts = AlignOptions {
        closeness: None,
        work_budget: Some(u64::MAX),
        ..AlignOptions::with_algorithm(Algorithm::Standard)
    };
    let latency = cancel_latency(Duration::from_millis(200), |cancel| {
        align(&input.input(), &opts, None, None, cancel).map(|_| ())
    });
    assert!(latency <= LATENCY, "cancellation took {latency:?}");
}

/// A slow scorer inside one large closeness gap: the DP checks cancellation every 4,096 cells.
#[test]
fn closeness_observes_cancellation_within_100_ms() {
    struct Slow;
    impl PairScorer for Slow {
        fn similarity(&self, l: u32, r: u32) -> u16 {
            // Some arithmetic per cell, so that the 4,000,000-cell gap takes seconds in a debug build.
            let mut h = u64::from(l) << 32 | u64::from(r);
            for _ in 0..64 {
                h = h.wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(17);
            }
            (h % 1001) as u16
        }
    }
    let left: Vec<TokenId> = (0..2000).map(TokenId).collect();
    let right: Vec<TokenId> = (2000..4000).map(TokenId).collect();
    let trivial = TrivialSet::default();
    let input = AlignInput {
        left: &left,
        right: &right,
        left_weights: &[],
        right_weights: &[],
        trivial: &trivial,
        distinct: 4000,
    };
    let opts = AlignOptions {
        closeness: Some(Closeness::default()),
        ..AlignOptions::with_algorithm(Algorithm::Standard)
    };
    let latency = cancel_latency(Duration::from_millis(30), |cancel| {
        align(&input, &opts, Some(&Slow), None, cancel).map(|_| ())
    });
    assert!(latency <= LATENCY, "cancellation took {latency:?}");
}

/// Regression (closeness cancellation granularity, G4): a 1 × 4,000,000 gap (the default per-gap cap) is one DP
/// row of 4,000,000 cells. The DP used to check cancellation once per row, so cancelling took the whole row
/// (0.6–1.4 s in release); it now checks every 4,096 cells, also along the longer side. The cancellation is sent
/// once the scorer runs (inside the DP) and measured on a single attempt. Standard reaches the gap without an
/// imara-diff call (Histogram would need a helper thread, which may wait for the helpers this binary abandons,
/// D-54); the DP is the same for every engine.
#[test]
fn closeness_on_a_wide_gap_cancels_within_100_ms() {
    struct Started<'a>(&'a AtomicBool);
    impl PairScorer for Started<'_> {
        fn similarity(&self, l: u32, r: u32) -> u16 {
            self.0.store(true, Ordering::Relaxed);
            let mut h = u64::from(l) << 32 | u64::from(r);
            for _ in 0..16 {
                h = h.wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(17);
            }
            (h % 1001) as u16
        }
    }
    let left = vec![TokenId(0)];
    let right: Vec<TokenId> = (1..=4_000_000).map(TokenId).collect();
    let trivial = TrivialSet::default();
    let input = AlignInput {
        left: &left,
        right: &right,
        left_weights: &[],
        right_weights: &[],
        trivial: &trivial,
        distinct: 4_000_001,
    };
    {
        let algorithm = Algorithm::Standard;
        let opts = AlignOptions::with_algorithm(algorithm);
        let started = AtomicBool::new(false);
        let scorer = Started(&started);
        let cancel = CancelToken::new();
        let latency = thread::scope(|scope| {
            let worker = scope.spawn(|| align(&input, &opts, Some(&scorer), None, &cancel));
            while !started.load(Ordering::Relaxed) && !worker.is_finished() {
                thread::yield_now();
            }
            thread::sleep(Duration::from_millis(5));
            assert!(
                !worker.is_finished(),
                "{algorithm}: the DP finished before it was cancelled"
            );
            let start = Instant::now();
            cancel.cancel();
            let result = worker.join().unwrap();
            let latency = start.elapsed();
            assert_eq!(result.map(|_| ()), Err(DiffError::Cancelled));
            latency
        });
        assert!(
            latency <= LATENCY,
            "{algorithm}: cancellation took {latency:?}"
        );
    }
}
