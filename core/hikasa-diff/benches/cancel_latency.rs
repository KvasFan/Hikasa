//! Bench `cancel_latency` (gate G4, diff part, §6.4: ≤ 100 ms from `cancel()` to return): a large imara-diff
//! call (300,000 random tokens per side, seconds of work) runs on the helper thread of D-54; each iteration
//! starts the alignment, cancels it after 50 ms and measures the time until `align` returns `Err(Cancelled)`.
//! The same is measured for Standard's own loops on the pathological input. Abandoned helper threads keep
//! running until their call ends, so the iterations are few and the process exits at the end.

#[path = "support/inputs.rs"]
mod inputs;

use std::thread;
use std::time::{Duration, Instant};

use criterion::{Criterion, criterion_group, criterion_main};
use hikasa_core::CancelToken;
use hikasa_diff::{Algorithm, AlignInput, AlignOptions, DiffError, TokenId, TrivialSet, align};
use inputs::{XorShift, pathological};

/// Starts `job`, cancels it after 50 ms, and returns the latency from `cancel()` to its return.
fn latency(job: &(dyn Fn(&CancelToken) -> Result<(), DiffError> + Sync)) -> Duration {
    let cancel = CancelToken::new();
    thread::scope(|scope| {
        let worker = scope.spawn(|| job(&cancel));
        thread::sleep(Duration::from_millis(50));
        let start = Instant::now();
        cancel.cancel();
        let result = worker.join().unwrap();
        let took = start.elapsed();
        assert_eq!(result, Err(DiffError::Cancelled));
        took
    })
}

fn benches(c: &mut Criterion) {
    let mut rng = XorShift(0x5EED_CA9C_E100_0001);
    let left: Vec<TokenId> = (0..300_000)
        .map(|_| TokenId(rng.below(1000) as u32))
        .collect();
    let right: Vec<TokenId> = (0..300_000)
        .map(|_| TokenId(rng.below(1000) as u32))
        .collect();
    let trivial = TrivialSet::default();
    let input = AlignInput {
        left: &left,
        right: &right,
        left_weights: &[],
        right_weights: &[],
        trivial: &trivial,
        distinct: 1000,
    };
    let imara = AlignOptions {
        closeness: None,
        ..AlignOptions::with_algorithm(Algorithm::Myers)
    };
    let imara_job = |cancel: &CancelToken| align(&input, &imara, None, None, cancel).map(|_| ());
    let patho = pathological(10_000, 100, 0x5EED_CA9C_E100_0002);
    let standard = AlignOptions {
        work_budget: Some(u64::MAX),
        ..AlignOptions::with_algorithm(Algorithm::Standard)
    };
    let standard_job =
        |cancel: &CancelToken| align(&patho.input(), &standard, None, None, cancel).map(|_| ());

    for (name, job) in [
        (
            "imara_helper",
            &imara_job as &(dyn Fn(&CancelToken) -> Result<(), DiffError> + Sync),
        ),
        ("standard", &standard_job),
    ] {
        let worst = (0..5).map(|_| latency(job)).max().unwrap_or_default();
        let verdict = if worst <= Duration::from_millis(100) {
            "PASS"
        } else {
            "FAIL"
        };
        println!(
            "gate G4 cancel_latency/{name}: worst of 5 {:.2} ms (limit 100 ms) {verdict}",
            worst.as_secs_f64() * 1e3
        );
    }

    let mut group = c.benchmark_group("cancel_latency");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(3));
    group.bench_function("imara_helper", |b| {
        b.iter_custom(|iters| (0..iters).map(|_| latency(&imara_job)).sum())
    });
    group.bench_function("standard", |b| {
        b.iter_custom(|iters| (0..iters).map(|_| latency(&standard_job)).sum())
    });
    group.finish();
}

criterion_group!(cancel_benches, benches);
criterion_main!(cancel_benches);
