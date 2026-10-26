//! Allocation count of `tokenize_all` (§6.2.4, §6.4 gate: ≤ 4 allocations per 8192-line chunk): one byte arena
//! and one end-offset vector per chunk, no per-line allocation, and a bounded number of per-task tokenizers.
//! The counting `#[global_allocator]` lives in this test binary only (never in library code, §6.1); the binary
//! holds a single test so that nothing else allocates while it measures.
// A counting allocator cannot be written without `unsafe` (`GlobalAlloc` is an unsafe trait); the workspace
// lint is `deny`, so this test binary opts out explicitly. Every occurrence carries a `// SAFETY:` comment.
#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};

use hikasa_core::cancel::CancelToken;
use hikasa_formats::{FormatId, FormatRegistry, LineSource, Token, tokenize_all};

struct Counting;

static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);

// SAFETY: every method forwards to `System`, which upholds the `GlobalAlloc` contract; counting is a relaxed
// atomic increment that neither allocates nor re-enters the allocator.
unsafe impl GlobalAlloc for Counting {
    // SAFETY: the caller's guarantees for `layout` are passed on to `System::alloc` unchanged.
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
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
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: forwarded with the caller's guarantees.
        unsafe { System.alloc_zeroed(layout) }
    }

    // SAFETY: `ptr` and `layout` come from this allocator, i.e. from `System`; `new_size` is checked by the
    // caller.
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: forwarded with the caller's guarantees.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn key(_: u32, text: &str, tokens: &[Token], out: &mut Vec<u8>) {
    out.reserve(text.len() + tokens.len());
    for t in tokens {
        out.push(t.element.0 as u8);
        out.extend_from_slice(t.text(text).as_bytes());
    }
}

/// Allocations per chunk of `tokenize_all` over `lines` (16 chunks of 8192 lines), sequential and on a pool.
fn per_chunk(
    grammar: &hikasa_formats::CompiledGrammar,
    lines: &dyn LineSource,
    pool: &rayon::ThreadPool,
) {
    let chunks = lines.line_count() as usize / 8192;
    let cancel = CancelToken::never();
    for pool in [None, Some(pool)] {
        // Warm-up: regex caches and the pool's threads.
        drop(tokenize_all(grammar, lines, pool, &cancel, key).unwrap());
        let before = ALLOCATIONS.load(Ordering::Relaxed);
        let out = tokenize_all(grammar, lines, pool, &cancel, key).unwrap();
        let used = ALLOCATIONS.load(Ordering::Relaxed) - before;
        assert_eq!(out.bytes.len(), lines.line_count());
        drop(out);
        let per_chunk = used as f64 / chunks as f64;
        assert!(
            per_chunk <= 4.0,
            "{used} allocations for {chunks} chunks ({per_chunk:.2} per chunk), pool: {}",
            pool.is_some()
        );
    }
}

#[test]
fn at_most_four_allocations_per_chunk() {
    let registry = FormatRegistry::builtin().unwrap();
    let c = registry.compiled(&FormatId::new("c-cpp").unwrap()).unwrap();
    let chunks = 16usize;
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .unwrap();
    let lines: Vec<String> = (0..chunks * 8192)
        .map(|k| match k % 11 {
            0 => "/* a block comment that".to_owned(),
            1 => "   continues here */ int y = 0x1F;".to_owned(),
            2 => "#include <stdio.h>".to_owned(),
            3 => format!("    printf(\"%d\\n\", value_{k}); // print"),
            _ => format!("    unsigned long total_{k} = compute({k}, &state) + 42;"),
        })
        .collect();
    per_chunk(c.grammar(), &lines, &pool);
    drop(lines);
    // Every chunk starts with 64 blank lines: the arena estimate made after them is far too small, and the
    // later checkpoints re-estimate it (wave-2 review: 9.25 allocations per chunk before).
    let lines: Vec<String> = (0..chunks * 8192)
        .map(|k| {
            if k % 8192 < 64 {
                String::new()
            } else {
                "    unsigned long total = compute(42, &state) + 7; // x".to_owned()
            }
        })
        .collect();
    per_chunk(c.grammar(), &lines, &pool);
}
