//! `hikasa.text_compare.v1` (§4.3.2): [`TextCompareJson`] and its builder [`text_compare_json`].

use std::collections::BTreeMap;

use hikasa_core::{ContentResult, FileTime, Side};
use hikasa_text::{
    ChangeCounts, ClassifiedInline, DetectReason, EncodingSource, RowIdx, RowState, TextComparison,
    TextDisplayFilter, TextDocument, TextRules, TextStats, TextToggles, ViewItem, ViewOptions,
};
use serde::Serialize;

use super::{DetectReasonJson, FormatJson, line_range, range};

/// The `result` of `hikasa text compare --json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TextCompareJson {
    /// Left document.
    pub left: TextSideJson,
    /// Right document.
    pub right: TextSideJson,
    /// Effective format (D-15); its `reason` is set by the caller ([`TextCompareJson::with_detection`]).
    pub format: FormatJson,
    /// Per-side detection reasons (set by the caller; `null` otherwise).
    pub detection: DetectionJson,
    /// Algorithm actually used (after resolving Auto, D-36).
    pub algorithm: &'static str,
    /// Rules used, in their TOML-equivalent shape.
    pub rules: TextRulesJson,
    /// Ignore-unimportant of the view.
    pub ignore_unimportant: bool,
    /// Verdict under `ignore_unimportant` (§2.17.5).
    pub verdict: ContentResult,
    /// Statistics (§2.17.3).
    pub stats: TextStatsJson,
    /// Every section (§2.17.2).
    pub sections: Vec<SectionJson>,
    /// The display filter of `items`.
    pub display: TextDisplayJson,
    /// The view's items: visible rows and gaps of hidden rows.
    pub items: Vec<TextItemJson>,
}

impl TextCompareJson {
    /// Sets the detection reasons that only the caller choosing the format knows (§4.3.2 `format.reason` and
    /// `detection`; `TextComparison` does not carry them).
    pub fn with_detection(
        mut self,
        format: Option<&DetectReason>,
        left: Option<&DetectReason>,
        right: Option<&DetectReason>,
    ) -> TextCompareJson {
        self.format.reason = format.map(DetectReasonJson::new);
        self.detection = DetectionJson {
            left: left.map(DetectReasonJson::new),
            right: right.map(DetectReasonJson::new),
        };
        self
    }
}

/// One document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TextSideJson {
    /// Canonical spec (`null` for in-memory documents).
    pub spec: Option<String>,
    /// Source size in bytes (before conversion).
    pub size: u64,
    /// Modification time.
    pub mtime: Option<FileTime>,
    /// Number of lines.
    pub lines: u32,
    /// Encoding name ("UTF-8", "windows-1252", …).
    pub encoding: &'static str,
    /// How the encoding was chosen.
    pub encoding_source: &'static str,
    /// A byte-order mark was stripped.
    pub bom: bool,
    /// Line-ending style (`lf`, `cr_lf`, `cr`, `mixed`, `none`).
    pub eol: &'static str,
    /// Classified binary (TC-003).
    pub binary: bool,
    /// Malformed byte sequences replaced while decoding.
    pub malformed: u64,
    /// The text comes from a conversion (FF-031).
    pub converted: bool,
}

/// The per-side detection reasons.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct DetectionJson {
    /// Left side.
    pub left: Option<DetectReasonJson>,
    /// Right side.
    pub right: Option<DetectReasonJson>,
}

/// [`TextRules`] in its TOML-equivalent shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TextRulesJson {
    /// Schema version.
    pub schema_version: u32,
    /// Importance rules.
    pub importance: ImportanceJson,
    /// Alignment rules.
    pub alignment: AlignmentJson,
}

/// Importance rules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImportanceJson {
    /// Element name → important (sorted by name).
    pub elements: BTreeMap<String, bool>,
    /// Leading whitespace important.
    pub leading_whitespace: bool,
    /// Embedded whitespace important.
    pub embedded_whitespace: bool,
    /// Trailing whitespace important.
    pub trailing_whitespace: bool,
    /// Everything-else text important.
    pub everything_else: bool,
    /// Everything-else text case-sensitive.
    pub everything_else_case_sensitive: bool,
    /// Unimportant-text patterns.
    pub unimportant: Vec<PatternJson>,
    /// TC-021.
    pub orphans_always_important: bool,
    /// TC-022.
    pub compare_line_endings: bool,
}

