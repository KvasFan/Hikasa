//! The block line table (D-260, contract §1.10.2): every document's line table is a list of copy-on-write blocks
//! of lines, so that an edit costs O(edit + blocks) instead of O(lines), and a snapshot of an edited document
//! shares every block the edit did not touch.
//!
//! ```text
//! LineTable
//!   blocks      Vec<LineBlock>        each one Arc<[u8]>: 4-byte block-relative starts + 1-byte terminators
//!                                     (+ 4-byte generations and file lines in a block an edit touched)
//!   first_line  prefix sums of lines  (empty while `uniform`: line → block is a division by BLOCK_TARGET)
//!   first_byte  prefix sums of bytes  (u32: texts are below 4 GiB, D-43)
//!   orig_first  empty until the first edit; then the file line of each unmarked block's first line
//!   artificial  sorted indices of `Artificial` lines (as M1)
//! ```
//!
//! A block is built with [`BLOCK_TARGET`] lines; an edit rebuilds the blocks it touches, splits a result above
//! [`BLOCK_MAX`] lines and merges one below [`BLOCK_MIN`] with a neighbour (unless it is the only block). Edit
//! marks (§1.10.5: the generation of the edit that last changed a line, and its index in the file as last saved
//! or loaded) live only in blocks an edit touched; an unmarked block's lines have generation 0 and consecutive
//! file lines.

use std::sync::Arc;

use crate::document::LineEnding;

/// Lines a block is built with.
pub(crate) const BLOCK_TARGET: u32 = 512;
/// An edit that grows a block past this splits it.
pub(crate) const BLOCK_MAX: u32 = 1024;
/// A block below this merges with a neighbour (unless it is the only one).
pub(crate) const BLOCK_MIN: u32 = 128;
/// `orig` of a line that has no line in the file (inserted or changed since the last save).
pub(crate) const NO_ORIG: u32 = u32::MAX;

/// Code of a terminator in a block.
#[inline]
const fn eol_code(eol: LineEnding) -> u8 {
    match eol {
        LineEnding::Lf => 0,
        LineEnding::CrLf => 1,
        LineEnding::Cr => 2,
        LineEnding::None => 3,
        LineEnding::Artificial => 4,
    }
}

/// Terminator of a code.
#[inline]
const fn eol_of(code: u8) -> LineEnding {
    match code {
        0 => LineEnding::Lf,
        1 => LineEnding::CrLf,
        2 => LineEnding::Cr,
        4 => LineEnding::Artificial,
        _ => LineEnding::None,
    }
}

/// Byte length of a terminator in the text.
#[inline]
pub(crate) const fn eol_len(eol: LineEnding) -> u32 {
    match eol {
        LineEnding::Lf | LineEnding::Cr => 1,
        LineEnding::CrLf => 2,
        LineEnding::None | LineEnding::Artificial => 0,
    }
}

/// One block: an `Arc<[u8]>` of `n` block-relative start offsets (little-endian `u32`; the first slot, whose
/// start is always 0, holds the *marked* flag instead) followed by `n` terminator codes, and, in a block an edit
/// touched (flag 1), `n` generations and `n` file lines (`u32` each): 5 bytes a line, 13 when marked, and one
/// 16-byte allocation header a block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LineBlock(Arc<[u8]>);

impl LineBlock {
    /// True when the block carries marks.
    #[inline]
    fn marked(&self) -> bool {
        self.0[0] == 1
    }

    /// Lines of the block.
    #[inline]
    fn lines(&self) -> usize {
        self.0.len() / if self.marked() { 13 } else { 5 }
    }

    #[inline]
    fn word(&self, at: usize) -> u32 {
        u32::from_le_bytes([self.0[at], self.0[at + 1], self.0[at + 2], self.0[at + 3]])
    }

    /// Block-relative start of line `i`.
    #[inline]
    fn start(&self, i: usize) -> u32 {
        if i == 0 { 0 } else { self.word(4 * i) }
    }

    /// Terminator of line `i`.
    #[inline]
    fn eol(&self, i: usize) -> LineEnding {
        eol_of(self.0[4 * self.lines() + i])
    }

