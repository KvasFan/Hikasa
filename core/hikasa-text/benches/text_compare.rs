//! Benches `text_compare_1m_auto`, `text_compare_1m_standard` and `text_compare_1m_identity` (gate G1, §6.4:
//! < 3 s end-to-end from bytes for two 1,000,000-line C-like files of about 48 bytes per line with about 1 % of
//! the lines changed in clusters; the memory half of G1 is the `perf_rss` test).
//!
//! End to end from bytes: both documents are built in parallel (`TextDocument::from_bytes`), then `compare`
//! runs on an 8-thread pool (the §0.8 reference machine has 8 threads). `auto` is the C preset with the default
//! rules (Histogram, the grammar path: the preset's Number element is case-insensitive); `standard` is the same
//! with the Standard algorithm; `identity` is the Rust preset, whose elements are all important and
//! case-sensitive, so the identity fast path (D-53) builds the keys and Histogram aligns them.
//!
//! Before the criterion measurements, every configuration runs 3 times and the best time is printed with the
//! gate verdict.

#[path = "support/inputs.rs"]
mod inputs;

use std::hint::black_box;
use std::sync::Arc;
use std::time::{Duration, Instant};

use criterion::{Criterion, criterion_group, criterion_main};
use hikasa_core::Ctl;
use hikasa_formats::{CompiledFormat, FormatId, FormatRegistry};
use hikasa_text::{
    AlgorithmChoice, CompareInput, DocOrigin, LoadOptions, TextComparison, TextDocument, TextRules,
    compare,
};
use inputs::c_pair;

const SEED: u64 = 0x5EED_0001;

fn format(id: &str) -> Arc<CompiledFormat> {
    FormatRegistry::builtin()
        .unwrap()
        .compiled(&FormatId::new(id).unwrap())
        .unwrap()
}

fn end_to_end(
    left: &[u8],
    right: &[u8],
    format: &Arc<CompiledFormat>,
    rules: &TextRules,
    pool: &rayon::ThreadPool,
) -> TextComparison {
    let load = LoadOptions::default();
    let doc = |b: &[u8]| TextDocument::from_bytes(b, format, &load, DocOrigin::default()).unwrap();
    let (l, r) = pool.join(|| doc(left), || doc(right));
    let input = CompareInput {
        left: Arc::new(l),
        right: Arc::new(r),
        format: Arc::clone(format),
        rules: rules.clone(),
        bytes_identical: left == right,
        force_tokenize: false,
    };
    compare(input, Some(pool), &Ctl::detached(), None).unwrap()
}

fn benches(c: &mut Criterion) {
    let (left, right) = c_pair(1_000_000, SEED);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(8)
        .build()
        .unwrap();
    let mut standard = TextRules::default();
    standard.alignment.algorithm = AlgorithmChoice::Standard;
    let configurations = [
        (
            "text_compare_1m_auto",
            format("c-cpp"),
            TextRules::default(),
        ),
        ("text_compare_1m_standard", format("c-cpp"), standard),
        (
            "text_compare_1m_identity",
            format("rust"),
            TextRules::default(),
        ),
    ];
    for (name, fmt, rules) in &configurations {
        let best = (0..3)
            .map(|_| {
                let start = Instant::now();
                black_box(end_to_end(&left, &right, fmt, rules, &pool));
                start.elapsed()
            })
            .min()
            .unwrap_or_default();
        println!(
            "gate G1 {name}: best of 3 {:.3} s (limit 3 s) {}",
            best.as_secs_f64(),
            if best < Duration::from_secs(3) {
                "PASS"
            } else {
                "FAIL"
            }
        );
    }
    let mut group = c.benchmark_group("text_compare");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(20));
    for (name, fmt, rules) in &configurations {
        group.bench_function(*name, |b| {
            b.iter(|| black_box(end_to_end(&left, &right, fmt, rules, &pool)))
        });
    }
    group.finish();
}

criterion_group!(text_benches, benches);
criterion_main!(text_benches);
