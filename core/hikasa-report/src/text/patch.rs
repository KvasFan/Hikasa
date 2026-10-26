//! Shared patch machinery: hunk planning, line ranges, file headers, line writing and the `patch_lossy_eol` and
//! `patch_encoding` warnings.
//!
//! **Exactness (wave-4 decision 1).** A row is written as context only when its two lines are byte-for-byte
//! identical, terminators (and a re-emitted UTF-8 BOM) included; every other row of a hunk is written as its
//! deletion and its insertion, whatever the comparison rules say. So a row that the rules call same but whose
//! line endings differ (line endings are not compared by default, D-04), or whose last line lacks the final
//! newline on one side only, is a difference of the patch: applied to the left file, every patch reproduces the
//! right file. Hunks are the clusters of such differing rows with `lines` of context (two differences at most
//! `2 × lines` identical rows apart share a hunk, which matches the Context view of §2.18 whenever the rules
//! compare line endings); with Ignore Unimportant, the clusters without an important row are left out, and the
//! rows of the kept ones are still written exactly.

use std::io::Write;
use std::ops::Range;

use hikasa_core::{Side, Warning, WarningCode};
use hikasa_text::{
    Importance, LineEnding, LineIdx, Row, RowState, TextComparison, TextDocument, TextEncoding,
};

use crate::escape;
use crate::options::plural;
use crate::style::Sgr;
use crate::text::spec_text;

/// The lines of one side covered by a hunk: `len` lines from `start` (0-based); when `len == 0`, `start` is the
/// insertion point (the number of lines before the hunk, i.e. the 1-based number of the line before).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LineSpan {
    /// First line (0-based) or insertion point.
    pub start: u32,
    /// Number of lines.
    pub len: u32,
}

/// A maximal run of rows whose lines differ (or one such row, for a normal diff of a `lines_independent`
/// format): written as all its left lines, then all its right lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Group {
    /// Rows `[start, end)`.
    pub rows: Range<u32>,
    /// Left lines before the group.
    left_before: u32,
    /// Right lines before the group.
    right_before: u32,
    /// Left lines in the group.
    pub left: u32,
    /// Right lines in the group.
    pub right: u32,
    /// The strongest importance of its rows (`None` when every row is same under the rules and differs only in
    /// its line ending or BOM).
    pub importance: Importance,
}

/// One hunk: a cluster of [`Group`]s with the identical rows around and between them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Hunk {
    /// Rows `[start, end)`.
    pub rows: Range<u32>,
    /// Left lines.
    pub left: LineSpan,
    /// Right lines.
    pub right: LineSpan,
    /// The strongest importance of its rows.
    pub importance: Importance,
    /// Its groups, in row order.
    pub groups: Vec<Group>,
}

impl Hunk {
    /// Some row is important.
    pub fn important(&self) -> bool {
        self.importance == Importance::Important
    }
}

/// What the patch writers know of both sides: the documents and whether each one is UTF-8 with a BOM, which the
/// patch writes back before line 1 (the decoded text does not hold it), so that the patch applies to the bytes.
pub(crate) struct Sides<'a> {
    /// Left and right documents.
    pub docs: [&'a TextDocument; 2],
    /// UTF-8 with a BOM.
    bom: [bool; 2],
}

impl Sides<'_> {
    /// Side `s` is a UTF-8 file holding only its BOM: no line to write the BOM before, so a patch cannot
    /// express it (unless the other side is the same).
    fn bom_only(&self, s: usize) -> bool {
        self.bom.get(s).copied().unwrap_or(false)
            && self.docs.get(s).is_some_and(|d| d.line_count() == 0)
    }
}

impl<'a> Sides<'a> {
    /// The two sides of `cmp`.
    pub fn new(cmp: &'a TextComparison) -> Sides<'a> {
        let docs = [
            cmp.document(Side::Left).as_ref(),
            cmp.document(Side::Right).as_ref(),
        ];
        let bom =
            docs.map(|d| d.encoding().bom_len > 0 && d.encoding().encoding == TextEncoding::UTF_8);
        Sides { docs, bom }
    }

    /// The text written before line `idx` of side `s`: the BOM before line 1 of a UTF-8 file with a BOM.
    pub fn prefix(&self, s: usize, idx: LineIdx) -> &'static str {
        if idx.0 == 0 && self.bom.get(s).copied().unwrap_or(false) {
            "\u{feff}"
        } else {
            ""
        }
    }

