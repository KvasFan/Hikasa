//! Allocation bound of the budget fallback (§6.4 `align_pathological`, INV-DENSE-1): with ≥ 10,000 anchor-less
//! gaps pending when Standard's budget runs out, every gap becomes one imara-diff call. Each call allocates
//! O(its alphabet) because the tokens are remapped densely first (D-16); without the remap every call would
//! allocate O(max token id) and the total would grow with gaps × distinct keys (quadratically here).
//!
//! The counting `#[global_allocator]` lives in this test binary only (§6.1) and counts per thread; the fallback
//! calls are small, so they all run inline on the test thread.
// A counting allocator cannot be written without `unsafe` (`GlobalAlloc` is an unsafe trait); the workspace
// lint is `deny`, so this test binary opts out explicitly. Every occurrence carries a `// SAFETY:` comment.
#![allow(unsafe_code)]

#[path = "../benches/support/inputs.rs"]
mod inputs;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use hikasa_core::{CancelToken, WarningCode};
use hikasa_diff::{Algorithm, AlignOptions, align};
use inputs::{Generated, HashScorer, pathological};

struct Counting;

thread_local! {
    static BYTES: Cell<u64> = const { Cell::new(0) };
}

fn count(bytes: usize) {
    // `try_with`: the slot may be gone while the thread shuts down.
    let _ = BYTES.try_with(|total| total.set(total.get() + bytes as u64));
}

// SAFETY: every method forwards to `System`, which upholds the `GlobalAlloc` contract; the counter is a
// const-initialised thread-local `Cell` without destructor, so counting never allocates or re-enters.
unsafe impl GlobalAlloc for Counting {
    // SAFETY: the caller's guarantees for `layout` are passed on to `System::alloc` unchanged.
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count(layout.size());
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
        count(layout.size());
        // SAFETY: forwarded with the caller's guarantees.
        unsafe { System.alloc_zeroed(layout) }
    }

    // SAFETY: `ptr` and `layout` come from this allocator, i.e. from `System`; `new_size` is checked by the
    // caller.
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count(new_size);
        // SAFETY: forwarded with the caller's guarantees.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// Bytes allocated by one Standard alignment whose budget runs out right after the top-level split.
fn fallback_bytes(input: &Generated) -> u64 {
    let opts = AlignOptions {
        // The top level costs one unit per token plus a few per anchor; the first gap then exhausts the rest,
        // so nearly every gap is aligned by the Histogram fallback.
        work_budget: Some(input.tokens() as u64 + 6 * input.gaps as u64),
        ..AlignOptions::with_algorithm(Algorithm::Standard)
    };
    let scorer = HashScorer(input);
    let before = BYTES.with(Cell::get);
    let alignment = align(
        &input.input(),
        &opts,
        Some(&scorer),
        None,
        &CancelToken::never(),
    )
    .unwrap();
    let bytes = BYTES.with(Cell::get) - before;
    assert!(
        alignment
            .warnings()
            .iter()
            .any(|w| w.code == WarningCode::WorkBudgetExhausted),
        "the budget must run out"
    );
    alignment.check_invariants(&input.input(), &opts).unwrap();
    drop(alignment);
    bytes
}

#[test]
fn the_budget_fallback_allocates_linearly() {
    let small = pathological(10_000, 20, 21);
    let large = pathological(20_000, 20, 21);
    assert!(small.gaps >= 10_000);
    let small_bytes = fallback_bytes(&small);
    let large_bytes = fallback_bytes(&large);
    let per_token = small_bytes as f64 / small.tokens() as f64;
    // Linear: about 50 B per token (runs, the remap tables, imara-diff's per-call buffers, closeness tables).
    assert!(
        small_bytes <= 96 * small.tokens() as u64,
        "{small_bytes} bytes for {} tokens ({per_token:.1} B/token)",
        small.tokens()
    );
    // Twice the gaps (and twice the distinct keys): about twice the bytes, not four times.
    let ratio = large_bytes as f64 / small_bytes as f64;
    assert!(
        (1.5..2.6).contains(&ratio),
        "bytes grew by {ratio:.2} for twice the input"
    );
}