/// An unimportant-text pattern.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PatternJson {
    /// Pattern text.
    pub pattern: String,
    /// Regular expression.
    pub regex: bool,
    /// Case-sensitive.
    pub match_case: bool,
}

/// Alignment rules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AlignmentJson {
    /// The chosen algorithm (`auto`, …; the one used is the top-level `algorithm`).
    pub algorithm: &'static str,
    /// Skew tolerance (lines).
    pub skew_tolerance: u32,
    /// Closeness matching.
    pub closeness: bool,
    /// Closeness threshold (per mille).
    pub closeness_threshold_permille: u16,
    /// Never align differences.
    pub never_align_differences: bool,
}

/// Statistics (§2.17.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TextStatsJson {
    /// Rows.
    pub rows: u64,
    /// Same rows.
    pub same_rows: u64,
    /// Changed rows.
    pub changed_rows: u64,
    /// Left-only rows.
    pub left_only_rows: u64,
    /// Right-only rows.
    pub right_only_rows: u64,
    /// Important rows.
    pub important_rows: u64,
    /// Unimportant rows.
    pub unimportant_rows: u64,
    /// Line counts of important sections (D-18).
    pub important: ChangeCountsJson,
    /// Line counts of unimportant sections.
    pub unimportant: ChangeCountsJson,
    /// Sections.
    pub sections: u64,
    /// Important sections.
    pub important_sections: u64,
    /// Left document lines.
    pub left_lines: u64,
    /// Right document lines.
    pub right_lines: u64,
}

/// Added/deleted/changed/original line counts (D-18).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ChangeCountsJson {
    /// Lines of the right side in Add sections.
    pub added: u64,
    /// Lines of the left side in Delete sections.
    pub deleted: u64,
    /// Right lines of Change sections.
    pub changed: u64,
    /// Left lines of Change sections.
    pub original: u64,
}

/// One section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SectionJson {
    /// Ordinal in row order.
    pub id: u32,
    /// `add`, `delete` or `change`.
    pub kind: &'static str,
    /// `important` or `unimportant`.
    pub importance: &'static str,
    /// Rows `[start, end)`.
    pub rows: [u32; 2],
    /// Left lines, 1-based start (`[11, 14]` = lines 11–13; empty at the insertion point).
    pub left_lines: [u64; 2],
    /// Right lines, likewise.
    pub right_lines: [u64; 2],
}

/// The display filter of the items: `mode` in the `--display` vocabulary (`all`, `mismatches`, `context`,
/// `matches`, `none`, `toggles`), `lines` for `context` (else `null`), and `toggles`: the shown categories, always
/// an array (§4.1 rule 3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TextDisplayJson {
    /// Mode.
    pub mode: &'static str,
    /// Context lines (`context` only).
    pub lines: Option<u32>,
    /// The shown categories as snake_case names in `TextToggles` bit order (`same`, `left_orphans`,
    /// `right_orphans`, `differences`): the toggle bits in `toggles` mode, else the categories the mode shows
    /// (`all`: all four; `mismatches` and `context`: the three difference categories, `context` adding the same
    /// rows around them; `matches`: `same`; `none`: none). [`TextDisplayJson::new`] always sets it, and a `None`
    /// is written as `[]`: never `null` (§4.1 rule 3; wave-4 decision).
    #[serde(serialize_with = "toggles_array")]
    pub toggles: Option<Vec<&'static str>>,
}

/// Writes `None` as `[]` (§4.1 rule 3: arrays are never `null`).
fn toggles_array<S: serde::Serializer>(
    toggles: &Option<Vec<&'static str>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    toggles.as_deref().unwrap_or(&[]).serialize(serializer)
}

