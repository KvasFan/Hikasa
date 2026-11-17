//! M2 W10b acceptance A10.14, second part (D-273…D-277): save plans and their execution.
//!
//! - The dry run: the plan's JSON and its human lines are golden (`tests/fixtures/save/`).
//! - The executed plan is the planned one; several documents are one plan.
//! - `SaveSource` streams the splice source in steps of at most 1 MiB, and the heap of a save of a 48 MB document
//!   peaks at most 8 MiB above the edited text (a counting allocator; on disk).
//! - A source changed between plan and execution (same size and time, other bytes: the CRC at the end of the read)
//!   fails the item before the commit; a document changed since the plan is refused.
//! - `saved()` rebases the marks and the stamp: the file written is the next splice source.
//! - The questions of §1.10.14 through the W6 machine; targets (links followed, special files refused, Save As,
//!   `--savetarget`, no location); backups (one generation, `OverwriteBackup` only for a file the document's saves did
//!   not make; members of one archive: one backup of the archive); the External save conversion.
// A counting allocator cannot be written without `unsafe` (`GlobalAlloc` is an unsafe trait); as in the other memory
// tests of the workspace, this test binary opts out explicitly and every occurrence carries a SAFETY note.
#![allow(unsafe_code)]

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::ffi::OsString;
use std::io::{Read, Seek, SeekFrom};
use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::save::{confirm_all, execute, open, plan, save, spec, text_format};
use hikasa_core::confirm::{Ask, ConfirmMode, Reply};
use hikasa_core::{CancelToken, CaseSensitivity, Ctl, FileTime, Spec, WarningCode};
use hikasa_formats::{
    ConvertError, DecodeOptions, EncodingSetting, ExternalConversion, ProcessRunner, TextEncoding,
};
use hikasa_text::edit::{
    EditOptions, EditRequest, Editability, EditableDocument, GroupHint, TextPos,
};
use hikasa_text::save::{
    ReencodeReason, SaveConfirmation, SaveEnv, SaveIssue, SaveMode, SavePolicy, SaveQuestion,
    SaveRequest, SaveSource, UnmappablePolicy, execute_save, plan_save, plan_save_with,
};
use hikasa_text::{LineIdx, LineMark, TextDocument};
use hikasa_vfs::ops::{BackupPolicy, BackupStyle, ItemStatus, SkipReason};
use hikasa_vfs::{
    AtomicWrite, Attrs, Capabilities, DosAttrs, Entry, FileName, FollowLinks, ListOptions, Listing,
    LocalVfs, MemMeta, MemoryVfs, ReadSeek, UnixMeta, Vfs, VfsError, WriteOptions,
};

// ---- the counting allocator ------------------------------------------------------------------------------------

struct Counting;

static LIVE: AtomicIsize = AtomicIsize::new(0);
static PEAK: AtomicIsize = AtomicIsize::new(0);

// SAFETY: every method forwards to `System`, which upholds the `GlobalAlloc` contract; the counters are static atomics,
// so counting never allocates or re-enters.
unsafe impl GlobalAlloc for Counting {
    // SAFETY: the caller's guarantees for `layout` are passed on to `System::alloc` unchanged.
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let now =
            LIVE.fetch_add(layout.size() as isize, Ordering::Relaxed) + layout.size() as isize;
        PEAK.fetch_max(now, Ordering::Relaxed);
        // SAFETY: forwarded with the caller's guarantees.
        unsafe { System.alloc(layout) }
    }
    // SAFETY: `ptr` and `layout` come from `alloc`/`realloc` above, as `System::dealloc` requires.
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size() as isize, Ordering::Relaxed);
        // SAFETY: forwarded with the caller's guarantees.
        unsafe { System.dealloc(ptr, layout) }
    }
    // SAFETY: the caller's guarantees for `ptr`, `layout` and `new_size` are passed on unchanged.
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let delta = new_size as isize - layout.size() as isize;
        let now = LIVE.fetch_add(delta, Ordering::Relaxed) + delta;
        PEAK.fetch_max(now, Ordering::Relaxed);
        // SAFETY: forwarded with the caller's guarantees.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

// ---- helpers ---------------------------------------------------------------------------------------------------

fn memory() -> MemoryVfs {
    let vfs = MemoryVfs::new(CaseSensitivity::Sensitive);
    vfs.insert_dir(&common::host("/w"), MemMeta::default())
        .unwrap();
    vfs
}

fn meta(secs: i64) -> MemMeta {
    MemMeta {
        mtime: FileTime::from_unix(secs, 0),
        attrs: Some(Attrs::Unix {
            meta: UnixMeta {
                mode: 0o644,
                uid: 0,
                gid: 0,
            },
        }),
        ..MemMeta::default()
    }
}

fn put(vfs: &MemoryVfs, path: &str, bytes: &[u8]) {
    vfs.insert_file(&common::host(path), bytes.to_vec(), meta(1_700_000_000))
        .unwrap();
}

fn read(vfs: &MemoryVfs, path: &str) -> Option<Vec<u8>> {
    vfs.read_back(&common::host(path)).map(|b| b.to_vec())
}

fn ee() -> hikasa_formats::CompiledFormat {
    text_format(EncodingSetting::Auto { fallback: None }, false)
}

fn doc_of(vfs: &dyn Vfs, path: &str) -> EditableDocument {
    open(vfs, &spec(path), &ee(), DecodeOptions::default(), None)
}

/// Inserts `text` at `(line, byte)`.
fn type_at(doc: &mut EditableDocument, line: u32, byte: u32, text: &str) {
    let at = TextPos::new(line, byte);
    doc.apply(
        &EditRequest::Replace {
            range: at..at,
            text: text.into(),
            keep_eols: false,
        },
        GroupHint::None,
    )
    .unwrap();
}

fn name_bak() -> BackupPolicy {
    BackupPolicy {
        style: BackupStyle::NameBak,
        folder: None,
    }
}

fn with_backup() -> SaveRequest {
    SaveRequest {
        backup: Some(name_bak()),
        ..SaveRequest::default()
    }
}

/// The text of a JSON value with host paths written as `/`-separated ones (a Windows `C:\w\a` is `/w/a`).
fn portable(text: &str) -> String {
    text.replace("C:\\\\", "/").replace("\\\\", "/")
}

// ---- the dry run -----------------------------------------------------------------------------------------------

