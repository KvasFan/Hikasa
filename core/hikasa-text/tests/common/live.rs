//! Helpers of the live-comparison tests (A10.9–A10.12): seeded document pairs of four generators, random edit
//! scripts on `EditableDocument`s, and the fresh comparison a live one must equal.

use std::sync::Arc;

use hikasa_core::{Ctl, Pair, Side};
use hikasa_text::edit::{
    EditOptions, EditOutcome, EditRequest, Editability, EditableDocument, GroupHint, TextPos,
};
use hikasa_text::{
    CompareInput, CompiledFormat, LineEnding, LineIdx, LiveComparison, RegexLimits, TextComparison,
    TextDocument, TextRules, compare_live, compare_with_limits,
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
        },
        &RegexLimits::default(),
        None,
        &Ctl::detached(),
        None,
    )
    .unwrap()
}

/// A fresh comparison of the documents as they stand, with the live comparison's anchors.
pub fn fresh(
    setup: &Setup,
    docs: Pair<&EditableDocument>,
    anchors: &[hikasa_text::Anchor],
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
