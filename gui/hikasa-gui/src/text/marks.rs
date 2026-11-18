//! Ignored lines, alignment overrides and conversions of a text view (TC-024, TC-013, TC-063; M2 W11b, contract
//! §1.11.19, D-310), toolkit-free. *Ignored* toggles the selected rows' lines in `LiveComparison::set_ignored` (no
//! recompare); *Align With* enters an align mode completed by a click or a selection and Enter in the other pane
//! (`Anchor::Pair`), *Isolate* toggles `Anchor::Isolate`, *Clear Alignment Overrides* drops them (`set_anchors`, the next
//! recompare runs in full); the conversions are `transform` on the focus pane as one step.

use std::ops::Range;

use hikasa_core::Side;
use hikasa_text::edit::LineSet;
use hikasa_text::{Anchor, AnchorSide, IgnoredLines, LineIdx, RowIdx, Transform, transform};

use super::edit::{self, Origin, current_section, selected_rows};
use super::{TextRequest, TextViewModel};
use crate::commands::CommandId;
use crate::status::Message;

/// The status text of the align mode (D-310).
pub const ALIGN_STATUS: &str =
    "Select the lines to align with in the other pane, then press Enter or click";

/// The align mode: the lines of `side` waiting for their partner in the other pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlignMode {
    /// The pane the lines were chosen in.
    pub side: Side,
    /// Its lines (0-based, end exclusive).
    pub lines: Range<u32>,
}

/// The rows of a command (the selection's, else the current section's; a single row of the caret otherwise).
fn rows_of(view: &TextViewModel) -> Option<Range<u32>> {
    selected_rows(view)
        .or_else(|| current_section(view))
        .or_else(|| view.cursor().map(|r| r.0..r.0 + 1))
}

/// The lines of each side that `rows` show (`None`: no line of that side).
pub fn lines_of_rows(view: &TextViewModel, rows: Range<u32>) -> [Option<Range<u32>>; 2] {
    let Some(cmp) = view.comparison() else {
        return [None, None];
    };
    let mut out: [Option<Range<u32>>; 2] = [None, None];
    for r in rows {
        let Some(row) = cmp.row(RowIdx(r)) else {
            continue;
        };
        for (k, line) in [row.left, row.right].into_iter().enumerate() {
            if let Some(l) = line {
                let e = out[k].get_or_insert(l.0..l.0 + 1);
                e.start = e.start.min(l.0);
                e.end = e.end.max(l.0 + 1);
            }
        }
    }
    out
}

/// *Ignored* (TC-024): the lines of the selected rows (no selection: the current section's) on each side are added to
/// the ignored lines, or removed when every one is already ignored; `set_ignored` (no recompare). True when it
/// changed something.
pub fn toggle_ignored(view: &mut TextViewModel) -> bool {
    let Some(rows) = rows_of(view) else {
        return false;
    };
    let lines = lines_of_rows(view, rows);
    let Some(live) = view.live_view() else {
        return false;
    };
    let mut ignored: IgnoredLines = live.live.ignored().clone();
    let all = lines.iter().enumerate().all(|(k, r)| {
        r.as_ref().is_none_or(|r| {
            if k == 0 {
                ignored.left.contains_range(r)
            } else {
                ignored.right.contains_range(r)
            }
        })
    });
    if lines.iter().all(Option::is_none) {
        return false;
    }
    for (k, r) in lines.into_iter().enumerate() {
        let Some(r) = r else { continue };
        let set = if k == 0 {
            &mut ignored.left
        } else {
            &mut ignored.right
        };
        if all {
            set.remove(r);
        } else {
            set.insert(r);
        }
    }
    if let Some(live) = view.view.as_mut() {
        live.live.set_ignored(ignored);
    }
    view.installed();
    view.rebuild_keep();
    true
}

/// The check state of *Ignored*: the cursor row is ignored.
pub fn row_ignored(view: &TextViewModel) -> bool {
    let (Some(cmp), Some(row)) = (view.comparison(), view.cursor()) else {
        return false;
    };
    cmp.row(row).is_some_and(|r| r.ignored)
}

/// The message of *Align With* and *Isolate* on a line set with gaps whose caret is in none of its blocks.
pub const GAPS: &str = "The selected lines have gaps: put the caret on the block of lines to use.";

