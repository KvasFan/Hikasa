//! Private storage of the model (§1.6.6): a chunked node arena whose children are contiguous id blocks, both
//! sides' metadata inline in the node (≤ 256 bytes per node, names excluded), one chunked name arena per side,
//! a folder table with the counters (D-55), and side tables for rare data (link targets, CRCs, lossy raw names,
//! errors, content outcomes with payloads, flatten folders).
//!
//! Every mutation keeps the counters of every ancestor equal to a recount (INV-FOLDER-5): a change yields a
//! delta that is applied to each ancestor from the parent up to the root, adding the change of each ancestor's
//! own category on the way (§2.6.4).

use std::collections::BTreeMap;
use std::ffi::OsString;

use hikasa_core::{
    CaseSensitivity, ContentKind, ContentMethod, ContentOutcome, ContentResult, FileTime, Pair,
    Spec, Warning, WarningCode,
};
use hikasa_vfs::{Attrs, DosAttrs, Entry, EntryKind, UnixMeta};

use crate::counts::{AggDelta, Aggregates, SideTotals, Slot};
use crate::model::BaseInfo;
use crate::model::node::{
    ContentSummary, EntryView, NodeError, NodeFlags, NodeId, NodeKind, NodeView, ScanStatus,
};
use crate::quick::DiffFlags;
use crate::resolve::ResolvedRules;
use crate::state::{Category, CompareState, file_category, pair_state};
use crate::stats::ContentCounts;
use crate::time_cmp::TimeRelation;

/// "No index" marker of `u32` links.
pub(crate) const NONE: u32 = u32::MAX;

/// Nodes per arena chunk (the slack of the arena is at most one chunk).
const CHUNK: usize = 4096;

/// Bytes per name-arena chunk (a longer name gets a chunk of its own).
const NAME_CHUNK: usize = 64 << 10;

/// A vector of fixed-capacity chunks: pushing never moves existing items, and unused capacity stays below one
/// chunk (a doubling `Vec` would keep up to half of a large arena unused, §1.6.6 bound).
#[derive(Debug)]
pub(crate) struct ChunkVec<T> {
    chunks: Vec<Vec<T>>,
    len: usize,
    /// Empty chunks allocated (and touched) outside the write lock for the next pushes of a large batch.
    spare: Vec<Vec<T>>,
}

impl<T> Default for ChunkVec<T> {
    fn default() -> ChunkVec<T> {
        ChunkVec {
            chunks: Vec::new(),
            len: 0,
            spare: Vec::new(),
        }
    }
}

impl<T: Copy> ChunkVec<T> {
    /// `count` empty chunks whose memory was written once (so that filling them under the write lock causes no
    /// page faults). Built outside the lock; `None` when `cancelled()` turns true meanwhile.
    pub(crate) fn prepare_chunks(
        count: usize,
        template: T,
        cancelled: &dyn Fn() -> bool,
    ) -> Option<Vec<Vec<T>>> {
        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            if cancelled() {
                return None;
            }
            let mut chunk = Vec::with_capacity(CHUNK);
            chunk.resize(CHUNK, template);
            chunk.clear();
            out.push(chunk);
        }
        Some(out)
    }
}

impl<T> ChunkVec<T> {
    /// Chunks of [`ChunkVec::prepare_chunks`] for the next pushes.
    pub(crate) fn add_spares(&mut self, spares: Vec<Vec<T>>) {
        self.spare.extend(
            spares
                .into_iter()
                .filter(|c| c.capacity() >= CHUNK && c.is_empty()),
        );
    }

    /// Takes back the spare chunks a batch did not use.
    pub(crate) fn take_spares(&mut self) -> Vec<Vec<T>> {
        std::mem::take(&mut self.spare)
    }

    pub(crate) fn push(&mut self, value: T) -> usize {
        let index = self.len;
        if self.chunks.last().is_none_or(|c| c.len() == CHUNK) {
            let chunk = self
                .spare
                .pop()
                .unwrap_or_else(|| Vec::with_capacity(CHUNK));
            self.chunks.push(chunk);
        }
        if let Some(chunk) = self.chunks.last_mut() {
            chunk.push(value);
        }
        self.len += 1;
        index
    }

    pub(crate) fn get(&self, index: usize) -> Option<&T> {
        self.chunks.get(index / CHUNK)?.get(index % CHUNK)
    }

    pub(crate) fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        self.chunks.get_mut(index / CHUNK)?.get_mut(index % CHUNK)
    }

    pub(crate) fn len(&self) -> usize {
        self.len
    }
}

/// Position of a name in a [`NameArena`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct NameRef {
    chunk: u32,
    start: u32,
    len: u32,
}

/// Names of one side, appended to large string chunks.
#[derive(Debug, Default)]
pub(crate) struct NameArena {
    chunks: Vec<String>,
}

impl NameArena {
    fn push(&mut self, name: &str) -> NameRef {
        let fits = self
            .chunks
            .last()
            .is_some_and(|c| c.capacity() - c.len() >= name.len());
        if !fits {
            self.chunks
                .push(String::with_capacity(NAME_CHUNK.max(name.len())));
        }
        let chunk = self.chunks.len().saturating_sub(1);
        let Some(text) = self.chunks.last_mut() else {
            return NameRef::default();
        };
        let start = text.len();
        text.push_str(name);
        NameRef {
            chunk: u32::try_from(chunk).unwrap_or(u32::MAX),
            start: u32::try_from(start).unwrap_or(u32::MAX),
            len: u32::try_from(name.len()).unwrap_or(0),
        }
    }

    fn get(&self, r: NameRef) -> &str {
        let start = r.start as usize;
        self.chunks
            .get(r.chunk as usize)
            .and_then(|c| c.get(start..start + r.len as usize))
            .unwrap_or("")
    }

    /// Appends a prepared chunk (a batch's names, exactly sized) and returns its chunk index.
    fn adopt(&mut self, mut chunk: String) -> u32 {
        chunk.shrink_to_fit();
        self.chunks.push(chunk);
        u32::try_from(self.chunks.len().saturating_sub(1)).unwrap_or(u32::MAX)
    }

