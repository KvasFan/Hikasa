//! Bench `mask_match` (gate §6.4: ≥ 20 M matches/s on a 20-item list; zero allocations for ASCII names).
//!
//! Inputs are generated with a seeded xorshift64* (§6.1): 4096 ASCII items at depth 0–4, a 20-item toolbar
//! filter mixing simple, extension, set, relative, anchored and `...` masks, and a 20-item format mask list.
//!
//! The gate is read in its strict sense: one "match" is one ASCII name checked against a **whole** 20-item list,
//! so the gated throughputs are the `*_20_items_*` functions, reported in names per second
//! (`NameFilter::file` and `NameFilter::folder` on the toolbar filter, `MaskList::matches_name` on the format
//! list). `single_mask_*` reports the throughput of single `Mask::matches(parent, name)` calls over the same
//! masks (20 calls per item) for comparison. Before measuring, one full pass runs under a counting
//! `#[global_allocator]` (this bench binary only, §6.1) and must allocate nothing; `tests/alloc.rs` asserts the
//! same in the regular test run.
// A counting allocator cannot be written without `unsafe` (`GlobalAlloc` is an unsafe trait); the workspace
// lint is `deny`, so this bench binary opts out explicitly. Every occurrence carries a `// SAFETY:` comment.
#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;
use std::time::Duration;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use hikasa_core::filter::{FolderVerdict, NameFilter};
use hikasa_core::mask::{CaseSensitivity, Mask, MaskList, MaskOptions};
use hikasa_core::path::RelPath;

struct Counting;

thread_local! {
    static ALLOCATIONS: Cell<u64> = const { Cell::new(0) };
}

fn bump() {
    let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
}

// SAFETY: every method forwards to `System`, which upholds the `GlobalAlloc` contract; the counter is a
// const-initialised thread-local `Cell` without destructor, so counting never allocates or re-enters.
unsafe impl GlobalAlloc for Counting {
    // SAFETY: the caller's guarantees for `layout` are passed on to `System::alloc` unchanged.
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        bump();
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
        bump();
        // SAFETY: forwarded with the caller's guarantees.
        unsafe { System.alloc_zeroed(layout) }
    }

    // SAFETY: `ptr` and `layout` come from this allocator; `new_size` is checked by the caller.
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        bump();
        // SAFETY: forwarded with the caller's guarantees.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// The 20-item list.
const MASKS: [&str; 20] = [
    "*.c",
    "*.h",
    "*.rs",
    "*.txt",
    "Makefile",
    "*.",
    "*.*",
    "README*",
    "test_*.py",
    "[a-f]*.o",
    "[!x]?.md",
    "*~",
    "src\\",
    "build\\",
    "src\\*.c",
    ".\\docs\\*.md",
    "...\\tests\\*.rs",
    "src\\...\\mod.rs",
    "-*.bak",
    "*.tar.gz",
];

/// The 20-item format mask list (file-format detection, D-10).
const FORMAT_MASKS: &str = "*.c;*.h;*.cpp;*.hpp;*.cc;*.rs;*.py;*.js;*.ts;*.java;*.go;*.rb;*.sh;*.pl;*.txt;*.md;*.toml;*.json;*.xml;*.csv";

/// Seeded xorshift64* (no `rand` crate).
struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[(self.next() % items.len() as u64) as usize]
    }
}

fn inputs(count: usize) -> Vec<(RelPath, String)> {
    const FOLDERS: [&str; 12] = [
        "src", "lib", "tests", "docs", "build", "util", "core", "a", "b", "vendor", "x86", "io",
    ];
    const STEMS: [&str; 14] = [
        "main",
        "lib",
        "mod",
        "util",
        "test_parse",
        "README",
        "Makefile",
        "x",
        "b1",
        "data",
        "notes",
        "abc",
        "e",
        "f",
    ];
    const EXTS: [&str; 12] = [
        ".c", ".h", ".rs", ".txt", ".md", ".o", ".py", "", ".tar.gz", ".bak", "~", ".json",
    ];
    let mut rng = XorShift(0x9E37_79B9_7F4A_7C15);
    (0..count)
        .map(|_| {
            let depth = (rng.next() % 5) as usize;
            let mut parent = RelPath::root();
            for _ in 0..depth {
                parent = parent.join(rng.pick(&FOLDERS)).unwrap();
            }
            let name = format!("{}{}", rng.pick(&STEMS), rng.pick(&EXTS));
            (parent, name)
        })
        .collect()
}

