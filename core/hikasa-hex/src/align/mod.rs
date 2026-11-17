//! The alignment model and its three modes (HX-002; D-327…D-330): [`HexAlignment`], [`align`],
//! [`align_none`], [`align_fast`], [`align_complete`].
//!
//! Every mode produces one ordered list of segments covering both files exactly once ([`SegmentStore`]). Whether
//! a paired byte is same or different is decided by the bytes, in the statistics pass that every mode ends with,
//! never by a hash: block hashes are hints (a collision can only pair two blocks that then show their real
//! differences). The pass also builds the sections and the thumbnail.
//!
//! | Mode | Segments | Pass over the paired bytes |
//! |---|---|---|
//! | None | one `Paired` of the shorter length + the longer file's tail | the pair visitor over `[0, shorter)` |
//! | Fast | content-defined blocks aligned by `hikasa_diff` (Standard), gaps ≤ 64 KiB refined byte-level, larger ones None-style | the left document through the visitor, the right bytes of each paired piece by positional reads |
//! | Complete | Fast's blocks, then every gap byte-level within the budget (Myers windows) | as Fast |

pub mod cdc;
mod complete;
mod fast;
mod none;
mod sections;
mod store;

use std::ops::ControlFlow;
use std::sync::Arc;

use hikasa_core::{Ctl, Phase, Progress, ProgressSink};
use hikasa_vfs::{StreamOptions, VISIT_CHUNK, visit_chunks};
use serde::{Deserialize, Serialize};

pub use crate::vocab::AlignMode;
pub(crate) use none::none_store;
pub use none::scan_none;
pub use sections::{
    DEFAULT_MAX_SECTIONS, SECTION_MERGE_GAP, Section, SectionKind, Sections, THUMBNAIL_BUCKETS,
    ThumbBucket, Thumbnail,
};
pub use store::{Segment, SegmentKind, SegmentStore};

pub(crate) use sections::{ChunkScan, Collector};

use crate::document::HexDocument;
use crate::error::{HexError, HexWarning, HexWarningCode};

/// Options of an alignment (`[rules.hex]`, §1.12.6, §1.12.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HexAlignOptions {
    /// The mode (default Fast).
    pub mode: AlignMode,
    /// Fast's average block in bytes: a power of two, 1 KiB…1 MiB (default 8 KiB).
    pub block: u32,
    /// The skew limit in bytes (default 16 MiB): block-level skew `max(1, skew / block)` blocks.
    pub skew: u64,
    /// Complete's byte window, 4 KiB…16 MiB (default 256 KiB); the byte-level skew is `window / 2`. Myers is given at
    /// most 64 KiB a side, so a larger window walks in 64 KiB windows (contract §1.12.33 W12a note 16).
    pub window: u32,
    /// Complete's byte budget in MiB (default 1,024).
    pub budget_mib: u64,
    /// Sections stored (default 1,048,576).
    pub max_sections: u64,
    /// The visitor's options (threads, mmap policy, read size).
    pub stream: StreamOptions,
}

impl HexAlignOptions {
    /// Default block (8 KiB).
    pub const DEFAULT_BLOCK: u32 = 8 << 10;
    /// Default skew (16 MiB).
    pub const DEFAULT_SKEW: u64 = 16 << 20;
    /// Default window (256 KiB).
    pub const DEFAULT_WINDOW: u32 = 256 << 10;
    /// Default budget (1,024 MiB).
    pub const DEFAULT_BUDGET_MIB: u64 = 1024;
    /// Gaps of Fast refined byte-level when both sides are at most this long (64 KiB).
    pub const FAST_REFINE_MAX: u64 = 64 << 10;

    /// The defaults with `mode`.
    pub fn with_mode(mode: AlignMode) -> HexAlignOptions {
        HexAlignOptions {
            mode,
            ..HexAlignOptions::default()
        }
    }

    /// `block` as a valid power of two in 1 KiB…1 MiB.
    pub(crate) fn effective_block(&self) -> u64 {
        u64::from(self.block.clamp(1 << 10, 1 << 20))
            .next_power_of_two()
            .min(1 << 20)
    }