    /// Bytes reserved by the arena (for the memory bench).
    pub(crate) fn reserved(&self) -> usize {
        self.chunks.iter().map(String::capacity).sum()
    }
}

/// Side-record bits.
pub(crate) mod bits {
    /// The side has an entry.
    pub(crate) const PRESENT: u8 = 1;
    /// The name is lossy (raw name in a side table).
    pub(crate) const LOSSY: u8 = 2;
    /// A followed symlink.
    pub(crate) const FOLLOWED: u8 = 4;
    /// Hidden by a filter on this side.
    pub(crate) const FILTERED: u8 = 8;
    /// Name-only entry (D-45).
    pub(crate) const STAT_SKIPPED: u8 = 16;
    /// The metadata could not be read (error-only entry).
    pub(crate) const NO_META: u8 = 32;
    /// A link target is in the side table.
    pub(crate) const LINK: u8 = 64;
    /// A CRC is in the side table.
    pub(crate) const CRC: u8 = 128;
}

/// Sentinel nanoseconds of a missing time.
const NO_TIME: u32 = u32::MAX;

/// Metadata of one side's entry, inline in the node (64 bytes).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SideRec {
    size: u64,
    mtime_secs: i64,
    ctime_secs: i64,
    mtime_nanos: u32,
    ctime_nanos: u32,
    attr_a: u32,
    attr_b: u32,
    attr_c: u32,
    /// Flatten folder index (`NONE` outside flatten mode).
    dir: u32,
    name: NameRef,
    kind: u8,
    attr_tag: u8,
    pub(crate) bits: u8,
}

impl SideRec {
    pub(crate) fn present(&self) -> bool {
        self.bits & bits::PRESENT != 0
    }

    pub(crate) fn has(&self, bit: u8) -> bool {
        self.bits & bit != 0
    }

    pub(crate) fn kind(&self) -> EntryKind {
        match self.kind {
            0 => EntryKind::Dir,
            1 => EntryKind::File,
            2 => EntryKind::Symlink,
            _ => EntryKind::Other,
        }
    }

    pub(crate) fn size(&self) -> u64 {
        self.size
    }

    pub(crate) fn mtime(&self) -> Option<FileTime> {
        time(self.mtime_secs, self.mtime_nanos)
    }

    fn ctime(&self) -> Option<FileTime> {
        time(self.ctime_secs, self.ctime_nanos)
    }

    pub(crate) fn attrs(&self) -> Attrs {
        match self.attr_tag {
            1 => Attrs::Dos {
                attrs: DosAttrs::from_bits_retain(self.attr_a),
            },
            2 => Attrs::Unix {
                meta: UnixMeta {
                    mode: self.attr_a,
                    uid: self.attr_b,
                    gid: self.attr_c,
                },
            },
            _ => Attrs::None,
        }
    }

    pub(crate) fn dir(&self) -> Option<u32> {
        (self.dir != NONE).then_some(self.dir)
    }

    /// The record of `entry` with its name position, flatten folder and side bits.
    fn of(entry: &Entry, name: NameRef, dir: Option<u32>, bits: u8) -> SideRec {
        let (mtime_secs, mtime_nanos) = split_time(entry.mtime);
        let (ctime_secs, ctime_nanos) = split_time(entry.ctime);
        let (attr_tag, attr_a, attr_b, attr_c) = match entry.attrs {
            Attrs::None => (0, 0, 0, 0),
            Attrs::Dos { attrs } => (1, attrs.bits(), 0, 0),
            Attrs::Unix { meta } => (2, meta.mode, meta.uid, meta.gid),
        };
        SideRec {
            size: entry.size,
            mtime_secs,
            ctime_secs,
            mtime_nanos,
            ctime_nanos,
            attr_a,
            attr_b,
            attr_c,
            dir: dir.unwrap_or(NONE),
            name,
            kind: entry_kind_code(entry.kind),
            attr_tag,
            bits,
        }
    }

    /// True when present and not filtered.
    fn counts(&self) -> bool {
        self.present() && !self.has(bits::FILTERED)
    }
}

fn time(secs: i64, nanos: u32) -> Option<FileTime> {
    if nanos == NO_TIME {
        None
    } else {
        FileTime::from_unix(secs, nanos)
    }
}

fn split_time(t: Option<FileTime>) -> (i64, u32) {
    match t {
        Some(t) => (t.unix_seconds(), t.subsec_nanos()),
        None => (0, NO_TIME),
    }
}

/// Internal node bits (not public flags).
pub(crate) mod internal {
    /// A child of a folder whose listing failed on the other side: its state is Unknown, not orphan.
    pub(crate) const PARTIAL_UNKNOWN: u8 = 1;
    /// A content test is pending (state Unknown until it reports).
    pub(crate) const CONTENT_PENDING: u8 = 2;
}

/// One node (both sides inline): 160 bytes.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Node {
    pub(crate) parent: u32,
    pub(crate) first_child: u32,
    pub(crate) child_count: u32,
    /// Index into the folder table (`NONE` for file-like nodes).
    folder: u32,
    pub(crate) kind: u8,
    /// Stored state of file-like nodes (folders derive theirs).
    state: u8,
    scan: u8,
    time: u8,
    content: u8,
    pub(crate) internal: u8,
    pub(crate) diffs: u16,
    pub(crate) flags: u16,
    pub(crate) sides: [SideRec; 2],
}

/// Counters of one folder (D-55).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct FolderData {
    pub(crate) agg: Aggregates,
    pub(crate) totals: Pair<SideTotals>,
}

const fn kind_code(kind: NodeKind) -> u8 {
    match kind {
        NodeKind::Folder => 0,
        NodeKind::File => 1,
        NodeKind::Symlink => 2,
        NodeKind::Other => 3,
    }
}

const fn entry_kind_code(kind: EntryKind) -> u8 {
    match kind {
        EntryKind::Dir => 0,
        EntryKind::File => 1,
        EntryKind::Symlink => 2,
        EntryKind::Other => 3,
    }
}

