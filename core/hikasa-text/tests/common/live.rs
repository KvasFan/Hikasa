//! Helpers of the live-comparison tests (A10.9–A10.12): seeded document pairs of four generators, random edit
//! scripts on `EditableDocument`s, and the fresh comparison a live one must equal.

use std::sync::Arc;

use hikasa_core::{Ctl, Pair, Side};
use hikasa_text::edit::{
    EditOptions, EditOutcome, EditRequest, Editability, EditableDocument, GroupHint, LineDelta,
    TextPos,
};
use hikasa_text::{
    CompareInput, CompiledFormat, IgnoredLines, LineEnding, LineIdx, LiveComparison, RegexLimits,
    TextComparison, TextDocument, TextRules, compare_live, compare_with_limits,
};

use super::XorShift;

/// The document generators of A10.9.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gen {
    /// G1-like: source lines, the right side an edited copy with clustered changes.
    Clustered,
    /// Few distinct lines, repeated (non-unique keys everywhere).
    Repetitive,
    /// C source with block comments and multi-line strings.
    Source,
    /// Mixed terminators and long lines (CPL-wrapped through the edit options).
    Mixed,
}

pub const GENS: [Gen; 4] = [Gen::Clustered, Gen::Repetitive, Gen::Source, Gen::Mixed];

fn source_line(rng: &mut XorShift, k: u64) -> String {
    match rng.below(12) {
        0 => "/* start".to_owned(),
        1 => "   comment */".to_owned(),
        2 => format!("    s = \"x{}\\", rng.below(9)),
        3 => "}".to_owned(),
        4 => String::new(),
        5 => format!("int f{k}(void) {{"),
        _ => format!(
            "    v{} = v{} + {};",
            rng.below(50),
            rng.below(50),
            rng.below(1000)
        ),
    }
}

/// A text of `n` lines from `kind`.
pub fn text(rng: &mut XorShift, kind: Gen, n: u64) -> String {
    let mut out = String::new();
    for k in 0..n {
        let line = match kind {
            Gen::Clustered => format!("line {k} value {}", k * 7 % 13),
            Gen::Repetitive => ["a", "b", "", "{", "}"][rng.below(5) as usize].to_owned(),
            Gen::Source => source_line(rng, k),
            Gen::Mixed => {
                let len = rng.below(30);
                (0..len)
                    .map(|i| (b'a' + ((k + i) % 7) as u8) as char)
                    .collect()
            }
        };
        out.push_str(&line);
        let eol = if kind == Gen::Mixed {
            ["\n", "\r\n", "\r"][rng.below(3) as usize]
        } else {
            "\n"
        };
        out.push_str(eol);
    }
    out
}

