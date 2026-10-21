//! `hikasa.folder_compare.v1` (§4.3.3, D-29): [`FolderCompareJson`] and its builder [`folder_compare_json`]. The
//! items are a flat pre-order list of the visible rows with `path` and `depth`; node ids are never printed. The
//! `rules` object has the full shape of [`FolderRules`] with explicit `null`s (§4.1 rule 3), not the TOML form
//! (which omits absent optional keys, §1.6.8 item 12).

use hikasa_core::{ContentResult, ErrorReport, FileTime, Side, Warning};
use hikasa_folder::{
    Attrs, CaseMode, Category, Cmp, DateCutoff, DisplayOptions, EntryView, FolderComparison,
    FolderMode, FolderRules, FolderStats, NodeKind, TimeRelation,
};
use serde::Serialize;

use crate::folder::{CHUNK_ROWS, visible};
use crate::meta::toggle_names;

/// The `result` of `hikasa folder compare --json`, and the JSON folder report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FolderCompareJson {
    /// Left base.
    pub left: FolderBaseJson,
    /// Right base.
    pub right: FolderBaseJson,
    /// Rules used (full shape).
    pub rules: FolderRulesJson,
    /// Case sensitivity used (`sensitive`, `insensitive`; `null` before the bases were loaded).
    pub case: Option<&'static str>,
    /// False when the scan was cancelled.
    pub complete: bool,
    /// Display options of `items`.
    pub display: FolderDisplayJson,
    /// Verdict under `display.ignore_unimportant` (D-02).
    pub verdict: &'static str,
    /// Statistics under `display.ignore_unimportant` (FC-064).
    pub stats: FolderStatsJson,
    /// Visible items in pre-order.
    pub items: Vec<FolderItemJson>,
}

/// The JSON folder report ([`crate::FolderLayout::Json`]): `"schema": "hikasa.folder_compare.v1"` as the first key
/// (§4.1 rule 10), then the fields of [`FolderCompareJson`] (additive, wave 4). Under `--json` the session's
/// envelope carries the schema and its `result` is the bare [`FolderCompareJson`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct FolderReportJson<'a> {
    /// [`super::FOLDER_COMPARE_SCHEMA`].
    pub schema: &'static str,
    /// The result fields, written inline.
    #[serde(flatten)]
    pub result: &'a FolderCompareJson,
}

impl<'a> FolderReportJson<'a> {
    /// The report document of `result`.
    pub fn new(result: &'a FolderCompareJson) -> FolderReportJson<'a> {
        FolderReportJson {
            schema: super::FOLDER_COMPARE_SCHEMA,
            result,
        }
    }
}

/// One base folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FolderBaseJson {
    /// Canonical spec.
    pub spec: String,
    /// Back-end capabilities (snake_case names in bit order; empty before the bases were loaded).
    pub capabilities: Vec<&'static str>,
}

/// [`FolderRules`] in full shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FolderRulesJson {
    /// Schema version.
    pub schema_version: u32,
    /// Criteria.
    pub criteria: CriteriaJson,
    /// Name alignment.
    pub alignment: NameAlignmentJson,
    /// Filters.
    pub filters: FilterRulesJson,
    /// Handling.
    pub handling: HandlingJson,
}

/// Comparison criteria.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CriteriaJson {
    /// FC-020.
    pub size: bool,
    /// FC-021.
    pub timestamps: TimestampRuleJson,
    /// FC-022.
    pub filename_case: bool,
    /// FC-023 attribute subset as DOS letters (D-30; `""` = off).
    pub dos_attributes: String,
    /// FC-024.
    pub unix: UnixCriteriaJson,
    /// FC-025…FC-027.
    pub content: ContentRuleJson,
}

/// Timestamp rule (D-52).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TimestampRuleJson {
    /// Compare timestamps.
    pub enabled: bool,
    /// Tolerance in seconds.
    pub tolerance_secs: u32,
    /// Ignore DST.
    pub ignore_dst: bool,
    /// Ignore time zones.
    pub ignore_timezone: bool,
    /// Whole-hour shift of one side (`null` = none).
    pub offset: Option<TzOffsetJson>,
}

/// A whole-hour shift.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct TzOffsetJson {
    /// `left` or `right`.
    pub side: &'static str,
    /// Hours (−12..=12).
    pub hours: i8,
}