    /// Generation and file line of line `i` of a marked block.
    #[inline]
    fn mark(&self, i: usize) -> (u32, u32) {
        let n = self.lines();
        (self.word(5 * n + 4 * i), self.word(9 * n + 4 * i))
    }
}

/// One line as an edit or a rebuild sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LineRec {
    /// Bytes of the content (terminator excluded).
    pub(crate) content: u32,
    /// Terminator.
    pub(crate) eol: LineEnding,
    /// Generation (0 = as loaded).
    pub(crate) generation: u32,
    /// File line, or [`NO_ORIG`].
    pub(crate) orig: u32,
}

/// Builds the blocks of a fresh table, [`BLOCK_TARGET`] lines each.
#[derive(Debug, Default)]
pub(crate) struct TableBuilder {
    table: LineTable,
    /// Absolute starts and terminators of the block being filled.
    starts: Vec<u32>,
    eols: Vec<u8>,
    block_start: u32,
}

impl TableBuilder {
    /// A builder for a text of `text_len` bytes.
    pub(crate) fn new(text_len: u32) -> TableBuilder {
        TableBuilder {
            table: LineTable {
                text_len,
                ..LineTable::default()
            },
            starts: Vec::with_capacity(BLOCK_TARGET as usize),
            eols: Vec::with_capacity(BLOCK_TARGET as usize),
            block_start: 0,
        }
    }

    /// Reserves room for about `lines` lines (the block list and its prefix sums).
    pub(crate) fn reserve(
        &mut self,
        lines: usize,
    ) -> Result<(), std::collections::TryReserveError> {
        let blocks = lines.div_ceil(BLOCK_TARGET as usize);
        self.table.blocks.try_reserve_exact(blocks)?;
        self.table.first_byte.try_reserve_exact(blocks + 1)?;
        Ok(())
    }

    /// Appends a line starting at absolute byte `start`.
    #[inline]
    pub(crate) fn push(&mut self, start: u32, eol: LineEnding) {
        if self.starts.is_empty() {
            self.block_start = start;
        }
        if eol == LineEnding::Artificial {
            self.table.artificial.push(self.table.len);
        }
        self.starts.push(start);
        self.eols.push(eol_code(eol));
        self.table.len += 1;
        if self.starts.len() == BLOCK_TARGET as usize {
            self.flush();
        }
    }

    /// Lines pushed so far.
    pub(crate) fn len(&self) -> u32 {
        self.table.len
    }

    fn flush(&mut self) {
        if self.starts.is_empty() {
            return;
        }
        let base = self.block_start;
        let mut data = Vec::with_capacity(5 * self.starts.len());
        for &s in &self.starts {
            data.extend_from_slice(&(s - base).to_le_bytes());
        }
        data.extend_from_slice(&self.eols);
        self.table.first_byte.push(base);
        data[..4].copy_from_slice(&[0; 4]);
        self.table.blocks.push(LineBlock(Arc::from(data)));
        self.starts.clear();
        self.eols.clear();
    }

    /// The table.
    pub(crate) fn finish(mut self) -> LineTable {
        self.flush();
        if !self.table.blocks.is_empty() {
            self.table.first_byte.push(self.table.text_len);
        }
        self.table.artificial.shrink_to_fit();
        self.table
    }
}

/// The block line table of one document (see the module documentation).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct LineTable {
    blocks: Vec<LineBlock>,
    /// Prefix sums of the blocks' line counts (`blocks.len() + 1` entries); empty while every block but the last
    /// has [`BLOCK_TARGET`] lines.
    first_line: Vec<u32>,
    /// Prefix sums of the blocks' byte lengths (`blocks.len() + 1` entries; empty without blocks).
    first_byte: Vec<u32>,
    /// Empty until the first edit; then, per block, the file line of its first line when it has no marks.
    orig_first: Vec<u32>,
    /// Sorted indices of `Artificial` lines.
    artificial: Vec<u32>,
    len: u32,
    text_len: u32,
    /// True once an edit changed the table.
    edited: bool,
}

impl LineTable {
    /// Number of lines.
    #[inline]
    pub(crate) fn len(&self) -> u32 {
        self.len
    }

    /// Number of `Artificial` lines.
    #[inline]
    pub(crate) fn artificial_count(&self) -> usize {
        self.artificial.len()
    }