/// A plan of four documents: an edited file in place with a backup, a file through a link, a Save As to a new file, a
/// text pair saved to an existing file. Its JSON and its human lines are golden.
#[test]
fn a10_14_the_dry_run_is_golden() {
    let vfs = memory();
    put(&vfs, "/w/a.txt", b"alpha\nbeta\ngamma\n");
    put(&vfs, "/w/real.txt", b"one\r\ntwo\r\n");
    vfs.insert_symlink(
        &common::host("/w/link.txt"),
        &common::host("/w/real.txt"),
        MemMeta::default(),
    )
    .unwrap();
    put(&vfs, "/w/b.txt", b"b\n");
    put(&vfs, "/w/old.txt", b"old\n");
    let mut a = doc_of(&vfs, "/w/a.txt");
    type_at(&mut a, 1, 4, "!");
    let mut link = doc_of(&vfs, "/w/link.txt");
    type_at(&mut link, 0, 3, "+");
    let b = doc_of(&vfs, "/w/b.txt");
    let pair = EditableDocument::new(
        Arc::new(TextDocument::from_text("from a pair\n")),
        EditOptions::default(),
        Editability {
            blocks: Vec::new(),
            can_save_in_place: false,
        },
    );
    let save_as = SaveRequest {
        target: Some(spec("/w/new.txt")),
        ..SaveRequest::default()
    };
    let over = SaveRequest {
        target: Some(spec("/w/old.txt")),
        backup: Some(name_bak()),
        ..SaveRequest::default()
    };
    let backup = with_backup();
    let plan = plan_save(
        &vfs,
        &[
            (Some(hikasa_core::Side::Left), &a, &backup),
            (
                Some(hikasa_core::Side::Right),
                &link,
                &SaveRequest::default(),
            ),
            (None, &b, &save_as),
            (None, &pair, &over),
        ],
        &Ctl::detached(),
    )
    .unwrap();
    assert!(!plan.confirmed, "the Save As over old.txt asks");
    let json = serde_json::to_string_pretty(&plan).unwrap();
    common::assert_golden(
        &common::fixtures("save").join("plan.json"),
        format!("{}\n", portable(&json)).as_bytes(),
    );
    let mut lines = plan.human_lines();
    let machine = SaveConfirmation::new(&plan, &SavePolicy::ALL, ConfirmMode::Prompt);
    lines.extend(machine.question().map(|q| format!("? {q}")));
    let text = portable(&lines.join("\n").replace('\\', "\\\\"));
    common::assert_golden(
        &common::fixtures("save").join("plan.txt"),
        format!("{text}\n").as_bytes(),
    );
    // The plan reads back.
    let back: hikasa_text::save::SavePlan = serde_json::from_str(&json).unwrap();
    assert_eq!(back, plan);
}

/// The executed plan is the planned one: every item done, the bytes as planned, one plan for several documents.
#[test]
fn a10_14_the_executed_plan_is_the_planned_one() {
    let vfs = memory();
    put(&vfs, "/w/a.txt", b"alpha\nbeta\ngamma\n");
    put(&vfs, "/w/b.txt", b"one\r\ntwo\r\n");
    let mut a = doc_of(&vfs, "/w/a.txt");
    let mut b = doc_of(&vfs, "/w/b.txt");
    type_at(&mut a, 2, 0, "new line\n");
    type_at(&mut b, 1, 3, "!");
    let req = with_backup();
    let planned = plan_save(
        &vfs,
        &[(None, &a, &req), (None, &b, &req)],
        &Ctl::detached(),
    )
    .unwrap();
    assert!(planned.confirmed);
    assert_eq!(planned.plan.items.len(), 2);
    assert_eq!(planned.plan.bytes(), 26 + 11);
    let source = SaveSource::new(&vfs, &planned, &[&a, &b]);
    let report = execute_save(&vfs, planned.clone(), source, &Ctl::detached());
    assert!(report.ok(), "{:?}", report.op.results);
    assert_eq!(report.op.plan, planned.plan);
    assert_eq!(report.op.summary.bytes, 26 + 11);
    assert_eq!(
        read(&vfs, "/w/a.txt").unwrap(),
        b"alpha\nbeta\nnew line\ngamma\n"
    );
    assert_eq!(read(&vfs, "/w/b.txt").unwrap(), b"one\r\ntwo!\r\n");
    assert_eq!(read(&vfs, "/w/a.txt.bak").unwrap(), b"alpha\nbeta\ngamma\n");
    let item = &planned.items[0];
    assert_eq!(item.content.spliced_bytes, 17);
    assert_eq!(item.content.encoded_lines, 1);
    for (i, stamp) in report.written.iter().enumerate() {
        let stamp = stamp.unwrap();
        let path = ["/w/a.txt", "/w/b.txt"][i];
        let bytes = read(&vfs, path).unwrap();
        assert_eq!(stamp.size, bytes.len() as u64);
        assert_eq!(stamp.crc32, hikasa_vfs::crc32_bytes(&bytes));
    }
}

// ---- streaming and memory --------------------------------------------------------------------------------------

/// A `LocalVfs` that records the largest read from the files it opens.
struct Recording {
    inner: LocalVfs,
    largest: Arc<AtomicUsize>,
}

struct RecordingReader {
    inner: Box<dyn ReadSeek>,
    largest: Arc<AtomicUsize>,
}

impl Read for RecordingReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.largest.fetch_max(n, Ordering::Relaxed);
        Ok(n)
    }
}

impl Seek for RecordingReader {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(to)
    }
}

impl Vfs for Recording {
    fn capabilities(&self, at: &Spec) -> Result<Capabilities, VfsError> {
        self.inner.capabilities(at)
    }
    fn stat(&self, spec: &Spec, follow: FollowLinks) -> Result<Entry, VfsError> {
        self.inner.stat(spec, follow)
    }
    fn list(
        &self,
        dir: &Spec,
        opts: &ListOptions<'_>,
        cancel: &CancelToken,
    ) -> Result<Listing, VfsError> {
        self.inner.list(dir, opts, cancel)
    }
    fn open_read(&self, spec: &Spec) -> Result<Box<dyn ReadSeek>, VfsError> {
        Ok(Box::new(RecordingReader {
            inner: self.inner.open_read(spec)?,
            largest: Arc::clone(&self.largest),
        }))
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
    fn rename(&self, from: &Spec, to: &Spec, replace: bool) -> Result<(), VfsError> {
        self.inner.rename(from, to, replace)
    }
    fn set_attrs(&self, spec: &Spec, change: &hikasa_vfs::AttrChange) -> Result<(), VfsError> {
        self.inner.set_attrs(spec, change)
    }
    fn create_dir(&self, spec: &Spec, parents: bool) -> Result<bool, VfsError> {
        self.inner.create_dir(spec, parents)
    }
    fn remove(&self, spec: &Spec) -> Result<(), VfsError> {
        self.inner.remove(spec)
    }
}

/// A 48 MB document with a few edits: the splice source is read in steps of at most 1 MiB, and the heap peaks at most
/// 8 MiB above what it held before the execution plus the edited text (the plan and the document stay).
#[test]
fn a10_14_save_source_streams_in_steps_of_at_most_1_mib() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().canonicalize().unwrap().join("big.txt");
    let line = |k: usize| format!("{k:08} the quick brown fox jumps over the lazy\n");
    let text: String = (0..1_000_000).map(line).collect();
    assert!(text.len() >= 48_000_000);
    std::fs::write(&path, &text).unwrap();
    drop(text);
    let largest = Arc::new(AtomicUsize::new(0));
    let vfs = Recording {
        inner: LocalVfs::default(),
        largest: Arc::clone(&largest),
    };
    let target = Spec::Local(hikasa_core::LocalPath::new(path.clone()).unwrap());
    let mut doc = open(&vfs, &target, &ee(), DecodeOptions::default(), None);
    let mut edited = 0usize;
    for k in [10u32, 400_000, 999_999] {
        type_at(&mut doc, k, 0, "EDITED ");
        edited += doc.line(LineIdx(k)).len() + 1;
    }
    let planned = plan(&vfs, &doc, &SaveRequest::default());
    assert!(planned.confirmed);
    assert_eq!(planned.items[0].content.encoded_lines, 3);
    largest.store(0, Ordering::Relaxed);
    let source = SaveSource::new(&vfs, &planned, &[&doc]);
    let base = LIVE.load(Ordering::Relaxed);
    PEAK.store(base, Ordering::Relaxed);
    let report = execute_save(&vfs, planned, source, &Ctl::detached());
    let peak = PEAK.load(Ordering::Relaxed) - base;
    assert!(report.ok(), "{:?}", report.op.results);
    assert!(
        largest.load(Ordering::Relaxed) <= 1 << 20,
        "a read of {}",
        largest.load(Ordering::Relaxed)
    );
    let bound = (8 << 20) + edited as isize;
    eprintln!(
        "save of {} bytes: heap peak {peak} bytes above the start (bound {bound})",
        report.written[0].map_or(0, |s| s.size)
    );
    assert!(
        peak <= bound,
        "peak {peak} bytes above the start, bound {bound}"
    );
    let written = std::fs::read(&path).unwrap();
    assert_eq!(written.len() as u64, report.written[0].unwrap().size);
    assert!(written.starts_with(b"00000000 the quick"));
    let text = String::from_utf8(written).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines[10],
        "EDITED 00000010 the quick brown fox jumps over the lazy"
    );
    assert_eq!(
        lines[999_999],
        "EDITED 00999999 the quick brown fox jumps over the lazy"
    );
    assert_eq!(
        lines[11],
        "00000011 the quick brown fox jumps over the lazy"
    );
}