/// Unix criteria.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct UnixCriteriaJson {
    /// Permissions.
    pub permissions: bool,
    /// Owner.
    pub owner: bool,
    /// Group.
    pub group: bool,
}

/// Content rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ContentRuleJson {
    /// `none`, `crc`, `binary`, `rules`.
    pub method: &'static str,
    /// FC-026.
    pub skip_if_quick_same: bool,
    /// FC-027.
    pub override_quick: bool,
}

/// Name alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct NameAlignmentJson {
    /// `auto`, `sensitive`, `insensitive`.
    pub case: &'static str,
    /// Align extensions.
    pub align_extensions: bool,
    /// Align NFC with NFD names.
    pub align_normalization: bool,
}

/// Filters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FilterRulesJson {
    /// Name filters (FC-050).
    pub names: NameFiltersJson,
    /// Other filters.
    pub other: OtherFiltersJson,
}

/// The four name-filter lists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NameFiltersJson {
    /// Include-file masks.
    pub include_files: Vec<String>,
    /// Exclude-file masks.
    pub exclude_files: Vec<String>,
    /// Include-folder masks.
    pub include_folders: Vec<String>,
    /// Exclude-folder masks.
    pub exclude_folders: Vec<String>,
}

/// Other filters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OtherFiltersJson {
    /// Exclude items with SYSTEM and HIDDEN.
    pub exclude_protected: bool,
    /// Size filter (`null` = none).
    pub size: Option<SizeFilterJson>,
    /// Date filter (`null` = none).
    pub date: Option<DateFilterJson>,
}

/// A size filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SizeFilterJson {
    /// `less` or `greater`.
    pub cmp: &'static str,
    /// Bound in bytes.
    pub bytes: u64,
}

/// A date filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct DateFilterJson {
    /// `less` or `greater`.
    pub cmp: &'static str,
    /// The cutoff.
    pub cutoff: DateCutoffJson,
}

/// A date cutoff: `{"type": "absolute", "at": …, "days": null}` or `{"type": "days_ago", "at": null, "days": n}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct DateCutoffJson {
    /// `absolute` or `days_ago`.
    #[serde(rename = "type")]
    pub kind: &'static str,
    /// The instant (`absolute`).
    pub at: Option<FileTime>,
    /// Days before today's local midnight (`days_ago`).
    pub days: Option<u32>,
}

/// Handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct HandlingJson {
    /// FC-013.
    pub follow_symlinks: bool,
    /// FC-014.
    pub flatten: bool,
    /// Scan orphan subfolders.
    pub scan_orphan_folders: bool,
}

/// Display options of the items.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FolderDisplayJson {
    /// The filter (`all`, `differences`, …, `none`, or `toggles`).
    pub filter: &'static str,
    /// `always_show_folders`, `compare_structure`, `only_files`.
    pub folders: &'static str,
    /// FC-057 Ignore Unimportant Differences.
    pub ignore_unimportant: bool,
    /// FC-057 Suppress Filters.
    pub suppress_filters: bool,
    /// The shown categories of the filter (snake_case names in bit order; additive field).
    pub toggles: Vec<&'static str>,
}

/// Statistics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FolderStatsJson {
    /// File-like pairs by category.
    pub files: CategoryCountsJson,
    /// Folder pairs by category (the base folders excluded).
    pub folders: CategoryCountsJson,
    /// Left totals.
    pub left: SideTotalsJson,
    /// Right totals.
    pub right: SideTotalsJson,
    /// Content-test results.
    pub content: ContentCountsJson,
}

/// Counts per category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CategoryCountsJson {
    /// Same.
    pub same: u32,
    /// Left newer.
    pub left_newer: u32,
    /// Right newer.
    pub right_newer: u32,
    /// Different.
    pub different: u32,
    /// Left only.
    pub left_orphan: u32,
    /// Right only.
    pub right_orphan: u32,
    /// Unknown.
    pub unknown: u32,
    /// Error.
    pub error: u32,
    /// Filtered.
    pub filtered: u32,
}

/// Totals of one side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SideTotalsJson {
    /// Files.
    pub files: u64,
    /// Folders.
    pub folders: u64,
    /// Bytes of files.
    pub bytes: u64,
}