    /// Bytes of the text.
    #[inline]
    pub(crate) fn text_len(&self) -> u32 {
        self.text_len
    }

    /// True while line → block is a division.
    #[inline]
    pub(crate) fn uniform(&self) -> bool {
        self.first_line.is_empty()
    }

    /// True when every block but the last has [`BLOCK_TARGET`] lines and the last at most that many.
    fn is_uniform(&self) -> bool {
        let n = self.blocks.len();
        n == 0
            || ((0..n - 1).all(|b| self.block_lines(b) == BLOCK_TARGET)
                && self.block_lines(n - 1) <= BLOCK_TARGET)
    }

    /// Number of blocks.
    pub(crate) fn block_count(&self) -> usize {
        self.blocks.len()
    }

    /// Lines of block `b`.
    pub(crate) fn block_lines(&self, b: usize) -> u32 {
        self.blocks[b].lines() as u32
    }

    /// The `Arc` of block `b` (sharing tests).
    pub(crate) fn block_ptr(&self, b: usize) -> *const () {
        Arc::as_ptr(&self.blocks[b].0).cast()
    }

    /// First line of block `b`.
    #[inline]
    fn block_first_line(&self, b: usize) -> u32 {
        if self.first_line.is_empty() {
            b as u32 * BLOCK_TARGET
        } else {
            self.first_line[b]
        }
    }

    /// Block and index in it of line `idx`.
    #[inline]
    pub(crate) fn locate(&self, idx: u32) -> Option<(usize, usize)> {
        if idx >= self.len {
            return None;
        }
        if self.first_line.is_empty() {
            let b = (idx / BLOCK_TARGET) as usize;
            return Some((b, (idx % BLOCK_TARGET) as usize));
        }
        let b = self.first_line.partition_point(|&f| f <= idx) - 1;
        Some((b, (idx - self.first_line[b]) as usize))
    }

    /// Byte range of the content of line `idx` (without its terminator); `None` out of range.
    #[inline]
    pub(crate) fn content(&self, idx: u32) -> Option<(usize, usize)> {
        let (b, i) = self.locate(idx)?;
        let block = &self.blocks[b];
        let base = self.first_byte[b];
        let start = base + block.start(i);
        let next = if i + 1 < block.lines() {
            base + block.start(i + 1)
        } else {
            self.first_byte[b + 1]
        };
        Some((start as usize, (next - eol_len(block.eol(i))) as usize))
    }

    /// Terminator of line `idx` (`None` out of range).
    #[inline]
    pub(crate) fn eol(&self, idx: u32) -> LineEnding {
        self.locate(idx)
            .map_or(LineEnding::None, |(b, i)| self.blocks[b].eol(i))
    }

    /// 1-based physical line number of `idx` (0 out of range).
    pub(crate) fn physical(&self, idx: u32) -> u32 {
        if idx >= self.len {
            return 0;
        }
        let artificial_before = self.artificial.partition_point(|&a| a < idx) as u32;
        idx + 1 - artificial_before
    }

    /// The marks of line `idx`: its generation (0 = as loaded) and its file line ([`NO_ORIG`] = none).
    pub(crate) fn marks(&self, idx: u32) -> (u32, u32) {
        let Some((b, i)) = self.locate(idx) else {
            return (0, NO_ORIG);
        };
        let block = &self.blocks[b];
        match block.marked() {
            true => block.mark(i),
            false => {
                let first = self
                    .orig_first
                    .get(b)
                    .copied()
                    .unwrap_or(self.block_first_line(b));
                (0, first + i as u32)
            }
        }
    }

    /// True once an edit changed the table.
    pub(crate) fn edited(&self) -> bool {
        self.edited
    }

    /// The record of line `idx` (content length, terminator, marks).
    pub(crate) fn rec(&self, idx: u32) -> LineRec {
        let (s, e) = self.content(idx).unwrap_or((0, 0));
        let (generation, orig) = self.marks(idx);
        LineRec {
            content: (e - s) as u32,
            eol: self.eol(idx),
            generation,
            orig,
        }
    }

