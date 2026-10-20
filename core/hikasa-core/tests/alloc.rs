//! `Mask::matches` and `NameFilter::file` (and `folder`, `may_match_below`, `MaskList::matches_name`) allocate
//! nothing for ASCII names (A1.4, §6.2.1). The counting `#[global_allocator]` lives in this test binary only
//! (never in library code, §6.1); it counts per thread, so concurrently running tests cannot disturb the
//! measurement.
// A counting allocator cannot be written without `unsafe` (`GlobalAlloc` is an unsafe trait); the workspace
// lint is `deny`, so this test binary opts out explicitly. Every occurrence carries a `// SAFETY:` comment.
#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use hikasa_core::filter::{FolderVerdict, NameFilter};
use hikasa_core::mask::{CaseSensitivity, Mask, MaskList, MaskOptions};
use hikasa_core::path::RelPath;

struct Counting;

thread_local! {
    static ALLOCATIONS: Cell<u64> = const { Cell::new(0) };
}

fn bump() {
    // `try_with`: the slot may be gone while the thread shuts down.
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

    // SAFETY: `ptr` and `layout` come from this allocator, i.e. from `System`; `new_size` is checked by the
    // caller.
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        bump();
        // SAFETY: forwarded with the caller's guarantees.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn allocations_during(f: impl FnOnce()) -> u64 {
    let before = ALLOCATIONS.with(Cell::get);
    f();
    ALLOCATIONS.with(Cell::get) - before
}

const MASKS: &[&str] = &[
    "*.c",
    "*.H",
    "Makefile",
    "*.",
    "*.*",
    "abc.*",
    "[a-c]?*",
    "[!x]*.txt",
    "test_*",
    "*~",
    "?.md",
    "src\\*.rs",
    ".\\docs\\*.md",
    "...\\tests\\*.rs",
    "src\\...\\mod.rs",
    "\\build\\",
    "obj\\",
    "-*.bak",
    "a*b*c",
    "README*",
];

const NAMES: &[(&str, &str)] = &[
    ("", "main.c"),
    ("src", "lib.rs"),
    ("src/util", "mod.rs"),
    ("docs", "intro.md"),
    ("tests/unit", "props.rs"),
    ("", "Makefile"),
    ("build", "out.o"),
    ("a/b/c/d", "abc.txt"),
    ("", ".bashrc"),
    ("x", "b1"),
];

#[test]
fn ascii_matching_allocates_nothing() {
    for case in [CaseSensitivity::Sensitive, CaseSensitivity::Insensitive] {
        let opts = MaskOptions { case };
        let masks: Vec<Mask> = MASKS
            .iter()
            .map(|m| Mask::parse(m, opts).unwrap())
            .collect();
        let parents: Vec<RelPath> = NAMES
            .iter()
            .map(|(p, _)| RelPath::parse(p).unwrap())
            .collect();
        let paths: Vec<RelPath> = NAMES
            .iter()
            .zip(&parents)
            .map(|((_, n), p)| p.join(n).unwrap())
            .collect();
        let filter = NameFilter::parse(
            "*.c;*.rs;*.md;Makefile;-*.bak;-*~;src\\;docs\\;.\\tests\\unit\\;-build\\;-.git\\",
            opts,
        )
        .unwrap();
        let list = MaskList::parse(&MASKS.join(";"), opts).unwrap();
        let mut hits = 0usize;
        let count = allocations_during(|| {
            for _ in 0..3 {
                for (_, name) in NAMES {
                    hits += usize::from(list.matches_name(name));
                }
                for mask in &masks {
                    for ((parent, (_, name)), path) in parents.iter().zip(NAMES).zip(&paths) {
                        hits += usize::from(mask.matches(parent, name));
                        hits += usize::from(mask.matches_path(path));
                        hits += usize::from(mask.matches_name(name));
                        hits += usize::from(mask.may_match_below(parent, name));
                    }
                }
                for (parent, (_, name)) in parents.iter().zip(NAMES) {
                    for verdict in [
                        FolderVerdict::Base,
                        FolderVerdict::Included,
                        FolderVerdict::PassThrough,
                    ] {
                        hits += usize::from(filter.file(parent, name, verdict));
                        hits += usize::from(
                            filter.folder(parent, name, verdict) == FolderVerdict::Included,
                        );
                    }
                }
            }
        });
        assert_eq!(
            count, 0,
            "allocations while matching ASCII names ({case:?})"
        );
        assert!(hits > 0);
    }
}

#[test]
fn nfc_names_allocate_nothing_either() {
    let opts = MaskOptions {
        case: CaseSensitivity::Insensitive,
    };
    let mask = Mask::parse("caf\u{e9}*", opts).unwrap();
    let parent = RelPath::parse("d\u{e9}j\u{e0}").unwrap();
    let count = allocations_during(|| {
        assert!(mask.matches(&parent, "CAF\u{c9}.txt"));
        assert!(!mask.matches(&parent, "tea"));
    });
    assert_eq!(count, 0);
}

#[test]
fn the_counter_counts() {
    let count = allocations_during(|| {
        let v = vec![1u8; 64];
        std::hint::black_box(v);
    });
    assert!(count >= 1);
}