/// Content-test results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ContentCountsJson {
    /// Binary same.
    pub binary_same: u32,
    /// Rules same.
    pub rules_same: u32,
    /// Unimportant only.
    pub unimportant_only: u32,
    /// Important differences.
    pub important_differences: u32,
    /// Binary different.
    pub binary_different: u32,
    /// Errors.
    pub error: u32,
    /// Skipped because the quick tests said same (FC-026).
    pub not_compared: u32,
}

/// One visible item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FolderItemJson {
    /// Relative path (`/`-separated; no trailing `/`).
    pub path: String,
    /// Depth (children of the bases = 0).
    pub depth: u32,
    /// `folder`, `file`, `symlink`, `other`.
    pub kind: &'static str,
    /// Canonical state.
    pub state: &'static str,
    /// Display category under `display.ignore_unimportant`.
    pub category: &'static str,
    /// `not_applicable`, `not_scanned`, `scanned`, `failed`.
    pub scan: &'static str,
    /// Folders: the categories with a non-zero descendant count, in category order; files: `null`.
    pub aggregate: Option<Vec<&'static str>>,
    /// Differences found (snake_case names in bit order).
    pub diffs: Vec<&'static str>,
    /// Node flags (snake_case names in bit order).
    pub flags: Vec<&'static str>,
    /// Timestamp relation (`same`, `left_newer`, `right_newer`), when compared.
    pub time: Option<&'static str>,
    /// The content outcome, when a content test ran.
    pub content: Option<ContentJson>,
    /// Errors of the node.
    pub errors: Vec<NodeErrorJson>,
    /// Left entry (`null` when absent).
    pub left: Option<EntryJson>,
    /// Right entry (`null` when absent).
    pub right: Option<EntryJson>,
}

/// A content outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContentJson {
    /// Method.
    pub method: &'static str,
    /// Result (`{"result": "…"}`, plus `"error"` for an error).
    pub result: ContentResult,
    /// Warnings of the content test.
    pub warnings: Vec<Warning>,
}

/// One error of a node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NodeErrorJson {
    /// `left`, `right`, or `null` for the pair.
    pub side: Option<&'static str>,
    /// The error.
    pub error: ErrorReport,
}

/// One side's entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EntryJson {
    /// Name (lossy when `name_lossy`).
    pub name: String,
    /// The OS name was not valid UTF-8.
    pub name_lossy: bool,
    /// Bytes; a folder: the total of the files below it on this side (FC-040).
    pub size: u64,
    /// Modification time.
    pub mtime: Option<FileTime>,
    /// Creation time.
    pub ctime: Option<FileTime>,
    /// DOS attribute letters (`null` without DOS attributes).
    pub attrs: Option<String>,
    /// Unix permission bits.
    pub mode: Option<u32>,
    /// Unix permission string (`drwxr-xr-x`).
    pub permissions: Option<String>,
    /// Unix owner.
    pub uid: Option<u32>,
    /// Unix group.
    pub gid: Option<u32>,
    /// Symlink target.
    pub link_target: Option<String>,
    /// A followed symlink.
    pub followed: bool,
    /// CRC32 as 8 lower-case hex digits.
    pub crc32: Option<String>,
    /// Hidden by a filter on this side (FC-054).
    pub filtered: bool,
    /// Name-only entry (D-45).
    pub stat_skipped: bool,
    /// Flatten mode: the containing folder (`""` = the base); `null` otherwise.
    pub dir: Option<String>,
}

/// `less` / `greater`.
fn cmp_name(c: Cmp) -> &'static str {
    match c {
        Cmp::Less => "less",
        Cmp::Greater => "greater",
    }
}