// ---- changes between plan and execution ------------------------------------------------------------------------

/// A source that changed between the plan and the execution, with its size and time kept, is found by the CRC at the
/// end of its read: the item fails before the commit, the file stays as changed; a Save As creates nothing.
#[test]
fn a10_14_a_source_changed_since_the_plan_fails_before_the_commit() {
    for save_as in [false, true] {
        let vfs = memory();
        put(&vfs, "/w/a.txt", b"alpha\nbeta\ngamma\n");
        let mut doc = doc_of(&vfs, "/w/a.txt");
        type_at(&mut doc, 1, 0, "x");
        let req = SaveRequest {
            target: save_as.then(|| spec("/w/copy.txt")),
            ..SaveRequest::default()
        };
        let planned = plan(&vfs, &doc, &req);
        assert!(planned.confirmed);
        // The same size and time, other bytes.
        put(&vfs, "/w/a.txt", b"ALPHA\nbeta\ngamma\n");
        let report = execute(&vfs, planned, &mut doc);
        assert_eq!(
            report.op.results[0].status,
            ItemStatus::Failed,
            "save as {save_as}"
        );
        let error = report.op.results[0].error.as_ref().unwrap();
        assert!(
            error.message.contains("changed since the plan was made"),
            "{}",
            error.message
        );
        assert_eq!(read(&vfs, "/w/a.txt").unwrap(), b"ALPHA\nbeta\ngamma\n");
        assert!(read(&vfs, "/w/copy.txt").is_none());
        assert!(
            doc.is_modified(),
            "a failed item leaves the document modified"
        );
        assert!(report.saved[0].is_none());
    }
}

/// A source that grew since the plan is read past its stamp's size and fails (a Save As, which no target stamp
/// guards); one with another length but the stamp's CRC (4 bytes forged onto it) is changed at planning: the size and
/// the CRC are checked together (W10b note 66).
#[test]
fn a10_14_a_source_that_grew_or_keeps_its_crc_is_changed() {
    let vfs = memory();
    put(&vfs, "/w/a.txt", b"alpha\nbeta\n");
    let mut doc = doc_of(&vfs, "/w/a.txt");
    type_at(&mut doc, 0, 0, "x");
    let req = SaveRequest {
        target: Some(spec("/w/copy.txt")),
        ..SaveRequest::default()
    };
    let planned = plan(&vfs, &doc, &req);
    put(&vfs, "/w/a.txt", b"alpha\nbeta\ngamma\n");
    let report = execute(&vfs, planned, &mut doc);
    assert_eq!(report.op.results[0].status, ItemStatus::Failed);
    assert!(read(&vfs, "/w/copy.txt").is_none());
    // Four bytes that keep the CRC-32, with another time: the same CRC, another length.
    let vfs = memory();
    let old = b"alpha\nbeta\n".to_vec();
    put(&vfs, "/w/a.txt", &old);
    let mut doc = doc_of(&vfs, "/w/a.txt");
    type_at(&mut doc, 0, 0, "x");
    let mut grown = old.clone();
    grown.extend_from_slice(&forge(&old));
    assert_eq!(
        hikasa_vfs::crc32_bytes(&grown),
        hikasa_vfs::crc32_bytes(&old)
    );
    vfs.insert_file(&common::host("/w/a.txt"), grown, meta(1_700_000_100))
        .unwrap();
    let planned = plan(&vfs, &doc, &SaveRequest::default());
    assert_eq!(questions(&planned, 0), vec![SaveQuestion::ChangedOnDisk]);
}