/// The focus pane's lines a command acts on: the selected lines, else the caret's line. A line set with gaps (Find's
/// *Select All*) gives the block of lines under the caret; `None` when the caret is in none of them (W11b review
/// finding 5).
fn focus_lines(view: &TextViewModel) -> Option<(Side, Range<u32>)> {
    let side = view.focus();
    let pane = view.pane(side);
    if let Some(set) = pane.lines.as_ref().filter(|l| !l.is_empty()) {
        let r = set.0.ranges();
        if let [one] = r {
            return Some((side, one.clone()));
        }
        let caret = pane.caret.line.0;
        return r
            .iter()
            .find(|b| b.contains(&caret))
            .map(|b| (side, b.clone()));
    }
    let r = pane.selection().range();
    let last = if r.end.byte == 0 && r.end.line > r.start.line {
        r.end.line.0 - 1
    } else {
        r.end.line.0
    };
    Some((side, r.start.line.0..last + 1))
}

/// [`focus_lines`], or the message panel's [`GAPS`].
fn focus_lines_or_say(view: &mut TextViewModel) -> Option<(Side, Range<u32>)> {
    let lines = focus_lines(view);
    if lines.is_none() {
        view.request_window(TextRequest::Message(Message::info(GAPS)));
    }
    lines
}

fn overlaps(a: &Range<u32>, b: &Range<u32>) -> bool {
    a.start < b.end && b.start < a.end
}

fn anchor_side(side: Side) -> AnchorSide {
    if side == Side::Right {
        AnchorSide::Right
    } else {
        AnchorSide::Left
    }
}

/// Replaces the anchors; the next recompare runs in full (scheduled at once).
fn set_anchors(view: &mut TextViewModel, anchors: Vec<Anchor>) {
    let now = view.now;
    if let Some(live) = view.view.as_mut() {
        live.live.set_anchors(anchors);
    }
    view.recompare.on_edit(now);
    view.rebuild_keep();
}

/// The anchors of now.
pub fn anchors(view: &TextViewModel) -> Vec<Anchor> {
    view.live_view()
        .map(|l| l.live.anchors().to_vec())
        .unwrap_or_default()
}

/// *Align With* (F7): with lines chosen in the focus pane the view enters the align mode; on lines inside an existing
/// pair, that pair is removed instead (D-310's open default: a toggle).
pub fn align_with(view: &mut TextViewModel) {
    if view.is_edit() || !view.loaded() {
        return;
    }
    let Some((side, lines)) = focus_lines_or_say(view) else {
        return;
    };
    let mut list = anchors(view);
    let before = list.len();
    list.retain(|a| match a {
        Anchor::Pair { left, right } => {
            let mine = if side == Side::Right { right } else { left };
            !overlaps(mine, &lines)
        }
        Anchor::Isolate { .. } => true,
    });
    if list.len() != before {
        view.align = None;
        set_anchors(view, list);
        return;
    }
    view.align = Some(AlignMode { side, lines });
    view.request_window(TextRequest::Message(Message::info(ALIGN_STATUS)));
    view.dirty = true;
}

/// Completes the align mode with `lines` of the other pane (a click on a line, or its selection and Enter).
pub fn finish_align_with(view: &mut TextViewModel, lines: Range<u32>) -> bool {
    let Some(mode) = view.align.take() else {
        return false;
    };
    let (left, right) = if mode.side == Side::Right {
        (lines, mode.lines)
    } else {
        (mode.lines, lines)
    };
    if left.is_empty() || right.is_empty() {
        return false;
    }
    let mut list = anchors(view);
    list.push(Anchor::Pair { left, right });
    set_anchors(view, list);
    view.request_window(TextRequest::Message(Message::info("Aligned.")));
    true
}

/// Esc in the align mode.
pub fn align_cancel(view: &mut TextViewModel) {
    if view.align.take().is_some() {
        view.dirty = true;
    }
}

/// A press in `side` at `line` during the align mode: the other pane's line completes it. True when taken.
pub fn align_click(view: &mut TextViewModel, side: Side, line: u32) -> bool {
    match &view.align {
        Some(mode) if mode.side != side => finish_align_with(view, line..line + 1),
        _ => false,
    }
}

/// Enter in the other pane during the align mode: its selected lines complete it. True when taken.
pub fn align_enter(view: &mut TextViewModel) -> bool {
    let focus = view.focus();
    match &view.align {
        Some(mode) if mode.side != focus => match focus_lines_or_say(view) {
            Some((_, lines)) => finish_align_with(view, lines),
            None => true,
        },
        _ => false,
    }
}

/// *Isolate*: `Anchor::Isolate` of the focus pane's lines, or removes it when they are isolated.
pub fn isolate(view: &mut TextViewModel) {
    if view.is_edit() || !view.loaded() {
        return;
    }
    let Some((side, lines)) = focus_lines_or_say(view) else {
        return;
    };
    let mut list = anchors(view);
    let before = list.len();
    list.retain(|a| match a {
        Anchor::Isolate { side: s, lines: l } => !(*s == anchor_side(side) && overlaps(l, &lines)),
        Anchor::Pair { .. } => true,
    });
    if list.len() == before {
        list.push(Anchor::Isolate {
            side: anchor_side(side),
            lines,
        });
    }
    set_anchors(view, list);
}