fn state_code(state: CompareState) -> u8 {
    CompareState::ALL
        .iter()
        .position(|&s| s == state)
        .and_then(|p| u8::try_from(p).ok())
        .unwrap_or(0)
}

const fn scan_code(scan: ScanStatus) -> u8 {
    match scan {
        ScanStatus::NotApplicable => 0,
        ScanStatus::NotScanned => 1,
        ScanStatus::Scanned => 2,
        ScanStatus::Failed => 3,
    }
}

const fn time_code(time: Option<TimeRelation>) -> u8 {
    match time {
        None => 0,
        Some(TimeRelation::Same) => 1,
        Some(TimeRelation::LeftNewer) => 2,
        Some(TimeRelation::RightNewer) => 3,
    }
}

const METHODS: [ContentMethod; 4] = [
    ContentMethod::None,
    ContentMethod::Crc,
    ContentMethod::Binary,
    ContentMethod::Rules,
];

fn content_code(summary: Option<ContentSummary>) -> u8 {
    let Some(s) = summary else {
        return 0;
    };
    let method = METHODS.iter().position(|&m| m == s.method).unwrap_or(0);
    let kind = ContentKind::ALL
        .iter()
        .position(|&k| k == s.kind)
        .unwrap_or(0);
    u8::try_from(1 + method * 8 + kind).unwrap_or(0)
}

impl Node {
    pub(crate) fn kind(&self) -> NodeKind {
        match self.kind {
            0 => NodeKind::Folder,
            1 => NodeKind::File,
            2 => NodeKind::Symlink,
            _ => NodeKind::Other,
        }
    }

    pub(crate) fn is_folder(&self) -> bool {
        self.kind == 0
    }

    pub(crate) fn stored_state(&self) -> CompareState {
        CompareState::ALL
            .get(self.state as usize)
            .copied()
            .unwrap_or(CompareState::Unknown)
    }

    pub(crate) fn scan(&self) -> ScanStatus {
        match self.scan {
            0 => ScanStatus::NotApplicable,
            1 => ScanStatus::NotScanned,
            2 => ScanStatus::Scanned,
            _ => ScanStatus::Failed,
        }
    }

    pub(crate) fn time(&self) -> Option<TimeRelation> {
        match self.time {
            1 => Some(TimeRelation::Same),
            2 => Some(TimeRelation::LeftNewer),
            3 => Some(TimeRelation::RightNewer),
            _ => None,
        }
    }

    pub(crate) fn content(&self) -> Option<ContentSummary> {
        if self.content == 0 {
            return None;
        }
        let code = usize::from(self.content - 1);
        Some(ContentSummary {
            method: METHODS
                .get(code / 8)
                .copied()
                .unwrap_or(ContentMethod::None),
            kind: ContentKind::ALL
                .get(code % 8)
                .copied()
                .unwrap_or(ContentKind::Error),
        })
    }

    pub(crate) fn diffs(&self) -> DiffFlags {
        DiffFlags::from_bits_retain(self.diffs)
    }

    pub(crate) fn flags(&self) -> NodeFlags {
        NodeFlags::from_bits_retain(self.flags)
    }

    pub(crate) fn children(&self) -> std::ops::Range<u32> {
        if self.first_child == NONE {
            0..0
        } else {
            self.first_child..self.first_child.saturating_add(self.child_count)
        }
    }

    /// Number of present sides and whether every present side is filtered.
    fn presence(&self) -> (usize, bool) {
        let present = self.sides.iter().filter(|s| s.present()).count();
        let all_filtered = present > 0
            && self
                .sides
                .iter()
                .filter(|s| s.present())
                .all(|s| s.has(bits::FILTERED));
        (present, all_filtered)
    }
}

/// Input of one side of a new node.
pub(crate) struct NewSide<'a> {
    /// The entry (metadata copied into the node).
    pub(crate) entry: &'a Entry,
    /// Hidden by a filter on this side.
    pub(crate) filtered: bool,
    /// Name-only entry.
    pub(crate) stat_skipped: bool,
    /// The metadata could not be read (only the name is known).
    pub(crate) no_meta: bool,
    /// Flatten folder index.
    pub(crate) dir: Option<u32>,
}

/// Input of a new node.
pub(crate) struct NewNode<'a> {
    pub(crate) kind: NodeKind,
    /// State of a file-like node (folders derive theirs).
    pub(crate) state: CompareState,
    pub(crate) scan: ScanStatus,
    pub(crate) diffs: DiffFlags,
    pub(crate) time: Option<TimeRelation>,
    pub(crate) flags: NodeFlags,
    pub(crate) internal: u8,
    pub(crate) sides: [Option<NewSide<'a>>; 2],
    pub(crate) errors: Vec<NodeError>,
}

/// Children prepared outside the write lock by [`stage`]: the nodes with batch-relative name chunks and folder
/// indices, the batch's names of each side (one exactly sized chunk per side), the side-table entries by offset in
/// the batch, and the counter delta and totals the batch adds to its folder.
pub(crate) struct Staged {
    nodes: Vec<Node>,
    /// Touched, empty node chunks for the insertion (large batches).
    spares: Vec<Vec<Node>>,
    names: [String; 2],
    folders: u32,
    errors: Vec<(u32, Vec<NodeError>)>,
    links: [Vec<(u32, LinkTarget)>; 2],
    crcs: [Vec<(u32, u32)>; 2],
    raw_names: [Vec<(u32, OsString)>; 2],
    delta: AggDelta,
    totals: Pair<SideTotals>,
    not_compared: u32,
}

/// Children staged between two cancellation checks.
const STAGE_CHECK_EVERY: u32 = 1024;

