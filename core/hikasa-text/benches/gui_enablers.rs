//! Bench `gui_enablers` (gate G5, §6.4) on the G1 result (`text_compare_1m_auto`): `TextComparison::row`
//! < 200 ns, `FilteredView::get` < 200 ns (Differences, Context 3 and All views), and `TextComparison::inline`
//! p99 < 50 µs over every changed row. The gate check prints the measured values first; criterion then
//! measures `row`, `get` and `inline` on random positions.

#[path = "support/inputs.rs"]
mod inputs;

use std::hint::black_box;
use std::sync::Arc;
use std::time::{Duration, Instant};

use criterion::{Criterion, criterion_group, criterion_main};
use hikasa_core::Ctl;
use hikasa_formats::{FormatId, FormatRegistry};
use hikasa_text::{
    CompareInput, DocOrigin, LoadOptions, RowIdx, TextDisplayFilter, TextDocument, TextRules,
    ViewOptions, compare,
};
use inputs::{XorShift, c_pair};

fn benches(c: &mut Criterion) {
    let (left, right) = c_pair(1_000_000, 0x5EED_0001);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(8)
        .build()
        .unwrap();
    let format = FormatRegistry::builtin()
        .unwrap()
        .compiled(&FormatId::new("c-cpp").unwrap())
        .unwrap();
    let load = LoadOptions::default();
    let doc = |b: &[u8]| TextDocument::from_bytes(b, &format, &load, DocOrigin::default()).unwrap();
    let input = CompareInput {
        left: Arc::new(doc(&left)),
        right: Arc::new(doc(&right)),
        format: Arc::clone(&format),
        rules: TextRules::default(),
        bytes_identical: false,
        force_tokenize: false,
    };
    drop((left, right));
    let cmp = compare(input, Some(&pool), &Ctl::detached(), None).unwrap();
    let rows = cmp.row_count();
    let mut rng = XorShift(7);
    let picks: Vec<u32> = (0..1_000_000)
        .map(|_| rng.below(u64::from(rows)) as u32)
        .collect();
    let views: Vec<_> = [
        TextDisplayFilter::Differences,
        TextDisplayFilter::Context { lines: 3 },
        TextDisplayFilter::All,
    ]
    .into_iter()
    .map(|filter| {
        cmp.view(&ViewOptions {
            filter,
            ignore_unimportant: false,
            expanded: vec![],
        })
    })
    .collect();
    let changed: Vec<u32> = (0..rows)
        .filter(|&k| cmp.inline(RowIdx(k)).is_some())
        .collect();

    // Gate check.
    let start = Instant::now();
    for &k in &picks {
        black_box(cmp.row(RowIdx(black_box(k))));
    }
    let row_ns = start.elapsed().as_nanos() as f64 / picks.len() as f64;
    let mut get_ns = 0f64;
    for view in &views {
        let len = view.len().max(1);
        let start = Instant::now();
        for &k in &picks {
            black_box(view.get(black_box(k % len)));
        }
        get_ns = get_ns.max(start.elapsed().as_nanos() as f64 / picks.len() as f64);
    }
    let mut times: Vec<Duration> = changed
        .iter()
        .map(|&k| {
            let start = Instant::now();
            black_box(cmp.inline(RowIdx(k)));
            start.elapsed()
        })
        .collect();
    times.sort();
    let p99 = times
        .get(times.len() * 99 / 100)
        .copied()
        .unwrap_or_default();
    let verdict = |ok: bool| if ok { "PASS" } else { "FAIL" };
    println!(
        "gate G5 row: {row_ns:.1} ns (limit 200 ns) {}",
        verdict(row_ns < 200.0)
    );
    println!(
        "gate G5 FilteredView::get: {get_ns:.1} ns (worst view; limit 200 ns) {}",
        verdict(get_ns < 200.0)
    );
    println!(
        "gate G5 inline p99: {:.1} µs over {} changed rows (limit 50 µs) {}",
        p99.as_secs_f64() * 1e6,
        times.len(),
        verdict(p99 < Duration::from_micros(50))
    );

    let mut group = c.benchmark_group("gui_enablers");
    let mut at = 0usize;
    group.bench_function("row", |b| {
        b.iter(|| {
            at = (at + 1) % picks.len();
            black_box(cmp.row(RowIdx(picks[at])))
        })
    });
    let view = &views[0];
    let len = view.len().max(1);
    group.bench_function("filtered_view_get", |b| {
        b.iter(|| {
            at = (at + 1) % picks.len();
            black_box(view.get(picks[at] % len))
        })
    });
    if !changed.is_empty() {
        group.bench_function("inline", |b| {
            b.iter(|| {
                at = (at + 1) % changed.len();
                black_box(cmp.inline(RowIdx(changed[at])))
            })
        });
    }
    group.finish();
}

criterion_group!(gui_benches, benches);
criterion_main!(gui_benches);