/// Four bytes that, appended to `data`, keep its CRC-32 (IEEE, reflected): the register after them is the one after
/// `data`.
fn forge(data: &[u8]) -> [u8; 4] {
    let mut table = [0u32; 256];
    let mut inverse = [0u8; 256];
    for i in 0..256u32 {
        let mut c = i;
        for _ in 0..8 {
            c = if c & 1 == 1 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
        table[i as usize] = c;
        inverse[(c >> 24) as usize] = i as u8;
    }
    let start = !hikasa_vfs::crc32_bytes(data);
    // The table entries the four steps must use, found from the end.
    let mut indices = [0u8; 4];
    let mut want = start;
    for k in (0..4).rev() {
        let i = inverse[(want >> 24) as usize];
        indices[k] = i;
        want = (want ^ table[i as usize]) << 8;
    }
    let mut out = [0u8; 4];
    let mut register = start;
    for k in 0..4 {
        let i = indices[k];
        out[k] = (register as u8) ^ i;
        register = (register >> 8) ^ table[i as usize];
    }
    out
}

/// An item whose document changed after the plan is refused; a plan read back has no content and is refused too.
#[test]
fn a10_14_a_document_changed_since_the_plan_is_refused() {
    let vfs = memory();
    put(&vfs, "/w/a.txt", b"alpha\n");
    let mut doc = doc_of(&vfs, "/w/a.txt");
    type_at(&mut doc, 0, 0, "1");
    let planned = plan(&vfs, &doc, &SaveRequest::default());
    type_at(&mut doc, 0, 0, "2");
    let source = SaveSource::new(&vfs, &planned, &[&doc]);
    let report = execute_save(&vfs, planned.clone(), source, &Ctl::detached());
    assert_eq!(report.op.results[0].status, ItemStatus::Failed);
    assert_eq!(read(&vfs, "/w/a.txt").unwrap(), b"alpha\n");
    let back: hikasa_text::save::SavePlan =
        serde_json::from_str(&serde_json::to_string(&planned).unwrap()).unwrap();
    let source = SaveSource::new(&vfs, &back, &[]);
    let report = execute_save(&vfs, back, source, &Ctl::detached());
    assert_eq!(report.op.results[0].status, ItemStatus::Failed);
    assert_eq!(read(&vfs, "/w/a.txt").unwrap(), b"alpha\n");
}

// ---- saved() -----------------------------------------------------------------------------------------------------

/// `saved()` rebases the marks and the stamp: the file written is the next splice source, every line keeps its own
/// index, `Edited` becomes `SavedEdit`; a later edit splices from the new file. A save of an older generation leaves the
/// document modified and its next save encodes the whole text.
#[test]
fn a10_14_saved_rebases_marks_and_stamp() {
    let vfs = memory();
    put(&vfs, "/w/a.txt", b"one\ntwo\nthree\n");
    let mut doc = doc_of(&vfs, "/w/a.txt");
    type_at(&mut doc, 1, 0, "new\n");
    assert_eq!(doc.mark(LineIdx(1)), LineMark::Edited);
    let report = save(&vfs, &mut doc, &SaveRequest::default());
    assert!(report.ok());
    let bytes = read(&vfs, "/w/a.txt").unwrap();
    assert_eq!(bytes, b"one\nnew\ntwo\nthree\n");
    assert!(!doc.is_modified());
    assert_eq!(doc.mark(LineIdx(1)), LineMark::SavedEdit);
    assert_eq!(doc.mark(LineIdx(2)), LineMark::Original);
    for l in 0..4 {
        assert_eq!(doc.marks(LineIdx(l)).orig, Some(LineIdx(l)));
    }
    let source = doc.splice_source().unwrap();
    assert_eq!(source.spec, spec("/w/a.txt"));
    assert_eq!(source.stamp.size, bytes.len() as u64);
    assert_eq!(source.stamp.crc32, hikasa_vfs::crc32_bytes(&bytes));
    let stat = vfs.stat(&spec("/w/a.txt"), FollowLinks::No).unwrap();
    assert_eq!(source.stamp.mtime, stat.mtime);
    // The next save splices from the file just written.
    type_at(&mut doc, 3, 5, "!");
    let planned = plan(&vfs, &doc, &SaveRequest::default());
    assert_eq!(planned.items[0].content.encoded_lines, 1);
    assert_eq!(planned.items[0].content.spliced_bytes, 12);
    assert!(execute(&vfs, planned, &mut doc).ok());
    assert_eq!(read(&vfs, "/w/a.txt").unwrap(), b"one\nnew\ntwo\nthree!\n");
    // An older generation: the file holds what was planned, the document stays modified and is re-encoded next.
    type_at(&mut doc, 0, 0, "A");
    let planned = plan(&vfs, &doc, &SaveRequest::default());
    let generation = planned.items[0].generation;
    let source = SaveSource::new(&vfs, &planned, &[]);
    let report = execute_save(&vfs, planned, source, &Ctl::detached());
    assert!(report.ok());
    type_at(&mut doc, 0, 0, "B");
    let saved = report.saved[0].clone().unwrap();
    assert_eq!(saved.generation, generation);
    doc.saved_to(&saved);
    assert!(doc.is_modified());
    assert!(!doc.spliceable());
    let planned = plan(&vfs, &doc, &SaveRequest::default());
    assert!(planned.items[0].issues.contains(&SaveIssue::Reencoded {
        reason: ReencodeReason::NoSource
    }));
    assert!(execute(&vfs, planned, &mut doc).ok());
    assert_eq!(
        read(&vfs, "/w/a.txt").unwrap(),
        b"BAone\nnew\ntwo\nthree!\n"
    );
    assert!(doc.spliceable());
}

/// A modified document re-wrapped by another characters-per-line limit is no longer spliced from its file (its line
/// numbers name pieces of the new wrapping): the save re-encodes it whole and writes its text (W11a review C8).
#[test]
fn a_modified_document_rewrapped_saves_its_text() {
    let vfs = memory();
    put(&vfs, "/w/a.txt", b"abcdefgh\nxy\nlast\n");
    let mut doc = doc_of(&vfs, "/w/a.txt");
    assert!(doc.spliceable());
    type_at(&mut doc, 1, 2, "!");
    let opts = EditOptions {
        chars_per_line_limit: Some(3),
        ..doc.options().clone()
    };
    doc.set_options(opts);
    assert!(doc.is_modified());
    assert!(
        !doc.spliceable(),
        "the old wrapping's lines cannot be spliced"
    );
    let planned = plan(&vfs, &doc, &SaveRequest::default());
    assert!(execute(&vfs, planned, &mut doc).ok());
    assert_eq!(read(&vfs, "/w/a.txt").unwrap(), b"abcdefgh\nxy!\nlast\n");
}

// ---- questions ---------------------------------------------------------------------------------------------------

/// The classes of an item's questions.
fn questions(plan: &hikasa_text::save::SavePlan, i: usize) -> Vec<SaveQuestion> {
    plan.items[i].questions()
}

/// `ChangedOnDisk` when the content differs, `source_touched` (no question) when only the time does; Yes writes a whole
/// re-encode, No skips the item.
#[test]
fn a10_15_core_changed_on_disk_and_source_touched() {
    let vfs = memory();
    put(&vfs, "/w/a.txt", b"one\ntwo\n");
    let mut doc = doc_of(&vfs, "/w/a.txt");
    type_at(&mut doc, 0, 3, "!");
    // A new time, the same bytes.
    vfs.insert_file(
        &common::host("/w/a.txt"),
        &b"one\ntwo\n"[..],
        meta(1_800_000_000),
    )
    .unwrap();
    let planned = plan(&vfs, &doc, &SaveRequest::default());
    assert!(planned.confirmed);
    assert!(planned.items[0].issues.contains(&SaveIssue::SourceTouched));
    assert!(
        planned
            .warnings
            .iter()
            .any(|w| w.code == WarningCode::SourceTouched)
    );
    assert_eq!(planned.items[0].content.encoded_lines, 1, "still spliced");
    // Other bytes of the same size: the content decides, not the size.
    vfs.insert_file(
        &common::host("/w/a.txt"),
        &b"ONE\ntwo\n"[..],
        meta(1_800_000_000),
    )
    .unwrap();
    let planned = plan(&vfs, &doc, &SaveRequest::default());
    assert_eq!(questions(&planned, 0), vec![SaveQuestion::ChangedOnDisk]);
    // Other bytes.
    vfs.insert_file(
        &common::host("/w/a.txt"),
        &b"ONE\ntwo\nthree\n"[..],
        meta(1_800_000_001),
    )
    .unwrap();
    let planned = plan(&vfs, &doc, &SaveRequest::default());
    assert_eq!(questions(&planned, 0), vec![SaveQuestion::ChangedOnDisk]);
    assert!(planned.items[0].issues.contains(&SaveIssue::Reencoded {
        reason: ReencodeReason::SourceChanged
    }));
    // No: skipped, the file stays.
    let mut machine = SaveConfirmation::new(&planned, &SavePolicy::ALL, ConfirmMode::Prompt);
    assert_eq!(machine.next().unwrap().kind, SaveQuestion::ChangedOnDisk);
    assert!(machine.question().unwrap().contains("changed on disk"));
    machine.reply(Reply::No);
    assert!(machine.next().is_none());
    let declined = machine.finish(planned.clone()).unwrap();
    assert_eq!(declined.plan.items[0].skip, Some(SkipReason::Declined));
    let report = execute(&vfs, declined, &mut doc);
    assert_eq!(report.op.results[0].status, ItemStatus::Skipped);
    assert_eq!(read(&vfs, "/w/a.txt").unwrap(), b"ONE\ntwo\nthree\n");
    // Executing without the answers refuses the item.
    let source = SaveSource::new(&vfs, &planned, &[&doc]);
    let report = execute_save(&vfs, planned.clone(), source, &Ctl::detached());
    assert_eq!(report.op.results[0].status, ItemStatus::Failed);
    // Yes: the whole text, over the changed file.
    let mut machine = SaveConfirmation::new(&planned, &SavePolicy::ALL, ConfirmMode::Prompt);
    machine.answer(Reply::Yes);
    let yes = machine.finish(planned).unwrap();
    assert!(execute(&vfs, yes, &mut doc).ok());
    assert_eq!(read(&vfs, "/w/a.txt").unwrap(), b"one!\ntwo\n");
}

/// W10b notes 62 and 66: what the encoder cannot write is asked about, refused or replaced, never written silently:
/// U+E5E5 in GB18030 (the WHATWG encoder has no bytes for it) and ISO-2022-JP's half-width katakana (it would write
/// them full-width); the count is the encoder's when it is larger. A literal U+FFFD of a clean decode is no lossy line.
#[test]
fn a10_15_core_what_the_encoder_cannot_write_is_asked() {
    for (label, file, typed, written) in [
        ("gb18030", &b"ab\n"[..], "\u{E5E5}", &b"?ab\n"[..]),
        ("ISO-2022-JP", &b"ab\n"[..], "\u{FF76}", &b"?ab\n"[..]),
    ] {
        let vfs = memory();
        put(&vfs, "/w/g.txt", file);
        let enc = TextEncoding::for_label(label).unwrap();
        let open_g = |vfs: &MemoryVfs| {
            let mut doc = open(
                vfs,
                &spec("/w/g.txt"),
                &ee(),
                common::save::forced(enc),
                None,
            );
            type_at(&mut doc, 0, 0, typed);
            doc
        };
        let mut doc = open_g(&vfs);
        let planned = plan(&vfs, &doc, &SaveRequest::default());
        assert_eq!(
            questions(&planned, 0),
            vec![SaveQuestion::Unmappable],
            "{label}"
        );
        let refused = plan(
            &vfs,
            &doc,
            &SaveRequest {
                unmappable: UnmappablePolicy::Refuse,
                ..SaveRequest::default()
            },
        );
        let error = refused.plan.items[0].error.as_ref().expect(label);
        assert!(
            error.message.contains("1 characters"),
            "{label}: {}",
            error.message
        );
        let replaced = plan(
            &vfs,
            &doc,
            &SaveRequest {
                unmappable: UnmappablePolicy::Replace,
                ..SaveRequest::default()
            },
        );
        assert!(execute(&vfs, replaced, &mut doc).ok(), "{label}");
        assert_eq!(read(&vfs, "/w/g.txt").unwrap(), written, "{label}");
    }
    // A literal U+FFFD in a file decoded without loss: no `LossyRewritten`.
    let vfs = memory();
    put(&vfs, "/w/f.txt", "a\u{FFFD}\nb\n".as_bytes());
    let mut doc = open(
        &vfs,
        &spec("/w/f.txt"),
        &ee(),
        common::save::forced(TextEncoding::UTF_8),
        None,
    );
    type_at(&mut doc, 0, 0, "x");
    let planned = plan(&vfs, &doc, &SaveRequest::default());
    assert!(
        questions(&planned, 0).is_empty(),
        "{:?}",
        planned.items[0].issues
    );
    assert!(execute(&vfs, planned, &mut doc).ok());
    assert_eq!(
        read(&vfs, "/w/f.txt").unwrap(),
        "xa\u{FFFD}\nb\n".as_bytes()
    );
}

/// Every class: `ReadOnlyTarget` (Yes clears a DOS `R`), `Unmappable` (Ask, Refuse, Replace; never `&#…;`),
/// `LossyRewritten`, `ReplaceExisting` (once for a save target), `OverwriteBackup` (not for the backup the document's
/// last save made); `YesToAll` per class, `NoToAll` as a mode, `Cancel`.
#[test]
fn a10_15_core_every_question_class() {
    // ReadOnlyTarget.
    let vfs = memory();
    vfs.insert_file(
        &common::host("/w/r.txt"),
        &b"r\n"[..],
        MemMeta {
            mtime: FileTime::from_unix(1, 0),
            attrs: Some(Attrs::Dos {
                attrs: DosAttrs::READONLY,
            }),
            ..MemMeta::default()
        },
    )
    .unwrap();
    let mut doc = doc_of(&vfs, "/w/r.txt");
    type_at(&mut doc, 0, 1, "!");
    let planned = plan(&vfs, &doc, &SaveRequest::default());
    assert_eq!(questions(&planned, 0), vec![SaveQuestion::ReadOnlyTarget]);
    let yes = confirm_all(planned);
    assert!(yes.plan.items[0].clear_read_only);
    // Unmappable in windows-1252: Ask, Refuse, Replace.
    let vfs = memory();
    put(&vfs, "/w/u.txt", b"caf\xE9\n");
    let w1252 = TextEncoding::for_label("windows-1252").unwrap();
    let open_u = |vfs: &MemoryVfs| {
        let mut doc = open(
            vfs,
            &spec("/w/u.txt"),
            &ee(),
            common::save::forced(w1252),
            None,
        );
        type_at(&mut doc, 0, 0, "\u{65e5}\u{672c} ");
        doc
    };
    let mut doc = open_u(&vfs);
    let planned = plan(&vfs, &doc, &SaveRequest::default());
    assert_eq!(questions(&planned, 0), vec![SaveQuestion::Unmappable]);
    let machine = SaveConfirmation::new(&planned, &SavePolicy::ALL, ConfirmMode::Prompt);
    assert!(
        machine.question().unwrap().contains("2 characters"),
        "{:?}",
        machine.question()
    );
    let refuse = SaveRequest {
        unmappable: UnmappablePolicy::Refuse,
        ..SaveRequest::default()
    };
    let refused = plan(&vfs, &doc, &refuse);
    assert!(refused.plan.items[0].error.is_some());
    let replace = SaveRequest {
        unmappable: UnmappablePolicy::Replace,
        ..SaveRequest::default()
    };
    let replaced = plan(&vfs, &doc, &replace);
    assert!(replaced.confirmed);
    assert!(
        replaced
            .warnings
            .iter()
            .any(|w| w.code == WarningCode::UnmappableReplaced)
    );
    assert!(execute(&vfs, replaced, &mut doc).ok());
    let bytes = read(&vfs, "/w/u.txt").unwrap();
    assert_eq!(bytes, b"?? caf\xE9\n");
    assert!(!bytes.windows(2).any(|w| w == b"&#"));
    // Shift_JIS: an emoji.
    let vfs = memory();
    put(&vfs, "/w/s.txt", b"\x93\xfa\n");
    let sjis = TextEncoding::for_label("Shift_JIS").unwrap();
    let mut doc = open(
        &vfs,
        &spec("/w/s.txt"),
        &ee(),
        common::save::forced(sjis),
        None,
    );
    type_at(&mut doc, 0, 3, "\u{1F600}");
    let planned = plan(&vfs, &doc, &SaveRequest::default());
    assert_eq!(questions(&planned, 0), vec![SaveQuestion::Unmappable]);
    let done = confirm_all(planned);
    assert!(
        done.warnings
            .iter()
            .any(|w| w.code == WarningCode::UnmappableReplaced)
    );
    assert!(execute(&vfs, done, &mut doc).ok());
    assert_eq!(read(&vfs, "/w/s.txt").unwrap(), b"\x93\xfa?\n");
    // LossyRewritten.
    let vfs = memory();
    put(&vfs, "/w/l.txt", b"ok\nbad\xFF\n");
    let utf8 = || common::save::forced(TextEncoding::UTF_8);
    let mut doc = open(&vfs, &spec("/w/l.txt"), &ee(), utf8(), None);
    type_at(&mut doc, 1, 0, "x");
    let planned = plan(&vfs, &doc, &SaveRequest::default());
    assert_eq!(questions(&planned, 0), vec![SaveQuestion::LossyRewritten]);
    let done = confirm_all(planned);
    assert!(
        done.warnings
            .iter()
            .any(|w| w.code == WarningCode::LossyLineRewritten)
    );
    // An unchanged lossy line is spliced: no question.
    let mut doc = open(&vfs, &spec("/w/l.txt"), &ee(), utf8(), None);
    type_at(&mut doc, 0, 0, "x");
    assert!(plan(&vfs, &doc, &SaveRequest::default()).confirmed);
    // ReplaceExisting, and a save target asks once.
    let vfs = memory();
    put(&vfs, "/w/a.txt", b"a\n");
    put(&vfs, "/w/t.txt", b"t\n");
    let mut doc = doc_of(&vfs, "/w/a.txt");
    doc.set_save_target(Some(spec("/w/t.txt")));
    type_at(&mut doc, 0, 0, "1");
    let planned = plan(&vfs, &doc, &SaveRequest::default());
    assert_eq!(planned.items[0].mode, SaveMode::SaveTarget);
    assert_eq!(questions(&planned, 0), vec![SaveQuestion::ReplaceExisting]);
    assert!(execute(&vfs, confirm_all(planned), &mut doc).ok());
    assert_eq!(read(&vfs, "/w/t.txt").unwrap(), b"1a\n");
    assert_eq!(
        read(&vfs, "/w/a.txt").unwrap(),
        b"a\n",
        "the location stays"
    );
    assert_eq!(doc.location(), Some(&spec("/w/a.txt")));
    assert_eq!(doc.splice_source().unwrap().spec, spec("/w/t.txt"));
    type_at(&mut doc, 0, 0, "2");
    let planned = plan(&vfs, &doc, &SaveRequest::default());
    assert!(
        planned.confirmed,
        "asked once: {:?}",
        planned.items[0].issues
    );
    assert!(execute(&vfs, planned, &mut doc).ok());
    assert_eq!(read(&vfs, "/w/t.txt").unwrap(), b"21a\n");
    // OverwriteBackup: a file at the backup's name; not the backup the last save made.
    let vfs = memory();
    put(&vfs, "/w/a.txt", b"a\n");
    put(&vfs, "/w/a.txt.bak", b"someone's\n");
    let mut doc = doc_of(&vfs, "/w/a.txt");
    type_at(&mut doc, 0, 0, "1");
    let planned = plan(&vfs, &doc, &with_backup());
    assert_eq!(questions(&planned, 0), vec![SaveQuestion::OverwriteBackup]);
    assert!(execute(&vfs, confirm_all(planned), &mut doc).ok());
    assert_eq!(read(&vfs, "/w/a.txt.bak").unwrap(), b"a\n");
    type_at(&mut doc, 0, 0, "2");
    let planned = plan(&vfs, &doc, &with_backup());
    assert!(planned.confirmed, "{:?}", planned.items[0].issues);
    assert!(execute(&vfs, planned, &mut doc).ok());
    assert_eq!(
        read(&vfs, "/w/a.txt.bak").unwrap(),
        b"1a\n",
        "one generation"
    );
    // A file of the backup's size at its name, with another time: not the one the last save made, so asked.
    let vfs = memory();
    put(&vfs, "/w/a.txt", b"a\n");
    let mut doc = doc_of(&vfs, "/w/a.txt");
    type_at(&mut doc, 0, 0, "1");
    assert!(
        execute(
            &vfs,
            confirm_all(plan(&vfs, &doc, &with_backup())),
            &mut doc
        )
        .ok()
    );
    assert_eq!(read(&vfs, "/w/a.txt.bak").unwrap(), b"a\n");
    vfs.insert_file(
        &common::host("/w/a.txt.bak"),
        &b"z\n"[..],
        meta(1_600_000_000),
    )
    .unwrap();
    type_at(&mut doc, 0, 0, "2");
    let planned = plan(&vfs, &doc, &with_backup());
    assert_eq!(questions(&planned, 0), vec![SaveQuestion::OverwriteBackup]);
}

/// The machine over several items: `YesToAll` answers a class, `NoToAll` as a mode declines every asking item,
/// `Cancel` declines the plan.
#[test]
fn a10_15_core_the_machine() {
    let vfs = memory();
    for name in ["a", "b", "c"] {
        put(&vfs, &format!("/w/{name}.txt"), b"x\n");
        put(&vfs, &format!("/w/{name}2.txt"), b"y\n");
    }
    let docs: Vec<EditableDocument> = ["a", "b", "c"]
        .iter()
        .map(|n| doc_of(&vfs, &format!("/w/{n}.txt")))
        .collect();
    let reqs: Vec<SaveRequest> = ["a", "b", "c"]
        .iter()
        .map(|n| SaveRequest {
            target: Some(spec(&format!("/w/{n}2.txt"))),
            ..SaveRequest::default()
        })
        .collect();
    let input: Vec<_> = docs.iter().zip(&reqs).map(|(d, r)| (None, d, r)).collect();
    let planned = plan_save(&vfs, &input, &Ctl::detached()).unwrap();
    let mut machine = SaveConfirmation::new(&planned, &SavePolicy::ALL, ConfirmMode::Prompt);
    assert_eq!(
        machine.open_questions().get(&SaveQuestion::ReplaceExisting),
        Some(&3)
    );
    assert_eq!(machine.remaining(), 3);
    machine.reply(Reply::No);
    machine.reply(Reply::YesToAll);
    assert!(machine.next().is_none());
    let done = machine.finish(planned.clone()).unwrap();
    let skips: Vec<_> = done.plan.items.iter().map(|i| i.skip).collect();
    assert_eq!(skips, [Some(SkipReason::Declined), None, None]);
    let machine = SaveConfirmation::new(&planned, &SavePolicy::ALL, ConfirmMode::NoToAll);
    assert!(machine.next().is_none());
    let done = machine.finish(planned.clone()).unwrap();
    assert!(
        done.plan
            .items
            .iter()
            .all(|i| i.skip == Some(SkipReason::Declined))
    );
    let mut machine = SaveConfirmation::new(&planned, &SavePolicy::ALL, ConfirmMode::Prompt);
    machine.reply(Reply::Yes);
    machine.reply(Reply::Cancel);
    assert!(machine.finish(planned).is_err());
}

// ---- targets ------------------------------------------------------------------------------------------------------

/// A link is followed (the link stays a link, `via_link` names it); a special file is refused; a document without a
/// location needs a target; Save As moves the location; duplicate targets are skipped.
#[test]
fn a10_17_core_targets() {
    let vfs = memory();
    put(&vfs, "/w/real.txt", b"real\n");
    vfs.insert_symlink(&common::host("/w/link.txt"), "real.txt", MemMeta::default())
        .unwrap();
    let mut doc = doc_of(&vfs, "/w/link.txt");
    type_at(&mut doc, 0, 0, "+");
    let planned = plan(&vfs, &doc, &SaveRequest::default());
    assert_eq!(planned.items[0].target, spec("/w/real.txt"));
    assert_eq!(planned.items[0].via_link, Some(spec("/w/link.txt")));
    assert!(
        planned.human_lines()[0].ends_with(&format!(", via {})", spec("/w/link.txt").canonical()))
    );
    assert!(execute(&vfs, planned, &mut doc).ok());
    assert_eq!(read(&vfs, "/w/real.txt").unwrap(), b"+real\n");
    let link = vfs.stat(&spec("/w/link.txt"), FollowLinks::No).unwrap();
    assert_eq!(link.kind, hikasa_vfs::EntryKind::Symlink);
    assert_eq!(doc.location(), Some(&spec("/w/link.txt")));
    // A special file.
    vfs.insert_other(&common::host("/w/fifo"), MemMeta::default())
        .unwrap();
    let req = SaveRequest {
        target: Some(spec("/w/fifo")),
        ..SaveRequest::default()
    };
    let planned = plan(&vfs, &doc, &req);
    let error = planned.plan.items[0].error.as_ref().unwrap();
    assert!(
        error.message.contains("is not a regular file"),
        "{}",
        error.message
    );
    // No location.
    let mut pair = EditableDocument::new(
        Arc::new(TextDocument::from_text("pair\n")),
        EditOptions::default(),
        Editability {
            blocks: Vec::new(),
            can_save_in_place: false,
        },
    );
    let planned = plan(&vfs, &pair, &SaveRequest::default());
    assert!(planned.plan.items[0].error.is_some());
    let req = SaveRequest {
        target: Some(spec("/w/pair.txt")),
        ..SaveRequest::default()
    };
    let planned = plan(&vfs, &pair, &req);
    assert!(planned.items[0].creates);
    assert!(planned.confirmed, "{:?}", planned.items[0].issues);
    assert!(execute(&vfs, planned, &mut pair).ok());
    assert_eq!(read(&vfs, "/w/pair.txt").unwrap(), b"pair\n");
    assert_eq!(pair.location(), Some(&spec("/w/pair.txt")));
    assert!(pair.can_save_in_place());
    type_at(&mut pair, 0, 0, "1");
    assert!(save(&vfs, &mut pair, &SaveRequest::default()).ok());
    assert_eq!(read(&vfs, "/w/pair.txt").unwrap(), b"1pair\n");
    // Two documents onto one file: the second is skipped.
    let req = SaveRequest {
        target: Some(spec("/w/pair.txt")),
        ..SaveRequest::default()
    };
    let other = doc_of(&vfs, "/w/real.txt");
    let planned = plan_save(
        &vfs,
        &[(None, &pair, &SaveRequest::default()), (None, &other, &req)],
        &Ctl::detached(),
    )
    .unwrap();
    assert_eq!(
        planned.plan.items[1].skip,
        Some(SkipReason::DuplicateTarget)
    );
}

// ---- archive members ----------------------------------------------------------------------------------------------

/// A back-end where `/w/a.zip` holds members: an archive member `a.zip/<m>` is the file `/w/a.zip.d/<m>` of a memory
/// tree, the archive itself `/w/a.zip`. What `plan_save` and `ops` ask of a member reaches that file.
struct Members(MemoryVfs);

impl Members {
    fn map(&self, at: &Spec) -> Spec {
        match at {
            Spec::ArchiveMember { archive, member } => spec(&format!(
                "/w/{}.d/{member}",
                archive.file_name().unwrap_or_default()
            )),
            other => other.clone(),
        }
    }
}

impl Vfs for Members {
    fn capabilities(&self, at: &Spec) -> Result<Capabilities, VfsError> {
        self.0.capabilities(&self.map(at))
    }
    fn stat(&self, spec: &Spec, follow: FollowLinks) -> Result<Entry, VfsError> {
        self.0.stat(&self.map(spec), follow)
    }
    fn list(
        &self,
        dir: &Spec,
        opts: &ListOptions<'_>,
        cancel: &CancelToken,
    ) -> Result<Listing, VfsError> {
        self.0.list(&self.map(dir), opts, cancel)
    }
    fn open_read(&self, spec: &Spec) -> Result<Box<dyn ReadSeek>, VfsError> {
        self.0.open_read(&self.map(spec))
    }
    fn open_write(
        &self,
        spec: &Spec,
        opts: &WriteOptions,
    ) -> Result<Box<dyn AtomicWrite>, VfsError> {
        self.0.open_write(&self.map(spec), opts)
    }
    fn child(&self, dir: &Spec, name: &FileName) -> Result<Spec, VfsError> {
        self.0.child(dir, name)
    }
    fn create_dir(&self, spec: &Spec, parents: bool) -> Result<bool, VfsError> {
        self.0.create_dir(&self.map(spec), parents)
    }
    fn set_times(&self, spec: &Spec, times: &hikasa_vfs::SetTimes) -> Result<(), VfsError> {
        self.0.set_times(&self.map(spec), times)
    }
    fn set_attrs(&self, spec: &Spec, change: &hikasa_vfs::AttrChange) -> Result<(), VfsError> {
        self.0.set_attrs(&self.map(spec), change)
    }
    fn rename(&self, from: &Spec, to: &Spec, replace: bool) -> Result<(), VfsError> {
        self.0.rename(&self.map(from), &self.map(to), replace)
    }
    fn remove(&self, spec: &Spec) -> Result<(), VfsError> {
        self.0.remove(&self.map(spec))
    }
    /// Members of `ro.zip` are in an archive that cannot be written.
    fn writable(&self, spec: &Spec) -> Result<Option<bool>, VfsError> {
        Ok(match spec {
            Spec::ArchiveMember { archive, .. }
                if archive.file_name().as_deref() == Some("ro.zip") =>
            {
                Some(false)
            }
            _ => None,
        })
    }
}

/// A member of an archive that cannot be written is an item error at planning (`permission_denied`), not a
/// `ReadOnlyTarget` question a `--yes` would answer (§1.10.14, W10b note 35).
#[test]
fn a10_16_core_a_member_of_a_read_only_archive_fails_at_planning() {
    let mem = memory();
    put(&mem, "/w/ro.zip", b"PK archive bytes");
    mem.insert_dir(&common::host("/w/ro.zip.d"), MemMeta::default())
        .unwrap();
    put(&mem, "/w/ro.zip.d/x.txt", b"x\n");
    let vfs = Members(mem);
    let member = Spec::archive_member(spec("/w/ro.zip"), "x.txt");
    let mut x = open(&vfs, &member, &ee(), DecodeOptions::default(), None);
    type_at(&mut x, 0, 0, "1");
    let planned = plan(&vfs, &x, &with_backup());
    let error = planned.plan.items[0].error.as_ref().expect("an item error");
    assert_eq!(error.kind, hikasa_core::ErrorKind::PermissionDenied);
    assert!(
        error.message.contains("cannot be written"),
        "{}",
        error.message
    );
    assert!(planned.items[0].questions().is_empty());
    assert!(planned.exec.archive_backups.is_empty());
}

/// Two members of one archive saved in one plan: no backup of their own, one backup of the archive, and an existing
/// file at that backup's name asked about by each.
#[test]
fn a10_16_core_members_of_one_archive_have_one_backup() {
    let mem = memory();
    put(&mem, "/w/a.zip", b"PK archive bytes");
    mem.insert_dir(&common::host("/w/a.zip.d"), MemMeta::default())
        .unwrap();
    put(&mem, "/w/a.zip.d/x.txt", b"x\n");
    put(&mem, "/w/a.zip.d/y.txt", b"y\n");
    let vfs = Members(mem);
    let member = |m: &str| Spec::archive_member(spec("/w/a.zip"), m);
    let mut x = open(
        &vfs,
        &member("x.txt"),
        &ee(),
        DecodeOptions::default(),
        None,
    );
    let mut y = open(
        &vfs,
        &member("y.txt"),
        &ee(),
        DecodeOptions::default(),
        None,
    );
    type_at(&mut x, 0, 0, "1");
    type_at(&mut y, 0, 0, "2");
    let req = with_backup();
    let planned = plan_save(
        &vfs,
        &[(None, &x, &req), (None, &y, &req)],
        &Ctl::detached(),
    )
    .unwrap();
    assert!(planned.plan.items.iter().all(|item| item.backup.is_none()));
    assert_eq!(planned.exec.archive_backups.len(), 1);
    assert_eq!(
        planned
            .exec
            .archive_backups
            .get(&spec("/w/a.zip").canonical()),
        Some(&spec("/w/a.zip.bak"))
    );
    assert!(planned.confirmed);
    assert!(planned.human_lines()[0].contains(", backup "));
    let source = SaveSource::new(&vfs, &planned, &[&x, &y]);
    let report = execute_save(&vfs, planned, source, &Ctl::detached());
    assert!(report.ok(), "{:?}", report.op.results);
    x.saved_to(report.saved[0].as_ref().unwrap());
    y.saved_to(report.saved[1].as_ref().unwrap());
    assert_eq!(read(&vfs.0, "/w/a.zip.d/x.txt").unwrap(), b"1x\n");
    assert_eq!(read(&vfs.0, "/w/a.zip.bak").unwrap(), b"PK archive bytes");
    // The backup the last save made is replaced without a question.
    type_at(&mut x, 0, 0, "3");
    let again = plan(&vfs, &x, &req);
    assert!(
        again.confirmed,
        "{:?} {:?}",
        again.items[0].issues,
        x.last_backup()
    );
    // A file at the archive's backup that no save of these documents made: each member asks.
    let mut x = open(
        &vfs,
        &member("x.txt"),
        &ee(),
        DecodeOptions::default(),
        None,
    );
    let mut y = open(
        &vfs,
        &member("y.txt"),
        &ee(),
        DecodeOptions::default(),
        None,
    );
    type_at(&mut x, 0, 0, "3");
    type_at(&mut y, 0, 0, "4");
    let planned = plan_save(
        &vfs,
        &[(None, &x, &req), (None, &y, &req)],
        &Ctl::detached(),
    )
    .unwrap();
    assert_eq!(questions(&planned, 0), vec![SaveQuestion::OverwriteBackup]);
    assert_eq!(questions(&planned, 1), vec![SaveQuestion::OverwriteBackup]);
}

// ---- the External save conversion ---------------------------------------------------------------------------------

/// A runner that writes `%t` as `%s` upper-cased (ASCII), after the program name `up`.
struct Upper(Mutex<Vec<Vec<OsString>>>);

impl ProcessRunner for Upper {
    fn run(
        &self,
        argv: &[OsString],
        _timeout: Option<Duration>,
        _cancel: &CancelToken,
    ) -> Result<i32, ConvertError> {
        self.0.lock().unwrap().push(argv.to_vec());
        let input = std::fs::read(&argv[1]).map_err(ConvertError::Io)?;
        std::fs::write(&argv[2], input.to_ascii_uppercase()).map_err(ConvertError::Io)?;
        Ok(0)
    }
}

/// With a save command line the encoded document is `%s` and the program's `%t` is written; without a runner the item
/// fails.
#[test]
fn a10_14_the_external_save_conversion() {
    let vfs = memory();
    put(&vfs, "/w/c.txt", b"one\ntwo\n");
    let mut doc = doc_of(&vfs, "/w/c.txt");
    type_at(&mut doc, 1, 0, "and ");
    let req = SaveRequest {
        conversion: Some(ExternalConversion {
            load: "down %s %t".into(),
            save: Some("up %s %t".into()),
            filename_encoding: Default::default(),
            timeout_secs: 5,
        }),
        ..SaveRequest::default()
    };
    let failed = plan(&vfs, &doc, &req);
    assert!(failed.plan.items[0].error.is_some());
    let temp = tempfile::tempdir().unwrap();
    let runner = Upper(Mutex::new(Vec::new()));
    let env = SaveEnv {
        external: Some((temp.path(), &runner)),
    };
    let planned = plan_save_with(&vfs, &[(None, &doc, &req)], &env, &Ctl::detached()).unwrap();
    assert_eq!(planned.items[0].external.as_deref(), Some("up %s %t"));
    assert_eq!(runner.0.lock().unwrap().len(), 1);
    assert!(execute(&vfs, planned, &mut doc).ok());
    assert_eq!(read(&vfs, "/w/c.txt").unwrap(), b"ONE\nAND TWO\n");
}