    /// True when the row's two lines are byte-for-byte identical: a `Same` row (equal text, §2.17.1) whose lines
    /// end alike and carry the same written BOM. Only such rows are patch context.
    pub fn identical(&self, row: &Row) -> bool {
        match (row.state, row.left, row.right) {
            (RowState::Same, Some(l), Some(r)) => {
                self.docs[0].line_ending(l) == self.docs[1].line_ending(r)
                    && self.prefix(0, l) == self.prefix(1, r)
            }
            _ => false,
        }
    }
}

/// The stronger of two importances (`Important` > `Unimportant` > `None`).
fn stronger(a: Importance, b: Importance) -> Importance {
    match (a, b) {
        (Importance::Important, _) | (_, Importance::Important) => Importance::Important,
        (Importance::Unimportant, _) | (_, Importance::Unimportant) => Importance::Unimportant,
        _ => Importance::None,
    }
}

/// The groups of a comparison: maximal runs of rows that are not [`Sides::identical`], or one group per such row
/// when `split_rows` (the normal diff of a `lines_independent` format, §2.17.2), in row order.
fn groups(cmp: &TextComparison, sides: &Sides<'_>, split_rows: bool) -> Vec<Group> {
    let mut out: Vec<Group> = Vec::new();
    let (mut left_before, mut right_before) = (0u32, 0u32);
    for (i, row) in (0u32..).zip(cmp.rows(0..cmp.row_count())) {
        if !sides.identical(&row) {
            let importance = match row.state {
                RowState::Same => Importance::None,
                _ => row.importance,
            };
            match out.last_mut() {
                Some(g) if g.rows.end == i && !split_rows => {
                    g.rows.end = i + 1;
                    g.left += u32::from(row.left.is_some());
                    g.right += u32::from(row.right.is_some());
                    g.importance = stronger(g.importance, importance);
                }
                _ => out.push(Group {
                    rows: i..i + 1,
                    left_before,
                    right_before,
                    left: u32::from(row.left.is_some()),
                    right: u32::from(row.right.is_some()),
                    importance,
                }),
            }
        }
        left_before += u32::from(row.left.is_some());
        right_before += u32::from(row.right.is_some());
    }
    out
}

/// The hunks of a patch with `lines` of context: the groups, clustered when at most `2 × lines` identical rows
/// separate them, each cluster widened by `lines` rows on both sides (clamped to the file). With
/// `ignore_unimportant`, the clusters without an important row are left out. `split_rows` gives one group per
/// differing row (normal diffs of `lines_independent` formats; it only makes sense with `lines == 0`).
pub(crate) fn plan(
    cmp: &TextComparison,
    sides: &Sides<'_>,
    lines: u32,
    ignore_unimportant: bool,
    split_rows: bool,
) -> Vec<Hunk> {
    let row_count = cmp.row_count();
    let mut clusters: Vec<Vec<Group>> = Vec::new();
    for g in groups(cmp, sides, split_rows) {
        let end = clusters
            .last()
            .and_then(|c| c.last())
            .map(|last| last.rows.end);
        match (end, clusters.last_mut()) {
            (Some(end), Some(cluster))
                if g.rows.start > end && u64::from(g.rows.start - end) <= 2 * u64::from(lines) =>
            {
                cluster.push(g);
            }
            _ => clusters.push(vec![g]),
        }
    }
    clusters
        .into_iter()
        .filter_map(|groups| {
            let (first, last) = (groups.first()?, groups.last()?);
            let importance = groups
                .iter()
                .fold(Importance::None, |acc, g| stronger(acc, g.importance));
            if ignore_unimportant && importance != Importance::Important {
                return None;
            }
            // The rows around the cluster are identical rows: each holds one line of both sides.
            let lead = first.rows.start.min(lines);
            let trail = row_count.saturating_sub(last.rows.end).min(lines);
            let span = |before_first: u32, before_last: u32, in_last: u32| {
                let start = before_first - lead;
                LineSpan {
                    start,
                    len: before_last + in_last + trail - start,
                }
            };
            Some(Hunk {
                rows: first.rows.start - lead..last.rows.end + trail,
                left: span(first.left_before, last.left_before, last.left),
                right: span(first.right_before, last.right_before, last.right),
                importance,
                groups,
            })
        })
        .collect()
}

/// A unified range: `l` for one line, `l,0` (the line before) for none, else `l,s` (§3.4).
pub(crate) fn unified_range(span: LineSpan) -> String {
    match span.len {
        0 => format!("{},0", span.start),
        1 => format!("{}", span.start + 1),
        n => format!("{},{}", span.start + 1, n),
    }
}

/// A normal-diff range of at least one line: `l` for one line, else `first,last`.
pub(crate) fn line_range(span: LineSpan) -> String {
    match span.len {
        0 | 1 => format!("{}", span.start + 1),
        n => format!("{},{}", span.start + 1, span.start + n),
    }
}