/// Prepares the children of folder `parent` (in their final order) without the store: node records, names,
/// side-table entries, counter contributions (`override_quick` as in the model, §2.6.3). `None` when `cancelled()`
/// turns true (checked every 1024 children), so building a huge batch never delays cancellation; only
/// [`Store::insert_staged`] runs under the write lock.
pub(crate) fn stage<'e>(
    parent: u32,
    override_quick: bool,
    capacity: usize,
    children: impl Iterator<Item = NewNode<'e>>,
    cancelled: &dyn Fn() -> bool,
) -> Option<Staged> {
    let mut batch = Staged {
        nodes: Vec::with_capacity(capacity),
        spares: Vec::new(),
        names: [String::new(), String::new()],
        folders: 0,
        errors: Vec::new(),
        links: [Vec::new(), Vec::new()],
        crcs: [Vec::new(), Vec::new()],
        raw_names: [Vec::new(), Vec::new()],
        delta: AggDelta::default(),
        totals: Pair::new(SideTotals::default(), SideTotals::default()),
        not_compared: 0,
    };
    let mut offset: u32 = 0;
    for child in children {
        if offset.is_multiple_of(STAGE_CHECK_EVERY) && cancelled() {
            return None;
        }
        let mut sides = [SideRec::default(); 2];
        for (index, side) in child.sides.iter().enumerate() {
            if let Some(s) = side
                && let Some(slot) = sides.get_mut(index)
            {
                *slot = stage_side(&mut batch, index, offset, s);
            }
        }
        let folder = if child.kind == NodeKind::Folder {
            let rel = batch.folders;
            batch.folders = batch.folders.saturating_add(1);
            rel
        } else {
            NONE
        };
        if !child.errors.is_empty() {
            batch.errors.push((offset, child.errors));
        }
        let node = Node {
            parent,
            first_child: NONE,
            child_count: 0,
            folder,
            kind: kind_code(child.kind),
            state: state_code(child.state),
            scan: scan_code(child.scan),
            time: time_code(child.time),
            content: 0,
            internal: child.internal,
            diffs: child.diffs.bits(),
            flags: child.flags.bits(),
            sides,
        };
        // A new folder has no counters yet.
        let category = |iu: bool| {
            if node.is_folder() {
                own_category_of(&node, Some(&Aggregates::default()), iu)
            } else {
                file_category(
                    node.stored_state(),
                    node.diffs(),
                    node.time(),
                    node.content().map(|c| c.kind),
                    override_quick,
                    iu,
                )
            }
        };
        let slot = if node.is_folder() {
            Slot::Folders
        } else {
            Slot::Files
        };
        batch.delta.add(false, slot, category(false), 1);
        batch.delta.add(true, slot, category(true), 1);
        let t = Store::totals_contribution(&node);
        batch.totals.left.add(&t.left);
        batch.totals.right.add(&t.right);
        if node.flags().contains(NodeFlags::CONTENT_SKIPPED) {
            batch.not_compared = batch.not_compared.saturating_add(1);
        }
        batch.nodes.push(node);
        offset = offset.saturating_add(1);
    }
    Some(batch)
}

impl Staged {
    /// A large batch brings its node chunks along, already touched, so that the copy under the write lock causes
    /// no page fault. Done outside the lock (after the batch's listing was freed); `None` when cancelled.
    pub(crate) fn with_spares(mut self, cancelled: &dyn Fn() -> bool) -> Option<Staged> {
        if self.nodes.len() >= CHUNK
            && let Some(&template) = self.nodes.first()
        {
            self.spares = ChunkVec::prepare_chunks(
                self.nodes.len().div_ceil(CHUNK) + 1,
                template,
                cancelled,
            )?;
        }
        Some(self)
    }
}

/// The side record of a staged child (its name goes to the batch's chunk of that side).
fn stage_side(batch: &mut Staged, side: usize, offset: u32, s: &NewSide<'_>) -> SideRec {
    let entry = s.entry;
    let mut bits_set = bits::PRESENT;
    let name = match batch.names.get_mut(side) {
        Some(text) => {
            let start = text.len();
            text.push_str(entry.name.as_str());
            NameRef {
                chunk: 0,
                start: u32::try_from(start).unwrap_or(u32::MAX),
                len: u32::try_from(entry.name.as_str().len()).unwrap_or(0),
            }
        }
        None => NameRef::default(),
    };
    if entry.name.is_lossy() {
        bits_set |= bits::LOSSY;
        if let Some(table) = batch.raw_names.get_mut(side) {
            table.push((offset, entry.name.to_os_string()));
        }
    }
    if entry.followed {
        bits_set |= bits::FOLLOWED;
    }
    if s.filtered {
        bits_set |= bits::FILTERED;
    }
    if s.stat_skipped {
        bits_set |= bits::STAT_SKIPPED;
    }
    if s.no_meta {
        bits_set |= bits::NO_META;
    }
    if let Some(text) = &entry.link_target {
        bits_set |= bits::LINK;
        if let Some(table) = batch.links.get_mut(side) {
            table.push((offset, LinkTarget { text: text.clone() }));
        }
    }
    if let Some(crc) = entry.crc32 {
        bits_set |= bits::CRC;
        if let Some(table) = batch.crcs.get_mut(side) {
            table.push((offset, crc));
        }
    }
    SideRec::of(entry, name, s.dir, bits_set)
}

/// A flatten-mode folder of one side.
#[derive(Debug, Clone)]
pub(crate) struct DirInfo {
    /// `/`-separated path relative to the base.
    pub(crate) text: String,
    /// Spec of the folder (when the back-end gave one).
    pub(crate) spec: Option<Spec>,
}

/// A symlink target (its UTF-8 text; exact targets are compared on the listed entries, §1.2.7 item 9).
#[derive(Debug, Clone)]
pub(crate) struct LinkTarget {
    pub(crate) text: String,
}