impl FolderRulesJson {
    /// The DTO of folder rules (full shape).
    pub fn new(rules: &FolderRules) -> FolderRulesJson {
        let c = &rules.criteria;
        let t = &c.timestamps;
        let other = &rules.filters.other;
        let names = &rules.filters.names;
        FolderRulesJson {
            schema_version: rules.schema_version,
            criteria: CriteriaJson {
                size: c.size,
                timestamps: TimestampRuleJson {
                    enabled: t.enabled,
                    tolerance_secs: t.tolerance_secs,
                    ignore_dst: t.ignore_dst,
                    ignore_timezone: t.ignore_timezone,
                    offset: t.offset.map(|o| TzOffsetJson {
                        side: o.side.as_str(),
                        hours: o.hours,
                    }),
                },
                filename_case: c.filename_case,
                dos_attributes: c.dos_attributes.letters(),
                unix: UnixCriteriaJson {
                    permissions: c.unix.permissions,
                    owner: c.unix.owner,
                    group: c.unix.group,
                },
                content: ContentRuleJson {
                    method: c.content.method.as_str(),
                    skip_if_quick_same: c.content.skip_if_quick_same,
                    override_quick: c.content.override_quick,
                },
            },
            alignment: NameAlignmentJson {
                case: match rules.alignment.case {
                    CaseMode::Auto => "auto",
                    CaseMode::Sensitive => "sensitive",
                    CaseMode::Insensitive => "insensitive",
                },
                align_extensions: rules.alignment.align_extensions,
                align_normalization: rules.alignment.align_normalization,
            },
            filters: FilterRulesJson {
                names: NameFiltersJson {
                    include_files: names.include_files.clone(),
                    exclude_files: names.exclude_files.clone(),
                    include_folders: names.include_folders.clone(),
                    exclude_folders: names.exclude_folders.clone(),
                },
                other: OtherFiltersJson {
                    exclude_protected: other.exclude_protected,
                    size: other.size.map(|s| SizeFilterJson {
                        cmp: cmp_name(s.cmp),
                        bytes: s.bytes,
                    }),
                    date: other.date.map(|d| DateFilterJson {
                        cmp: cmp_name(d.cmp),
                        cutoff: match d.cutoff {
                            DateCutoff::Absolute { at } => DateCutoffJson {
                                kind: "absolute",
                                at: Some(at),
                                days: None,
                            },
                            DateCutoff::DaysAgo { days } => DateCutoffJson {
                                kind: "days_ago",
                                at: None,
                                days: Some(days),
                            },
                        },
                    }),
                },
            },
            handling: HandlingJson {
                follow_symlinks: rules.handling.follow_symlinks,
                flatten: rules.handling.flatten,
                scan_orphan_folders: rules.handling.scan_orphan_folders,
            },
        }
    }
}

/// The serde name of a display filter preset (`toggles` for toggle mode).
fn filter_name(filter: hikasa_folder::FolderDisplayFilter) -> &'static str {
    use hikasa_folder::FolderDisplayFilter as F;
    match filter {
        F::All => "all",
        F::Differences => "differences",
        F::Same => "same",
        F::NoOrphans => "no_orphans",
        F::DifferencesNoOrphans => "differences_no_orphans",
        F::Orphans => "orphans",
        F::LeftNewer => "left_newer",
        F::RightNewer => "right_newer",
        F::LeftNewerAndLeftOrphans => "left_newer_and_left_orphans",
        F::RightNewerAndRightOrphans => "right_newer_and_right_orphans",
        F::LeftOrphans => "left_orphans",
        F::RightOrphans => "right_orphans",
        F::None => "none",
        F::Toggles { .. } => "toggles",
    }
}

impl FolderDisplayJson {
    /// The DTO of display options.
    pub fn new(display: &DisplayOptions) -> FolderDisplayJson {
        FolderDisplayJson {
            filter: filter_name(display.filter),
            folders: match display.folders {
                FolderMode::AlwaysShowFolders => "always_show_folders",
                FolderMode::CompareStructure => "compare_structure",
                FolderMode::OnlyFiles => "only_files",
            },
            ignore_unimportant: display.ignore_unimportant,
            suppress_filters: display.suppress_filters,
            toggles: toggle_names(display.filter.toggles()),
        }
    }
}

impl From<&hikasa_folder::CategoryCounts> for CategoryCountsJson {
    fn from(c: &hikasa_folder::CategoryCounts) -> CategoryCountsJson {
        CategoryCountsJson {
            same: c.same,
            left_newer: c.left_newer,
            right_newer: c.right_newer,
            different: c.different,
            left_orphan: c.left_orphan,
            right_orphan: c.right_orphan,
            unknown: c.unknown,
            error: c.error,
            filtered: c.filtered,
        }
    }
}

