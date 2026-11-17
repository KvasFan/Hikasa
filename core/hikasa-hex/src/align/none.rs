//! None alignment (HX-002; D-328): byte n with byte n, and its scan.

use std::ops::ControlFlow;

use hikasa_core::{Ctl, Phase};
use hikasa_vfs::visit_chunk_pairs;

use super::sections::{ChunkScan, Collector, Sections, Thumbnail};
use super::store::{SegmentKind, SegmentStore, StoreBuilder};
use super::{HexAlignOptions, PhaseSink, windows};
use crate::document::HexDocument;
use crate::error::HexError;

/// One `Paired` segment of the shorter length plus the longer file's tail.
pub(crate) fn none_store(left: u64, right: u64) -> SegmentStore {
    let mut b = StoreBuilder::default();
    b.push(SegmentKind::Paired, left.min(right));
    if left > right {
        b.push(SegmentKind::LeftOnly, left - right);
    } else {
        b.push(SegmentKind::RightOnly, right - left);
    }
    b.finish()
}

/// The None scan (normative, D-328): the pair visitor over the paired range finds each chunk's runs of differing
/// bytes (32 bytes at a time with `u64` XOR words, byte-exact at the run edges), its same/different counts and
/// its thumbnail contributions; runs merge across chunk edges with the 8-column rule and the tail is a section.
/// It never breaks early, and its result is identical streamed, mapped and with any thread count.
///
/// `store` must be the None store of the two lengths. Returns the sections, the same and different counts and
/// the thumbnail.
///
/// # Errors
///
/// [`HexError::Truncated`] when an input is shorter than when it was opened; a read error; cancellation.
pub fn scan_none(
    left: &HexDocument,
    right: &HexDocument,
    store: &SegmentStore,
    opts: &HexAlignOptions,
    ctl: &Ctl<'_>,
) -> Result<(Sections, u64, u64, Thumbnail), HexError> {
    let mut collector = Collector::new(store, opts.max_sections);
    let geo = collector.geometry();
    let paired = left.len().min(right.len());
    let sink = PhaseSink::new(ctl, Phase::Classifying, paired);
    let inner = Ctl {
        cancel: ctl.cancel,
        progress: &sink,
    };
    let f = |offset: u64, a: &[u8], b: &[u8]| {
        let mut scan = ChunkScan::default();
        scan.scan(a, b, offset, &geo);
        ControlFlow::<ChunkScan, ChunkScan>::Continue(scan)
    };
    for window in windows(paired, opts.stream.effective_threads()) {
        sink.base(window.start);
        let visit = visit_chunk_pairs(
            left.source(),
            right.source(),
            window,
            &opts.stream,
            &inner,
            &f,
        )
        .map_err(|e| HexError::from_vfs(left.display(), e))?;
        for scan in visit.values {
            collector.add(scan);
        }
    }
    Ok(collector.finish())
}
