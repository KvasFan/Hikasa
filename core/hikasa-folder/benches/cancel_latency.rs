//! Gate G4, folder part (§6.4): the time from `cancel()` to the return of `compare_into` (threshold ≤ 100 ms).
//!
//! - `folder_100k_*`: the 100k scan of G2 cancelled half-way (after half of the folder pairs were listed), without
//!   and with a binary content test.
//! - `huge_folder_1m`: one folder pair with 1,000,000 entries per side (`MemoryVfs`); the cancellation arrives right
//!   after both listings returned and at fixed delays after that, so it lands in every step of the batch
//!   (alignment, planning, insertion).
//! - `restat_200k`: the FC-054 re-stat storm — 200,000 excluded left entries aligned with visible right ones, each
//!   stat'ed after alignment — cancelled half-way through the re-stats.
//!
//! The worst latency of every case is checked against the gate (assert; `HIKASA_GATES=report` only prints).

mod support;

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use criterion::{Criterion, criterion_group, criterion_main};
use hikasa_core::{CancelToken, CaseSensitivity, FileTime, LocalPath, Spec};
use hikasa_folder::{FolderRules, FolderRulesPatch, NullObserver, ScanEvent, ScanObserver};
use hikasa_vfs::{
    AtomicWrite, Capabilities, Entry, FileName, FollowLinks, ListOptions, Listing, LocalOptions,
    LocalVfs, MemMeta, MemoryVfs, ReadSeek, Vfs, VfsError, WriteOptions,
};

const GATE: Duration = Duration::from_millis(100);

/// Cancels after `after` listed folders and records the instant.
struct Halfway<'a> {
    after: usize,
    seen: AtomicUsize,
    cancel: &'a CancelToken,
    at: Mutex<Option<Instant>>,
}

impl ScanObserver for Halfway<'_> {
    fn on_event(&self, event: ScanEvent) {
        if let ScanEvent::FolderListed { .. } = event
            && self.seen.fetch_add(1, Ordering::SeqCst) + 1 == self.after
        {
            *self.at.lock().unwrap() = Some(Instant::now());
            self.cancel.cancel();
        }
    }
}

fn one(
    vfs: &LocalVfs,
    dir: &std::path::Path,
    pool: &rayon::ThreadPool,
    rules: &FolderRules,
) -> Duration {
    let cancel = CancelToken::new();
    let observer = Halfway {
        after: support::FOLDERS / 2,
        seen: AtomicUsize::new(0),
        cancel: &cancel,
        at: Mutex::new(None),
    };
    let (model, cmp) = support::compare(
        vfs,
        support::spec(&dir.join("l")),
        support::spec(&dir.join("r")),
        rules,
        Some(pool),
        &cancel,
        &observer,
    );
    let returned = Instant::now();
    let cmp = cmp.expect("cancellation after the bases are loaded is Ok");
    assert!(!cmp.is_complete());
    assert!(model.is_finished());
    let at = observer.at.lock().unwrap().expect("cancelled half-way");
    returned.saturating_duration_since(at)
}

/// When to cancel inside the huge batch or the re-stat storm.
#[derive(Clone, Copy)]
enum Trigger {
    /// `delay` after the second listing of the folder `…/big` returned.
    AfterListing { delay: Duration },
    /// At the `n`-th `stat` call.
    AtStat { n: usize },
}

/// A `Vfs` wrapper that cancels at a [`Trigger`] and records the instant of `cancel()`.
struct Canceller<'a> {
    inner: &'a MemoryVfs,
    token: &'a CancelToken,
    trigger: Trigger,
    listed: AtomicUsize,
    stats: AtomicUsize,
    at: Arc<Mutex<Option<Instant>>>,
}

impl Canceller<'_> {
    /// Cancels now, or after `delay` on a timer thread (the scan goes on meanwhile).
    fn fire(&self, delay: Duration) {
        let token = self.token.clone();
        let at = Arc::clone(&self.at);
        let cancel = move || {
            let now = Instant::now();
            token.cancel();
            *at.lock().unwrap() = Some(now);
        };
        if delay.is_zero() {
            cancel();
        } else {
            std::thread::spawn(move || {
                std::thread::sleep(delay);
                cancel();
            });
        }
    }
}

