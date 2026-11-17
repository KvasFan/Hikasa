//! `hikasa.hex_compare.v1`, `hikasa.hex_dump.v1` and `hikasa.hex_find.v1` (§1.12.24, D-347): built from the hex
//! model as `hikasa.table_compare.v1` is from the table one (T-20). Offsets and counts are JSON numbers; hex text is
//! lower case, two digits a byte, no separator; 64-bit integers of the value panel are decimal strings; arrays are
//! never `null`.

use hikasa_core::{ContentResult, ErrorReport, Side, Warning};
use hikasa_hex::{HexLayout, Interpreted, Section, Values};
use serde::Serialize;

use super::FormatJson;
use crate::error::ReportError;
use crate::hex::{HexSide, HexSource, hex_text, input_error, preview_bytes};

/// One side: `{spec, size, mtime, format, converted, load}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HexSideJson {
    /// The canonical spec.
    pub spec: String,
    /// Size in bytes (of the bytes compared: the converted output when converted).
    pub size: u64,
    /// Modification time, RFC 3339 UTC.
    pub mtime: Option<String>,
    /// The Hikasa format of the Hex kind, `null` when none matched.
    pub format: Option<FormatJson>,
    /// Read through the format's load conversion.
    pub converted: bool,
    /// `memory`, `file`, `temp_cache`.
    pub load: &'static str,
}

impl HexSideJson {
    /// The DTO of a side.
    pub fn new(s: &HexSide) -> HexSideJson {
        HexSideJson {
            spec: s.spec.clone(),
            size: s.size,
            mtime: s.mtime.map(|t| t.to_string()),
            format: s.format.clone(),
            converted: s.converted,
            load: s.load.as_str(),
        }
    }
}

/// The alignment: `{mode, requested, block, skew, window, budget_mib, degraded}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HexAlignmentJson {
    /// The mode of the result.
    pub mode: &'static str,
    /// The mode asked for.
    pub requested: &'static str,
    /// Fast's average block in bytes.
    pub block: u32,
    /// The skew limit in bytes.
    pub skew: u64,
    /// Complete's window in bytes.
    pub window: u32,
    /// Complete's budget in MiB.
    pub budget_mib: u64,
    /// Complete ran out of its budget.
    pub degraded: bool,
}

/// Statistics: `{left_bytes, right_bytes, same_bytes, differing_bytes, left_only_bytes, right_only_bytes, sections}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HexStatsJson {
    /// Left length.
    pub left_bytes: u64,
    /// Right length.
    pub right_bytes: u64,
    /// Paired equal bytes.
    pub same_bytes: u64,
    /// Paired differing bytes.
    pub differing_bytes: u64,
    /// Left bytes without a partner.
    pub left_only_bytes: u64,
    /// Right bytes without a partner.
    pub right_only_bytes: u64,
    /// Every section, stored or not.
    pub sections: u64,
}

/// A byte range of a side: `{offset, len}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct HexRangeJson {
    /// First byte.
    pub offset: u64,
    /// Bytes.
    pub len: u64,
}

/// The preview bytes of a section: `{left, right}` as hex text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HexPreviewJson {
    /// Up to `preview` left bytes.
    pub left: String,
    /// Up to `preview` right bytes.
    pub right: String,
}

/// One section: `{index, kind, left, right, columns, preview}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HexSectionJson {
    /// 0-based index.
    pub index: u64,
    /// `changed`, `left_only`, `right_only`, `mixed`.
    pub kind: &'static str,
    /// Its left bytes (`null`: none).
    pub left: Option<HexRangeJson>,
    /// Its right bytes.
    pub right: Option<HexRangeJson>,
    /// Aligned columns it spans.
    pub columns: u64,
    /// The first bytes of each side.
    pub preview: HexPreviewJson,
}

/// `hikasa.hex_compare.v1` (§1.12.24).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HexCompareJson {
    /// Left side.
    pub left: HexSideJson,
    /// Right side.
    pub right: HexSideJson,
    /// The alignment.
    pub alignment: HexAlignmentJson,
    /// The verdict (`{"result": …}`).
    pub verdict: ContentResult,
    /// An input changed during the job.
    pub stale: bool,
    /// Statistics.
    pub stats: HexStatsJson,
    /// The listed sections.
    pub sections: Vec<HexSectionJson>,
    /// Sections listed.
    pub sections_shown: u64,
    /// The engine's section list was cut at `max_sections` (the statistics stay exact).
    pub sections_truncated: bool,
    /// The comparison's warnings.
    pub warnings: Vec<Warning>,
}

fn range_json(r: &std::ops::Range<u64>) -> Option<HexRangeJson> {
    (!r.is_empty()).then(|| HexRangeJson {
        offset: r.start,
        len: r.end - r.start,
    })
}