    /// First absolute byte of line `idx` (`text_len` for `idx == len`).
    pub(crate) fn start_of(&self, idx: u32) -> u32 {
        match self.locate(idx) {
            Some((b, i)) => self.first_byte[b] + self.blocks[b].start(i),
            None => self.text_len,
        }
    }

    /// The first line in `[from, end)` with a non-zero generation (unmarked blocks are skipped whole).
    pub(crate) fn next_marked(&self, from: u32, end: u32) -> Option<u32> {
        let mut line = from;
        while line < end.min(self.len) {
            let (b, i) = self.locate(line)?;
            let block = &self.blocks[b];
            if !block.marked() {
                line += (block.lines() - i) as u32;
                continue;
            }
            for k in i..block.lines() {
                if line >= end {
                    return None;
                }
                if block.mark(k).0 != 0 {
                    return Some(line);
                }
                line += 1;
            }
        }
        None
    }

    /// The last line before `before` with a non-zero generation (unmarked blocks are skipped whole).
    pub(crate) fn prev_marked(&self, before: u32) -> Option<u32> {
        let mut line = before.min(self.len);
        while line > 0 {
            let (b, i) = self.locate(line - 1)?;
            let block = &self.blocks[b];
            if !block.marked() {
                line -= i as u32 + 1;
                continue;
            }
            for k in (0..=i).rev() {
                if block.mark(k).0 != 0 {
                    return Some(line - 1 - (i - k) as u32);
                }
            }
            line -= i as u32 + 1;
        }
        None
    }