impl Vfs for Canceller<'_> {
    fn capabilities(&self, at: &Spec) -> Result<Capabilities, VfsError> {
        self.inner.capabilities(at)
    }

    fn stat(&self, spec: &Spec, follow: FollowLinks) -> Result<Entry, VfsError> {
        let n = self.stats.fetch_add(1, Ordering::SeqCst) + 1;
        if let Trigger::AtStat { n: at } = self.trigger
            && n == at
        {
            self.fire(Duration::ZERO);
        }
        self.inner.stat(spec, follow)
    }

    fn list(
        &self,
        dir: &Spec,
        opts: &ListOptions<'_>,
        cancel: &CancelToken,
    ) -> Result<Listing, VfsError> {
        let listing = self.inner.list(dir, opts, cancel)?;
        if let Trigger::AfterListing { delay } = self.trigger
            && dir.canonical().ends_with("/big")
            && self.listed.fetch_add(1, Ordering::SeqCst) == 1
        {
            self.fire(delay);
        }
        Ok(listing)
    }

    fn open_read(&self, spec: &Spec) -> Result<Box<dyn ReadSeek>, VfsError> {
        self.inner.open_read(spec)
    }

    fn open_write(
        &self,
        spec: &Spec,
        opts: &WriteOptions,
    ) -> Result<Box<dyn AtomicWrite>, VfsError> {
        self.inner.open_write(spec, opts)
    }

    fn child(&self, dir: &Spec, name: &FileName) -> Result<Spec, VfsError> {
        self.inner.child(dir, name)
    }

    fn local_path(&self, spec: &Spec) -> Option<PathBuf> {
        self.inner.local_path(spec)
    }
}

fn mem_spec(path: &str) -> Spec {
    let host = if cfg!(windows) {
        format!("C:{}", path.replace('/', "\\"))
    } else {
        path.to_owned()
    };
    Spec::Local(LocalPath::new(PathBuf::from(host)).unwrap())
}

fn mem_path(path: &str) -> String {
    if cfg!(windows) {
        format!("C:{}", path.replace('/', "\\"))
    } else {
        path.to_owned()
    }
}

fn meta() -> MemMeta {
    MemMeta {
        mtime: Some(FileTime::from_unix(1_790_000_000, 0).unwrap()),
        ..MemMeta::default()
    }
}

/// `/l/big`, `/r/big` with `n` identical files each.
fn huge_folder(n: usize) -> MemoryVfs {
    let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
    let content: std::sync::Arc<[u8]> = std::sync::Arc::from(&b"content"[..]);
    for base in ["/l", "/r"] {
        vfs.insert_dir(&mem_path(&format!("{base}/big")), meta())
            .unwrap();
        for f in 0..n {
            vfs.insert_file(
                &mem_path(&format!("{base}/big/file{f:07}.dat")),
                std::sync::Arc::clone(&content),
                meta(),
            )
            .unwrap();
        }
    }
    vfs
}

/// `/l/d/x{i}.bak` (excluded) against `/r/d/x{i}.txt`, aligned by extension: `n` re-stats.
fn restat_storm(n: usize) -> (MemoryVfs, FolderRules) {
    let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
    for f in 0..n {
        vfs.insert_file(
            &mem_path(&format!("/l/d/x{f:07}.bak")),
            b"x".to_vec(),
            meta(),
        )
        .unwrap();
        vfs.insert_file(
            &mem_path(&format!("/r/d/x{f:07}.txt")),
            b"x".to_vec(),
            meta(),
        )
        .unwrap();
    }
    let mut rules = FolderRules::default();
    rules
        .apply(&FolderRulesPatch {
            name_filter: Some("-*.bak".into()),
            align_extensions: Some(true),
            ..FolderRulesPatch::default()
        })
        .unwrap();
    (vfs, rules)
}