/// The whole storage.
#[derive(Debug)]
pub(crate) struct Store {
    nodes: ChunkVec<Node>,
    folders: ChunkVec<FolderData>,
    names: [NameArena; 2],
    pub(crate) dirs: [Vec<DirInfo>; 2],
    errors: BTreeMap<u32, Vec<NodeError>>,
    outcomes: BTreeMap<u32, ContentOutcome>,
    links: [BTreeMap<u32, LinkTarget>; 2],
    crcs: [BTreeMap<u32, u32>; 2],
    raw_names: [BTreeMap<u32, OsString>; 2],
    pub(crate) content_counts: ContentCounts,
    pub(crate) bases: Pair<Option<BaseInfo>>,
    pub(crate) case: Option<CaseSensitivity>,
    pub(crate) resolved: Option<ResolvedRules>,
    pub(crate) warnings: Vec<Warning>,
    pub(crate) complete: bool,
    pub(crate) override_quick: bool,
    pub(crate) flatten: bool,
}

impl Store {
    /// A store whose root is the base pair (no entries yet, `NotScanned`).
    pub(crate) fn new(override_quick: bool, flatten: bool) -> Store {
        let mut store = Store {
            nodes: ChunkVec::default(),
            folders: ChunkVec::default(),
            names: [NameArena::default(), NameArena::default()],
            dirs: [Vec::new(), Vec::new()],
            errors: BTreeMap::new(),
            outcomes: BTreeMap::new(),
            links: [BTreeMap::new(), BTreeMap::new()],
            crcs: [BTreeMap::new(), BTreeMap::new()],
            raw_names: [BTreeMap::new(), BTreeMap::new()],
            content_counts: ContentCounts::default(),
            bases: Pair::new(None, None),
            case: None,
            resolved: None,
            warnings: Vec::new(),
            complete: false,
            override_quick,
            flatten,
        };
        let folder = store.folders.push(FolderData::default());
        store.nodes.push(Node {
            parent: NONE,
            first_child: NONE,
            child_count: 0,
            folder: u32::try_from(folder).unwrap_or(NONE),
            kind: kind_code(NodeKind::Folder),
            state: state_code(CompareState::Unknown),
            scan: scan_code(ScanStatus::NotScanned),
            time: 0,
            content: 0,
            internal: 0,
            diffs: 0,
            flags: 0,
            sides: [SideRec::default(); 2],
        });
        store
    }

    pub(crate) fn len(&self) -> u32 {
        u32::try_from(self.nodes.len()).unwrap_or(u32::MAX)
    }

    pub(crate) fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id.0 as usize)
    }

    fn folder(&self, node: &Node) -> Option<&FolderData> {
        if node.folder == NONE {
            None
        } else {
            self.folders.get(node.folder as usize)
        }
    }

    /// Sets the base entries on the root (after the bases were stat'ed).
    pub(crate) fn set_bases(
        &mut self,
        bases: Pair<BaseInfo>,
        case: CaseSensitivity,
        resolved: ResolvedRules,
    ) {
        let left = self.side_rec(0, 0, &bases.left.entry, false, false, false, None);
        let right = self.side_rec(1, 0, &bases.right.entry, false, false, false, None);
        if let Some(root) = self.nodes.get_mut(0) {
            root.sides = [left, right];
        }
        self.bases = Pair::new(Some(bases.left), Some(bases.right));
        self.case = Some(case);
        self.resolved = Some(resolved);
    }

    /// Builds a side record for node `id` (side tables are filled for `id`).
    #[allow(clippy::too_many_arguments)]
    fn side_rec(
        &mut self,
        side: usize,
        id: u32,
        entry: &Entry,
        filtered: bool,
        stat_skipped: bool,
        no_meta: bool,
        dir: Option<u32>,
    ) -> SideRec {
        let mut bits_set = bits::PRESENT;
        let name = match self.names.get_mut(side) {
            Some(arena) => arena.push(entry.name.as_str()),
            None => NameRef::default(),
        };
        if entry.name.is_lossy() {
            bits_set |= bits::LOSSY;
            if let Some(table) = self.raw_names.get_mut(side) {
                table.insert(id, entry.name.to_os_string());
            }
        }
        if entry.followed {
            bits_set |= bits::FOLLOWED;
        }
        if filtered {
            bits_set |= bits::FILTERED;
        }
        if stat_skipped {
            bits_set |= bits::STAT_SKIPPED;
        }
        if no_meta {
            bits_set |= bits::NO_META;
        }
        if let Some(text) = &entry.link_target {
            bits_set |= bits::LINK;
            if let Some(table) = self.links.get_mut(side) {
                table.insert(id, LinkTarget { text: text.clone() });
            }
        }
        if let Some(crc) = entry.crc32 {
            bits_set |= bits::CRC;
            if let Some(table) = self.crcs.get_mut(side) {
                table.insert(id, crc);
            }
        }
        SideRec::of(entry, name, dir, bits_set)
    }

    // ----------------------------------------------------------------------------------------------------
    // Categories, counters, colours
    // ----------------------------------------------------------------------------------------------------

    /// Own category of a folder node (§2.6.4, as amended in §1.6.8): a folder whose present sides are all filtered
    /// is `Filtered` even when it is an orphan (§2.3: "a node whose present sides are all filtered has state
    /// Filtered"); a folder that could not be listed (or closed a symlink loop) is `Error` even when it exists on
    /// one side only (flatten shows failed folders as one-sided rows), so a failure is never claimed as an orphan;
    /// a child of a failed listing is `Unknown` (§2.6.2).
    pub(crate) fn own_category(&self, node: &Node, iu: bool) -> Category {
        self.own_category_from(node, self.folder(node).map(|d| &d.agg), iu)
    }

    /// [`Store::own_category`] with explicit counters (the INV-FOLDER-5/6 recount uses recounted ones).
    pub(crate) fn own_category_from(
        &self,
        node: &Node,
        agg: Option<&Aggregates>,
        iu: bool,
    ) -> Category {
        own_category_of(node, agg, iu)
    }

    /// Display category of any node (§2.6.3), O(1).
    pub(crate) fn category_of(&self, node: &Node, iu: bool) -> Category {
        if node.is_folder() {
            return self.own_category(node, iu);
        }
        file_category(
            node.stored_state(),
            node.diffs(),
            node.time(),
            node.content().map(|c| c.kind),
            self.override_quick,
            iu,
        )
    }
}