    /// The line holding absolute byte `off` (its content or terminator); `len()` at or past the end.
    pub(crate) fn line_at_byte(&self, off: u32) -> u32 {
        if off >= self.text_len || self.blocks.is_empty() {
            return self.len;
        }
        let b = self.first_byte.partition_point(|&f| f <= off) - 1;
        let block = &self.blocks[b];
        let rel = off - self.first_byte[b];
        let (mut lo, mut hi) = (0usize, block.lines());
        while hi - lo > 1 {
            let mid = (lo + hi) / 2;
            if block.start(mid) <= rel {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        self.block_first_line(b) + lo as u32
    }

    /// Replaces lines `[at, at + removed)` with `new` lines (in order) whose text replaced the old lines' text
    /// in the rope. Only the blocks holding the replaced lines (and a neighbour when the result is too small)
    /// are rebuilt; the others are shared with every earlier clone. O(new + touched blocks + blocks).
    pub(crate) fn splice(&mut self, at: u32, removed: u32, new: &[LineRec]) {
        debug_assert!(at + removed <= self.len);
        if !self.edited {
            self.orig_first = (0..self.blocks.len())
                .map(|b| self.block_first_line(b))
                .collect();
            self.edited = true;
        }
        // The blocks holding [at, at + removed); an insertion at the end goes into the last block.
        let (mut lo, mut hi) = if self.blocks.is_empty() {
            (0, 0)
        } else {
            let first = self.locate(at).map_or(self.blocks.len() - 1, |(b, _)| b);
            let last = if removed == 0 {
                first
            } else {
                self.locate(at + removed - 1).map_or(first, |(b, _)| b)
            };
            (first, last + 1)
        };
        let mut total: u32 =
            (lo..hi).map(|b| self.block_lines(b)).sum::<u32>() + new.len() as u32 - removed;
        // A result below BLOCK_MIN takes a neighbour (unless it is all there is).
        while total < BLOCK_MIN && (lo > 0 || hi < self.blocks.len()) {
            if hi < self.blocks.len() {
                total += self.block_lines(hi);
                hi += 1;
            } else {
                lo -= 1;
                total += self.block_lines(lo);
            }
        }
        let range_start = if lo < self.blocks.len() {
            self.block_first_line(lo)
        } else {
            self.len
        };
        let range_end = if hi < self.blocks.len() {
            self.block_first_line(hi)
        } else {
            self.len
        };
        let mut recs = Vec::with_capacity(total as usize);
        recs.extend((range_start..at).map(|line| self.rec(line)));
        recs.extend_from_slice(new);
        recs.extend((at + removed..range_end).map(|line| self.rec(line)));
        debug_assert_eq!(recs.len(), total as usize);
        // The new blocks of the range.
        let pieces = if recs.is_empty() {
            0
        } else if recs.len() <= BLOCK_MAX as usize {
            1
        } else {
            recs.len().div_ceil(BLOCK_TARGET as usize)
        };
        let mut blocks = Vec::with_capacity(pieces);
        let mut origs = Vec::with_capacity(pieces);
        let mut bytes = Vec::with_capacity(pieces);
        let mut from = 0usize;
        for k in 0..pieces {
            let to = recs.len() * (k + 1) / pieces;
            let (block, orig, len) = build_block(&recs[from..to]);
            blocks.push(block);
            origs.push(orig);
            bytes.push(len);
            from = to;
        }
        // Byte lengths of every block, then the prefix sums.
        let old_bytes = |t: &LineTable, b: usize| -> u32 { t.first_byte[b + 1] - t.first_byte[b] };
        let mut lengths: Vec<u32> = Vec::with_capacity(self.blocks.len() - (hi - lo) + pieces);
        lengths.extend((0..lo).map(|b| old_bytes(self, b)));
        lengths.extend_from_slice(&bytes);
        lengths.extend((hi..self.blocks.len()).map(|b| old_bytes(self, b)));
        self.blocks.splice(lo..hi, blocks);
        self.orig_first.splice(lo..hi, origs);
        self.first_byte.clear();
        if !self.blocks.is_empty() {
            self.first_byte.reserve_exact(self.blocks.len() + 1);
            let mut at_byte = 0u32;
            self.first_byte.push(0);
            for len in lengths {
                at_byte += len;
                self.first_byte.push(at_byte);
            }
        }
        self.text_len = self.first_byte.last().copied().unwrap_or(0);
        self.len = self.len + new.len() as u32 - removed;
        // Line → block.
        let uniform = self.is_uniform();
        self.first_line.clear();
        if !uniform {
            self.first_line.reserve_exact(self.blocks.len() + 1);
            let mut line = 0u32;
            self.first_line.push(0);
            for b in 0..self.blocks.len() {
                line += self.block_lines(b);
                self.first_line.push(line);
            }
        }
        // Artificial breaks: drop the replaced ones, shift the later ones, add the new ones.
        let lo_a = self.artificial.partition_point(|&a| a < at);
        let hi_a = self.artificial.partition_point(|&a| a < at + removed);
        let shift = new.len() as i64 - i64::from(removed);
        let added: Vec<u32> = new
            .iter()
            .enumerate()
            .filter(|(_, r)| r.eol == LineEnding::Artificial)
            .map(|(k, _)| at + k as u32)
            .collect();
        for a in &mut self.artificial[hi_a..] {
            *a = (i64::from(*a) + shift) as u32;
        }
        self.artificial.splice(lo_a..hi_a, added);
    }

    /// Sets every line's file line to its own index (a save, §1.10.5: the saved file is the new splice source;
    /// generations stay, so `Edited` lines become `SavedEdit`). Blocks without a non-zero generation lose their
    /// marks.
    pub(crate) fn rebase_orig(&mut self) {
        if !self.edited {
            return;
        }
        for b in 0..self.blocks.len() {
            let first = self.block_first_line(b);
            self.orig_first[b] = first;
            if self.blocks[b].marked() {
                let n = self.block_lines(b);
                let recs: Vec<LineRec> = (first..first + n)
                    .map(|line| LineRec {
                        orig: line,
                        ..self.rec(line)
                    })
                    .collect();
                let (block, orig, _) = build_block(&recs);
                self.blocks[b] = block;
                self.orig_first[b] = orig;
            }
        }
    }

    /// Checks the structure (P-BLOCK): block sizes, prefix sums, `uniform`, the artificial list, marks lengths.
    pub(crate) fn check_structure(&self) -> Result<(), String> {
        let n = self.blocks.len();
        if n > 1 {
            for b in 0..n {
                let lines = self.block_lines(b);
                // The last block may be short (as the load built it, or after a removal at the end).
                if lines > BLOCK_MAX || (lines < BLOCK_MIN && b != n - 1) {
                    return Err(format!("block {b} has {lines} lines"));
                }
            }
        }
        if n == 1 && self.block_lines(0) == 0 {
            return Err("an empty block".to_owned());
        }
        let total: u32 = (0..n).map(|b| self.block_lines(b)).sum();
        if total != self.len {
            return Err(format!("{total} lines in blocks, len {}", self.len));
        }
        let uniform = self.is_uniform();
        if uniform != self.first_line.is_empty() {
            return Err(format!(
                "uniform {uniform} but first_line has {}",
                self.first_line.len()
            ));
        }
        if !self.first_line.is_empty() {
            let mut line = 0;
            for b in 0..n {
                if self.first_line[b] != line {
                    return Err(format!(
                        "first_line[{b}] = {} != {line}",
                        self.first_line[b]
                    ));
                }
                line += self.block_lines(b);
            }
            if self.first_line[n] != line {
                return Err("first_line end".to_owned());
            }
        }
        if n > 0 {
            if self.first_byte.len() != n + 1 || self.first_byte[0] != 0 {
                return Err("first_byte shape".to_owned());
            }
            if self.first_byte[n] != self.text_len {
                return Err("first_byte end != text_len".to_owned());
            }
            for b in 0..n {
                let block = &self.blocks[b];
                if block.start(0) != 0 {
                    return Err(format!("block {b} does not start at 0"));
                }
                let len = self.first_byte[b + 1] - self.first_byte[b];
                for i in 0..block.lines() {
                    let next = if i + 1 < block.lines() {
                        block.start(i + 1)
                    } else {
                        len
                    };
                    if next < block.start(i) + eol_len(block.eol(i)) {
                        return Err(format!("block {b} line {i}: start after the next one"));
                    }
                }
                let unit = if block.marked() { 13 } else { 5 };
                if block.0.len() != unit * block.lines() || block.0[1..4] != [0, 0, 0] {
                    return Err(format!("block {b}: wrong data length or flag"));
                }
            }
        } else if !self.first_byte.is_empty() || self.text_len != 0 {
            return Err("an empty table with bytes".to_owned());
        }
        let artificial: Vec<u32> = (0..self.len)
            .filter(|&i| self.eol(i) == LineEnding::Artificial)
            .collect();
        if artificial != self.artificial {
            return Err("artificial list".to_owned());
        }
        if self.edited && self.orig_first.len() != n {
            return Err("orig_first length".to_owned());
        }
        Ok(())
    }

    /// Heap bytes of the table (memory accounting in tests): every block with its `Arc` header, the block
    /// list and the prefix sums.
    pub(crate) fn heap_bytes(&self) -> usize {
        let blocks: usize = self.blocks.iter().map(|b| 16 + b.0.len()).sum();
        blocks
            + self.blocks.capacity() * std::mem::size_of::<LineBlock>()
            + 4 * (self.first_line.capacity()
                + self.first_byte.capacity()
                + self.orig_first.capacity()
                + self.artificial.capacity())
    }
}

/// One block of `recs` (relative starts, terminators, marks when a line has a generation or a file line out of
/// sequence), the file line of its first line when unmarked, and its byte length.
fn build_block(recs: &[LineRec]) -> (LineBlock, u32, u32) {
    let n = recs.len();
    let first_orig = recs.first().map_or(NO_ORIG, |r| r.orig);
    let plain = first_orig != NO_ORIG
        && recs
            .iter()
            .enumerate()
            .all(|(i, r)| r.generation == 0 && first_orig.checked_add(i as u32) == Some(r.orig));
    let mut data = Vec::with_capacity(if plain { 5 * n } else { 13 * n });
    let mut at = 0u32;
    for r in recs {
        data.extend_from_slice(&at.to_le_bytes());
        at += r.content + eol_len(r.eol);
    }
    data.extend(recs.iter().map(|r| eol_code(r.eol)));
    if !plain {
        for r in recs {
            data.extend_from_slice(&r.generation.to_le_bytes());
        }
        for r in recs {
            data.extend_from_slice(&r.orig.to_le_bytes());
        }
    }
    if n > 0 {
        data[..4].copy_from_slice(&[u8::from(!plain), 0, 0, 0]);
    }
    (
        LineBlock(Arc::from(data)),
        if plain { first_orig } else { NO_ORIG },
        at,
    )
}