/// The names of the set bits of `t` in bit order.
fn text_toggle_names(t: TextToggles) -> Vec<&'static str> {
    [
        (TextToggles::SAME, "same"),
        (TextToggles::LEFT_ORPHANS, "left_orphans"),
        (TextToggles::RIGHT_ORPHANS, "right_orphans"),
        (TextToggles::DIFFERENCES, "differences"),
    ]
    .iter()
    .filter(|(b, _)| t.contains(*b))
    .map(|(_, n)| *n)
    .collect()
}

impl TextDisplayJson {
    /// The DTO of a display filter.
    pub fn new(filter: TextDisplayFilter) -> TextDisplayJson {
        let differences =
            TextToggles::LEFT_ORPHANS | TextToggles::RIGHT_ORPHANS | TextToggles::DIFFERENCES;
        let (mode, lines, shown) = match filter {
            TextDisplayFilter::All => ("all", None, TextToggles::all()),
            TextDisplayFilter::Differences => ("mismatches", None, differences),
            TextDisplayFilter::Same => ("matches", None, TextToggles::SAME),
            TextDisplayFilter::None => ("none", None, TextToggles::empty()),
            TextDisplayFilter::Context { lines } => ("context", Some(lines), differences),
            TextDisplayFilter::Toggles { bits } => {
                ("toggles", None, TextToggles::from_bits_truncate(bits))
            }
        };
        TextDisplayJson {
            mode,
            lines,
            toggles: Some(text_toggle_names(shown)),
        }
    }
}

/// One item of the view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TextItemJson {
    /// Hidden rows.
    Gap {
        /// Rows `[start, end)`.
        rows: [u32; 2],
    },
    /// A visible row.
    Row {
        /// Row index (0-based).
        row: u32,
        /// `same`, `changed`, `left_only`, `right_only`.
        state: &'static str,
        /// `none`, `unimportant`, `important`.
        importance: &'static str,
        /// The left line (`null` for a right-only row).
        left: Option<LineJson>,
        /// The right line (`null` for a left-only row).
        right: Option<LineJson>,
        /// Classified inline differences of a two-sided non-same row, else `null`.
        inline: Option<InlineJson>,
    },
}

/// One line of a row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LineJson {
    /// 1-based line number.
    pub line: u64,
    /// Text without its terminator.
    pub text: String,
}

/// Classified inline differences (TC-014).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InlineJson {
    /// Left ranges.
    pub left: Vec<InlineRangeJson>,
    /// Right ranges.
    pub right: Vec<InlineRangeJson>,
    /// The line exceeded the inline limit (the ranges cover the whole differing middle).
    pub whole_line: bool,
}

/// One inline range: byte offsets `[start, end)` in the line text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct InlineRangeJson {
    /// First byte.
    pub start: u32,
    /// End byte (exclusive).
    pub end: u32,
    /// Important (red) or unimportant (blue).
    pub important: bool,
}

/// The snake_case name of an encoding source.
fn encoding_source(s: EncodingSource) -> &'static str {
    match s {
        EncodingSource::Override => "override",
        EncodingSource::Bom => "bom",
        EncodingSource::Fixed => "fixed",
        EncodingSource::UtfPattern => "utf_pattern",
        EncodingSource::FormatFallback => "format_fallback",
        EncodingSource::LocaleFallback => "locale_fallback",
        EncodingSource::Ascii => "ascii",
        EncodingSource::Utf8Valid => "utf8_valid",
        EncodingSource::Utf8Lossy => "utf8_lossy",
        EncodingSource::Detector => "detector",
    }
}

impl TextSideJson {
    /// The DTO of a document.
    pub fn new(doc: &TextDocument) -> TextSideJson {
        let enc = doc.encoding();
        TextSideJson {
            spec: doc.origin().spec.as_ref().map(|s| s.canonical()),
            size: doc.origin().size,
            mtime: doc.origin().mtime,
            lines: doc.line_count(),
            encoding: enc.encoding.name(),
            encoding_source: encoding_source(enc.source),
            bom: enc.bom_len > 0,
            eol: doc.eol().style().as_str(),
            binary: doc.binary().is_binary,
            malformed: doc.malformed(),
            converted: doc.converted(),
        }
    }
}