    /// `window` clamped to 4 KiB…16 MiB.
    pub(crate) fn effective_window(&self) -> u64 {
        u64::from(self.window.clamp(4 << 10, 16 << 20))
    }
}

impl Default for HexAlignOptions {
    fn default() -> HexAlignOptions {
        HexAlignOptions {
            mode: AlignMode::Fast,
            block: HexAlignOptions::DEFAULT_BLOCK,
            skew: HexAlignOptions::DEFAULT_SKEW,
            window: HexAlignOptions::DEFAULT_WINDOW,
            budget_mib: HexAlignOptions::DEFAULT_BUDGET_MIB,
            max_sections: DEFAULT_MAX_SECTIONS,
            stream: StreamOptions::default(),
        }
    }
}

/// Exact statistics of an alignment (every mode).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HexStats {
    /// Left file length.
    pub left_bytes: u64,
    /// Right file length.
    pub right_bytes: u64,
    /// Paired bytes that are equal.
    pub same_bytes: u64,
    /// Paired bytes that differ.
    pub differing_bytes: u64,
    /// Left-only bytes.
    pub left_only_bytes: u64,
    /// Right-only bytes.
    pub right_only_bytes: u64,
    /// Sections (all of them, stored or not).
    pub sections: u64,
}

impl HexStats {
    /// True when the files are byte-identical: equal lengths and every paired byte same.
    pub fn identical(&self) -> bool {
        self.left_bytes == self.right_bytes
            && self.differing_bytes == 0
            && self.left_only_bytes == 0
            && self.right_only_bytes == 0
    }
}

/// The result of an alignment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HexAlignment {
    /// The mode that produced it.
    pub mode: AlignMode,
    /// The segments.
    pub segments: SegmentStore,
    /// The sections.
    pub sections: Sections,
    /// Exact statistics.
    pub stats: HexStats,
    /// The thumbnail's data.
    pub thumbnail: Thumbnail,
    /// Complete ran out of its budget (some gaps laid out None-style).
    pub degraded: bool,
    /// `alignment_degraded`, `sections_truncated`.
    pub warnings: Vec<HexWarning>,
}

/// Aligns two documents with `opts.mode`.
///
/// # Errors
///
/// [`HexError::Truncated`] when an input is shorter than when it was opened; a read error; cancellation.
pub fn align(
    left: &HexDocument,
    right: &HexDocument,
    opts: &HexAlignOptions,
    ctl: &Ctl<'_>,
) -> Result<HexAlignment, HexError> {
    match opts.mode {
        AlignMode::None => align_none(left, right, opts, ctl),
        AlignMode::Fast => align_fast(left, right, opts, ctl),
        AlignMode::Complete => align_complete(left, right, opts, ctl),
    }
}

/// None alignment (D-328): byte n with byte n; the scan counts and finds the sections.
///
/// # Errors
///
/// As [`align`].
pub fn align_none(
    left: &HexDocument,
    right: &HexDocument,
    opts: &HexAlignOptions,
    ctl: &Ctl<'_>,
) -> Result<HexAlignment, HexError> {
    let store = none::none_store(left.len(), right.len());
    let (sections, same, different, thumbnail) = scan_none(left, right, &store, opts, ctl)?;
    Ok(finish(
        AlignMode::None,
        store,
        sections,
        same,
        different,
        thumbnail,
        None,
        opts,
    ))
}

/// Fast alignment (D-329).
///
/// # Errors
///
/// As [`align`].
pub fn align_fast(
    left: &HexDocument,
    right: &HexDocument,
    opts: &HexAlignOptions,
    ctl: &Ctl<'_>,
) -> Result<HexAlignment, HexError> {
    fast::align_blocks(left, right, opts, false, cdc::block_hash, ctl)
}

/// Complete alignment (D-330).
///
/// # Errors
///
/// As [`align`].
pub fn align_complete(
    left: &HexDocument,
    right: &HexDocument,
    opts: &HexAlignOptions,
    ctl: &Ctl<'_>,
) -> Result<HexAlignment, HexError> {
    fast::align_blocks(left, right, opts, true, cdc::block_hash, ctl)
}