/// The DTO of one section.
fn section_json(
    src: &HexSource<'_>,
    index: u64,
    s: &Section,
) -> Result<HexSectionJson, ReportError> {
    let preview = |doc, r: &std::ops::Range<u64>| {
        preview_bytes(doc, r, src.preview)
            .map(|b| hex_text(&b))
            .map_err(|e| input_error(&e))
    };
    Ok(HexSectionJson {
        index,
        kind: s.kind.as_str(),
        left: range_json(&s.left),
        right: range_json(&s.right),
        columns: s.columns.end - s.columns.start,
        preview: HexPreviewJson {
            left: preview(src.documents.left, &s.left)?,
            right: preview(src.documents.right, &s.right)?,
        },
    })
}

/// Builds `hikasa.hex_compare.v1`: the first `src.max_sections` sections (all with 0) with `src.preview` bytes a side.
///
/// # Errors
///
/// [`ReportError::Input`] when a preview cannot be read.
pub fn hex_compare_json(src: &HexSource<'_>) -> Result<HexCompareJson, ReportError> {
    let a = src.alignment;
    let o = src.options;
    let s = &a.stats;
    let sections = src
        .listed_sections()
        .enumerate()
        .map(|(i, section)| section_json(src, i as u64, &section))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(HexCompareJson {
        left: HexSideJson::new(src.side(Side::Left)),
        right: HexSideJson::new(src.side(Side::Right)),
        alignment: HexAlignmentJson {
            mode: a.mode.as_str(),
            requested: o.mode.as_str(),
            block: o.block,
            skew: o.skew,
            window: o.window,
            budget_mib: o.budget_mib,
            degraded: a.degraded,
        },
        verdict: src.verdict(),
        stale: src.stale,
        stats: HexStatsJson {
            left_bytes: s.left_bytes,
            right_bytes: s.right_bytes,
            same_bytes: s.same_bytes,
            differing_bytes: s.differing_bytes,
            left_only_bytes: s.left_only_bytes,
            right_only_bytes: s.right_only_bytes,
            sections: s.sections,
        },
        sections_shown: sections.len() as u64,
        sections,
        sections_truncated: a.sections.truncated(),
        warnings: src.warnings.to_vec(),
    })
}

/// The byte layout: `{bytes_per_line, group, chars, address}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HexLayoutJson {
    /// Bytes per line.
    pub bytes_per_line: u16,
    /// Digit grouping.
    pub group: u8,
    /// `ascii`, `latin1`, `none`.
    pub chars: &'static str,
    /// `hex`, `decimal`.
    pub address: &'static str,
}

impl HexLayoutJson {
    /// The DTO of a layout.
    pub fn new(l: &HexLayout) -> HexLayoutJson {
        HexLayoutJson {
            bytes_per_line: l.bytes_per_line,
            group: l.group,
            chars: l.chars.as_str(),
            address: l.address.as_str(),
        }
    }
}

/// One dump row: `{address, hex, chars}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HexDumpRowJson {
    /// Offset of its first byte.
    pub address: u64,
    /// Its bytes as hex text.
    pub hex: String,
    /// Its characters (`null` with `chars: none`).
    pub chars: Option<String>,
}

/// A float of the value panel: a number, or `"NaN"`, `"inf"`, `"-inf"`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(untagged)]
pub enum FloatJson {
    /// A finite single-precision value (written with its shortest round-trip digits).
    F32(f32),
    /// A finite double-precision value.
    F64(f64),
    /// `NaN`, `inf`, `-inf`.
    Text(&'static str),
}

impl FloatJson {
    fn of32(v: f32) -> FloatJson {
        if v.is_nan() {
            FloatJson::Text("NaN")
        } else if v.is_infinite() {
            FloatJson::Text(if v > 0.0 { "inf" } else { "-inf" })
        } else {
            FloatJson::F32(v)
        }
    }

    fn of64(v: f64) -> FloatJson {
        if v.is_nan() {
            FloatJson::Text("NaN")
        } else if v.is_infinite() {
            FloatJson::Text(if v > 0.0 { "inf" } else { "-inf" })
        } else {
            FloatJson::F64(v)
        }
    }
}

/// The values of one byte order: `{i8, u8, i16, u16, i32, u32, i64, u64, f32, f64, utf8}`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ValuesJson {
    /// Signed byte.
    pub i8: Option<i8>,
    /// Unsigned byte.
    pub u8: Option<u8>,
    /// Signed 16 bits.
    pub i16: Option<i16>,
    /// Unsigned 16 bits.
    pub u16: Option<u16>,
    /// Signed 32 bits.
    pub i32: Option<i32>,
    /// Unsigned 32 bits.
    pub u32: Option<u32>,
    /// Signed 64 bits, as a decimal string.
    pub i64: Option<String>,
    /// Unsigned 64 bits, as a decimal string.
    pub u64: Option<String>,
    /// Single precision.
    pub f32: Option<FloatJson>,
    /// Double precision.
    pub f64: Option<FloatJson>,
    /// The UTF-8 scalar starting at the offset.
    pub utf8: Option<String>,
}