impl From<&hikasa_folder::SideTotals> for SideTotalsJson {
    fn from(t: &hikasa_folder::SideTotals) -> SideTotalsJson {
        SideTotalsJson {
            files: t.files,
            folders: t.folders,
            bytes: t.bytes,
        }
    }
}

impl From<&FolderStats> for FolderStatsJson {
    fn from(s: &FolderStats) -> FolderStatsJson {
        let c = &s.content;
        FolderStatsJson {
            files: (&s.files).into(),
            folders: (&s.folders).into(),
            left: (&s.left).into(),
            right: (&s.right).into(),
            content: ContentCountsJson {
                binary_same: c.binary_same,
                rules_same: c.rules_same,
                unimportant_only: c.unimportant_only,
                important_differences: c.important_differences,
                binary_different: c.binary_different,
                error: c.error,
                not_compared: c.not_compared,
            },
        }
    }
}

impl EntryJson {
    /// The DTO of one side's entry; `folder_bytes` is the side's subtree total (used for folders).
    pub fn new(kind: NodeKind, entry: &EntryView<'_>, folder_bytes: u64) -> EntryJson {
        let unix = match entry.attrs {
            Attrs::Unix { meta } => Some(meta),
            _ => None,
        };
        EntryJson {
            name: entry.name.to_owned(),
            name_lossy: entry.name_lossy,
            size: if kind == NodeKind::Folder {
                folder_bytes
            } else {
                entry.size
            },
            mtime: entry.mtime,
            ctime: entry.ctime,
            attrs: match entry.attrs {
                Attrs::Dos { attrs } => Some(attrs.letters()),
                _ => None,
            },
            mode: unix.map(|m| m.mode),
            permissions: unix.map(|m| m.permissions_string(entry.kind)),
            uid: unix.map(|m| m.uid),
            gid: unix.map(|m| m.gid),
            link_target: entry.link_target.map(str::to_owned),
            followed: entry.followed,
            crc32: entry.crc32.map(|c| format!("{c:08x}")),
            filtered: entry.filtered,
            stat_skipped: entry.stat_skipped,
            dir: entry.dir.map(str::to_owned),
        }
    }
}

/// `same`, `left_newer`, `right_newer`.
fn time_name(t: TimeRelation) -> &'static str {
    match t {
        TimeRelation::Same => "same",
        TimeRelation::LeftNewer => "left_newer",
        TimeRelation::RightNewer => "right_newer",
    }
}