/// Assembles a result: statistics from the counts and segment lengths, the truncation and degradation warnings.
#[allow(clippy::too_many_arguments)]
pub(crate) fn finish(
    mode: AlignMode,
    segments: SegmentStore,
    sections: Sections,
    same: u64,
    different: u64,
    thumbnail: Thumbnail,
    degraded: Option<String>,
    opts: &HexAlignOptions,
) -> HexAlignment {
    let (mut left_only, mut right_only) = (0, 0);
    for seg in segments.iter() {
        match seg.kind {
            SegmentKind::Paired => {}
            SegmentKind::LeftOnly => left_only += seg.len,
            SegmentKind::RightOnly => right_only += seg.len,
        }
    }
    let stats = HexStats {
        left_bytes: segments.left_len(),
        right_bytes: segments.right_len(),
        same_bytes: same,
        differing_bytes: different,
        left_only_bytes: left_only,
        right_only_bytes: right_only,
        sections: sections.total(),
    };
    let mut warnings = Vec::new();
    if let Some(message) = &degraded {
        warnings.push(HexWarning {
            code: HexWarningCode::AlignmentDegraded,
            message: message.clone(),
            spec: None,
        });
    }
    if sections.truncated() {
        warnings.push(HexWarning {
            code: HexWarningCode::SectionsTruncated,
            message: format!(
                "{} sections; the first {} are listed (max_sections)",
                sections.total(),
                opts.max_sections
            ),
            spec: None,
        });
    }
    HexAlignment {
        mode,
        segments,
        sections,
        stats,
        thumbnail,
        degraded: degraded.is_some(),
        warnings,
    }
}

/// Visits run in windows of this many chunks per thread, so the runs of chunks waiting for their turn stay
/// bounded (a pair that differs everywhere has a run every 9 bytes until the sections are truncated).
const WINDOW_CHUNKS_PER_THREAD: u64 = 2;

/// The window ranges of a visit of `len` bytes: multiples of [`VISIT_CHUNK`], so the cuts are the global ones.
pub(crate) fn windows(len: u64, threads: usize) -> impl Iterator<Item = std::ops::Range<u64>> {
    let step = VISIT_CHUNK * WINDOW_CHUNKS_PER_THREAD * threads.max(1) as u64;
    (0..len.div_ceil(step)).map(move |k| k * step..((k + 1) * step).min(len))
}

/// The statistics pass of Fast and Complete (§1.12.4): the left document through the visitor; for each paired
/// piece of a chunk, the right bytes by positional reads; the one-sided segments placed by the collector.
pub(crate) fn scan_paired(
    left: &HexDocument,
    right: &HexDocument,
    store: &SegmentStore,
    opts: &HexAlignOptions,
    ctl: &Ctl<'_>,
) -> Result<(Sections, u64, u64, Thumbnail), HexError> {
    let mut collector = Collector::new(store, opts.max_sections);
    let geo = collector.geometry();
    let sink = PhaseSink::new(ctl, Phase::Classifying, left.len());
    let inner = Ctl {
        cancel: ctl.cancel,
        progress: &sink,
    };
    let f = |offset: u64,
             bytes: &[u8]|
     -> ControlFlow<Result<ChunkScan, HexError>, Result<ChunkScan, HexError>> {
        match scan_left_chunk(right, store, offset, bytes, &geo) {
            Ok(scan) => ControlFlow::Continue(Ok(scan)),
            Err(e) => ControlFlow::Break(Err(e)),
        }
    };
    for window in windows(left.len(), opts.stream.effective_threads()) {
        sink.base(window.start);
        let visit = visit_chunks(left.source(), window, &opts.stream, &inner, &f)
            .map_err(|e| HexError::from_vfs(left.display(), e))?;
        for value in visit.values {
            collector.add(value?);
        }
    }
    Ok(collector.finish())
}

