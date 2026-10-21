//! Benches `align_1m_histogram`, `align_1m_standard` and `align_pathological` (§6.2.3; gates §6.4:
//! < 0.3 s, < 1.0 s and < 1.0 s with O(n + m) allocated bytes), and `align_random_standard` (bounded Standard,
//! §2.14.5 as amended in wave 2: < 2 s).
//!
//! Inputs (§6.1, seeded xorshift64*): 1,000,000 lines with about 1 % of the lines changed in clusters;
//! 10,000 anchor-less gaps of 100 lines over two repetitive keys, which exhausts Standard's default work budget,
//! so thousands of pending ranges go through the Histogram fallback, each through `DenseRemap`; and 1,000,000
//! random lines per side over 50 symbols, which exhausts both the work budget and the fallback budget (the
//! degraded split, warning `alignment_degraded`).
//!
//! Before the criterion measurements, the gate check runs every case 5 times, prints the median and the gate,
//! and counts the bytes allocated by one pathological alignment with a counting `#[global_allocator]` (this
//! bench binary only, §6.1).
// A counting allocator cannot be written without `unsafe` (`GlobalAlloc` is an unsafe trait); the workspace
// lint is `deny`, so this bench binary opts out explicitly. Every occurrence carries a `// SAFETY:` comment.
#![allow(unsafe_code)]

#[path = "support/inputs.rs"]
mod inputs;

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use criterion::{Criterion, criterion_group, criterion_main};
use hikasa_core::{CancelToken, WarningCode};
use hikasa_diff::{Algorithm, Alignment, DiffWarningCode, align};
use inputs::{Generated, HashScorer, clustered_changes, options, pathological, random_lines};

struct Counting;

static ALLOCATED: AtomicU64 = AtomicU64::new(0);

// SAFETY: every method forwards to `System`, which upholds the `GlobalAlloc` contract; the counter is a plain
// atomic, so counting never allocates or re-enters.
unsafe impl GlobalAlloc for Counting {
    // SAFETY: the caller's guarantees for `layout` are passed on to `System::alloc` unchanged.
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATED.fetch_add(layout.size() as u64, Ordering::Relaxed);
        // SAFETY: forwarded with the caller's guarantees.
        unsafe { System.alloc(layout) }
    }

    // SAFETY: `ptr` and `layout` come from this allocator, i.e. from `System`.
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: forwarded with the caller's guarantees.
        unsafe { System.dealloc(ptr, layout) }
    }

    // SAFETY: the caller's guarantees for `layout` are passed on to `System::alloc_zeroed` unchanged.
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOCATED.fetch_add(layout.size() as u64, Ordering::Relaxed);
        // SAFETY: forwarded with the caller's guarantees.
        unsafe { System.alloc_zeroed(layout) }
    }

    // SAFETY: `ptr` and `layout` come from this allocator, i.e. from `System`; `new_size` is checked by the
    // caller.
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATED.fetch_add(new_size as u64, Ordering::Relaxed);
        // SAFETY: forwarded with the caller's guarantees.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

const SEED: u64 = 0x5EED_1234_ABCD_0001;

/// Gaps and gap length of the pathological input.
const PATHOLOGICAL_GAPS: usize = 10_000;
const PATHOLOGICAL_GAP_LEN: usize = 100;

fn run(input: &Generated, algorithm: Algorithm) -> Alignment {
    align(
        &input.input(),
        &options(algorithm),
        Some(&HashScorer(input)),
        None,
        &CancelToken::never(),
    )
    .unwrap()
}

fn median_time(input: &Generated, algorithm: Algorithm) -> Duration {
    let mut times: Vec<Duration> = (0..5)
        .map(|_| {
            let start = Instant::now();
            black_box(run(input, algorithm));
            start.elapsed()
        })
        .collect();
    times.sort();
    times[2]
}

/// Prints the gates of §6.4 once per bench run.
fn gate_check(million: &Generated, patho: &Generated, random: &Generated) {
    let report = |name: &str, took: Duration, limit_ms: u64| {
        let verdict = if took < Duration::from_millis(limit_ms) {
            "PASS"
        } else {
            "FAIL"
        };
        println!(
            "gate {name}: median {:.1} ms (limit {limit_ms} ms) {verdict}",
            took.as_secs_f64() * 1e3
        );
    };
    report(
        "align_1m_histogram",
        median_time(million, Algorithm::Histogram),
        300,
    );
    report(
        "align_1m_standard",
        median_time(million, Algorithm::Standard),
        1000,
    );
    report(
        "align_pathological",
        median_time(patho, Algorithm::Standard),
        1000,
    );
    report(
        "align_random_standard",
        median_time(random, Algorithm::Standard),
        2000,
    );
    let degraded = run(random, Algorithm::Standard)
        .diff_warnings()
        .iter()
        .any(|w| w.code == DiffWarningCode::AlignmentDegraded);
    println!("gate align_random_standard: alignment_degraded: {degraded}");
    let before = ALLOCATED.load(Ordering::Relaxed);
    let alignment = run(patho, Algorithm::Standard);
    let bytes = ALLOCATED.load(Ordering::Relaxed) - before;
    let exhausted = alignment
        .warnings()
        .iter()
        .any(|w| w.code == WarningCode::WorkBudgetExhausted);
    println!(
        "gate align_pathological: {} gaps, {} tokens, budget exhausted: {exhausted}, allocated {} bytes = {:.1} B/token",
        patho.gaps,
        patho.tokens(),
        bytes,
        bytes as f64 / patho.tokens() as f64
    );
}

fn benches(c: &mut Criterion) {
    let million = clustered_changes(1_000_000, SEED);
    let patho = pathological(PATHOLOGICAL_GAPS, PATHOLOGICAL_GAP_LEN, SEED);
    let random = random_lines(1_000_000, 50, SEED);
    gate_check(&million, &patho, &random);
    let mut group = c.benchmark_group("align");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(8));
    group.bench_function("align_1m_histogram", |b| {
        b.iter(|| black_box(run(&million, Algorithm::Histogram)))
    });
    group.bench_function("align_1m_standard", |b| {
        b.iter(|| black_box(run(&million, Algorithm::Standard)))
    });
    group.bench_function("align_pathological", |b| {
        b.iter(|| black_box(run(&patho, Algorithm::Standard)))
    });
    group.bench_function("align_random_standard", |b| {
        b.iter(|| black_box(run(&random, Algorithm::Standard)))
    });
    group.finish();
}

criterion_group!(align_benches, benches);
criterion_main!(align_benches);