/// A context-diff range: `l` for one line, `first,last` for more, and for none `0` at the start of the file,
/// else `l+1,l` after line `l` (an empty `first,last` range). Patch programs read a lone non-zero number as one
/// line, so the `l` form of an empty range (which `diff -c` writes) makes them reject a zero-context deletion
/// that is not at the start of the file; `l+1,l` reads as no line after line `l`.
pub(crate) fn context_range(span: LineSpan) -> String {
    match (span.len, span.start) {
        (0, 0) => "0".to_owned(),
        (0, l) => format!("{},{l}", l + 1),
        _ => line_range(span),
    }
}

/// What a patch cannot write as it is, per side (index 0 left, 1 right): the terminators that lose information,
/// and whether the files differ at all (then a side in another encoding cannot be reproduced byte for byte).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Lossy {
    /// A lone-CR line is written with LF.
    pub cr: [bool; 2],
    /// An artificial (characters-per-line) break is written as LF.
    pub artificial: [bool; 2],
    /// The files differ: the patch has a hunk, or the bytes differ (two files whose text is the same but whose
    /// encodings are not give no hunk).
    pub differ: bool,
}

impl Lossy {
    /// Records that line `idx` of side `s` is written.
    fn note(&mut self, s: usize, doc: &TextDocument, idx: LineIdx) {
        let (Some(cr), Some(artificial)) = (self.cr.get_mut(s), self.artificial.get_mut(s)) else {
            return;
        };
        match doc.line_ending(idx) {
            LineEnding::Cr => *cr = true,
            LineEnding::Artificial => *artificial = true,
            LineEnding::Lf | LineEnding::CrLf | LineEnding::None => {}
        }
    }

    /// The lines every patch style writes for `hunks`: the lines of both sides of every row (context rows stand
    /// for both files; the context style may leave a part out, but then its lines are the other part's context).
    pub fn of_hunks(cmp: &TextComparison, sides: &Sides<'_>, hunks: &[Hunk]) -> Lossy {
        let mut lossy = Lossy {
            differ: !hunks.is_empty() || !cmp.bytes_identical(),
            ..Lossy::default()
        };
        for h in hunks {
            for row in cmp.rows(h.rows.clone()) {
                if let Some(l) = row.left {
                    lossy.note(0, sides.docs[0], l);
                }
                if let Some(r) = row.right {
                    lossy.note(1, sides.docs[1], r);
                }
            }
        }
        lossy
    }

    /// The warnings: `patch_lossy_eol` (left CR, left artificial, right CR, right artificial), then
    /// `patch_encoding` once per side that the patch cannot reproduce byte for byte: when the files differ, a side
    /// stored in another encoding than UTF-8 or decoded lossily (the patch is UTF-8 text); and a side holding
    /// nothing but a UTF-8 BOM when the other side does not (there is no line to write the BOM with).
    pub fn warnings(&self, sides: &Sides<'_>) -> Vec<Warning> {
        let mut out = Vec::new();
        for (s, side) in [(0usize, Side::Left), (1, Side::Right)] {
            let spec = spec_text(sides.docs[s]);
            if self.cr[s] {
                out.push(Warning {
                    code: WarningCode::PatchLossyEol,
                    message: format!(
                        "{side} lines ending in a lone CR are written with LF in the patch ({spec})"
                    ),
                    spec: Some(spec.clone()),
                });
            }
            if self.artificial[s] {
                out.push(Warning {
                    code: WarningCode::PatchLossyEol,
                    message: format!(
                        "{side} lines broken by the characters-per-line limit are written as separate lines in \
                         the patch ({spec})"
                    ),
                    spec: Some(spec),
                });
            }
        }
        for (s, side) in [(0usize, Side::Left), (1, Side::Right)] {
            let doc = sides.docs[s];
            let spec = spec_text(doc);
            if sides.bom_only(s) && !sides.bom_only(1 - s) {
                out.push(Warning {
                    code: WarningCode::PatchEncoding,
                    message: format!(
                        "the {side} file holds only a byte-order mark, which a patch cannot express ({spec})"
                    ),
                    spec: Some(spec),
                });
                continue;
            }
            if !self.differ {
                continue;
            }
            let encoding = doc.encoding().encoding;
            let why = if encoding != TextEncoding::UTF_8 {
                format!("is stored in {}", encoding.name())
            } else if doc.malformed() > 0 {
                format!(
                    "is not valid UTF-8 ({} replaced with U+FFFD)",
                    plural(doc.malformed(), "malformed sequence")
                )
            } else {
                continue;
            };
            out.push(Warning {
                code: WarningCode::PatchEncoding,
                message: format!(
                    "the {side} file {why}; the patch is written in UTF-8 and may not apply to its bytes ({spec})"
                ),
                spec: Some(spec),
            });
        }
        out
    }
}

