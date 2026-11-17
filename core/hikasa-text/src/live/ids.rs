//! Interned key ids of a live comparison (§1.10.11 kept state, G8 c): every line's key as a `u32` id, per side in
//! chunked `Arc` blocks, and one table of the ids shared by both sides.
//!
//! ```text
//! IdLines   per side: chunks of ≤ ID_CHUNK ids in Arcs plus prefix sums; a recompare rebuilds only the chunks
//!           of its dirty spans and shares every other one with the last exact result
//! IdTable   per id: the key's 64-bit hash, the occurrences per side (capped at u16::MAX) and whether the key is
//!           trivial; the ids of the full run are numbered in hash order (lookup by binary search), ids made
//!           by edits sit in an overlay map. Equal keys have equal ids on both sides; a new key whose hash meets
//!           a live id is checked against that id's key before it takes it
//! ```
//!
//! Equality of two lines is then one comparison of ids, and a key is counted in a range by a walk of the ids.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

/// Ids a chunk holds at most.
pub(crate) const ID_CHUNK: usize = 1024;

/// A hasher for interned ids (G8 follow-up): one multiplication. The ids are the recompare's own numbers, so no
/// input chooses them, and the maps hashed with it sit on the walks of the certificate (a lookup per line walked).
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct IdHasher(u64);

impl std::hash::Hasher for IdHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0.rotate_left(8) ^ u64::from(b)).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        }
    }

    #[inline]
    fn write_u32(&mut self, n: u32) {
        self.0 = (self.0 ^ u64::from(n)).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }

    #[inline]
    fn write_usize(&mut self, n: usize) {
        self.0 = (self.0 ^ n as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    }
}

/// A map keyed by interned ids.
pub(crate) type IdMap<V> = HashMap<u32, V, std::hash::BuildHasherDefault<IdHasher>>;

/// The ids of one side's lines (see the module documentation).
#[derive(Debug, Clone, Default)]
pub(crate) struct IdLines {
    chunks: Vec<Arc<[u32]>>,
    /// Prefix sums of the chunks' lengths (`chunks + 1` entries).
    first: Vec<u32>,
}

impl IdLines {
    /// The ids `ids`, in full chunks.
    pub(crate) fn from_vec(ids: &[u32]) -> IdLines {
        let mut out = IdLines {
            chunks: Vec::with_capacity(ids.len().div_ceil(ID_CHUNK)),
            first: vec![0],
        };
        for part in ids.chunks(ID_CHUNK) {
            out.chunks.push(Arc::from(part));
            out.first
                .push(out.first.last().copied().unwrap_or(0) + part.len() as u32);
        }
        out
    }

    /// Lines.
    pub(crate) fn len(&self) -> u32 {
        self.first.last().copied().unwrap_or(0)
    }

    /// The chunk holding line `line`.
    #[inline]
    fn chunk_of(&self, line: u32) -> usize {
        self.first.partition_point(|&f| f <= line) - 1
    }

    /// The id of line `line`.
    #[inline]
    pub(crate) fn get(&self, line: u32) -> u32 {
        let c = self.chunk_of(line);
        self.chunks[c][(line - self.first[c]) as usize]
    }

    /// Calls `f(line, id)` for every line of `lines`, in order; stops when `f` returns false.
    pub(crate) fn walk(&self, lines: Range<u32>, mut f: impl FnMut(u32, u32) -> bool) {
        if lines.is_empty() {
            return;
        }
        let mut c = self.chunk_of(lines.start);
        let mut line = lines.start;
        while line < lines.end && c < self.chunks.len() {
            let base = self.first[c];
            let chunk = &self.chunks[c];
            let from = (line - base) as usize;
            let to = ((lines.end - base) as usize).min(chunk.len());
            for (i, &id) in chunk[from..to].iter().enumerate() {
                if !f(base + (from + i) as u32, id) {
                    return;
                }
            }
            line = base + to as u32;
            c += 1;
        }
    }