/// *Clear Alignment Overrides*.
pub fn clear_anchors(view: &mut TextViewModel) {
    view.align = None;
    set_anchors(view, Vec::new());
}

/// The mark of a row in the centre strip: `Some(true)` a pair's row, `Some(false)` an isolated row (D-310).
pub fn anchor_mark(view: &TextViewModel, row: u32) -> Option<bool> {
    let cmp = view.comparison()?;
    let r = cmp.row(RowIdx(row))?;
    let in_range = |l: Option<LineIdx>, range: &Range<u32>| l.is_some_and(|l| range.contains(&l.0));
    let list = view.live_view()?.live.anchors();
    for a in list {
        match a {
            Anchor::Pair { left, right } if in_range(r.left, left) || in_range(r.right, right) => {
                return Some(true);
            }
            Anchor::Isolate { side, lines } => {
                let line = if *side == AnchorSide::Right {
                    r.right
                } else {
                    r.left
                };
                if in_range(line, lines) {
                    return Some(false);
                }
            }
            _ => {}
        }
    }
    None
}

/// The transform of a conversion command.
pub fn transform_of(id: CommandId) -> Option<Transform> {
    Some(match id {
        CommandId::TrimTrailWS => Transform::TrimTrailing,
        CommandId::LeadSpaceToTab => Transform::LeadingSpacesToTabs,
        CommandId::TabToSpace => Transform::TabsToSpaces,
        CommandId::ToWindows => Transform::LineEndings(hikasa_text::LineEnding::CrLf),
        CommandId::ToUnix => Transform::LineEndings(hikasa_text::LineEnding::Lf),
        CommandId::ToMac => Transform::LineEndings(hikasa_text::LineEnding::Cr),
        _ => return None,
    })
}

/// *Converted N lines* (TC-063); *No line needed converting.* for none.
pub fn converted_text(n: u64) -> String {
    match n {
        0 => "No line needed converting.".to_owned(),
        1 => "Converted 1 line.".to_owned(),
        n => format!("Converted {n} lines."),
    }
}

/// A conversion of the focus pane (D-279): the lines of a selection that spans lines, each block of a line set (Find's
/// *Select All*: the lines between its blocks stay as they are, W11b review finding 5), else the file; one undo step;
/// the message panel says how many lines changed.
pub fn convert(view: &mut TextViewModel, t: Transform) -> bool {
    let side = view.focus();
    if view.doc(side).is_none() {
        return false;
    }
    let pane = view.pane(side);
    let sel = pane.selection().range();
    let blocks: Vec<Option<Range<u32>>> =
        if let Some(set) = pane.lines.as_ref().filter(|l: &&LineSet| !l.is_empty()) {
            // The last block first: a conversion never moves the lines before it.
            set.0.ranges().iter().rev().cloned().map(Some).collect()
        } else if sel.start.line != sel.end.line {
            let last = if sel.end.byte == 0 {
                sel.end.line.0
            } else {
                sel.end.line.0 + 1
            };
            vec![Some(sel.start.line.0..last)]
        } else {
            vec![None]
        };
    let origin = if blocks.len() > 1 {
        Origin::Group(view.find.next_group())
    } else {
        Origin::Single
    };
    let mut n = 0u64;
    let mut applied = true;
    for lines in blocks {
        let Some(doc) = view.doc(side) else {
            return false;
        };
        let out = transform(doc, t, lines, doc.options().tab_size);
        if let Some(req) = out.request {
            if !edit::edit(view, side, req, origin, None) {
                applied = false;
                break;
            }
            n += out.lines;
        }
    }
    if applied {
        view.request_window(TextRequest::Message(Message::info(converted_text(n))));
    }
    applied
}

/// The side a Compare Selection to Clipboard names its left text after: the focus pane.
pub fn clipboard_left(view: &TextViewModel) -> Option<(String, String)> {
    let side = view.focus();
    let doc = view.doc(side)?;
    let pane = view.pane(side);
    let text = if let Some(lines) = pane.lines.as_ref().filter(|l| !l.is_empty()) {
        hikasa_text::edit::lines_text(doc, lines)
    } else if pane.anchor != pane.caret {
        doc.text(pane.selection().range())
    } else {
        // D-310's open default: the whole document without a selection.
        doc.text(hikasa_text::edit::TextPos::ZERO..hikasa_text::edit::doc_end(doc))
    };
    let name = super::header::title(view, side);
    Some((name, text))
}