impl TextRulesJson {
    /// The DTO of text rules.
    pub fn new(rules: &TextRules) -> TextRulesJson {
        let i = &rules.importance;
        let a = &rules.alignment;
        TextRulesJson {
            schema_version: rules.schema_version,
            importance: ImportanceJson {
                elements: i.elements.clone(),
                leading_whitespace: i.leading_whitespace,
                embedded_whitespace: i.embedded_whitespace,
                trailing_whitespace: i.trailing_whitespace,
                everything_else: i.everything_else,
                everything_else_case_sensitive: i.everything_else_case_sensitive,
                unimportant: i
                    .unimportant
                    .iter()
                    .map(|p| PatternJson {
                        pattern: p.pattern.clone(),
                        regex: p.regex,
                        match_case: p.match_case,
                    })
                    .collect(),
                orphans_always_important: i.orphans_always_important,
                compare_line_endings: i.compare_line_endings,
            },
            alignment: AlignmentJson {
                algorithm: a.algorithm.as_str(),
                skew_tolerance: a.skew_tolerance,
                closeness: a.closeness,
                closeness_threshold_permille: a.closeness_threshold_permille,
                never_align_differences: a.never_align_differences,
            },
        }
    }
}

impl From<&ChangeCounts> for ChangeCountsJson {
    fn from(c: &ChangeCounts) -> ChangeCountsJson {
        ChangeCountsJson {
            added: c.added,
            deleted: c.deleted,
            changed: c.changed,
            original: c.original,
        }
    }
}

impl From<&TextStats> for TextStatsJson {
    fn from(s: &TextStats) -> TextStatsJson {
        TextStatsJson {
            rows: s.rows,
            same_rows: s.same_rows,
            changed_rows: s.changed_rows,
            left_only_rows: s.left_only_rows,
            right_only_rows: s.right_only_rows,
            important_rows: s.important_rows,
            unimportant_rows: s.unimportant_rows,
            important: (&s.important).into(),
            unimportant: (&s.unimportant).into(),
            sections: s.sections,
            important_sections: s.important_sections,
            left_lines: s.left_lines,
            right_lines: s.right_lines,
        }
    }
}

impl From<&ClassifiedInline> for InlineJson {
    fn from(c: &ClassifiedInline) -> InlineJson {
        let ranges = |v: &[hikasa_text::InlineRange]| {
            v.iter()
                .map(|r| InlineRangeJson {
                    start: r.range.start,
                    end: r.range.end,
                    important: r.important,
                })
                .collect()
        };
        InlineJson {
            left: ranges(&c.left),
            right: ranges(&c.right),
            whole_line: c.whole_line,
        }
    }
}

