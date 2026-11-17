//! Rows, layout and display filters (HX-001, HX-004, HX-005; D-332): [`HexView`], [`HexLayout`], [`HexRow`],
//! [`HexCell`].
//!
//! Rows cut the aligned columns every `bytes_per_line`; an address is that side's offset of its first byte in the
//! row (none when the row holds no byte of that side), so a gap shows as empty cells on the other side. A cell's
//! state is decided by its bytes. The `differences` filter shows the rows that intersect a section (a row of at
//! least 8 bytes does so only when it holds a non-same cell; with fewer bytes per line a row inside a section's
//! merge gap counts too: the section is the unit of navigation and filtering), with `context` rows around them;
//! `same` shows the others. Hidden runs are counted in `hidden_before`. Past a truncated section list, filtered
//! views end with one marker row (no cells) whose `hidden_before` counts the rows shown under `all` only.

use std::borrow::Cow;
use std::ops::Range;
use std::sync::OnceLock;

use hikasa_core::Side;
use serde::{Deserialize, Serialize};

use crate::align::{HexAlignment, Sections, SegmentKind, SegmentStore, none_store};
use crate::document::HexDocument;
use crate::error::HexError;
use crate::vocab::{CharSet, DisplayFilter, Radix};

/// The layout of the panes and reports (HX-001; the Layout dialog of W13 edits it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HexLayout {
    /// Bytes per line, 1…256 (default 16).
    pub bytes_per_line: u16,
    /// Digits grouped by 1, 2, 4 or 8 bytes (default 1; spacing only).
    pub group: u8,
    /// The character column (default ASCII).
    pub chars: CharSet,
    /// The address radix (default hex).
    pub address: Radix,
}

impl Default for HexLayout {
    fn default() -> HexLayout {
        HexLayout {
            bytes_per_line: 16,
            group: 1,
            chars: CharSet::Ascii,
            address: Radix::Hex,
        }
    }
}

impl HexLayout {
    fn bpl(&self) -> u64 {
        u64::from(self.bytes_per_line.clamp(1, 256))
    }

    /// The character a byte shows in the character column (`None` when the column is hidden): ASCII shows
    /// 0x20…0x7E, Latin-1 also 0xA0…0xFF, other bytes `.`.
    pub fn char_of(&self, byte: u8) -> Option<char> {
        match self.chars {
            CharSet::None => None,
            CharSet::Ascii if (0x20..=0x7E).contains(&byte) => Some(char::from(byte)),
            CharSet::Latin1 if (0x20..=0x7E).contains(&byte) || byte >= 0xA0 => {
                Some(char::from(byte))
            }
            CharSet::Ascii | CharSet::Latin1 => Some('.'),
        }
    }
}

/// The state of one cell (HX-004).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ByteState {
    /// Paired, equal.
    Same,
    /// Paired, different.
    Different,
    /// Left byte, gap on the right.
    LeftOnly,
    /// Right byte, gap on the left.
    RightOnly,
}

/// The state of a row: `different` when a paired cell differs, else `orphan` when it holds one-sided cells, else
/// `same`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowState {
    /// Every cell same.
    Same,
    /// A paired cell differs.
    Different,
    /// One-sided cells and no differing paired cell.
    Orphan,
}

/// One cell: an aligned column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HexCell {
    /// The left byte (none in a gap).
    pub left: Option<u8>,
    /// The right byte.
    pub right: Option<u8>,
    /// Its state.
    pub state: ByteState,
}

/// One row of a pane pair.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HexRow {
    /// Index of the row in the unfiltered view.
    pub index: u64,
    /// Left offset of the row's first left byte.
    pub left_address: Option<u64>,
    /// Right offset of the row's first right byte.
    pub right_address: Option<u64>,
    /// The cells (`bytes_per_line`, fewer in the last row; none in a marker row).
    pub cells: Vec<HexCell>,
    /// Its state.
    pub state: RowState,
    /// Rows hidden by the filter just before this one.
    pub hidden_before: u64,
}