/// Builds the folder JSON DTO (§4.3.3): the visible rows of `display` (every visible folder expanded), read in
/// chunks of 4096 rows.
pub fn folder_compare_json(cmp: &FolderComparison, display: &DisplayOptions) -> FolderCompareJson {
    let iu = display.ignore_unimportant;
    let rows = visible(cmp, display);
    let mut items = Vec::with_capacity(rows.len());
    for chunk in rows.chunks(CHUNK_ROWS) {
        let read = cmp.read();
        for row in chunk {
            let id = row.node;
            let Some(node) = read.node(id) else { continue };
            let totals = read.totals(id);
            let aggregate = (node.kind == NodeKind::Folder).then(|| {
                let a = read.aggregates(id).get(iu).both();
                Category::ALL
                    .iter()
                    .filter(|&&c| a.get(c) > 0)
                    .map(|c| c.as_str())
                    .collect()
            });
            let entry = |side: Side| {
                node.side(side).map(|e| {
                    let bytes = totals.get(side).map_or(0, |t| t.bytes);
                    EntryJson::new(node.kind, e, bytes)
                })
            };
            items.push(FolderItemJson {
                path: read.path(id).as_str().to_owned(),
                depth: row.depth,
                kind: node.kind.as_str(),
                state: node.state.as_str(),
                category: read.category(id, iu).as_str(),
                scan: node.scan.as_str(),
                aggregate,
                diffs: node.diffs.names(),
                flags: node.flags.names(),
                time: node.time.map(time_name),
                content: read.content_outcome(id).map(|o| ContentJson {
                    method: o.method.as_str(),
                    result: o.result,
                    warnings: o.warnings,
                }),
                errors: read
                    .errors(id)
                    .iter()
                    .map(|e| NodeErrorJson {
                        side: e.side.map(Side::as_str),
                        error: e.error.clone(),
                    })
                    .collect(),
                left: entry(Side::Left),
                right: entry(Side::Right),
            });
        }
    }
    let read = cmp.read();
    let specs = cmp.model().specs();
    let bases = read.bases();
    let base = |spec: &hikasa_core::Spec, info: Option<&hikasa_folder::BaseInfo>| FolderBaseJson {
        spec: spec.canonical(),
        capabilities: info.map(|b| b.capabilities.names()).unwrap_or_default(),
    };
    FolderCompareJson {
        left: base(&specs.left, bases.left),
        right: base(&specs.right, bases.right),
        rules: FolderRulesJson::new(cmp.rules()),
        case: read.case().map(|c| match c {
            hikasa_core::CaseSensitivity::Sensitive => "sensitive",
            hikasa_core::CaseSensitivity::Insensitive => "insensitive",
        }),
        complete: read.is_complete(),
        display: FolderDisplayJson::new(display),
        verdict: read.verdict(iu).as_str(),
        stats: (&read.stats(iu)).into(),
        items,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hikasa_folder::{FolderDisplayFilter, SizeFilter, TzOffset};

    #[test]
    fn rules_have_full_shape() {
        let rules = FolderRules::default();
        let json = serde_json::to_value(FolderRulesJson::new(&rules)).unwrap();
        assert_eq!(
            json["criteria"]["timestamps"]["offset"],
            serde_json::Value::Null
        );
        assert_eq!(json["filters"]["other"]["size"], serde_json::Value::Null);
        assert_eq!(json["filters"]["other"]["date"], serde_json::Value::Null);
        assert_eq!(json["criteria"]["dos_attributes"], "");
        assert_eq!(json["alignment"]["case"], "auto");
        assert_eq!(json["criteria"]["content"]["method"], "none");
        let mut rules = FolderRules::default();
        rules.criteria.timestamps.offset = Some(TzOffset {
            side: Side::Right,
            hours: -3,
        });
        rules.filters.other.size = Some(SizeFilter {
            cmp: Cmp::Less,
            bytes: 1024,
        });
        rules.filters.other.date = Some(hikasa_folder::DateFilter {
            cmp: Cmp::Greater,
            cutoff: DateCutoff::DaysAgo { days: 30 },
        });
        let json =
            serde_json::to_string(&FolderRulesJson::new(&rules).criteria.timestamps).unwrap();
        assert_eq!(
            json,
            r#"{"enabled":true,"tolerance_secs":2,"ignore_dst":false,"ignore_timezone":false,"offset":{"side":"right","hours":-3}}"#
        );
        let json = serde_json::to_string(&FolderRulesJson::new(&rules).filters.other).unwrap();
        assert_eq!(
            json,
            r#"{"exclude_protected":true,"size":{"cmp":"less","bytes":1024},"date":{"cmp":"greater","cutoff":{"type":"days_ago","at":null,"days":30}}}"#
        );
    }

    #[test]
    fn display_and_names_match_serde() {
        for f in FolderDisplayFilter::PRESETS {
            assert_eq!(
                serde_json::to_string(&f).unwrap(),
                format!("\"{}\"", filter_name(f))
            );
        }
        for m in FolderMode::ALL {
            let d = DisplayOptions {
                filter: FolderDisplayFilter::Differences,
                folders: m,
                ignore_unimportant: false,
                suppress_filters: true,
            };
            assert_eq!(
                serde_json::to_string(&m).unwrap(),
                format!("\"{}\"", FolderDisplayJson::new(&d).folders)
            );
        }
        for t in [
            TimeRelation::Same,
            TimeRelation::LeftNewer,
            TimeRelation::RightNewer,
        ] {
            assert_eq!(
                serde_json::to_string(&t).unwrap(),
                format!("\"{}\"", time_name(t))
            );
        }
        let d = DisplayOptions {
            filter: FolderDisplayFilter::Orphans,
            folders: FolderMode::CompareStructure,
            ignore_unimportant: false,
            suppress_filters: false,
        };
        assert_eq!(
            serde_json::to_string(&FolderDisplayJson::new(&d)).unwrap(),
            r#"{"filter":"orphans","folders":"compare_structure","ignore_unimportant":false,"suppress_filters":false,"toggles":["left_orphans","right_orphans"]}"#
        );
    }
}