/// The own category of a folder node with the given counters (§2.6.4); see [`Store::own_category`].
pub(crate) fn own_category_of(node: &Node, agg: Option<&Aggregates>, iu: bool) -> Category {
    let (present, all_filtered) = node.presence();
    if present == 0 {
        return Category::Unknown;
    }
    if all_filtered {
        return Category::Filtered;
    }
    if node.scan() == ScanStatus::Failed {
        return Category::Error;
    }
    if node.internal & internal::PARTIAL_UNKNOWN != 0 {
        return Category::Unknown;
    }
    if present == 1 {
        return if node.sides[0].present() {
            Category::LeftOrphan
        } else {
            Category::RightOrphan
        };
    }
    match node.scan() {
        ScanStatus::Failed => return Category::Error,
        ScanStatus::NotScanned | ScanStatus::NotApplicable => return Category::Unknown,
        ScanStatus::Scanned => {}
    }
    let Some(agg) = agg else {
        return Category::Unknown;
    };
    let a = agg.get(iu);
    if a.files.differences() + a.folders.differences() > 0 {
        Category::Different
    } else if a.files.error + a.folders.error > 0 {
        Category::Error
    } else if a.files.unknown + a.folders.unknown > 0 {
        Category::Unknown
    } else {
        Category::Same
    }
}

impl Store {
    /// Categories for both ignore-unimportant values.
    fn categories(&self, node: &Node) -> [Category; 2] {
        [self.category_of(node, false), self.category_of(node, true)]
    }

    /// Canonical state of a node (folders: `own_category(F, false)`).
    pub(crate) fn state_of(&self, node: &Node) -> CompareState {
        if node.is_folder() {
            self.own_category(node, false).state()
        } else {
            node.stored_state()
        }
    }

    pub(crate) fn aggregates(&self, node: &Node) -> Aggregates {
        self.folder(node).map(|d| d.agg).unwrap_or_default()
    }

    /// Totals of a subtree (a file-like node: its own present, non-filtered entries).
    pub(crate) fn totals(&self, node: &Node) -> Pair<SideTotals> {
        if let Some(data) = self.folder(node) {
            return data.totals;
        }
        let own = |rec: &SideRec| {
            if !rec.counts() {
                return SideTotals::default();
            }
            SideTotals {
                files: 1,
                folders: 0,
                bytes: if rec.kind() == EntryKind::File {
                    rec.size
                } else {
                    0
                },
            }
        };
        Pair::new(own(&node.sides[0]), own(&node.sides[1]))
    }

    /// Contribution of a node to its parent's totals.
    pub(crate) fn totals_contribution(node: &Node) -> Pair<SideTotals> {
        let one = |rec: &SideRec| {
            if !rec.counts() {
                return SideTotals::default();
            }
            if node.is_folder() {
                SideTotals {
                    files: 0,
                    folders: 1,
                    bytes: 0,
                }
            } else {
                SideTotals {
                    files: 1,
                    folders: 0,
                    bytes: if rec.kind() == EntryKind::File {
                        rec.size
                    } else {
                        0
                    },
                }
            }
        };
        Pair::new(one(&node.sides[0]), one(&node.sides[1]))
    }

    /// Applies `delta` (counters) and `totals` (added) to `start` and every ancestor, adding the change of each
    /// ancestor's own category to the delta passed further up (§2.6.4 maintenance).
    fn propagate(&mut self, start: u32, mut delta: AggDelta, totals: Pair<SideTotals>) {
        let mut current = start;
        while current != NONE {
            let Some(node) = self.nodes.get(current as usize).copied() else {
                break;
            };
            let old = self.categories(&node);
            if node.folder != NONE
                && let Some(data) = self.folders.get_mut(node.folder as usize)
            {
                data.agg.apply(&delta);
                data.totals.left.add(&totals.left);
                data.totals.right.add(&totals.right);
            }
            let new = self.categories(&node);
            if node.parent != NONE {
                delta.change(Slot::Folders, old, new);
            }
            if delta.is_zero() && totals.left.is_zero() && totals.right.is_zero() {
                break;
            }
            current = node.parent;
        }
    }

    // ----------------------------------------------------------------------------------------------------
    // Mutations
    // ----------------------------------------------------------------------------------------------------