    /// Calls `f(first line, ids)` for the ids of `lines` slice by slice (chunk parts), in order; stops when `f`
    /// returns false.
    pub(crate) fn walk_slices(&self, lines: Range<u32>, mut f: impl FnMut(u32, &[u32]) -> bool) {
        if lines.is_empty() {
            return;
        }
        let mut c = self.chunk_of(lines.start);
        let mut line = lines.start;
        while line < lines.end && c < self.chunks.len() {
            let base = self.first[c];
            let chunk = &self.chunks[c];
            let from = (line - base) as usize;
            let to = ((lines.end - base) as usize).min(chunk.len());
            if !f(line, &chunk[from..to]) {
                return;
            }
            line = base + to as u32;
            c += 1;
        }
    }

    /// The ids after replacing old lines `old` by `new` ids for each span (sorted, disjoint, old numbering):
    /// the chunks the spans touch are rebuilt (with their neighbours' lines when they would be small), every other
    /// chunk is shared.
    pub(crate) fn spliced(&self, spans: &[(Range<u32>, &[u32])]) -> IdLines {
        let mut out = IdLines {
            chunks: Vec::with_capacity(self.chunks.len() + spans.len()),
            first: vec![0],
        };
        let mut pending: Vec<u32> = Vec::new();
        let mut line = 0u32;
        for (r, ids) in spans {
            self.copy_old(line..r.start, &mut out, &mut pending);
            pending.extend_from_slice(ids);
            line = r.end;
        }
        self.copy_old(line..self.len(), &mut out, &mut pending);
        out.flush(&mut pending);
        out
    }

    /// Appends old lines `lines`: whole chunks are shared once nothing is pending (a small pending piece takes
    /// the next chunk's ids first, so no tiny chunk is left), partial chunks are copied.
    fn copy_old(&self, lines: Range<u32>, out: &mut IdLines, pending: &mut Vec<u32>) {
        if lines.is_empty() {
            return;
        }
        let mut c = self.chunk_of(lines.start);
        let mut line = lines.start;
        while line < lines.end {
            let (lo, hi) = (self.first[c], self.first[c + 1]);
            if line == lo
                && hi <= lines.end
                && (pending.is_empty() || pending.len() >= ID_CHUNK / 4)
            {
                out.flush(pending);
                out.push(self.chunks[c].clone());
            } else {
                let to = hi.min(lines.end);
                pending
                    .extend_from_slice(&self.chunks[c][(line - lo) as usize..(to - lo) as usize]);
            }
            line = hi.min(lines.end);
            c += 1;
        }
    }

    /// Appends `part` as one chunk.
    fn push(&mut self, part: Arc<[u32]>) {
        if part.is_empty() {
            return;
        }
        let at = self.first.last().copied().unwrap_or(0);
        self.first.push(at + part.len() as u32);
        self.chunks.push(part);
    }

    /// Appends the pending ids as chunks of at most `ID_CHUNK`.
    fn flush(&mut self, pending: &mut Vec<u32>) {
        for part in pending.chunks(ID_CHUNK) {
            self.push(Arc::from(part));
        }
        pending.clear();
    }

    /// Heap bytes (memory accounting).
    pub(crate) fn heap_bytes(&self) -> usize {
        4 * self.len() as usize + 16 * self.chunks.len() + 4 * self.first.capacity()
    }

    /// Every id, in order (tests).
    #[cfg(test)]
    pub(crate) fn to_vec(&self) -> Vec<u32> {
        let mut v = Vec::new();
        self.walk(0..self.len(), |_, id| {
            v.push(id);
            true
        });
        v
    }
}

/// The ids' table (see the module documentation).
#[derive(Debug, Clone, Default)]
pub(crate) struct IdTable {
    /// The key's hash, per id; ids below `sorted` are in hash order.
    hash: Vec<u64>,
    sorted: u32,
    /// Occurrences per side, capped at `u16::MAX` (a capped count is only a lower bound).
    counts: Vec<[u16; 2]>,
    /// Trivial keys, one bit per id.
    trivial: Vec<u64>,
    /// Ids made by edits, by hash.
    overlay: HashMap<u64, Vec<u32>>,
}

/// What a recompare changed in the table: new ids (hash, trivial) and count changes; applied at `install`.
#[derive(Debug, Clone, Default)]
pub(crate) struct TableDelta {
    pub(crate) new_ids: Vec<(u64, bool)>,
    pub(crate) counts: IdMap<[i64; 2]>,
}

