//! Bench `folder_model_1m` (§1.6.6, §6.4): a 1,000,000-entry pair (1,000 folders × 1,000 files per side) in
//! `MemoryVfs`. Measures the model's memory per node — live heap bytes added by the scan, divided by the node
//! count, excluding the name bytes (gate: ≤ 256 B per node with both sides' metadata) — and times the scan.
//!
//! The counting allocator is the §6.1 exemption for `unsafe` in test and bench binaries.
// A counting allocator cannot be written without `unsafe` (`GlobalAlloc` is an unsafe trait); the workspace lint
// is `deny`, so this bench binary opts out explicitly. Every occurrence carries a `// SAFETY:` comment.
#![allow(unsafe_code)]

mod support;

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use criterion::Criterion;
use hikasa_core::{CancelToken, CaseSensitivity, FileTime, LocalPath, Spec};
use hikasa_folder::{FolderRules, NullObserver};
use hikasa_vfs::{MemMeta, MemoryVfs};

/// Counts live heap bytes.
struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);

// SAFETY: every method forwards to the system allocator with the caller's arguments unchanged, so the
// `GlobalAlloc` contract (valid layouts, pointers returned by this allocator) is upheld by `System`; the counter
// is only bookkeeping.
unsafe impl GlobalAlloc for Counting {
    // SAFETY: the caller's guarantees for `layout` are passed on to `System::alloc` unchanged.
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forwarded unchanged; the caller guarantees a non-zero-sized layout.
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        }
        p
    }

    // SAFETY: `ptr` and `layout` come from this allocator, i.e. from `System`.
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` was returned by `alloc`/`realloc` of this allocator (that is, of `System`) with `layout`.
        unsafe { System.dealloc(ptr, layout) };
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    // SAFETY: the caller's guarantees for `layout` are passed on to `System::alloc_zeroed` unchanged.
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forwarded unchanged; the caller guarantees a non-zero-sized layout.
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        }
        p
    }

    // SAFETY: `ptr` and `layout` come from this allocator, i.e. from `System`; `new_size` is checked by the
    // caller.
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: `ptr`/`layout` come from this allocator; `new_size` is the caller's valid new size.
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if !p.is_null() {
            LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
            LIVE.fetch_add(new_size, Ordering::Relaxed);
        }
        p
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

const FOLDERS: usize = 1_000;
const FILES: usize = 1_000;

fn host(path: &str) -> String {
    if cfg!(windows) {
        format!("C:{}", path.replace('/', "\\"))
    } else {
        path.to_owned()
    }
}

fn mspec(path: &str) -> Spec {
    Spec::Local(LocalPath::new(std::path::PathBuf::from(host(path))).unwrap())
}

fn build() -> MemoryVfs {
    let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
    let meta = MemMeta {
        mtime: Some(FileTime::from_unix(1_790_000_000, 0).unwrap()),
        ..MemMeta::default()
    };
    let content: std::sync::Arc<[u8]> = std::sync::Arc::from(&b"content"[..]);
    for base in ["/l", "/r"] {
        vfs.insert_dir(&host(base), meta.clone()).unwrap();
        for d in 0..FOLDERS {
            let dir = format!("{base}/folder{d:04}");
            vfs.insert_dir(&host(&dir), meta.clone()).unwrap();
            for f in 0..FILES {
                vfs.insert_file(
                    &host(&format!("{dir}/file{f:04}.dat")),
                    std::sync::Arc::clone(&content),
                    meta.clone(),
                )
                .unwrap();
            }
        }
    }
    vfs
}

fn main() {
    let vfs = build();
    let pool = support::service_pool();
    let rules = FolderRules::default();
    let before = LIVE.load(Ordering::SeqCst);
    let (model, cmp) = support::compare(
        &vfs,
        mspec("/l"),
        mspec("/r"),
        &rules,
        Some(&pool),
        &CancelToken::never(),
        &NullObserver,
    );
    let cmp = cmp.unwrap();
    let after = LIVE.load(Ordering::SeqCst);
    let nodes = cmp.read().len() as usize;
    let (storage, names) = model.reserved_bytes();
    let added = after.saturating_sub(before);
    println!(
        "folder_model_1m: {nodes} nodes; heap added by the scan {added} B = {:.1} B/node; name arenas {names} B; \
         node+folder storage {storage} B = {:.1} B/node; excluding names {:.1} B/node (gate: ≤ 256)",
        added as f64 / nodes as f64,
        storage as f64 / nodes as f64,
        added.saturating_sub(names) as f64 / nodes as f64,
    );
    assert!(
        added.saturating_sub(names) <= 256 * nodes,
        "more than 256 B per node excluding names"
    );
    drop(cmp);
    drop(model);
    let mut c = Criterion::default()
        .configure_from_args()
        .sample_size(10)
        .measurement_time(Duration::from_secs(30));
    c.bench_function("folder_model_1m/scan", |b| {
        b.iter(|| {
            let (_, cmp) = support::compare(
                &vfs,
                mspec("/l"),
                mspec("/r"),
                &rules,
                Some(&pool),
                &CancelToken::never(),
                &NullObserver,
            );
            cmp.map(|c| c.read().len())
        });
    });
    c.final_summary();
}