/// The right side of a pair: a copy of `left` with clustered changes.
pub fn edited_copy(rng: &mut XorShift, kind: Gen, left: &str) -> String {
    let mut lines: Vec<String> = left.lines().map(str::to_owned).collect();
    let clusters = 1 + rng.below(1 + lines.len() as u64 / 40);
    for _ in 0..clusters {
        if lines.is_empty() {
            break;
        }
        let at = rng.below(lines.len() as u64) as usize;
        match rng.below(3) {
            0 => {
                let n = 1 + rng.below(4) as usize;
                for _ in 0..n.min(lines.len() - at) {
                    lines.remove(at);
                }
            }
            1 => {
                for k in 0..1 + rng.below(4) {
                    lines.insert(
                        at,
                        text(rng, kind, 1).trim_end().to_owned() + &format!(" new{k}"),
                    );
                }
            }
            _ => lines[at].push_str(" changed"),
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// A document pair of `kind` with about `n` lines.
pub fn pair(rng: &mut XorShift, kind: Gen, n: u64) -> (String, String) {
    let left = text(rng, kind, n);
    let right = edited_copy(rng, kind, &left);
    (left, right)
}

/// The editor options of `kind` (a CPL for `Mixed`).
pub fn edit_options(kind: Gen) -> EditOptions {
    EditOptions {
        chars_per_line_limit: (kind == Gen::Mixed).then_some(12),
        ..EditOptions::default()
    }
}

/// Editable documents of a pair (CPL-wrapped through an edit of everything when the options have a limit).
pub fn editable(text: &str, opts: &EditOptions) -> EditableDocument {
    let mut doc = EditableDocument::new(
        Arc::new(TextDocument::from_text(text)),
        opts.clone(),
        Editability::EDITABLE,
    );
    if opts.chars_per_line_limit.is_some() {
        let end = doc.clamp(TextPos::new(u32::MAX, u32::MAX));
        let all = doc.text(TextPos::ZERO..end);
        doc.apply(
            &EditRequest::Replace {
                range: TextPos::ZERO..end,
                text: all,
                keep_eols: true,
            },
            GroupHint::None,
        )
        .unwrap();
        doc.clear_undo();
    }
    doc
}

/// A random valid position.
pub fn position(rng: &mut XorShift, doc: &EditableDocument) -> TextPos {
    let line = rng.below(u64::from(doc.position_lines())) as u32;
    let text = doc.line(LineIdx(line)).into_owned();
    let mut byte = rng.below(text.len() as u64 + 1) as usize;
    while !text.is_char_boundary(byte) {
        byte -= 1;
    }
    TextPos::new(line, byte as u32)
}

/// One random edit (or undo/redo) of `doc`; `None` when nothing applied.
pub fn random_edit(
    rng: &mut XorShift,
    kind: Gen,
    doc: &mut EditableDocument,
) -> Option<EditOutcome> {
    match rng.below(14) {
        0 => doc.undo(),
        1 => doc.redo(),
        2 if doc.line_count() > 0 => {
            let n = doc.line_count();
            let s = rng.below(u64::from(n)) as u32;
            let e = (s + 1 + rng.below(4) as u32).min(n);
            let eol = [LineEnding::Lf, LineEnding::CrLf, LineEnding::Cr][rng.below(3) as usize];
            doc.apply(&EditRequest::SetEols { lines: s..e, eol }, GroupHint::None)
                .ok()
        }
        _ => {
            let a = position(rng, doc);
            let b = if rng.below(3) == 0 {
                position(rng, doc)
            } else {
                // Small edits mostly: within the line or a few lines on.
                let line = (a.line.0 + rng.below(2) as u32).min(doc.position_lines() - 1);
                let len = doc.line_len(LineIdx(line));
                let mut b = TextPos::new(line, rng.below(u64::from(len) + 1) as u32);
                b = doc.clamp(b);
                b
            };
            let t = match rng.below(6) {
                0 => String::new(),
                1 => "\n".to_owned(),
                2 => {
                    let n = 1 + rng.below(3);
                    text(rng, kind, n)
                }
                3 => "x".to_owned(),
                4 => " changed".to_owned(),
                _ => text(rng, kind, 1).trim_end().to_owned(),
            };
            doc.apply(
                &EditRequest::Replace {
                    range: a..b,
                    text: t,
                    keep_eols: rng.below(2) == 0,
                },
                GroupHint::None,
            )
            .ok()
        }
    }
}

/// A user format whose first line is a heading item whatever it holds (FF-024 `or_line_one`, element `Head`),
/// followed by `next_lines` lines: the key of line 0 depends on its being line 0.
pub fn line_one_format(next_lines: u32) -> Arc<CompiledFormat> {
    use hikasa_formats::{ElementName, FileFormat, FormatId, FormatKind, GrammarItem, ItemKind};
    let mut f = FileFormat::new(FormatId::new("hdr").unwrap(), "Hdr", FormatKind::Text);
    f.grammar.push(GrammarItem {
        element: ElementName::new("Head").unwrap(),
        kind: ItemKind::Lines {
            heading: String::new(),
            regex: false,
            match_case: true,
            next_lines,
            or_line_one: true,
        },
    });
    Arc::new(CompiledFormat::compile(Arc::new(f), &RegexLimits::default()).unwrap())
}

/// The settings of a live test.
#[derive(Debug, Clone)]
pub struct Setup {
    pub format: Arc<CompiledFormat>,
    pub rules: TextRules,
    pub force_tokenize: bool,
    pub anchors: Vec<hikasa_text::Anchor>,
    /// The right side's own format (D-281).
    pub right_format: Option<Arc<CompiledFormat>>,
    /// The ignored lines at the start (§1.10.17).
    pub ignored: IgnoredLines,
}

/// The live comparison of two editable documents.
pub fn live(setup: &Setup, docs: Pair<&EditableDocument>) -> LiveComparison {
    compare_live(
        CompareInput {
            left: docs.left.snapshot(),
            right: docs.right.snapshot(),
            format: setup.format.clone(),
            rules: setup.rules.clone(),
            bytes_identical: false,
            force_tokenize: setup.force_tokenize,
            anchors: setup.anchors.clone(),
            right_format: setup.right_format.clone(),
            ignored: setup.ignored.clone(),
        },
        &RegexLimits::default(),
        None,
        &Ctl::detached(),
        None,
    )
    .unwrap()
}

/// A fresh comparison of the documents as they stand, with the live comparison's anchors (and the setup's
/// ignored lines).
pub fn fresh(
    setup: &Setup,
    docs: Pair<&EditableDocument>,
    anchors: &[hikasa_text::Anchor],
) -> TextComparison {
    fresh_ignored(setup, docs, anchors, &setup.ignored)
}

/// [`fresh`] with the given ignored lines (the live comparison's, moved by the edits).
pub fn fresh_ignored(
    setup: &Setup,
    docs: Pair<&EditableDocument>,
    anchors: &[hikasa_text::Anchor],
    ignored: &IgnoredLines,
) -> TextComparison {
    compare_with_limits(
        CompareInput {
            left: docs.left.snapshot(),
            right: docs.right.snapshot(),
            format: setup.format.clone(),
            rules: setup.rules.clone(),
            bytes_identical: false,
            force_tokenize: setup.force_tokenize,
            anchors: anchors.to_vec(),
            right_format: setup.right_format.clone(),
            ignored: ignored.clone(),
        },
        &RegexLimits::default(),
        None,
        &Ctl::detached(),
        None,
    )
    .unwrap()
}

/// A random side.
pub fn side(rng: &mut XorShift) -> Side {
    if rng.below(2) == 0 {
        Side::Left
    } else {
        Side::Right
    }
}

/// An oracle of the anchors of a live comparison independent of its own bookkeeping (`move_anchors`): every
/// line of each side has an identity (a delta's first `min(removed, inserted)` lines keep theirs, the other new
/// lines get fresh ones); an anchor boundary is the identity of the line at it (or the end), and a boundary
/// whose line is removed moves to the first line after the change; an anchor whose range becomes empty is
/// dropped.
#[derive(Debug, Clone)]
pub struct AnchorModel {
    ids: [Vec<u64>; 2],
    next: u64,
    /// Per anchor: kind, and per side the boundaries `(start, end)` (`None` = the end of the document).
    anchors: Vec<ModelAnchor>,
}

#[derive(Debug, Clone)]
enum ModelAnchor {
    Pair([(Option<u64>, Option<u64>); 2]),
    Isolate(usize, (Option<u64>, Option<u64>)),
}

impl AnchorModel {
    /// The model of `anchors` over documents of `lines` lines a side.
    pub fn new(anchors: &[hikasa_text::Anchor], lines: [u32; 2]) -> AnchorModel {
        let ids = [
            (0..u64::from(lines[0])).collect::<Vec<u64>>(),
            (1_000_000_000..1_000_000_000 + u64::from(lines[1])).collect(),
        ];
        let at = |ids: &[u64], x: u32| ids.get(x as usize).copied();
        let anchors = anchors
            .iter()
            .map(|a| match a {
                hikasa_text::Anchor::Pair { left, right } => ModelAnchor::Pair([
                    (at(&ids[0], left.start), at(&ids[0], left.end)),
                    (at(&ids[1], right.start), at(&ids[1], right.end)),
                ]),
                hikasa_text::Anchor::Isolate { side, lines } => {
                    let k = usize::from(*side == hikasa_text::AnchorSide::Right);
                    ModelAnchor::Isolate(k, (at(&ids[k], lines.start), at(&ids[k], lines.end)))
                }
            })
            .collect();
        AnchorModel {
            ids,
            next: 2_000_000_000,
            anchors,
        }
    }

    /// Applies one change of `side`.
    pub fn apply(&mut self, side: Side, d: LineDelta) {
        if d.is_empty() {
            return;
        }
        let k = usize::from(side == Side::Right);
        let ids = &mut self.ids[k];
        let (at, removed, inserted) = (d.at as usize, d.removed as usize, d.inserted as usize);
        let kept = removed.min(inserted);
        let gone: std::collections::HashSet<u64> =
            ids[at + kept..at + removed].iter().copied().collect();
        let after = ids.get(at + removed).copied();
        let fresh: Vec<u64> = (0..inserted - kept)
            .map(|_| {
                self.next += 1;
                self.next
            })
            .collect();
        ids.splice(at + kept..at + removed, fresh);
        let moved = |b: &mut Option<u64>| {
            if b.is_some_and(|id| gone.contains(&id)) {
                *b = after;
            }
        };
        for a in &mut self.anchors {
            match a {
                ModelAnchor::Pair(sides) => {
                    moved(&mut sides[k].0);
                    moved(&mut sides[k].1);
                }
                ModelAnchor::Isolate(s, (b0, b1)) if *s == k => {
                    moved(b0);
                    moved(b1);
                }
                ModelAnchor::Isolate(..) => {}
            }
        }
        let ids = self.ids.clone();
        let pos = |k: usize, b: Option<u64>| -> u32 {
            b.map_or(ids[k].len(), |id| {
                ids[k].iter().position(|&x| x == id).unwrap()
            }) as u32
        };
        self.anchors.retain(|a| match a {
            ModelAnchor::Pair(sides) => (0..2).all(|k| pos(k, sides[k].0) < pos(k, sides[k].1)),
            ModelAnchor::Isolate(k, (b0, b1)) => pos(*k, *b0) < pos(*k, *b1),
        });
    }

    /// The anchors in the current numbering.
    pub fn anchors(&self) -> Vec<hikasa_text::Anchor> {
        let pos = |k: usize, b: Option<u64>| -> u32 {
            b.map_or(self.ids[k].len(), |id| {
                self.ids[k].iter().position(|&x| x == id).unwrap()
            }) as u32
        };
        self.anchors
            .iter()
            .map(|a| match a {
                ModelAnchor::Pair(sides) => hikasa_text::Anchor::Pair {
                    left: pos(0, sides[0].0)..pos(0, sides[0].1),
                    right: pos(1, sides[1].0)..pos(1, sides[1].1),
                },
                ModelAnchor::Isolate(k, (b0, b1)) => hikasa_text::Anchor::Isolate {
                    side: if *k == 1 {
                        hikasa_text::AnchorSide::Right
                    } else {
                        hikasa_text::AnchorSide::Left
                    },
                    lines: pos(*k, *b0)..pos(*k, *b1),
                },
            })
            .collect()
    }
}

/// An oracle of the ignored lines of a live comparison (§1.10.17) independent of `RangeSet::move_through`: one
/// mark per line and side. A change keeps the marks of its first `min(removed, inserted)` lines, drops those of
/// its other removed lines, and gives its other inserted lines the mark of its last kept line when it removed
/// some, else (a pure insertion) a mark only when the lines on both sides of the insertion point are marked.
#[derive(Debug, Clone)]
pub struct IgnoredModel {
    marks: [Vec<bool>; 2],
}

impl IgnoredModel {
    /// The model of `ignored` over documents of `lines` lines a side.
    pub fn new(ignored: &IgnoredLines, lines: [u32; 2]) -> IgnoredModel {
        let side = |set: &hikasa_text::RangeSet, n: u32| (0..n).map(|l| set.contains(l)).collect();
        IgnoredModel {
            marks: [
                side(&ignored.left, lines[0]),
                side(&ignored.right, lines[1]),
            ],
        }
    }

    /// Applies one change of `side`.
    pub fn apply(&mut self, side: Side, d: LineDelta) {
        if d.is_empty() {
            return;
        }
        let marks = &mut self.marks[usize::from(side == Side::Right)];
        let (at, removed, inserted) = (d.at as usize, d.removed as usize, d.inserted as usize);
        let kept = removed.min(inserted);
        let fill = if inserted <= removed {
            false
        } else if removed > 0 {
            marks[at + removed - 1]
        } else {
            at > 0 && marks[at - 1] && marks.get(at).copied().unwrap_or(false)
        };
        marks.splice(
            at + kept..at + removed,
            std::iter::repeat_n(fill, inserted - kept),
        );
    }

    /// The *Ignored* command on `lines` of `side`: unmarks them when all are marked, else marks them.
    pub fn toggle(&mut self, side: Side, lines: std::ops::Range<u32>) {
        let marks = &mut self.marks[usize::from(side == Side::Right)];
        let r = lines.start as usize..(lines.end as usize).min(marks.len());
        if r.is_empty() {
            return;
        }
        let all = marks[r.clone()].iter().all(|&m| m);
        for m in &mut marks[r] {
            *m = !all;
        }
    }

    /// The marks as `IgnoredLines`.
    pub fn ignored(&self) -> IgnoredLines {
        let set = |marks: &[bool]| {
            hikasa_text::RangeSet::from_ranges(
                marks
                    .iter()
                    .enumerate()
                    .filter(|(_, m)| **m)
                    .map(|(l, _)| l as u32..l as u32 + 1)
                    .collect(),
            )
        };
        IgnoredLines {
            left: set(&self.marks[0]),
            right: set(&self.marks[1]),
        }
    }
}

/// Random ignored lines: up to three ranges a side of documents of `lines` lines.
pub fn random_ignored(rng: &mut XorShift, lines: [u32; 2]) -> IgnoredLines {
    let side = |rng: &mut XorShift, n: u32| {
        let mut set = hikasa_text::RangeSet::new();
        if n == 0 {
            return set;
        }
        for _ in 0..rng.below(4) {
            let s = rng.below(u64::from(n)) as u32;
            set.insert(s..(s + 1 + rng.below(6) as u32).min(n));
        }
        set
    };
    IgnoredLines {
        left: side(rng, lines[0]),
        right: side(rng, lines[1]),
    }
}