impl IdTable {
    /// The table of a full run and its lines renumbered: `left`/`right` are dense ids (`0..distinct`), `hash` the
    /// hash of each dense id's key, `trivial` whether it is trivial. Returns the table and both sides' ids.
    pub(crate) fn build(
        left: &[u32],
        right: &[u32],
        hash: Vec<u64>,
        trivial: &[bool],
    ) -> (IdTable, [Vec<u32>; 2]) {
        let distinct = hash.len();
        let mut order: Vec<u32> = (0..distinct as u32).collect();
        order.sort_unstable_by_key(|&i| (hash[i as usize], i));
        let mut rank = vec![0u32; distinct];
        for (r, &i) in order.iter().enumerate() {
            rank[i as usize] = r as u32;
        }
        let mut table = IdTable {
            hash: order.iter().map(|&i| hash[i as usize]).collect(),
            sorted: distinct as u32,
            counts: vec![[0, 0]; distinct],
            trivial: vec![0; distinct.div_ceil(64)],
            overlay: HashMap::new(),
        };
        for (r, &i) in order.iter().enumerate() {
            if trivial.get(i as usize).copied().unwrap_or(false) {
                table.trivial[r / 64] |= 1 << (r % 64);
            }
        }
        let mut sides = [
            Vec::with_capacity(left.len()),
            Vec::with_capacity(right.len()),
        ];
        for (k, ids) in [left, right].into_iter().enumerate() {
            for &id in ids {
                let r = rank[id as usize];
                sides[k].push(r);
                let c = &mut table.counts[r as usize][k];
                *c = c.saturating_add(1);
            }
        }
        (table, sides)
    }

    /// Ids in the table.
    pub(crate) fn len(&self) -> u32 {
        self.hash.len() as u32
    }

    /// True when `id`'s key is trivial.
    #[inline]
    pub(crate) fn trivial(&self, id: u32) -> bool {
        self.trivial
            .get(id as usize / 64)
            .is_some_and(|w| w & (1 << (id % 64)) != 0)
    }

    /// Occurrences of `id` per side (capped).
    pub(crate) fn counts(&self, id: u32) -> [u16; 2] {
        self.counts.get(id as usize).copied().unwrap_or([0, 0])
    }

    /// The ids whose key has hash `h`.
    pub(crate) fn lookup(&self, h: u64) -> Vec<u32> {
        let sorted = &self.hash[..self.sorted as usize];
        let from = sorted.partition_point(|&x| x < h);
        let mut out: Vec<u32> = sorted[from..]
            .iter()
            .take_while(|&&x| x == h)
            .enumerate()
            .map(|(i, _)| (from + i) as u32)
            .collect();
        if let Some(more) = self.overlay.get(&h) {
            out.extend_from_slice(more);
        }
        out
    }

    /// Applies a recompare's changes (in place).
    pub(crate) fn apply(&mut self, delta: &TableDelta) {
        for &(h, trivial) in &delta.new_ids {
            let id = self.hash.len() as u32;
            self.hash.push(h);
            self.counts.push([0, 0]);
            if self.trivial.len() * 64 <= id as usize {
                self.trivial.push(0);
            }
            if trivial {
                self.trivial[id as usize / 64] |= 1 << (id % 64);
            }
            self.overlay.entry(h).or_default().push(id);
        }
        for (&id, d) in &delta.counts {
            if let Some(c) = self.counts.get_mut(id as usize) {
                for k in 0..2 {
                    // A capped count stays capped: it is only a lower bound.
                    if c[k] != u16::MAX {
                        c[k] = (i64::from(c[k]) + d[k]).clamp(0, i64::from(u16::MAX)) as u16;
                    }
                }
            }
        }
    }