    /// Appends a batch prepared by [`stage`] as one contiguous block under `folder` (which must have no children
    /// yet), sets the folder's scan status, flags and errors, and updates every ancestor's counters. Returns the id
    /// of the first child and the node chunks the batch did not need (to be freed outside the lock). The work under
    /// the write lock is a copy of the prepared nodes with their name and folder indices rebased, plus the side
    /// tables of the (rare) entries that have any, so a huge folder holds the lock briefly (§5.2, §5.3).
    pub(crate) fn insert_staged(
        &mut self,
        folder: NodeId,
        scan: ScanStatus,
        folder_flags: NodeFlags,
        folder_errors: Vec<NodeError>,
        batch: Staged,
    ) -> (u32, Vec<Vec<Node>>) {
        let Staged {
            nodes,
            spares,
            names,
            folders,
            errors,
            links,
            crcs,
            raw_names,
            mut delta,
            totals,
            not_compared,
        } = batch;
        let Some(parent) = self.nodes.get(folder.0 as usize).copied() else {
            return (NONE, spares);
        };
        self.nodes.add_spares(spares);
        let old_parent = self.categories(&parent);
        let first = self.len();
        let folder_base = u32::try_from(self.folders.len()).unwrap_or(NONE);
        for _ in 0..folders {
            self.folders.push(FolderData::default());
        }
        let mut chunks = [0u32; 2];
        for (side, text) in names.into_iter().enumerate() {
            if text.is_empty() {
                continue;
            }
            if let (Some(arena), Some(slot)) = (self.names.get_mut(side), chunks.get_mut(side)) {
                *slot = arena.adopt(text);
            }
        }
        let mut count: u32 = 0;
        for mut node in nodes {
            for (side, rec) in node.sides.iter_mut().enumerate() {
                if rec.present() {
                    rec.name.chunk = chunks.get(side).copied().unwrap_or(0);
                }
            }
            if node.folder != NONE {
                node.folder = folder_base.saturating_add(node.folder);
            }
            self.nodes.push(node);
            count = count.saturating_add(1);
        }
        let unused = self.nodes.take_spares();
        for (offset, list) in errors {
            self.errors.insert(first.saturating_add(offset), list);
        }
        for (side, list) in links.into_iter().enumerate() {
            if let Some(table) = self.links.get_mut(side) {
                table.extend(list.into_iter().map(|(o, l)| (first.saturating_add(o), l)));
            }
        }
        for (side, list) in crcs.into_iter().enumerate() {
            if let Some(table) = self.crcs.get_mut(side) {
                table.extend(list.into_iter().map(|(o, c)| (first.saturating_add(o), c)));
            }
        }
        for (side, list) in raw_names.into_iter().enumerate() {
            if let Some(table) = self.raw_names.get_mut(side) {
                table.extend(list.into_iter().map(|(o, r)| (first.saturating_add(o), r)));
            }
        }
        self.content_counts.not_compared = self
            .content_counts
            .not_compared
            .saturating_add(not_compared);
        if let Some(p) = self.nodes.get_mut(folder.0 as usize) {
            p.first_child = if count == 0 { NONE } else { first };
            p.child_count = count;
            p.scan = scan_code(scan);
            p.flags |= folder_flags.bits();
        }
        if !folder_errors.is_empty() {
            self.errors
                .entry(folder.0)
                .or_default()
                .extend(folder_errors);
        }
        // The folder itself: counters of its new children, then its own category change goes up.
        let Some(updated) = self.nodes.get(folder.0 as usize).copied() else {
            return (first, unused);
        };
        if updated.folder != NONE
            && let Some(data) = self.folders.get_mut(updated.folder as usize)
        {
            data.agg.apply(&delta);
            data.totals.left.add(&totals.left);
            data.totals.right.add(&totals.right);
        }
        let new_parent = self.categories(&updated);
        if updated.parent != NONE {
            delta.change(Slot::Folders, old_parent, new_parent);
            self.propagate(updated.parent, delta, totals);
        }
        (first, unused)
    }

    /// Applies a content outcome to a file node (§2.6.2) and updates every ancestor's counters.
    pub(crate) fn apply_content(&mut self, id: NodeId, outcome: ContentOutcome) {
        let Some(node) = self.nodes.get(id.0 as usize).copied() else {
            return;
        };
        let old = self.categories(&node);
        let kind = outcome.result.kind();
        let summary = ContentSummary {
            method: outcome.method,
            kind,
        };
        let (state, diffs) = pair_state(node.diffs(), node.time(), Some(kind), self.override_quick);
        let mut flags = node.flags();
        if outcome
            .warnings
            .iter()
            .any(|w| w.code == WarningCode::RulesSizeLimit)
        {
            flags |= NodeFlags::RULES_SKIPPED_SIZE;
        }
        if let ContentResult::Error { error } = &outcome.result {
            self.errors.entry(id.0).or_default().push(NodeError {
                side: None,
                error: error.clone(),
            });
        }
        let counts = &mut self.content_counts;
        let slot = match kind {
            ContentKind::BinarySame => &mut counts.binary_same,
            ContentKind::RulesSame => &mut counts.rules_same,
            ContentKind::UnimportantOnly => &mut counts.unimportant_only,
            ContentKind::ImportantDifferences => &mut counts.important_differences,
            ContentKind::BinaryDifferent => &mut counts.binary_different,
            ContentKind::Error => &mut counts.error,
        };
        *slot = slot.saturating_add(1);
        if !outcome.warnings.is_empty() || kind == ContentKind::Error {
            self.outcomes.insert(id.0, outcome);
        }
        let Some(target) = self.nodes.get_mut(id.0 as usize) else {
            return;
        };
        target.content = content_code(Some(summary));
        target.state = state_code(state);
        target.diffs = diffs.bits();
        target.flags = flags.bits();
        target.internal &= !internal::CONTENT_PENDING;
        let updated = *target;
        let new = self.categories(&updated);
        let mut delta = AggDelta::default();
        delta.change(Slot::Files, old, new);
        if updated.parent != NONE && !delta.is_zero() {
            self.propagate(updated.parent, delta, Pair::default());
        }
    }

    /// Adds a comparison-level warning.
    pub(crate) fn warn(&mut self, warning: Warning) {
        self.warnings.push(warning);
    }

    /// Ends the scan: sorts and deduplicates the warnings (deterministic output, T2) and records completeness.
    pub(crate) fn finish(&mut self, complete: bool) {
        self.warnings
            .sort_by(|a, b| (a.code, &a.spec, &a.message).cmp(&(b.code, &b.spec, &b.message)));
        self.warnings.dedup();
        self.complete = complete;
    }

    // ----------------------------------------------------------------------------------------------------
    // Reads
    // ----------------------------------------------------------------------------------------------------

    pub(crate) fn errors(&self, id: NodeId) -> &[NodeError] {
        self.errors.get(&id.0).map_or(&[], Vec::as_slice)
    }

    pub(crate) fn content_outcome(&self, id: NodeId) -> Option<ContentOutcome> {
        let node = self.node(id)?;
        let summary = node.content()?;
        if let Some(outcome) = self.outcomes.get(&id.0) {
            return Some(outcome.clone());
        }
        let result = match summary.kind {
            ContentKind::BinarySame => ContentResult::BinarySame,
            ContentKind::RulesSame => ContentResult::RulesSame,
            ContentKind::UnimportantOnly => ContentResult::UnimportantOnly,
            ContentKind::ImportantDifferences => ContentResult::ImportantDifferences,
            ContentKind::BinaryDifferent => ContentResult::BinaryDifferent,
            // Error outcomes are always stored; this arm is a defensive fallback.
            ContentKind::Error => return None,
        };
        Some(ContentOutcome {
            method: summary.method,
            result,
            warnings: Vec::new(),
        })
    }

