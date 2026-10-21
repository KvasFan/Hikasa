//! Gate G2 (§6.4): a folder comparison of 100,000 files per side (10,000 folders, depth ≤ 8) on the local file
//! system, scan + quick tests (size and timestamps), on the service pool: identical trees and trees with 1 %
//! differences. Warm-cache runs (criterion); a cold run needs dropped caches (perf runner, root).
//!
//! The trees are generated below `$HIKASA_BENCH_DIR` (see `support`) and reused.

mod support;

use std::time::Duration;

use criterion::{Criterion, criterion_group, criterion_main};
use hikasa_core::CancelToken;
use hikasa_folder::{FolderRules, NullObserver};
use hikasa_vfs::{LocalOptions, LocalVfs};

fn folder_scan_100k(c: &mut Criterion) {
    let vfs = LocalVfs::new(LocalOptions::default());
    let pool = support::service_pool();
    let rules = FolderRules::default();
    let mut group = c.benchmark_group("folder_scan_100k");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(20));
    for kind in ["identical", "one_percent"] {
        let dir = support::tree_100k(kind);
        let (left, right) = (support::spec(&dir.join("l")), support::spec(&dir.join("r")));
        // One checked run first.
        let (_, cmp) = support::compare(
            &vfs,
            left.clone(),
            right.clone(),
            &rules,
            Some(&pool),
            &CancelToken::never(),
            &NullObserver,
        );
        let cmp = cmp.unwrap();
        let stats = cmp.stats(false);
        println!(
            "folder_scan_100k/{kind}: {} file pairs ({} same, {} different, {} newer), {} folder pairs, verdict {}",
            stats.files.total(),
            stats.files.same,
            stats.files.different,
            stats.files.left_newer + stats.files.right_newer,
            stats.folders.total(),
            cmp.verdict(false)
        );
        drop(cmp);
        // The gate (< 1.5 s warm): the fastest of five timed runs (the low-load figure on a shared machine).
        let mut best = Duration::MAX;
        for _ in 0..5 {
            let start = std::time::Instant::now();
            let (_, cmp) = support::compare(
                &vfs,
                left.clone(),
                right.clone(),
                &rules,
                Some(&pool),
                &CancelToken::never(),
                &NullObserver,
            );
            assert!(cmp.is_some_and(|c| c.is_complete()));
            best = best.min(start.elapsed());
        }
        support::check_gate(
            "G2 warm",
            best < Duration::from_millis(1500),
            &format!("folder_scan_100k/{kind}: fastest of 5 runs {best:?} (gate < 1.5 s)"),
        );
        group.bench_function(kind, |b| {
            b.iter(|| {
                let (_, cmp) = support::compare(
                    &vfs,
                    left.clone(),
                    right.clone(),
                    &rules,
                    Some(&pool),
                    &CancelToken::never(),
                    &NullObserver,
                );
                cmp.map(|c| c.verdict(false))
            });
        });
    }
    group.finish();
}

criterion_group!(benches, folder_scan_100k);
criterion_main!(benches);