    /// Heap bytes (memory accounting).
    pub(crate) fn heap_bytes(&self) -> usize {
        8 * self.hash.capacity()
            + 4 * self.counts.capacity()
            + 8 * self.trivial.capacity()
            + self
                .overlay
                .values()
                .map(|v| 40 + 4 * v.capacity())
                .sum::<usize>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splices_rebuild_touched_chunks_only() {
        let ids: Vec<u32> = (0..5000).collect();
        let lines = IdLines::from_vec(&ids);
        assert_eq!(lines.len(), 5000);
        assert_eq!(lines.get(4321), 4321);
        let new = [7u32, 8, 9];
        let spliced = lines.spliced(&[
            (10..12, &new[..]),
            (3000..3000, &[1u32][..]),
            (5000..5000, &[2u32][..]),
        ]);
        let mut want: Vec<u32> = ids.clone();
        want.insert(5000, 2);
        want.insert(3000, 1);
        want.splice(10..12, new);
        assert_eq!(spliced.to_vec(), want);
        // Chunks without a span are shared.
        let shared = spliced
            .chunks
            .iter()
            .filter(|c| lines.chunks.iter().any(|o| Arc::ptr_eq(o, c)))
            .count();
        assert!(shared >= 1, "{shared}");
        for (i, &id) in want.iter().enumerate() {
            assert_eq!(spliced.get(i as u32), id);
        }
        // Spans covering whole chunks and the start.
        let big: Vec<u32> = vec![1; 3];
        let spliced = lines.spliced(&[(0..2500, &big[..])]);
        let mut want: Vec<u32> = ids.clone();
        want.splice(0..2500, big.clone());
        assert_eq!(spliced.to_vec(), want);
    }

    #[test]
    fn the_table_numbers_by_hash() {
        let (t, sides) =
            IdTable::build(&[0, 1, 0], &[2, 1], vec![30, 10, 20], &[false, true, false]);
        assert_eq!(sides, [vec![2, 0, 2], vec![1, 0]]);
        assert_eq!(t.lookup(10), [0]);
        assert_eq!(t.lookup(30), [2]);
        assert!(t.lookup(15).is_empty());
        assert!(t.trivial(0) && !t.trivial(1));
        assert_eq!(t.counts(2), [2, 0]);
        let mut t = t;
        let mut delta = TableDelta::default();
        delta.new_ids.push((10, false));
        delta.counts.insert(2, [-1, 1]);
        delta.counts.insert(3, [1, 0]);
        t.apply(&delta);
        assert_eq!(t.lookup(10), [0, 3]);
        assert_eq!(t.counts(2), [1, 1]);
        assert_eq!(t.counts(3), [1, 0]);
    }
}

/// The table as a recompare sees it: the last exact result's ids and the ids this recompare made.
#[derive(Debug)]
pub(crate) struct IdView<'a> {
    pub(crate) table: &'a IdTable,
    /// Trivial flags of the new ids (`table.len()..`).
    pub(crate) new_trivial: Vec<bool>,
    /// Count changes of this recompare, per id.
    pub(crate) delta: IdMap<[i64; 2]>,
    /// The ids of `delta`, one bit each (most lookups need no hash).
    pub(crate) delta_bits: Vec<u64>,
}

impl IdView<'_> {
    /// Marks the ids of `delta` (once the interning has filled it).
    pub(crate) fn mark_delta(&mut self) {
        let mut bits: Vec<u64> = Vec::new();
        for &id in self.delta.keys() {
            let w = id as usize / 64;
            if bits.len() <= w {
                bits.resize(w + 1, 0);
            }
            bits[w] |= 1 << (id % 64);
        }
        self.delta_bits = bits;
    }

    /// True when `id`'s key is trivial.
    pub(crate) fn trivial(&self, id: u32) -> bool {
        match id.checked_sub(self.table.len()) {
            Some(k) => self.new_trivial.get(k as usize).copied().unwrap_or(false),
            None => self.table.trivial(id),
        }
    }

    /// The occurrences of `id` per side after this recompare's changes, `None` on a side whose count was capped
    /// (unknown).
    pub(crate) fn counts(&self, id: u32) -> [Option<u64>; 2] {
        let base = if id < self.table.len() {
            self.table.counts(id)
        } else {
            [0, 0]
        };
        let marked = self
            .delta_bits
            .get(id as usize / 64)
            .is_some_and(|w| w & (1 << (id % 64)) != 0);
        let d = if marked {
            self.delta.get(&id).copied().unwrap_or([0, 0])
        } else {
            [0, 0]
        };
        [0, 1].map(|k| (base[k] != u16::MAX).then(|| (i64::from(base[k]) + d[k]).max(0) as u64))
    }
}

