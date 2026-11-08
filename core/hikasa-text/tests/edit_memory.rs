//! A10.12 (D-271, CI share of G8 memory) and A10.1's memory table: a counting allocator measures the live heap.
//! - the line table holds 5 bytes a line plus at most 0.1 for headers and prefix sums, and 8 more a line only in
//!   blocks an edit touched (1,000,000 lines, 1,000 edits in distinct blocks);
//! - a live comparison of 100,000 lines grows by at most 10 MiB over 1,000 edits with their `note_edit`s and a
//!   recompare after every 50 (the undo log included).
// A counting allocator cannot be written without `unsafe` (`GlobalAlloc` is an unsafe trait); as in the other
// memory tests of the workspace, this test binary opts out explicitly and every occurrence carries a SAFETY note.
#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Arc;
use std::sync::atomic::{AtomicIsize, Ordering};

use hikasa_core::{Ctl, Pair, Side};
use hikasa_text::edit::{
    EditOptions, EditRequest, Editability, EditableDocument, GroupHint, TextPos,
};
use hikasa_text::{
    CompareInput, CompiledFormat, LineIdx, RegexLimits, TextDocument, TextRules, compare_live,
};

struct Counting;

static LIVE: AtomicIsize = AtomicIsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        LIVE.fetch_add(layout.size() as isize, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size() as isize, Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        LIVE.fetch_add(
            new_size as isize - layout.size() as isize,
            Ordering::Relaxed,
        );
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn live_bytes() -> isize {
    LIVE.load(Ordering::Relaxed)
}

/// A line of about 48 bytes.
fn line(k: u64) -> String {
    format!(
        "    value_{k:08} = compute(alpha, beta, {:06});\n",
        k * 7 % 999_983
    )
}

#[test]
fn the_line_table_memory_table() {
    let text: String = (0..1_000_000u64).map(line).collect();
    let doc = TextDocument::from_text(&text);
    drop(text);
    let lines = f64::from(doc.line_count());
    let table = doc.line_table_bytes() as f64;
    assert!(table / lines <= 5.1, "{:.3} bytes a line", table / lines);
    let mut ed =
        EditableDocument::new(Arc::new(doc), EditOptions::default(), Editability::EDITABLE);
    // 1,000 edits in distinct blocks (512 lines each at load).
    for k in 0..1_000u32 {
        let at = TextPos::new(k * 997 % 1_000_000 / 512 * 512 + 3, 2);
        ed.apply(
            &EditRequest::Replace {
                range: at..at,
                text: "x".into(),
                keep_eols: false,
            },
            GroupHint::None,
        )
        .unwrap();
    }
    let touched = 1_000.0 * 512.0;
    let after = ed.document().line_table_bytes() as f64;
    let growth = after - table;
    assert!(
        growth <= 8.0 * touched * 1.05,
        "marks: {growth} bytes for {touched} lines in touched blocks"
    );
}

#[test]
fn a_live_comparison_grows_by_less_than_10_mib_over_1000_edits() {
    let left: String = (0..100_000u64).map(line).collect();
    let right: String = (0..100_000u64)
        .map(|k| {
            if k % 101 == 0 {
                format!("changed {k}\n")
            } else {
                line(k)
            }
        })
        .collect();
    let ed = |t: &str| {
        EditableDocument::new(
            Arc::new(TextDocument::from_text(t)),
            EditOptions::default(),
            Editability::EDITABLE,
        )
    };
    let (mut l, r) = (ed(&left), ed(&right));
    drop((left, right));
    let mut live = compare_live(
        CompareInput {
            left: l.snapshot(),
            right: r.snapshot(),
            format: Arc::new(CompiledFormat::everything_else()),
            rules: TextRules::default(),
            bytes_identical: false,
            force_tokenize: false,
            anchors: Vec::new(),
        },
        &RegexLimits::default(),
        None,
        &Ctl::detached(),
        None,
    )
    .unwrap();
    let before = live_bytes();
    let mut x = 0x9E37_79B9u64;
    for k in 0..1_000 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let line_no = (x % 100_000) as u32;
        let len = l.line_len(LineIdx(line_no));
        let at = TextPos::new(line_no, len / 2);
        let out = l
            .apply(
                &EditRequest::Replace {
                    range: at..at,
                    text: "x".into(),
                    keep_eols: false,
                },
                GroupHint::None,
            )
            .unwrap();
        live.note_edit(Side::Left, &l, &out);
        if k % 50 == 49 {
            let done = live
                .recompare_job(Pair::new(&l, &r))
                .run(None, &Ctl::detached())
                .unwrap();
            live.install(done, Pair::new(&l, &r));
        }
    }
    let growth = live_bytes() - before;
    println!(
        "live heap growth over 1,000 edits: {:.2} MiB",
        growth as f64 / 1048576.0
    );
    assert!(growth < 10 * 1024 * 1024, "{growth} bytes");
}