/// Builds the text JSON DTO (§4.3.2): every section, and the items of `cmp.view(view)`. `format.reason` and
/// `detection` are `null`: the caller that chose the format sets them with [`TextCompareJson::with_detection`].
pub fn text_compare_json(cmp: &TextComparison, view: &ViewOptions) -> TextCompareJson {
    let docs = [cmp.document(Side::Left), cmp.document(Side::Right)];
    let items = cmp
        .view(view)
        .iter()
        .filter_map(|item| match item {
            ViewItem::Gap { rows } => Some(TextItemJson::Gap { rows: range(&rows) }),
            ViewItem::Row { row: idx } => {
                let row = cmp.row(idx)?;
                let line = |side: usize, l: Option<hikasa_text::LineIdx>| {
                    l.map(|l| LineJson {
                        line: u64::from(l.0) + 1,
                        text: docs[side].line(l).into_owned(),
                    })
                };
                let inline = if row.state == RowState::Same {
                    None
                } else {
                    cmp.inline(RowIdx(idx.0)).as_ref().map(InlineJson::from)
                };
                Some(TextItemJson::Row {
                    row: idx.0,
                    state: row.state.as_str(),
                    importance: row.importance.as_str(),
                    left: line(0, row.left),
                    right: line(1, row.right),
                    inline,
                })
            }
        })
        .collect();
    TextCompareJson {
        left: TextSideJson::new(docs[0]),
        right: TextSideJson::new(docs[1]),
        format: FormatJson::new(cmp.format(), None),
        detection: DetectionJson::default(),
        algorithm: cmp.algorithm().as_str(),
        rules: TextRulesJson::new(cmp.rules()),
        ignore_unimportant: view.ignore_unimportant,
        verdict: cmp.verdict(view.ignore_unimportant),
        stats: cmp.stats().into(),
        sections: cmp
            .sections()
            .iter()
            .map(|s| SectionJson {
                id: s.id.0,
                kind: s.kind.as_str(),
                importance: s.importance.as_str(),
                rows: range(&s.rows),
                left_lines: line_range(&s.left_lines),
                right_lines: line_range(&s.right_lines),
            })
            .collect(),
        display: TextDisplayJson::new(view.filter),
        items,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_shapes() {
        let json = |f| serde_json::to_string(&TextDisplayJson::new(f)).unwrap();
        // `toggles` is always an array (§4.1 rule 3: arrays are never null).
        assert_eq!(
            json(TextDisplayFilter::All),
            r#"{"mode":"all","lines":null,"toggles":["same","left_orphans","right_orphans","differences"]}"#
        );
        assert_eq!(
            json(TextDisplayFilter::Differences),
            r#"{"mode":"mismatches","lines":null,"toggles":["left_orphans","right_orphans","differences"]}"#
        );
        assert_eq!(
            json(TextDisplayFilter::Context { lines: 2 }),
            r#"{"mode":"context","lines":2,"toggles":["left_orphans","right_orphans","differences"]}"#
        );
        assert_eq!(
            json(TextDisplayFilter::Toggles { bits: 1 | 8 }),
            r#"{"mode":"toggles","lines":null,"toggles":["same","differences"]}"#
        );
        assert_eq!(
            json(TextDisplayFilter::Same),
            r#"{"mode":"matches","lines":null,"toggles":["same"]}"#
        );
        assert_eq!(
            json(TextDisplayFilter::None),
            r#"{"mode":"none","lines":null,"toggles":[]}"#
        );
        // A DTO built by hand with `None` is still written as an array.
        let hand = TextDisplayJson {
            mode: "all",
            lines: None,
            toggles: None,
        };
        assert_eq!(
            serde_json::to_string(&hand).unwrap(),
            r#"{"mode":"all","lines":null,"toggles":[]}"#
        );
    }

    #[test]
    fn item_shapes() {
        let gap = TextItemJson::Gap { rows: [0, 10] };
        assert_eq!(
            serde_json::to_string(&gap).unwrap(),
            r#"{"type":"gap","rows":[0,10]}"#
        );
        let row = TextItemJson::Row {
            row: 13,
            state: "right_only",
            importance: "important",
            left: None,
            right: Some(LineJson {
                line: 14,
                text: "return y;".into(),
            }),
            inline: None,
        };
        assert_eq!(
            serde_json::to_string(&row).unwrap(),
            r#"{"type":"row","row":13,"state":"right_only","importance":"important","left":null,"right":{"line":14,"text":"return y;"},"inline":null}"#
        );
        assert_eq!(encoding_source(EncodingSource::Utf8Valid), "utf8_valid");
        assert_eq!(encoding_source(EncodingSource::UtfPattern), "utf_pattern");
    }

    #[test]
    fn encoding_sources_match_serde() {
        for s in [
            EncodingSource::Override,
            EncodingSource::Bom,
            EncodingSource::Fixed,
            EncodingSource::UtfPattern,
            EncodingSource::FormatFallback,
            EncodingSource::LocaleFallback,
            EncodingSource::Ascii,
            EncodingSource::Utf8Valid,
            EncodingSource::Utf8Lossy,
            EncodingSource::Detector,
        ] {
            assert_eq!(
                serde_json::to_string(&s).unwrap(),
                format!("\"{}\"", encoding_source(s))
            );
        }
    }
}