/// `name[\t<mtime RFC 3339>]` of a patch file header.
fn file_label(doc: &TextDocument) -> String {
    let name = spec_text(doc);
    let mut label = escape::patch_name(&name).into_owned();
    if let Some(mtime) = doc.origin().mtime {
        label.push('\t');
        label.push_str(&mtime.to_rfc3339());
    }
    label
}

/// Writes the two file header lines (`--- left` / `+++ right`, or `*** left` / `--- right`).
pub(crate) fn file_headers(
    w: &mut dyn Write,
    sides: &Sides<'_>,
    left_marker: &str,
    right_marker: &str,
    ansi: bool,
) -> std::io::Result<()> {
    let (on, off) = if ansi {
        (Sgr::BOLD, Sgr::RESET)
    } else {
        ("", "")
    };
    writeln!(w, "{on}{left_marker}{}{off}", file_label(sides.docs[0]))?;
    writeln!(w, "{on}{right_marker}{}{off}", file_label(sides.docs[1]))?;
    Ok(())
}

/// The colour of a changed line under `ansi` (a row that is same under the rules and differs only in its line
/// ending or BOM is shown as unimportant).
pub(crate) fn tone(ansi: bool, importance: Importance) -> &'static str {
    match (ansi, importance) {
        (false, _) => "",
        (true, Importance::Important) => Sgr::RED,
        (true, Importance::Unimportant | Importance::None) => Sgr::BLUE,
    }
}

/// The importance a differing row is coloured with.
pub(crate) fn row_importance(row: &Row) -> Importance {
    match row.state {
        RowState::Same => Importance::None,
        _ => row.importance,
    }
}

