//! M2 W6c acceptance A6.32, the memory budgets of §1.6.22 (D-192): a `SyncRow` is at most 16 bytes, and an `OpPlan` of
//! 100,000 items holds less than 64 MiB, on a counting allocator. One test in this binary, so no other test allocates
//! while it counts.
//!
//! The counting allocator is the §6.1 exemption for `unsafe` in test and bench binaries.
// A counting allocator cannot be written without `unsafe` (`GlobalAlloc` is an unsafe trait); the workspace lint is `deny`,
// so this test binary opts out explicitly. Every occurrence carries a `// SAFETY:` comment.
#![allow(unsafe_code)]

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use common::{compared, host};
use hikasa_core::{CaseSensitivity, Ctl, FileTime, Side};
use hikasa_folder::FolderRules;
use hikasa_folder::ops::{FolderOp, OpScope, OpSettings, ReadOnlySides, plan_op};
use hikasa_folder::select::{Include, Selection, SideSet};
use hikasa_folder::sync::{
    SyncOptions, SyncPreset, SyncRequest, SyncRow, plan_sync, sync_folder_rules,
};
use hikasa_vfs::ops::OverwritePolicy;
use hikasa_vfs::{MemMeta, MemoryVfs};

/// Counts live heap bytes.
struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);

// SAFETY: every method forwards to the system allocator with the caller's arguments unchanged, so the `GlobalAlloc`
// contract (valid layouts, pointers returned by this allocator) is upheld by `System`; the counter is only bookkeeping.
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

    // SAFETY: `ptr` and `layout` come from this allocator, i.e. from `System`; `new_size` is checked by the caller.
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
const FILES: usize = 100;
const LIMIT: usize = 64 << 20;

/// 100,000 files in 1,000 folders on the left; the right base empty.
fn tree() -> MemoryVfs {
    let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
    let meta = MemMeta {
        mtime: FileTime::from_unix(1_790_000_000, 0),
        ..MemMeta::default()
    };
    let content: std::sync::Arc<[u8]> = std::sync::Arc::from(&b"content of a file"[..]);
    vfs.insert_dir(&host("/r"), meta.clone()).unwrap();
    vfs.insert_dir(&host("/l"), meta.clone()).unwrap();
    for d in 0..FOLDERS {
        let dir = format!("/l/folder{d:04}");
        vfs.insert_dir(&host(&dir), meta.clone()).unwrap();
        for f in 0..FILES {
            vfs.insert_file(
                &host(&format!("{dir}/file{f:03}.dat")),
                std::sync::Arc::clone(&content),
                meta.clone(),
            )
            .unwrap();
        }
    }
    vfs
}

#[test]
fn a6_32_sync_row_size_and_a_plan_of_100k_items_in_memory() {
    assert!(
        size_of::<SyncRow>() <= 16,
        "SyncRow is {} bytes",
        size_of::<SyncRow>()
    );
    let vfs = tree();
    let options = SyncOptions {
        preset: SyncPreset::MirrorRight,
        ..SyncOptions::default()
    };
    let cmp = compared(
        &vfs,
        &sync_folder_rules(&FolderRules::default(), &options).unwrap(),
    );
    let read = cmp.read();
    // `folder op copy` of everything: 101,000 items.
    let mut selection = Selection::default();
    for row in read.children(read.root()) {
        selection.insert(row, SideSet::LEFT);
    }
    let scope = OpScope {
        selection,
        include: Include::visible(),
        iu: false,
        read_only: ReadOnlySides::default(),
    };
    let op = FolderOp::Copy {
        to: Side::Right,
        overwrite: OverwritePolicy::Always,
    };
    let before = LIVE.load(Ordering::SeqCst);
    let plan = plan_op(
        &read,
        &vfs,
        &scope,
        &op,
        &OpSettings::default(),
        &Ctl::detached(),
    )
    .unwrap();
    let held = LIVE.load(Ordering::SeqCst).saturating_sub(before);
    let items = plan.plan.items.len();
    eprintln!(
        "ops_memory: OpPlan of {items} items holds {held} B = {:.1} MiB, {:.0} B per item (limit 64 MiB)",
        held as f64 / f64::from(1 << 20),
        held as f64 / items as f64
    );
    assert!(items >= 100_000, "{items} items");
    assert!(held < LIMIT, "an OpPlan of {items} items holds {held} B");
    drop(plan);
    // The sync plan of the same tree: its rows and its batch.
    let request = SyncRequest {
        options: &options,
        just_selection: None,
        visible: None,
        include_hidden: false,
        iu: false,
        read_only: ReadOnlySides::default(),
        allow_empty_source: false,
        settings: OpSettings::default(),
    };
    let before = LIVE.load(Ordering::SeqCst);
    let sync = plan_sync(&read, &vfs, &request, &Ctl::detached()).unwrap();
    let held = LIVE.load(Ordering::SeqCst).saturating_sub(before);
    let items = sync.op.plan.items.len();
    eprintln!(
        "ops_memory: SyncPlan of {} rows and {items} items holds {held} B = {:.1} MiB (limit 64 MiB)",
        sync.rows.len(),
        held as f64 / f64::from(1 << 20)
    );
    assert!(items >= 100_000);
    assert!(held < LIMIT, "a SyncPlan of {items} items holds {held} B");
}