impl ValuesJson {
    /// The DTO of one byte order's values.
    pub fn new(v: &Interpreted) -> ValuesJson {
        ValuesJson {
            i8: v.i8,
            u8: v.u8,
            i16: v.i16,
            u16: v.u16,
            i32: v.i32,
            u32: v.u32,
            i64: v.i64.map(|n| n.to_string()),
            u64: v.u64.map(|n| n.to_string()),
            f32: v.f32.map(FloatJson::of32),
            f64: v.f64.map(FloatJson::of64),
            utf8: v.utf8.map(String::from),
        }
    }
}

/// The value panel: `{offset, little, big}`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ValuesBlockJson {
    /// The offset interpreted.
    pub offset: u64,
    /// Little endian.
    pub little: ValuesJson,
    /// Big endian.
    pub big: ValuesJson,
}

impl ValuesBlockJson {
    /// The DTO of the values at `offset`.
    pub fn new(offset: u64, v: &Values) -> ValuesBlockJson {
        ValuesBlockJson {
            offset,
            little: ValuesJson::new(&v.little),
            big: ValuesJson::new(&v.big),
        }
    }
}

/// `hikasa.hex_dump.v1` (§1.12.24).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HexDumpJson {
    /// The side dumped.
    pub side: HexSideJson,
    /// The byte layout.
    pub layout: HexLayoutJson,
    /// First byte dumped.
    pub offset: u64,
    /// Bytes dumped.
    pub length: u64,
    /// The rows.
    pub rows: Vec<HexDumpRowJson>,
    /// The value panel (`--values`), else `null`.
    pub values: Option<ValuesBlockJson>,
}

/// The rows of `bytes` read at `offset` under `layout` (the JSON's and the human output's cut).
pub fn hex_dump_rows(layout: &HexLayout, offset: u64, bytes: &[u8]) -> Vec<HexDumpRowJson> {
    let bpl = usize::from(layout.bytes_per_line.clamp(1, 256));
    bytes
        .chunks(bpl)
        .enumerate()
        .map(|(i, chunk)| HexDumpRowJson {
            address: offset + (i * bpl) as u64,
            hex: hex_text(chunk),
            chars: (layout.chars != hikasa_hex::CharSet::None)
                .then(|| chunk.iter().filter_map(|&b| layout.char_of(b)).collect()),
        })
        .collect()
}

/// The pattern of a find: `{kind, pattern, encoding, match_case}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HexQueryJson {
    /// `text`, `bytes`.
    pub kind: &'static str,
    /// The pattern as given.
    pub pattern: String,
    /// The text's encoding (`null` for bytes).
    pub encoding: Option<&'static str>,
    /// ASCII letters matched exactly (text only).
    pub match_case: bool,
}

/// One searched file with hits: `{spec, size, hits, hit_count, stopped}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HexFindFileJson {
    /// The canonical spec.
    pub spec: String,
    /// Size in bytes.
    pub size: u64,
    /// Hit offsets in order (empty with `--count` and `--files-with-matches`).
    pub hits: Vec<u64>,
    /// Hits counted.
    pub hit_count: u64,
    /// The per-file limit stopped the search of this file.
    pub stopped: bool,
}

/// A skipped item: `{spec, reason, error}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HexSkippedJson {
    /// The canonical spec.
    pub spec: String,
    /// `error`, `limit`.
    pub reason: &'static str,
    /// The error, when the reason is one.
    pub error: Option<ErrorReport>,
}

/// The counts: `{files_searched, files_matched, hits, skipped}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct HexFindCountsJson {
    /// Files searched.
    pub files_searched: u64,
    /// Files with at least one hit.
    pub files_matched: u64,
    /// Hits.
    pub hits: u64,
    /// Items skipped.
    pub skipped: u64,
}

/// `hikasa.hex_find.v1` (§1.12.24).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HexFindJson {
    /// The pattern.
    pub query: HexQueryJson,
    /// The roots as canonical specs.
    pub roots: Vec<String>,
    /// `--mask` (`null`: every file).
    pub mask: Option<String>,
    /// `--recursive`.
    pub recursive: bool,
    /// `--include-hidden`.
    pub include_hidden: bool,
    /// `files`, `opened`.
    pub archives: &'static str,
    /// The files with hits, in walk order.
    pub files: Vec<HexFindFileJson>,
    /// Items that could not be searched.
    pub skipped: Vec<HexSkippedJson>,
    /// The counts.
    pub counts: HexFindCountsJson,
    /// `--max-matches` stopped the search.
    pub limit_reached: bool,
}
