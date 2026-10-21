//! The stable JSON DTOs of the comparison results (§4.3.2, §4.3.3), owned here because the JSON folder report and
//! `--json` are the same document (T5: JSON shapes are owned by DTOs, never by engine serde).
//!
//! Serialisation rules (§4.1): field order is declaration order; enum values are snake_case strings; maps are
//! `BTreeMap`; absent optional values are `null` with the key always present; integers only; timestamps RFC 3339
//! UTC; line numbers 1-based, row indices and byte offsets 0-based, ranges half-open `[start, end)` (line ranges
//! with 1-based starts); bit sets are arrays of snake_case names in bit order. The envelope (`schema`,
//! `hikasa_version`, `ok`, `exit_code`, `warnings`, `error`) belongs to hikasa-session.

mod folder;
mod text;

use std::ops::Range;

use hikasa_text::{CompiledFormat, DetectReason};
use serde::Serialize;

pub use folder::{
    CategoryCountsJson, ContentCountsJson, ContentJson, ContentRuleJson, CriteriaJson,
    DateCutoffJson, DateFilterJson, EntryJson, FilterRulesJson, FolderBaseJson, FolderCompareJson,
    FolderDisplayJson, FolderItemJson, FolderReportJson, FolderRulesJson, FolderStatsJson,
    HandlingJson, NameAlignmentJson, NameFiltersJson, NodeErrorJson, OtherFiltersJson,
    SideTotalsJson, SizeFilterJson, TimestampRuleJson, TzOffsetJson, UnixCriteriaJson,
    folder_compare_json,
};
pub use text::{
    AlignmentJson, ChangeCountsJson, DetectionJson, ImportanceJson, InlineJson, InlineRangeJson,
    LineJson, PatternJson, SectionJson, TextCompareJson, TextDisplayJson, TextItemJson,
    TextRulesJson, TextSideJson, TextStatsJson, text_compare_json,
};

/// Schema id of `hikasa text compare --json` (§4.3.2).
pub const TEXT_COMPARE_SCHEMA: &str = "hikasa.text_compare.v1";
/// Schema id of `hikasa folder compare --json` and of the JSON folder report (§4.3.3).
pub const FOLDER_COMPARE_SCHEMA: &str = "hikasa.folder_compare.v1";

/// A 0-based half-open range `[start, end)` (row indices, byte offsets).
pub(crate) fn range(r: &Range<u32>) -> [u32; 2] {
    [r.start, r.end]
}

/// A line range with 1-based start: `[start + 1, end + 1)` (§4.1 rule 7: `[11, 14]` means lines 11–13).
pub(crate) fn line_range(r: &Range<u32>) -> [u64; 2] {
    [u64::from(r.start) + 1, u64::from(r.end) + 1]
}

/// Why a format was chosen (FF-003), full shape: `{"reason": "mask"|"forced"|"fallback", "id": …, "mask": …}`
/// with `null` for what the reason does not carry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DetectReasonJson {
    /// `mask`, `forced` or `fallback`.
    pub reason: &'static str,
    /// The format id (`null` for `fallback`).
    pub id: Option<String>,
    /// The mask item that matched (`mask` only).
    pub mask: Option<String>,
}

impl DetectReasonJson {
    /// The DTO of a detection reason.
    pub fn new(reason: &DetectReason) -> DetectReasonJson {
        match reason {
            DetectReason::Mask { id, mask } => DetectReasonJson {
                reason: "mask",
                id: Some(id.as_str().to_owned()),
                mask: Some(mask.clone()),
            },
            DetectReason::Forced { id } => DetectReasonJson {
                reason: "forced",
                id: Some(id.as_str().to_owned()),
                mask: None,
            },
            DetectReason::Fallback => DetectReasonJson {
                reason: "fallback",
                id: None,
                mask: None,
            },
        }
    }
}

/// The effective format: `{"id", "name", "kind", "reason"}` (`id` is `null` for Everything Else; `reason` is
/// supplied by the caller that chose the format, else `null`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FormatJson {
    /// Format id (`null` for the implicit Everything Else).
    pub id: Option<String>,
    /// Display name.
    pub name: String,
    /// Kind (`text`, `table`, …).
    pub kind: &'static str,
    /// Why it was chosen.
    pub reason: Option<DetectReasonJson>,
}

impl FormatJson {
    /// The DTO of a compiled format and (when the caller knows it) its detection reason.
    pub fn new(format: &CompiledFormat, reason: Option<&DetectReason>) -> FormatJson {
        FormatJson {
            id: format.id().map(|id| id.as_str().to_owned()),
            name: format.name().to_owned(),
            kind: format.kind().as_str(),
            reason: reason.map(DetectReasonJson::new),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hikasa_text::FormatId;

    #[test]
    fn ranges() {
        assert_eq!(range(&(3..7)), [3, 7]);
        assert_eq!(line_range(&(10..13)), [11, 14]);
        assert_eq!(line_range(&(4..4)), [5, 5]);
    }

    #[test]
    fn detect_reasons_have_full_shape() {
        let id = FormatId::new("c-cpp").unwrap();
        let mask = DetectReasonJson::new(&DetectReason::Mask {
            id: id.clone(),
            mask: "*.c".into(),
        });
        assert_eq!(
            serde_json::to_string(&mask).unwrap(),
            r#"{"reason":"mask","id":"c-cpp","mask":"*.c"}"#
        );
        let forced = DetectReasonJson::new(&DetectReason::Forced { id });
        assert_eq!(
            serde_json::to_string(&forced).unwrap(),
            r#"{"reason":"forced","id":"c-cpp","mask":null}"#
        );
        assert_eq!(
            serde_json::to_string(&DetectReasonJson::new(&DetectReason::Fallback)).unwrap(),
            r#"{"reason":"fallback","id":null,"mask":null}"#
        );
        let ee = FormatJson::new(&CompiledFormat::everything_else(), None);
        assert_eq!(
            serde_json::to_string(&ee).unwrap(),
            r#"{"id":null,"name":"Everything Else","kind":"text","reason":null}"#
        );
    }
}