/// The rows the filter shows: runs of unfiltered rows and the visible rows before each.
#[derive(Debug, Clone, Default)]
struct Visible {
    /// Shown runs of unfiltered rows, in order.
    runs: Vec<Range<u64>>,
    /// Shown rows before run `k`.
    before: Vec<u64>,
    /// Rows past a truncated section list (shown under `all` only; one marker row).
    unclassified: Option<Range<u64>>,
}

impl Visible {
    fn shown(&self) -> u64 {
        self.runs
            .last()
            .map_or(0, |r| self.before[self.runs.len() - 1] + (r.end - r.start))
    }
}

/// Two documents, their alignment (or None's provisional one), a layout and a filter: the rows W13 draws and the
/// reports print.
pub struct HexView<'a> {
    left: &'a HexDocument,
    right: &'a HexDocument,
    segments: Cow<'a, SegmentStore>,
    sections: Option<&'a Sections>,
    layout: HexLayout,
    filter: DisplayFilter,
    context: u8,
    visible: OnceLock<Visible>,
}

impl<'a> HexView<'a> {
    /// A view of an alignment (filter `all`).
    pub fn new(
        left: &'a HexDocument,
        right: &'a HexDocument,
        alignment: &'a HexAlignment,
        layout: HexLayout,
    ) -> HexView<'a> {
        HexView {
            left,
            right,
            segments: Cow::Borrowed(&alignment.segments),
            sections: Some(&alignment.sections),
            layout,
            filter: DisplayFilter::All,
            context: 0,
            visible: OnceLock::new(),
        }
    }

    /// None's layout before the alignment lands: states from the bytes, no scan (the first screen needs only the
    /// first pages). Filters other than `all` show every row.
    pub fn provisional(
        left: &'a HexDocument,
        right: &'a HexDocument,
        layout: HexLayout,
    ) -> HexView<'a> {
        HexView {
            left,
            right,
            segments: Cow::Owned(none_store(left.len(), right.len())),
            sections: None,
            layout,
            filter: DisplayFilter::All,
            context: 0,
            visible: OnceLock::new(),
        }
    }

    /// The same view with a display filter and `context` rows (0…64) around differing rows of `differences`.
    pub fn with_filter(mut self, filter: DisplayFilter, context: u8) -> HexView<'a> {
        self.filter = filter;
        self.context = context.min(64);
        self.visible = OnceLock::new();
        self
    }

    /// The layout.
    pub fn layout(&self) -> &HexLayout {
        &self.layout
    }

    /// The segments the rows are cut from.
    pub fn segments(&self) -> &SegmentStore {
        &self.segments
    }

    /// Rows without a filter.
    pub fn full_row_count(&self) -> u64 {
        self.segments.columns().div_ceil(self.layout.bpl())
    }

    fn filtered(&self) -> Option<&Visible> {
        let sections = self.sections?;
        if self.filter == DisplayFilter::All {
            return None;
        }
        Some(self.visible.get_or_init(|| self.compute_visible(sections)))
    }

    fn compute_visible(&self, sections: &Sections) -> Visible {
        let bpl = self.layout.bpl();
        let total = self.full_row_count();
        let classified_end = if sections.truncated() {
            sections.stored_end().div_ceil(bpl).min(total)
        } else {
            total
        };
        // Rows that intersect a section, merged.
        let mut differing: Vec<Range<u64>> = Vec::new();
        for i in 0..sections.len() {
            let Some(cols) = sections.columns(i) else {
                continue;
            };
            let rows = cols.start / bpl..(cols.end - 1) / bpl + 1;
            match differing.last_mut() {
                Some(last) if rows.start <= last.end => last.end = last.end.max(rows.end),
                _ => differing.push(rows),
            }
        }
        let runs: Vec<Range<u64>> = match self.filter {
            DisplayFilter::Differences => {
                let c = u64::from(self.context);
                let mut out: Vec<Range<u64>> = Vec::new();
                for r in &differing {
                    let wide = r.start.saturating_sub(c)..(r.end + c).min(classified_end);
                    match out.last_mut() {
                        Some(last) if wide.start <= last.end => last.end = last.end.max(wide.end),
                        _ => out.push(wide),
                    }
                }
                out
            }
            _ => {
                let mut out = Vec::new();
                let mut at = 0;
                for r in &differing {
                    if r.start > at {
                        out.push(at..r.start);
                    }
                    at = r.end;
                }
                if at < classified_end {
                    out.push(at..classified_end);
                }
                out
            }
        };
        let mut before = Vec::with_capacity(runs.len());
        let mut shown = 0;
        for r in &runs {
            before.push(shown);
            shown += r.end - r.start;
        }
        Visible {
            runs,
            before,
            unclassified: (classified_end < total).then_some(classified_end..total),
        }
    }

    /// Rows under the filter (a marker row past a truncated section list included).
    pub fn row_count(&self) -> u64 {
        match self.filtered() {
            None => self.full_row_count(),
            Some(v) => v.shown() + u64::from(v.unclassified.is_some()),
        }
    }

    /// Rows `first..first + count` under the filter (fewer at the end). Reads through the documents' page caches
    /// (blocking: W13 calls it in a job or checks `is_cached` first).
    ///
    /// # Errors
    ///
    /// A read error; [`HexError::Truncated`].
    pub fn rows(&self, first: u64, count: u32) -> Result<Vec<HexRow>, HexError> {
        let mut out = Vec::with_capacity(count as usize);
        let end = first.saturating_add(u64::from(count)).min(self.row_count());
        let Some(v) = self.filtered() else {
            for r in first..end {
                out.push(self.full_row(r, 0)?);
            }
            return Ok(out);
        };
        let shown = v.shown();
        for k in first..end {
            if k >= shown {
                // The marker row past a truncated section list.
                let rest = v.unclassified.clone().unwrap_or(0..0);
                let prev_end = v.runs.last().map_or(0, |r| r.end);
                out.push(HexRow {
                    index: rest.start,
                    left_address: None,
                    right_address: None,
                    cells: Vec::new(),
                    state: RowState::Same,
                    hidden_before: rest.start.saturating_sub(prev_end) + (rest.end - rest.start),
                });
                continue;
            }
            let run = v.before.partition_point(|&b| b <= k) - 1;
            let row = v.runs[run].start + (k - v.before[run]);
            let hidden = if row == v.runs[run].start {
                let prev_end = if run == 0 { 0 } else { v.runs[run - 1].end };
                row - prev_end
            } else {
                0
            };
            out.push(self.full_row(row, hidden)?);
        }
        Ok(out)
    }

    /// The unfiltered row `row`.
    fn full_row(&self, row: u64, hidden_before: u64) -> Result<HexRow, HexError> {
        let bpl = self.layout.bpl();
        let c0 = row * bpl;
        let c1 = (c0 + bpl).min(self.segments.columns());
        // Pass 1: the kinds of the columns and the byte ranges of each side.
        let mut kinds = Vec::with_capacity((c1 - c0) as usize);
        let (mut left_first, mut right_first) = (None, None);
        let (mut nl, mut nr) = (0u64, 0u64);
        if let Some((mut i, mut start)) = self.segments.at_column(c0) {
            let mut col = c0;
            while col < c1 {
                let Some(seg) = self.segments.get(i) else {
                    break;
                };
                let stop = (start + seg.len).min(c1);
                let within = col - start;
                if seg.left_len() > 0 {
                    left_first.get_or_insert(seg.left + within);
                    nl += stop - col;
                }
                if seg.right_len() > 0 {
                    right_first.get_or_insert(seg.right + within);
                    nr += stop - col;
                }
                kinds.extend(std::iter::repeat_n(seg.kind, (stop - col) as usize));
                col = stop;
                start += seg.len;
                i += 1;
            }
        }
        let mut lb = vec![0u8; nl as usize];
        let mut rb = vec![0u8; nr as usize];
        if let Some(at) = left_first {
            read_exact(self.left, at, &mut lb)?;
        }
        if let Some(at) = right_first {
            read_exact(self.right, at, &mut rb)?;
        }
        let (mut li, mut ri) = (0usize, 0usize);
        let mut state = RowState::Same;
        let cells = kinds
            .into_iter()
            .map(|kind| {
                let (left, right) = match kind {
                    SegmentKind::Paired => {
                        li += 1;
                        ri += 1;
                        (Some(lb[li - 1]), Some(rb[ri - 1]))
                    }
                    SegmentKind::LeftOnly => {
                        li += 1;
                        (Some(lb[li - 1]), None)
                    }
                    SegmentKind::RightOnly => {
                        ri += 1;
                        (None, Some(rb[ri - 1]))
                    }
                };
                let cell_state = match (left, right) {
                    (Some(a), Some(b)) if a == b => ByteState::Same,
                    (Some(_), Some(_)) => ByteState::Different,
                    (Some(_), None) => ByteState::LeftOnly,
                    _ => ByteState::RightOnly,
                };
                state = match (state, cell_state) {
                    (_, ByteState::Different) | (RowState::Different, _) => RowState::Different,
                    (_, ByteState::LeftOnly | ByteState::RightOnly) => RowState::Orphan,
                    (s, ByteState::Same) => s,
                };
                HexCell {
                    left,
                    right,
                    state: cell_state,
                }
            })
            .collect();
        Ok(HexRow {
            index: row,
            left_address: left_first,
            right_address: right_first,
            cells,
            state,
            hidden_before,
        })
    }

    /// The row (under the filter) nearest to the one showing byte `offset` of `side` (`None` past that side's end
    /// or when the filter shows nothing).
    pub fn row_of(&self, side: Side, offset: u64) -> Option<u64> {
        let i = self.segments.at_offset(side, offset)?;
        let seg = self.segments.get(i)?;
        let start = if side == Side::Right {
            seg.right
        } else {
            seg.left
        };
        let col = self.segments.column_of(i) + (offset - start);
        let row = col / self.layout.bpl();
        let Some(v) = self.filtered() else {
            return Some(row);
        };
        if v.unclassified.as_ref().is_some_and(|u| u.contains(&row)) {
            return Some(v.shown());
        }
        let run = v.runs.partition_point(|r| r.end <= row);
        match v.runs.get(run) {
            Some(r) if r.start <= row => Some(v.before[run] + (row - r.start)),
            Some(_) => Some(v.before[run]),
            None if run > 0 => Some(v.shown() - 1),
            None => None,
        }
    }

    /// The unfiltered row index of filtered row `k` (`None` past the end or for the marker row).
    pub fn unfiltered_row(&self, k: u64) -> Option<u64> {
        match self.filtered() {
            None => (k < self.full_row_count()).then_some(k),
            Some(v) => {
                if k >= v.shown() {
                    return None;
                }
                let run = v.before.partition_point(|&b| b <= k) - 1;
                Some(v.runs[run].start + (k - v.before[run]))
            }
        }
    }

    /// The filtered row showing unfiltered row `row`, or the next shown one (`None` when none follows).
    pub fn filtered_row(&self, row: u64) -> Option<u64> {
        match self.filtered() {
            None => (row < self.full_row_count()).then_some(row),
            Some(v) => {
                let run = v.runs.partition_point(|r| r.end <= row);
                let r = v.runs.get(run)?;
                Some(v.before[run] + row.saturating_sub(r.start))
            }
        }
    }

    /// Digits of an address of the larger file (hex: at least 8; decimal: at least 1).
    pub fn address_width(&self) -> u8 {
        address_width(self.left.len().max(self.right.len()), self.layout.address)
    }
}

/// Digits of the addresses of a file of `len` bytes in `radix`.
pub(crate) fn address_width(len: u64, radix: Radix) -> u8 {
    let last = len.saturating_sub(1);
    match radix {
        Radix::Hex => ((64 - last.leading_zeros()).div_ceil(4) as u8).max(8),
        Radix::Decimal => last.checked_ilog10().map_or(1, |d| d as u8 + 1),
    }
}

/// Reads exactly `buf.len()` bytes through the page cache.
fn read_exact(doc: &HexDocument, at: u64, buf: &mut [u8]) -> Result<(), HexError> {
    let n = doc.read(at, buf)?;
    if n < buf.len() {
        return Err(HexError::Truncated {
            spec: doc.display().to_owned(),
        });
    }
    Ok(())
}