/// One full pass of every mask over every item must not allocate (the second half of the gate).
fn assert_zero_allocations(items: &[(RelPath, String)]) {
    for case in [CaseSensitivity::Insensitive, CaseSensitivity::Sensitive] {
        let opts = MaskOptions { case };
        let masks: Vec<Mask> = MASKS
            .iter()
            .map(|m| Mask::parse(m, opts).unwrap())
            .collect();
        let filter = NameFilter::parse(&MASKS.join(";"), opts).unwrap();
        let formats = MaskList::parse(FORMAT_MASKS, opts).unwrap();
        let before = ALLOCATIONS.with(Cell::get);
        let mut hits = 0usize;
        for (parent, name) in items {
            for mask in &masks {
                hits += usize::from(mask.matches(parent, name));
            }
            hits += usize::from(filter.file(parent, name, FolderVerdict::Included));
            hits += usize::from(
                filter.folder(parent, name, FolderVerdict::Base) == FolderVerdict::Included,
            );
            hits += usize::from(formats.matches_name(name));
        }
        let allocations = ALLOCATIONS.with(Cell::get) - before;
        black_box(hits);
        assert_eq!(
            allocations, 0,
            "mask_match gate: allocations while matching ASCII names ({case:?})"
        );
    }
}

fn bench(c: &mut Criterion) {
    let items = inputs(4096);
    assert_zero_allocations(&items);
    let mut group = c.benchmark_group("mask_match");
    for case in [CaseSensitivity::Insensitive, CaseSensitivity::Sensitive] {
        let opts = MaskOptions { case };
        let label = format!("{case:?}").to_lowercase();
        let filter = NameFilter::parse(&MASKS.join(";"), opts).unwrap();
        // Gated: names per second, each checked against the whole 20-item filter.
        group.throughput(Throughput::Elements(items.len() as u64));
        group.bench_function(format!("name_filter_file_20_items_{label}"), |b| {
            b.iter(|| {
                items
                    .iter()
                    .filter(|(parent, name)| {
                        filter.file(black_box(parent), black_box(name), FolderVerdict::Included)
                    })
                    .count()
            })
        });
        group.bench_function(format!("name_filter_folder_20_items_{label}"), |b| {
            b.iter(|| {
                items
                    .iter()
                    .filter(|(parent, name)| {
                        filter.folder(black_box(parent), black_box(name), FolderVerdict::Base)
                            != FolderVerdict::Excluded
                    })
                    .count()
            })
        });
        let formats = MaskList::parse(FORMAT_MASKS, opts).unwrap();
        assert_eq!(formats.masks().len(), 20);
        group.bench_function(format!("format_list_20_items_matches_name_{label}"), |b| {
            b.iter(|| {
                items
                    .iter()
                    .filter(|(_, name)| formats.matches_name(black_box(name)))
                    .count()
            })
        });
        // For comparison: single mask evaluations, 20 per item.
        let masks: Vec<Mask> = MASKS
            .iter()
            .map(|m| Mask::parse(m, opts).unwrap())
            .collect();
        group.throughput(Throughput::Elements((items.len() * masks.len()) as u64));
        group.bench_function(format!("single_mask_20_masks_x_4096_items_{label}"), |b| {
            b.iter(|| {
                let mut hits = 0usize;
                for (parent, name) in &items {
                    for mask in &masks {
                        hits += usize::from(mask.matches(black_box(parent), black_box(name)));
                    }
                }
                hits
            })
        });
    }
    group.finish();
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(30).warm_up_time(Duration::from_secs(1)).measurement_time(Duration::from_secs(3));
    targets = bench
}
criterion_main!(benches);