/// One node of a live comparison's derivation (G8 d): the recorded node, shared by `Arc` with every later
/// derivation that keeps it unchanged, its coordinates moved lazily by `(dx, dy)`, and its children.
#[derive(Debug, Clone)]
pub(crate) struct LiveNode {
    /// The node as recorded (its own `children` are not read: `children` below are).
    pub(crate) node: Arc<hikasa_diff::TraceNode>,
    pub(crate) dx: i64,
    pub(crate) dy: i64,
    pub(crate) children: Vec<hikasa_diff::NodeId>,
}

impl LiveNode {
    /// A node in place (no move).
    pub(crate) fn new(node: hikasa_diff::TraceNode) -> LiveNode {
        let children = node.children.clone();
        LiveNode {
            node: Arc::new(node),
            dx: 0,
            dy: 0,
            children,
        }
    }

    /// The node with its coordinates moved: borrowed when nothing moves (its `children` are then the recorded
    /// ones; read `LiveNode::children`).
    pub(crate) fn absolute(&self) -> std::borrow::Cow<'_, hikasa_diff::TraceNode> {
        let n = &*self.node;
        if self.dx == 0 && self.dy == 0 {
            return std::borrow::Cow::Borrowed(n);
        }
        let mx = |x: u32| (i64::from(x) + self.dx) as u32;
        let my = |y: u32| (i64::from(y) + self.dy) as u32;
        std::borrow::Cow::Owned(hikasa_diff::TraceNode {
            left: mx(n.left.start)..mx(n.left.end),
            right: my(n.right.start)..my(n.right.end),
            chain: n.chain.iter().map(|&(x, y)| (mx(x), my(y))).collect(),
            blocks: n
                .blocks
                .iter()
                .map(|&(x, y, l)| (mx(x), my(y), l))
                .collect(),
            diag: n.diag.iter().map(|&(x, y, l)| (mx(x), my(y), l)).collect(),
            ..n.clone()
        })
    }

    /// The node's moved ranges.
    pub(crate) fn ranges(&self) -> (Range<u32>, Range<u32>) {
        let n = &self.node;
        let mx = |x: u32| (i64::from(x) + self.dx) as u32;
        let my = |y: u32| (i64::from(y) + self.dy) as u32;
        (
            mx(n.left.start)..mx(n.left.end),
            my(n.right.start)..my(n.right.end),
        )
    }
}

/// One anchor segment of a live derivation: the segment's record (its `nodes` empty) and its live nodes.
#[derive(Debug, Clone)]
pub(crate) struct LiveSegment {
    pub(crate) seg: hikasa_diff::SegmentTrace,
    pub(crate) nodes: Vec<LiveNode>,
}

/// A live comparison's derivation: nodes shared with the earlier ones (G8 d).
#[derive(Debug, Clone, Default)]
pub(crate) struct LiveTrace {
    pub(crate) segments: Vec<LiveSegment>,
    pub(crate) work: hikasa_diff::WorkTotals,
}

impl LiveTrace {
    /// The live form of a full run's derivation.
    pub(crate) fn from_derivation(d: hikasa_diff::Derivation) -> LiveTrace {
        LiveTrace {
            work: d.work,
            segments: d
                .segments
                .into_iter()
                .map(|mut seg| {
                    let nodes = std::mem::take(&mut seg.nodes)
                        .into_iter()
                        .map(LiveNode::new)
                        .collect();
                    LiveSegment { seg, nodes }
                })
                .collect(),
        }
    }

    /// The derivation with every node's coordinates in place (tests and measurements).
    pub(crate) fn to_derivation(&self) -> hikasa_diff::Derivation {
        hikasa_diff::Derivation {
            work: self.work,
            segments: self
                .segments
                .iter()
                .map(|s| hikasa_diff::SegmentTrace {
                    nodes: s
                        .nodes
                        .iter()
                        .map(|n| {
                            let mut t = n.absolute().into_owned();
                            t.children = n.children.clone();
                            t
                        })
                        .collect(),
                    ..s.seg.clone()
                })
                .collect(),
        }
    }

    /// Bytes held (each shared node counted once per derivation).
    pub(crate) fn heap_bytes(&self) -> usize {
        self.segments
            .iter()
            .flat_map(|s| s.nodes.iter())
            .map(|n| {
                let t = &n.node;
                136 + 8 * t.chain.capacity()
                    + 12 * t.blocks.capacity()
                    + 12 * t.diag.capacity()
                    + 4 * (t.children.capacity() + n.children.capacity())
            })
            .sum()
    }
}