    pub(crate) fn name(&self, side: usize, rec: &SideRec) -> &str {
        self.names.get(side).map_or("", |arena| arena.get(rec.name))
    }

    /// The exact OS name of a side's entry.
    pub(crate) fn raw_name(&self, side: usize, id: u32, rec: &SideRec) -> OsString {
        if rec.has(bits::LOSSY)
            && let Some(raw) = self.raw_names.get(side).and_then(|t| t.get(&id))
        {
            return raw.clone();
        }
        OsString::from(self.name(side, rec))
    }

    pub(crate) fn link_target(&self, side: usize, id: u32) -> Option<&LinkTarget> {
        self.links.get(side)?.get(&id)
    }

    pub(crate) fn entry_view(&self, node: &Node, id: u32, side: usize) -> Option<EntryView<'_>> {
        let rec = node.sides.get(side)?;
        if !rec.present() {
            return None;
        }
        let kind = if rec.has(bits::NO_META) {
            node.kind().entry_kind()
        } else {
            rec.kind()
        };
        Some(EntryView {
            name: self.name(side, rec),
            name_lossy: rec.has(bits::LOSSY),
            kind,
            size: if kind == EntryKind::Dir || rec.has(bits::STAT_SKIPPED) {
                0
            } else {
                rec.size
            },
            mtime: rec.mtime(),
            ctime: rec.ctime(),
            attrs: rec.attrs(),
            link_target: if rec.has(bits::LINK) {
                self.link_target(side, id).map(|l| l.text.as_str())
            } else {
                None
            },
            followed: rec.has(bits::FOLLOWED),
            crc32: if rec.has(bits::CRC) {
                self.crcs.get(side).and_then(|t| t.get(&id)).copied()
            } else {
                None
            },
            filtered: rec.has(bits::FILTERED),
            stat_skipped: rec.has(bits::STAT_SKIPPED),
            dir: rec
                .dir()
                .and_then(|d| self.dirs.get(side)?.get(d as usize))
                .map(|d| d.text.as_str()),
        })
    }

    pub(crate) fn view(&self, id: NodeId) -> Option<NodeView<'_>> {
        let node = self.node(id)?;
        Some(NodeView {
            id,
            parent: (node.parent != NONE).then_some(NodeId(node.parent)),
            kind: node.kind(),
            state: self.state_of(node),
            diffs: node.diffs(),
            time: node.time(),
            content: node.content(),
            scan: node.scan(),
            flags: node.flags(),
            left: self.entry_view(node, id.0, 0),
            right: self.entry_view(node, id.0, 1),
        })
    }

    /// Memory reserved by the node, folder and name storage (the `folder_model_1m` bench).
    pub(crate) fn reserved_bytes(&self) -> (usize, usize) {
        let nodes = self.nodes.chunks.len() * CHUNK * std::mem::size_of::<Node>();
        let folders = self.folders.chunks.len() * CHUNK * std::mem::size_of::<FolderData>();
        let names = self.names.iter().map(NameArena::reserved).sum();
        (nodes + folders, names)
    }

    pub(crate) fn dir_info(&self, side: usize, index: u32) -> Option<&DirInfo> {
        self.dirs.get(side)?.get(index as usize)
    }

    /// The representative name of a node (left when present).
    pub(crate) fn representative_name(&self, node: &Node) -> &str {
        if node.sides[0].present() {
            self.name(0, &node.sides[0])
        } else {
            self.name(1, &node.sides[1])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_bounds() {
        assert!(
            std::mem::size_of::<SideRec>() <= 64,
            "{}",
            std::mem::size_of::<SideRec>()
        );
        assert!(
            std::mem::size_of::<Node>() <= 160,
            "{}",
            std::mem::size_of::<Node>()
        );
        assert_eq!(std::mem::size_of::<Aggregates>(), 2 * 18 * 4);
        assert_eq!(std::mem::size_of::<Pair<SideTotals>>(), 48);
    }

    #[test]
    fn chunk_vec_and_names() {
        let mut v = ChunkVec::default();
        for i in 0..(CHUNK * 2 + 3) {
            assert_eq!(v.push(i), i);
        }
        assert_eq!(v.len(), CHUNK * 2 + 3);
        assert_eq!(v.get(CHUNK + 1), Some(&(CHUNK + 1)));
        assert_eq!(v.get(CHUNK * 3), None);
        *v.get_mut(5).unwrap() = 99;
        assert_eq!(v.get(5), Some(&99));
        let mut arena = NameArena::default();
        let a = arena.push("alpha");
        let long = "x".repeat(NAME_CHUNK + 10);
        let b = arena.push(&long);
        let c = arena.push("γ");
        assert_eq!(arena.get(a), "alpha");
        assert_eq!(arena.get(b), long);
        assert_eq!(arena.get(c), "γ");
        assert!(arena.reserved() >= NAME_CHUNK * 2);
    }

    #[test]
    fn codes_round_trip() {
        for state in CompareState::ALL {
            let mut node = Store::new(true, false).nodes.get(0).copied().unwrap();
            node.state = state_code(state);
            assert_eq!(node.stored_state(), state);
        }
        for method in METHODS {
            for kind in ContentKind::ALL {
                let summary = ContentSummary { method, kind };
                let mut node = Store::new(true, false).nodes.get(0).copied().unwrap();
                node.content = content_code(Some(summary));
                assert_eq!(node.content(), Some(summary));
            }
        }
        for t in [
            None,
            Some(TimeRelation::Same),
            Some(TimeRelation::LeftNewer),
            Some(TimeRelation::RightNewer),
        ] {
            let mut node = Store::new(true, false).nodes.get(0).copied().unwrap();
            node.time = time_code(t);
            assert_eq!(node.time(), t);
        }
        for scan in [
            ScanStatus::NotApplicable,
            ScanStatus::NotScanned,
            ScanStatus::Scanned,
            ScanStatus::Failed,
        ] {
            let mut node = Store::new(true, false).nodes.get(0).copied().unwrap();
            node.scan = scan_code(scan);
            assert_eq!(node.scan(), scan);
        }
    }
}