/// Writes `prefix` + the line (with the BOM before line 1 of a UTF-8 file with a BOM) + its terminator: LF and
/// CRLF as they are, a lone CR and an artificial break as LF (`patch_lossy_eol`), and after a last line without
/// terminator a newline plus `\ No newline at end of file`. With `ansi` (terminal output, never applied), the
/// line's control and bidirectional characters are escaped and `colour` (an SGR sequence or `""`) wraps the prefix
/// and the text.
pub(crate) fn write_line(
    w: &mut dyn Write,
    sides: &Sides<'_>,
    s: usize,
    prefix: &str,
    idx: LineIdx,
    ansi: bool,
    colour: &str,
) -> std::io::Result<()> {
    let doc = sides.docs[s];
    let text = doc.line(idx);
    let bom = sides.prefix(s, idx);
    if ansi {
        let text = escape::escape_terminal(&text);
        if colour.is_empty() {
            write!(w, "{prefix}{bom}{text}")?;
        } else {
            write!(w, "{colour}{prefix}{bom}{text}{}", Sgr::RESET)?;
        }
    } else {
        w.write_all(prefix.as_bytes())?;
        w.write_all(bom.as_bytes())?;
        w.write_all(text.as_bytes())?;
    }
    match doc.line_ending(idx) {
        LineEnding::CrLf => w.write_all(b"\r\n"),
        LineEnding::Lf | LineEnding::Cr | LineEnding::Artificial => w.write_all(b"\n"),
        LineEnding::None => w.write_all(b"\n\\ No newline at end of file\n"),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use hikasa_core::Ctl;
    use hikasa_text::{CompareInput, CompiledFormat, TextRules, compare};

    use super::*;

    fn cmp(left: &[u8], right: &[u8], compare_eol: bool) -> TextComparison {
        let mut rules = TextRules::default();
        rules.importance.compare_line_endings = compare_eol;
        let doc = |b: &[u8]| Arc::new(TextDocument::from_text(std::str::from_utf8(b).unwrap()));
        compare(
            CompareInput {
                left: doc(left),
                right: doc(right),
                format: Arc::new(CompiledFormat::everything_else()),
                rules,
                bytes_identical: left == right,
                force_tokenize: false,
            },
            None,
            &Ctl::detached(),
            None,
        )
        .unwrap()
    }

    /// `(left span, right span)` of every hunk, as `(start, len)`.
    fn hunks(
        c: &TextComparison,
        lines: u32,
        iu: bool,
        split: bool,
    ) -> Vec<((u32, u32), (u32, u32))> {
        let sides = Sides::new(c);
        plan(c, &sides, lines, iu, split)
            .into_iter()
            .map(|h| ((h.left.start, h.left.len), (h.right.start, h.right.len)))
            .collect()
    }

    #[test]
    fn ranges() {
        let s = |start, len| LineSpan { start, len };
        assert_eq!(unified_range(s(0, 0)), "0,0");
        assert_eq!(unified_range(s(4, 0)), "4,0");
        assert_eq!(unified_range(s(4, 1)), "5");
        assert_eq!(unified_range(s(4, 3)), "5,3");
        assert_eq!(line_range(s(4, 1)), "5");
        assert_eq!(line_range(s(4, 3)), "5,7");
        assert_eq!(context_range(s(0, 0)), "0");
        assert_eq!(context_range(s(4, 0)), "5,4");
        assert_eq!(context_range(s(4, 1)), "5");
        assert_eq!(context_range(s(4, 3)), "5,7");
    }

    #[test]
    fn endings_make_rows_differ_whatever_the_rules() {
        // Same under the default rules (line endings not compared), but not byte-identical.
        let c = cmp(b"a\nb\r\nc\n", b"a\nb\nc\n", false);
        assert_eq!(c.stats().same_rows, 3);
        assert_eq!(hunks(&c, 1, false, false), [((0, 3), (0, 3))]);
        // A missing final newline on one side.
        let c = cmp(b"a\nb", b"a\nb\nc\n", false);
        assert_eq!(hunks(&c, 0, false, false), [((1, 1), (1, 2))]);
        // Identical files: no hunk.
        let c = cmp(b"a\r\nb", b"a\r\nb", false);
        assert!(hunks(&c, 3, false, false).is_empty());
    }

    #[test]
    fn clusters_merge_within_twice_the_context() {
        let left = b"1\n2\n3\n4\n5\n6\n7\n8\n9\n";
        // Changes of lines 2 and 6 (three identical lines between).
        let right = b"1\nX\n3\n4\n5\nY\n7\n8\n9\n";
        let c = cmp(left, right, true);
        assert_eq!(
            hunks(&c, 1, false, false),
            [((0, 3), (0, 3)), ((4, 3), (4, 3))]
        );
        assert_eq!(hunks(&c, 2, false, false), [((0, 8), (0, 8))]);
        assert_eq!(
            hunks(&c, 0, false, false),
            [((1, 1), (1, 1)), ((5, 1), (5, 1))]
        );
        // A pure insertion: the empty left span starts at the insertion point.
        let c = cmp(b"a\nb\n", b"a\nX\nb\n", true);
        assert_eq!(hunks(&c, 0, false, false), [((1, 0), (1, 1))]);
        assert_eq!(hunks(&c, 5, false, false), [((0, 2), (0, 3))]);
        // A pure deletion at the end.
        let c = cmp(b"a\nb\n", b"a\n", true);
        assert_eq!(hunks(&c, 0, false, false), [((1, 1), (1, 0))]);
    }

    #[test]
    fn ignore_unimportant_drops_clusters_without_an_important_row() {
        // Line 2 (its ending only) and line 7 (an important change) are far apart: two clusters.
        let c = cmp(b"1\n2\r\n3\n4\n5\n6\n7\n", b"1\n2\n3\n4\n5\n6\nX\n", false);
        assert_eq!(hunks(&c, 1, false, false).len(), 2);
        assert_eq!(hunks(&c, 1, true, false), [((5, 2), (5, 2))]);
        // Close together they form one cluster, kept whole, the ending row included.
        assert_eq!(hunks(&c, 3, true, false), [((0, 7), (0, 7))]);
    }

    #[test]
    fn split_rows_gives_one_group_per_row() {
        let c = cmp(b"ab 1\nab 2\nc\n", b"ab 3\nab 4\nc\n", true);
        assert_eq!(c.stats().changed_rows, 2);
        assert_eq!(hunks(&c, 0, false, false), [((0, 2), (0, 2))]);
        assert_eq!(
            hunks(&c, 0, false, true),
            [((0, 1), (0, 1)), ((1, 1), (1, 1))]
        );
    }

    #[test]
    fn warnings_name_lossy_endings_and_encodings() {
        let c = cmp(b"a\rb\r", b"a\rB\r", false);
        let sides = Sides::new(&c);
        let lossy = Lossy::of_hunks(&c, &sides, &plan(&c, &sides, 1, false, false));
        let codes: Vec<_> = lossy.warnings(&sides).iter().map(|w| w.code).collect();
        assert_eq!(
            codes,
            [WarningCode::PatchLossyEol, WarningCode::PatchLossyEol]
        );
        assert_eq!(tone(true, Importance::None), Sgr::BLUE);
        assert_eq!(tone(false, Importance::Important), "");
    }
}