/// One cancelled scan on `vfs`; the latency from `cancel()` to the return (`None` when the scan ended first).
fn cancelled_scan(
    vfs: &MemoryVfs,
    rules: &FolderRules,
    pool: &rayon::ThreadPool,
    trigger: Trigger,
) -> Option<Duration> {
    let token = CancelToken::new();
    let at = Arc::new(Mutex::new(None));
    let canceller = Canceller {
        inner: vfs,
        token: &token,
        trigger,
        listed: AtomicUsize::new(0),
        stats: AtomicUsize::new(0),
        at: Arc::clone(&at),
    };
    let (_, cmp) = support::compare(
        &canceller,
        mem_spec("/l"),
        mem_spec("/r"),
        rules,
        Some(pool),
        &token,
        &NullObserver,
    );
    let returned = Instant::now();
    let cmp = cmp.expect("cancellation after the bases are loaded is Ok");
    if cmp.is_complete() {
        // The scan ended before the cancellation took effect (a late timer): nothing to measure.
        return None;
    }
    // The timer thread records its instant right after `cancel()`.
    let deadline = Instant::now() + Duration::from_secs(10);
    let at = loop {
        if let Some(at) = *at.lock().unwrap() {
            break at;
        }
        if Instant::now() > deadline {
            return None;
        }
        std::thread::yield_now();
    };
    Some(returned.saturating_duration_since(at))
}

fn cancel_latency(c: &mut Criterion) {
    let vfs = LocalVfs::new(LocalOptions::default());
    let pool = support::service_pool();
    let dir = support::tree_100k("identical");
    let mut rules = FolderRules::default();
    let mut worst = Duration::ZERO;
    for method in [
        hikasa_core::ContentMethod::None,
        hikasa_core::ContentMethod::Binary,
    ] {
        rules.criteria.content.method = method;
        let mut group = c.benchmark_group("cancel_latency");
        group.sample_size(10);
        group.bench_function(format!("folder_100k_{}", method.as_str()), |b| {
            b.iter_custom(|iters| {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    let latency = one(&vfs, &dir, &pool, &rules);
                    worst = worst.max(latency);
                    total += latency;
                }
                total
            });
        });
        group.finish();
    }
    support::check_gate(
        "G4 folder_100k",
        worst <= GATE,
        &format!("worst latency {worst:?} (gate ≤ 100 ms)"),
    );

    // One folder with 1,000,000 entries per side, cancelled inside its batch.
    let huge = huge_folder(1_000_000);
    let plain = FolderRules::default();
    let mut worst = Duration::ZERO;
    let mut landed = 0;
    for delay_ms in [
        0u64, 25, 50, 75, 100, 125, 150, 175, 200, 225, 250, 275, 300, 350, 400,
    ] {
        let trigger = Trigger::AfterListing {
            delay: Duration::from_millis(delay_ms),
        };
        if let Some(latency) = cancelled_scan(&huge, &plain, &pool, trigger) {
            println!(
                "cancel_latency/huge_folder_1m: cancel {delay_ms} ms after the listing → {latency:?}"
            );
            worst = worst.max(latency);
            landed += 1;
        }
    }
    drop(huge);
    support::check_gate(
        "G4 huge_folder_1m",
        landed > 0 && worst <= GATE,
        &format!("worst latency {worst:?} over {landed} cancellations (gate ≤ 100 ms)"),
    );

    // The FC-054 re-stat storm, cancelled half-way through the re-stats.
    let (storm, storm_rules) = restat_storm(200_000);
    let mut worst = Duration::ZERO;
    for _ in 0..5 {
        let trigger = Trigger::AtStat { n: 100_000 };
        if let Some(latency) = cancelled_scan(&storm, &storm_rules, &pool, trigger) {
            worst = worst.max(latency);
        }
    }
    support::check_gate(
        "G4 restat_200k",
        worst <= GATE,
        &format!("worst latency {worst:?} (gate ≤ 100 ms)"),
    );
}

criterion_group!(benches, cancel_latency);
criterion_main!(benches);
