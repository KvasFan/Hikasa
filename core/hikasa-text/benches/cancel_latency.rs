//! Bench `cancel_latency`, text part (gate G4, §6.4: ≤ 100 ms from `cancel()` to return): each iteration
//! starts a G1 comparison (two 1,000,000-line C sources, `compare` on an 8-thread pool), cancels it at 50 % of
//! the time a full comparison takes, and measures the time until `compare` returns `Err(Cancelled)`. The gate
//! check also cancels at 10, 30, 70 and 90 % (every phase: keys, interning, alignment, classification) and
//! prints the worst latency per configuration.

#[path = "support/inputs.rs"]
mod inputs;

use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use criterion::{Criterion, criterion_group, criterion_main};
use hikasa_core::{CancelToken, Ctl, NullProgress};
use hikasa_formats::{CompiledFormat, FormatId, FormatRegistry};
use hikasa_text::{
    AlgorithmChoice, CompareInput, DocOrigin, LoadOptions, TextDocument, TextError, TextRules,
    compare,
};
use inputs::c_pair;

fn format(id: &str) -> Arc<CompiledFormat> {
    FormatRegistry::builtin()
        .unwrap()
        .compiled(&FormatId::new(id).unwrap())
        .unwrap()
}

/// A comparison input of already built documents (the latency of `compare` is measured; document loading has
/// its own 1 MiB check points, §5.3).
fn input(
    left: &Arc<TextDocument>,
    right: &Arc<TextDocument>,
    format: &Arc<CompiledFormat>,
    rules: &TextRules,
) -> CompareInput {
    CompareInput {
        left: Arc::clone(left),
        right: Arc::clone(right),
        format: Arc::clone(format),
        rules: rules.clone(),
        bytes_identical: false,
        force_tokenize: false,
    }
}

/// Starts the comparison, cancels it after `after`, returns the latency from `cancel()` to the return.
fn latency(
    make: &(dyn Fn() -> CompareInput + Sync),
    pool: &rayon::ThreadPool,
    after: Duration,
) -> Duration {
    let cancel = CancelToken::new();
    thread::scope(|scope| {
        let worker = scope.spawn(|| {
            let ctl = Ctl {
                cancel: &cancel,
                progress: &NullProgress,
            };
            compare(make(), Some(pool), &ctl, None)
        });
        thread::sleep(after);
        let start = Instant::now();
        cancel.cancel();
        let result = worker.join().unwrap();
        let took = start.elapsed();
        assert!(
            matches!(result, Err(TextError::Cancelled)) || result.is_ok(),
            "{result:?}"
        );
        took
    })
}

fn benches(c: &mut Criterion) {
    let (left, right) = c_pair(1_000_000, 0x5EED_0001);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(8)
        .build()
        .unwrap();
    let mut standard = TextRules::default();
    standard.alignment.algorithm = AlgorithmChoice::Standard;
    let configurations = [
        ("auto", format("c-cpp"), TextRules::default()),
        ("standard", format("c-cpp"), standard),
        ("identity", format("rust"), TextRules::default()),
    ];
    let load = LoadOptions::default();
    let mut group = c.benchmark_group("cancel_latency");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(10));
    for (name, fmt, rules) in &configurations {
        let doc = |b: &[u8]| {
            Arc::new(TextDocument::from_bytes(b, fmt, &load, DocOrigin::default()).unwrap())
        };
        let (l, r) = (doc(&left), doc(&right));
        let make = || input(&l, &r, fmt, rules);
        let start = Instant::now();
        compare(make(), Some(&pool), &Ctl::detached(), None).unwrap();
        let full = start.elapsed();
        let worst = [10u32, 30, 50, 70, 90]
            .iter()
            .map(|&percent| latency(&make, &pool, full * percent / 100))
            .max()
            .unwrap_or_default();
        println!(
            "gate G4 cancel_latency/text_{name}: worst of 5 (cancel at 10–90 %) {:.2} ms (limit 100 ms) {}",
            worst.as_secs_f64() * 1e3,
            if worst <= Duration::from_millis(100) {
                "PASS"
            } else {
                "FAIL"
            }
        );
        group.bench_function(format!("text_{name}"), |b| {
            b.iter_custom(|iters| (0..iters).map(|_| latency(&make, &pool, full / 2)).sum())
        });
    }
    group.finish();
}

criterion_group!(cancel_benches, benches);
criterion_main!(cancel_benches);