/// The paired pieces of one left chunk, compared with their right bytes.
fn scan_left_chunk(
    right: &HexDocument,
    store: &SegmentStore,
    offset: u64,
    bytes: &[u8],
    geo: &sections::Geometry,
) -> Result<ChunkScan, HexError> {
    let end = offset + bytes.len() as u64;
    let mut scan = ChunkScan::default();
    let first = store.first_at_left(offset);
    let mut col = store.column_of(first);
    // The paired pieces: (left start, right start, len, column).
    let mut pieces = Vec::new();
    let mut i = first;
    while let Some(seg) = store.get(i) {
        if seg.left >= end {
            break;
        }
        if seg.kind == SegmentKind::Paired {
            let from = seg.left.max(offset);
            let to = seg.left_end().min(end);
            if from < to {
                pieces.push((
                    from,
                    seg.right + (from - seg.left),
                    to - from,
                    col + (from - seg.left),
                ));
            }
        }
        col += seg.len;
        i += 1;
    }
    let (Some(first), Some(last)) = (pieces.first(), pieces.last()) else {
        return Ok(scan);
    };
    let span = (last.1 + last.2).saturating_sub(first.1);
    if span <= 2 * bytes.len() as u64 + (64 << 10) {
        // One read covers every piece.
        let mut rb = vec![0u8; span as usize];
        right.read_bulk(first.1, &mut rb)?;
        for &(l, r, len, c) in &pieces {
            let a = &bytes[(l - offset) as usize..(l - offset + len) as usize];
            let b = &rb[(r - first.1) as usize..(r - first.1 + len) as usize];
            scan.scan(a, b, c, geo);
        }
    } else {
        for &(l, r, len, c) in &pieces {
            let mut rb = vec![0u8; len as usize];
            right.read_bulk(r, &mut rb)?;
            let a = &bytes[(l - offset) as usize..(l - offset + len) as usize];
            scan.scan(a, &rb, c, geo);
        }
    }
    Ok(scan)
}

/// A progress sink that reports the visitor's bytes as `phase`, offset by the window's start, over `total`.
pub(crate) struct PhaseSink<'a> {
    inner: &'a dyn ProgressSink,
    phase: Phase,
    total: u64,
    base: std::sync::atomic::AtomicU64,
}

impl<'a> PhaseSink<'a> {
    pub(crate) fn new(ctl: &Ctl<'a>, phase: Phase, total: u64) -> PhaseSink<'a> {
        PhaseSink {
            inner: ctl.progress,
            phase,
            total,
            base: std::sync::atomic::AtomicU64::new(0),
        }
    }

    pub(crate) fn base(&self, base: u64) {
        self.base.store(base, std::sync::atomic::Ordering::Relaxed);
    }
}

impl ProgressSink for PhaseSink<'_> {
    fn report(&self, progress: Progress) {
        let done = self.base.load(std::sync::atomic::Ordering::Relaxed) + progress.done;
        self.inner.report(Progress {
            phase: self.phase,
            done: done.min(self.total),
            total: Some(self.total),
            ..progress
        });
    }

    fn due(&self, _phase: Phase) -> bool {
        self.inner.due(self.phase)
    }
}

/// Shared documents of a comparison (the Service keeps them in `Arc`s).
pub type DocPair = hikasa_core::Pair<Arc<HexDocument>>;

/// Test entry points (hidden): Fast and Complete with another block hash, to plant collisions (A12.6).
#[doc(hidden)]
pub mod testing {
    use super::*;

    /// [`align_fast`] or [`align_complete`] (`complete`) with `hash` as the block hash.
    ///
    /// # Errors
    ///
    /// As [`align`].
    pub fn align_blocks_with_hash(
        left: &HexDocument,
        right: &HexDocument,
        opts: &HexAlignOptions,
        complete: bool,
        hash: fn(&[u8]) -> u64,
        ctl: &Ctl<'_>,
    ) -> Result<HexAlignment, HexError> {
        fast::align_blocks(left, right, opts, complete, hash, ctl)
    }

    /// The edit cost (deleted + inserted bytes) of `hikasa_diff`'s Myers on two byte strings, as the byte-level
    /// window runs it (before the choice between its layout and the None-style one).
    ///
    /// # Errors
    ///
    /// Cancellation.
    pub fn myers_cost(a: &[u8], b: &[u8]) -> Result<u64, HexError> {
        complete::myers_cost(a, b)
    }
}
