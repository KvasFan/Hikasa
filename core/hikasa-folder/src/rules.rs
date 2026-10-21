//! Folder rules (FC-020…FC-027, FC-050…FC-054, FC-010…FC-014): [`FolderRules`] and its parts, the script
//! `criteria` vocabulary ([`Criteria::parse_list`], D-57), the other-filter vocabularies ([`SizeFilter::parse`],
//! [`DateFilter::parse`], D-37) and the TOML form of rules files (`--rules-file`, D-31, INV-RULES-2).
//!
//! Every type here is plain data. Defaults are documented on each field; every field round-trips through TOML
//! (no `Option` field has a non-`None` default, D-52).

use std::fmt;

use hikasa_core::{
    CaseSensitivity, ContentMethod, FileTime, MaskOptions, NameFilter, NameFilterLists, Side,
    Warning, WarningCode, check_schema_version,
};
use hikasa_vfs::DosAttrs;
use serde::de::{self, Deserializer, Visitor};
use serde::{Deserialize, Serialize, Serializer};
use toml::{Table, Value};

use crate::error::FolderError;

/// Schema version of folder rules files.
pub const FOLDER_RULES_SCHEMA_VERSION: u32 = 1;

/// Largest whole-hour shift of [`TzOffset`] (script `timezone:left+12`).
const MAX_OFFSET_HOURS: i8 = 12;

/// Folder session rules. `Default` gives the values in the field comments. Every field round-trips through TOML
/// (INV-RULES-2): no `Option` field has a non-`None` default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FolderRules {
    /// Schema version ([`FOLDER_RULES_SCHEMA_VERSION`]).
    pub schema_version: u32,
    /// Comparison criteria.
    pub criteria: Criteria,
    /// Name alignment.
    pub alignment: NameAlignment,
    /// Filters.
    pub filters: FilterRules,
    /// Handling.
    pub handling: Handling,
}

impl Default for FolderRules {
    fn default() -> FolderRules {
        FolderRules {
            schema_version: FOLDER_RULES_SCHEMA_VERSION,
            criteria: Criteria::default(),
            alignment: NameAlignment::default(),
            filters: FilterRules::default(),
            handling: Handling::default(),
        }
    }
}

/// Comparison criteria (FC-020…FC-027).
///
/// The quick tests apply to pairs of files, symbolic links and other entries. **Folder pairs are not
/// quick-tested**: a folder is compared by aggregating its contents (its state comes from the counters of the
/// items below it, §2.6.4), so its own size, time, name case, attributes, permissions, owner and group never make
/// it different (docs/04 §3; contract §2.6.1 and §1.6.8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Criteria {
    /// FC-020 file size. Default true.
    pub size: bool,
    /// FC-021. Default enabled, 2 s, no DST, no TZ, no offset.
    pub timestamps: TimestampRule,
    /// FC-022 file name case (not folders). Default false.
    pub filename_case: bool,
    /// FC-023 DOS attribute subset (empty = off; not compared on folders). Default empty. Serialised as letters
    /// (`"HRS"`, D-30).
    pub dos_attributes: DosAttrs,
    /// FC-024 (not compared on folders). Default all false.
    pub unix: UnixCriteria,
    /// FC-025…FC-027.
    pub content: ContentRule,
}

impl Default for Criteria {
    fn default() -> Criteria {
        Criteria {
            size: true,
            timestamps: TimestampRule::default(),
            filename_case: false,
            dos_attributes: DosAttrs::empty(),
            unix: UnixCriteria::default(),
            content: ContentRule::default(),
        }
    }
}

/// Result of [`Criteria::parse_list`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CriteriaSpec {
    /// Every criteria field is `Some`: listed criteria on, unlisted quick tests and the content method off;
    /// `follow_symlinks` is `Some(true)` only when listed, else `None`.
    pub patch: FolderRulesPatch,
    /// `ignore-unimportant` was listed (a display option, FC-057).
    pub ignore_unimportant: bool,
}

/// Timestamp test (FC-021, D-52). An explicit `enabled` flag instead of an `Option`, because TOML has no null.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct TimestampRule {
    /// Compare timestamps at all. Default true.
    pub enabled: bool,
    /// Tolerance in seconds (inclusive). Default 2.
    pub tolerance_secs: u32,
    /// Exactly 1 h apart (± tolerance) counts as same. Default false.
    pub ignore_dst: bool,
    /// Any whole number of hours apart (|k| ≤ 26, ± tolerance) counts as same. Default false.
    pub ignore_timezone: bool,
    /// Shift one side's times by whole hours before comparing (script `timezone:left+2`). Default `None`,
    /// omitted from TOML.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<TzOffset>,
}

impl Default for TimestampRule {
    fn default() -> TimestampRule {
        TimestampRule {
            enabled: true,
            tolerance_secs: 2,
            ignore_dst: false,
            ignore_timezone: false,
            offset: None,
        }
    }
}

/// Whole-hour shift applied to one side's timestamps (`side` is Left or Right; `hours` in −12..=12).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TzOffset {
    /// The shifted side (Left or Right).
    pub side: Side,
    /// Hours added to that side's times (−12..=12).
    pub hours: i8,
}

/// Unix metadata tests (FC-024).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UnixCriteria {
    /// Compare permission bits.
    pub permissions: bool,
    /// Compare the owner (uid).
    pub owner: bool,
    /// Compare the group (gid).
    pub group: bool,
}

/// Content test (FC-025…FC-027).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct ContentRule {
    /// Method. Default None.
    pub method: ContentMethod,
    /// FC-026. Default false (D-34).
    pub skip_if_quick_same: bool,
    /// FC-027 (D-48: cancels only TIME and SIZE differences). Default true.
    pub override_quick: bool,
}

impl Default for ContentRule {
    fn default() -> ContentRule {
        ContentRule {
            method: ContentMethod::None,
            skip_if_quick_same: false,
            override_quick: true,
        }
    }
}

/// Case handling for name alignment and masks (FC-010).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaseMode {
    /// Sensitive iff both bases' back-ends report `CASE_SENSITIVE`.
    #[default]
    Auto,
    /// Always sensitive.
    Sensitive,
    /// Always insensitive.
    Insensitive,
}

/// Name alignment options (FC-010).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NameAlignment {
    /// Case handling.
    pub case: CaseMode,
    /// Align `x.txt` with `x.csv` (files only).
    pub align_extensions: bool,
    /// Align NFC with NFD names.
    pub align_normalization: bool,
}

/// Comparison operator for other filters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cmp {
    /// The value must be smaller than the bound (`<`).
    Less,
    /// The value must be larger than the bound (`>`).
    Greater,
}

impl Cmp {
    /// `a < b` for `Less`, `a > b` for `Greater`.
    pub fn holds<T: PartialOrd>(self, a: T, b: T) -> bool {
        match self {
            Cmp::Less => a < b,
            Cmp::Greater => a > b,
        }
    }
}

/// Size filter (`size:<10MB`); files only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SizeFilter {
    /// Comparison.
    pub cmp: Cmp,
    /// Bound in bytes. Serialised as an integer, or as a decimal string above `i64::MAX` (TOML integers are
    /// signed 64-bit), so every value round-trips.
    #[serde(with = "u64_text")]
    pub bytes: u64,
}

/// Date cutoff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DateCutoff {
    /// An absolute instant.
    Absolute {
        /// The instant.
        at: FileTime,
    },
    /// N days before today's local midnight (FC-053: "relative to midnight"); resolved with `FolderEnv::now`.
    DaysAgo {
        /// Number of days.
        days: u32,
    },
}

/// Date filter (`cutoff:<30days`); files only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DateFilter {
    /// Comparison of the modification time with the cutoff.
    pub cmp: Cmp,
    /// The cutoff.
    pub cutoff: DateCutoff,
}

/// Binary size units (D-37).
const SIZE_UNITS: [(&str, u64); 13] = [
    ("", 1),
    ("b", 1),
    ("k", 1 << 10),
    ("kb", 1 << 10),
    ("kib", 1 << 10),
    ("m", 1 << 20),
    ("mb", 1 << 20),
    ("mib", 1 << 20),
    ("g", 1 << 30),
    ("gb", 1 << 30),
    ("gib", 1 << 30),
    ("tb", 1 << 40),
    ("tib", 1 << 40),
];

/// Splits an optional leading `<` or `>` (after trimming ASCII whitespace).
fn split_cmp(s: &str) -> (Option<Cmp>, &str) {
    let s = s.trim_matches(|c: char| c.is_ascii_whitespace());
    if let Some(rest) = s.strip_prefix('<') {
        (Some(Cmp::Less), rest.trim_start())
    } else if let Some(rest) = s.strip_prefix('>') {
        (Some(Cmp::Greater), rest.trim_start())
    } else {
        (None, s)
    }
}

/// Splits leading ASCII digits from `s`.
fn split_digits(s: &str) -> (&str, &str) {
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    s.split_at(end)
}

impl SizeFilter {
    /// Parses the script/CLI vocabulary `[<|>]<n>[KB|MB|GB|TB]` (D-37: binary units, KB = 1024; `none` is handled
    /// by the caller). Units are case-insensitive; `B`, `K`/`M`/`G` and `KiB`…`TiB` are accepted as well. Without
    /// an operator the filter keeps files **smaller** than the bound (`Less`).
    pub fn parse(s: &str) -> Result<SizeFilter, FolderError> {
        let invalid = |why: &str| FolderError::InvalidRules(format!("size filter {s:?}: {why}"));
        let (cmp, rest) = split_cmp(s);
        let (digits, unit) = split_digits(rest);
        if digits.is_empty() {
            return Err(invalid("expected [<|>]<n>[KB|MB|GB|TB]"));
        }
        let n: u64 = digits.parse().map_err(|_| invalid("number out of range"))?;
        let unit = unit.trim().to_ascii_lowercase();
        let factor = SIZE_UNITS
            .iter()
            .find(|(name, _)| *name == unit)
            .map(|(_, factor)| *factor)
            .ok_or_else(|| invalid("unknown unit (use B, KB, MB, GB or TB)"))?;
        let bytes = n
            .checked_mul(factor)
            .ok_or_else(|| invalid("size out of range"))?;
        Ok(SizeFilter {
            cmp: cmp.unwrap_or(Cmp::Less),
            bytes,
        })
    }
}

impl DateFilter {
    /// Parses `[<|>](<YYYY-MM-DD>|<RFC 3339>|<n>days)`. A bare date is local midnight in the zone of `zone` (the
    /// adapter's zone, injected; core never reads it). `<n>days` (also `<n>day`) is [`DateCutoff::DaysAgo`].
    /// Without an operator the filter keeps files modified **after** the cutoff (`Greater`, "newer than").
    pub fn parse(s: &str, zone: &jiff::tz::TimeZone) -> Result<DateFilter, FolderError> {
        let invalid = |why: String| FolderError::InvalidRules(format!("date filter {s:?}: {why}"));
        let (cmp, rest) = split_cmp(s);
        let cmp = cmp.unwrap_or(Cmp::Greater);
        let lower = rest.to_ascii_lowercase();
        let (digits, unit) = split_digits(&lower);
        if !digits.is_empty() && matches!(unit.trim(), "days" | "day") {
            let days: u32 = digits
                .parse()
                .map_err(|_| invalid("number of days out of range".into()))?;
            return Ok(DateFilter {
                cmp,
                cutoff: DateCutoff::DaysAgo { days },
            });
        }
        if rest.is_empty() {
            return Err(invalid(
                "expected [<|>](YYYY-MM-DD|RFC 3339 time|<n>days)".into(),
            ));
        }
        if rest.len() == 10 && rest.as_bytes().get(4) == Some(&b'-') {
            let date: jiff::civil::Date = rest
                .parse()
                .map_err(|e: jiff::Error| invalid(format!("invalid date: {e}")))?;
            let zoned = date
                .to_zoned(zone.clone())
                .map_err(|e| invalid(format!("invalid date in this time zone: {e}")))?;
            return Ok(DateFilter {
                cmp,
                cutoff: DateCutoff::Absolute {
                    at: file_time(zoned.timestamp()),
                },
            });
        }
        let at = FileTime::parse(rest).map_err(|e| invalid(e.to_string()))?;
        Ok(DateFilter {
            cmp,
            cutoff: DateCutoff::Absolute { at },
        })
    }
}

/// A jiff instant as a `FileTime` (exact: every jiff instant is representable).
pub(crate) fn file_time(ts: jiff::Timestamp) -> FileTime {
    let nanos = ts.as_nanosecond();
    let secs = nanos.div_euclid(1_000_000_000);
    let sub = nanos.rem_euclid(1_000_000_000);
    let secs = i64::try_from(secs).unwrap_or(if secs < 0 { i64::MIN } else { i64::MAX });
    let sub = u32::try_from(sub).unwrap_or(0);
    FileTime::from_unix(secs, sub).unwrap_or(FileTime::from_unix(secs, 0).unwrap_or_else(epoch))
}

/// The Unix epoch (fallback that cannot fail).
fn epoch() -> FileTime {
    FileTime::from_unix(0, 0).unwrap_or_else(epoch_fallback)
}

/// Unreachable fallback of [`epoch`]: `from_unix(0, 0)` is always valid; this keeps the code total (T9).
fn epoch_fallback() -> FileTime {
    FileTime::from_system_time(std::time::UNIX_EPOCH)
}

/// Other filters (FC-053 subset in M1: size, date, exclude protected).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct OtherFilters {
    /// Exclude items with both SYSTEM and HIDDEN attributes. Default true (no effect without DOS attributes).
    pub exclude_protected: bool,
    /// Size filter. Default `None` (omitted from TOML).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<SizeFilter>,
    /// Date filter. Default `None` (omitted from TOML).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<DateFilter>,
}

impl Default for OtherFilters {
    fn default() -> OtherFilters {
        OtherFilters {
            exclude_protected: true,
            size: None,
            date: None,
        }
    }
}

/// Filters.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FilterRules {
    /// Name filters (FC-050).
    pub names: NameFilterLists,
    /// Other filters.
    pub other: OtherFilters,
}

/// Folder handling (FC-012…FC-014).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct Handling {
    /// FC-013 follow symbolic links. Default false.
    pub follow_symlinks: bool,
    /// FC-014 ignore folder structure. Default false.
    pub flatten: bool,
    /// Scan orphan subfolders (headless: always useful for complete statistics). Default true.
    pub scan_orphan_folders: bool,
}

impl Default for Handling {
    fn default() -> Handling {
        Handling {
            follow_symlinks: false,
            flatten: false,
            scan_orphan_folders: true,
        }
    }
}

/// A patch value that can also clear an optional rule (serialised `"clear"` or `{"set": …}`), so that "clear"
/// survives JSON transport (IPC, D-50), unlike `Option<Option<T>>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Change<T> {
    /// Remove the optional rule.
    Clear,
    /// Set the optional rule.
    Set(T),
}

impl<T> Change<T> {
    /// `Clear` → `None`, `Set(v)` → `Some(v)`.
    pub fn into_option(self) -> Option<T> {
        match self {
            Change::Clear => None,
            Change::Set(value) => Some(value),
        }
    }
}

/// CLI/IPC overrides (`None` = keep). Applied after `--rules-file` and `--criteria` (§3.5). Serialised with
/// `skip_serializing_if = "Option::is_none"` on every field.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FolderRulesPatch {
    /// `criteria.size`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<bool>,
    /// `criteria.timestamps.enabled`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamps_enabled: Option<bool>,
    /// `criteria.timestamps.tolerance_secs`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tolerance_secs: Option<u32>,
    /// `criteria.timestamps.ignore_dst`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ignore_dst: Option<bool>,
    /// `criteria.timestamps.ignore_timezone`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ignore_timezone: Option<bool>,
    /// `criteria.timestamps.offset`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tz_offset: Option<Change<TzOffset>>,
    /// `criteria.filename_case`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename_case: Option<bool>,
    /// `criteria.dos_attributes`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dos_attributes: Option<DosAttrs>,
    /// `criteria.unix`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unix: Option<UnixCriteria>,
    /// `criteria.content.method`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_method: Option<ContentMethod>,
    /// `criteria.content.skip_if_quick_same`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip_if_quick_same: Option<bool>,
    /// `criteria.content.override_quick`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub override_quick: Option<bool>,
    /// `alignment.case`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub case: Option<CaseMode>,
    /// `alignment.align_extensions`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub align_extensions: Option<bool>,
    /// `alignment.align_normalization`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub align_normalization: Option<bool>,
    /// Toolbar syntax (FC-050); replaces `filters.names` with the parsed lists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name_filter: Option<String>,
    /// `filters.other.exclude_protected`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude_protected: Option<bool>,
    /// `filters.other.size`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_filter: Option<Change<SizeFilter>>,
    /// `filters.other.date`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_filter: Option<Change<DateFilter>>,
    /// `handling.follow_symlinks`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub follow_symlinks: Option<bool>,
    /// `handling.flatten`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flatten: Option<bool>,
    /// `handling.scan_orphan_folders`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scan_orphan_folders: Option<bool>,
}

/// Mask options used to validate and convert name filters. Mask syntax does not depend on case sensitivity;
/// the scanner compiles the filters again with the resolved case (§3.3 step b).
const SYNTAX_ONLY: MaskOptions = MaskOptions {
    case: CaseSensitivity::Sensitive,
};

impl FolderRules {
    /// Parses a rules file (D-31): a `schema_version` newer than [`FOLDER_RULES_SCHEMA_VERSION`] is
    /// [`FolderError::Schema`]; unknown keys produce `unknown_config_key` warnings; the values are validated
    /// ([`FolderRules::validate`]). TOML syntax and type errors are [`FolderError::InvalidRules`].
    pub fn from_toml(text: &str) -> Result<(FolderRules, Vec<Warning>), FolderError> {
        let table: Table = toml::from_str(text)
            .map_err(|e| FolderError::InvalidRules(format!("invalid TOML: {}", one_line(&e))))?;
        if let Some(Value::Integer(found)) = table.get("schema_version")
            && *found >= 0
        {
            let found = u32::try_from(*found).unwrap_or(u32::MAX);
            check_schema_version(found, FOLDER_RULES_SCHEMA_VERSION)?;
        }
        let mut warnings = Vec::new();
        unknown_keys(&table, &mut String::new(), &mut warnings);
        let rules: FolderRules = Value::Table(table)
            .try_into()
            .map_err(|e: toml::de::Error| {
                FolderError::InvalidRules(format!("invalid folder rules: {}", one_line(&e)))
            })?;
        rules.validate()?;
        Ok((rules, warnings))
    }

    /// Canonical TOML (round-trips: INV-RULES-2). Keys follow the declaration order; `None` options are
    /// omitted.
    pub fn to_toml(&self) -> String {
        match toml::to_string(self) {
            Ok(text) => text,
            // Every field is representable in TOML (`SizeFilter::bytes` above `i64::MAX` is written as a string),
            // so this arm is unreachable; it keeps the function total (T9).
            Err(e) => format!("# cannot serialise folder rules: {e}\n"),
        }
    }

    /// Applies a patch (validates `name_filter` and ranges). On error `self` is unchanged.
    pub fn apply(&mut self, patch: &FolderRulesPatch) -> Result<(), FolderError> {
        let mut next = self.clone();
        let c = &mut next.criteria;
        set(&mut c.size, patch.size);
        set(&mut c.timestamps.enabled, patch.timestamps_enabled);
        set(&mut c.timestamps.tolerance_secs, patch.tolerance_secs);
        set(&mut c.timestamps.ignore_dst, patch.ignore_dst);
        set(&mut c.timestamps.ignore_timezone, patch.ignore_timezone);
        if let Some(change) = patch.tz_offset {
            c.timestamps.offset = change.into_option();
        }
        set(&mut c.filename_case, patch.filename_case);
        set(&mut c.dos_attributes, patch.dos_attributes);
        set(&mut c.unix, patch.unix);
        set(&mut c.content.method, patch.content_method);
        set(&mut c.content.skip_if_quick_same, patch.skip_if_quick_same);
        set(&mut c.content.override_quick, patch.override_quick);
        let a = &mut next.alignment;
        set(&mut a.case, patch.case);
        set(&mut a.align_extensions, patch.align_extensions);
        set(&mut a.align_normalization, patch.align_normalization);
        if let Some(filter) = &patch.name_filter {
            next.filters.names = NameFilter::parse(filter, SYNTAX_ONLY)?.to_lists();
        }
        let o = &mut next.filters.other;
        set(&mut o.exclude_protected, patch.exclude_protected);
        if let Some(change) = patch.size_filter {
            o.size = change.into_option();
        }
        if let Some(change) = patch.date_filter {
            o.date = change.into_option();
        }
        let h = &mut next.handling;
        set(&mut h.follow_symlinks, patch.follow_symlinks);
        set(&mut h.flatten, patch.flatten);
        set(&mut h.scan_orphan_folders, patch.scan_orphan_folders);
        next.validate()?;
        *self = next;
        Ok(())
    }

    /// Validates the rules: the schema version is supported, the name filters compile and the time-zone offset
    /// names Left or Right with hours in −12..=12. Additive API (called by [`FolderRules::from_toml`],
    /// [`FolderRules::apply`] and `FolderModel::new`).
    pub fn validate(&self) -> Result<(), FolderError> {
        check_schema_version(self.schema_version, FOLDER_RULES_SCHEMA_VERSION)?;
        if let Some(offset) = self.criteria.timestamps.offset {
            if !matches!(offset.side, Side::Left | Side::Right) {
                return Err(FolderError::InvalidRules(format!(
                    "time zone offset side must be left or right, not {}",
                    offset.side
                )));
            }
            if !(-MAX_OFFSET_HOURS..=MAX_OFFSET_HOURS).contains(&offset.hours) {
                return Err(FolderError::InvalidRules(format!(
                    "time zone offset {} h is outside -12..=12",
                    offset.hours
                )));
            }
        }
        NameFilter::from_lists(&self.filters.names, SYNTAX_ONLY)?;
        Ok(())
    }
}

/// `*target = value` when `value` is `Some`.
fn set<T>(target: &mut T, value: Option<T>) {
    if let Some(value) = value {
        *target = value;
    }
}

/// A parser message in one line (TOML errors carry a multi-line source excerpt).
fn one_line(e: &dyn fmt::Display) -> String {
    e.to_string()
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Known keys of each table of a rules file (dotted paths).
fn known_keys(path: &str) -> Option<&'static [&'static str]> {
    Some(match path {
        "" => &[
            "schema_version",
            "criteria",
            "alignment",
            "filters",
            "handling",
        ],
        "criteria" => &[
            "size",
            "timestamps",
            "filename_case",
            "dos_attributes",
            "unix",
            "content",
        ],
        "criteria.timestamps" => &[
            "enabled",
            "tolerance_secs",
            "ignore_dst",
            "ignore_timezone",
            "offset",
        ],
        "criteria.timestamps.offset" => &["side", "hours"],
        "criteria.unix" => &["permissions", "owner", "group"],
        "criteria.content" => &["method", "skip_if_quick_same", "override_quick"],
        "alignment" => &["case", "align_extensions", "align_normalization"],
        "filters" => &["names", "other"],
        "filters.names" => &[
            "include_files",
            "exclude_files",
            "include_folders",
            "exclude_folders",
        ],
        "filters.other" => &["exclude_protected", "size", "date"],
        "filters.other.size" => &["cmp", "bytes"],
        "filters.other.date" => &["cmp", "cutoff"],
        "filters.other.date.cutoff" => &["type", "at", "days"],
        "handling" => &["follow_symlinks", "flatten", "scan_orphan_folders"],
        _ => return None,
    })
}

/// Pushes an `unknown_config_key` warning for every key of `table` (at dotted `path`) that the rules do not
/// know, recursing into known sub-tables. Keys are visited in the table's (sorted) order.
fn unknown_keys(table: &Table, path: &mut String, out: &mut Vec<Warning>) {
    let Some(known) = known_keys(path) else {
        return;
    };
    for (key, value) in table {
        let len = path.len();
        if !path.is_empty() {
            path.push('.');
        }
        path.push_str(key);
        if !known.contains(&key.as_str()) {
            out.push(Warning {
                code: WarningCode::UnknownConfigKey,
                message: format!("folder rules: unknown key `{path}`"),
                spec: None,
            });
        } else if let Value::Table(sub) = value {
            unknown_keys(sub, path, out);
        }
        path.truncate(len);
    }
}

impl Criteria {
    /// Parses the script `criteria` vocabulary (D-57, §2.6.6). Shared by `--criteria` and (M2) scripts.
    ///
    /// Items are separated by `,` and ASCII whitespace; empty items are ignored; words match ASCII
    /// case-insensitively. Listed criteria are on; unlisted quick tests and the content method are off. Only the
    /// documented spellings are accepted (one vocabulary for the CLI and scripts, T11): the tolerance unit is
    /// `sec` (`timestamp:3sec`), the offset sides are `left` and `right` (`timezone:left+2`).
    pub fn parse_list(list: &str) -> Result<CriteriaSpec, FolderError> {
        let mut patch = FolderRulesPatch {
            size: Some(false),
            timestamps_enabled: Some(false),
            tolerance_secs: Some(2),
            ignore_dst: Some(false),
            ignore_timezone: Some(false),
            tz_offset: Some(Change::Clear),
            filename_case: Some(false),
            dos_attributes: Some(DosAttrs::empty()),
            unix: Some(UnixCriteria::default()),
            content_method: Some(ContentMethod::None),
            ..FolderRulesPatch::default()
        };
        let mut unix = UnixCriteria::default();
        let mut method: Option<ContentMethod> = None;
        let mut ignore_unimportant = false;
        let words = list
            .split(|c: char| c == ',' || c.is_ascii_whitespace())
            .filter(|w| !w.is_empty());
        for word in words {
            let lower = word.to_ascii_lowercase();
            let unknown = || FolderError::InvalidRules(format!("unknown criterion `{word}`"));
            match lower.as_str() {
                "size" => patch.size = Some(true),
                "owner" => unix.owner = true,
                "group" => unix.group = true,
                "permissions" => unix.permissions = true,
                "filename-case" => patch.filename_case = Some(true),
                "follow-symlinks" => patch.follow_symlinks = Some(true),
                "ignore-unimportant" => ignore_unimportant = true,
                "version" => {
                    return Err(FolderError::InvalidRules(
                        "criterion `version` (FC-028) is not available in this version".into(),
                    ));
                }
                "crc" | "binary" | "rules-based" | "rules" => {
                    let m = match lower.as_str() {
                        "crc" => ContentMethod::Crc,
                        "binary" => ContentMethod::Binary,
                        _ => ContentMethod::Rules,
                    };
                    if let Some(previous) = method {
                        return Err(FolderError::InvalidRules(format!(
                            "criteria: two content methods (`{}` and `{word}`)",
                            previous.as_str()
                        )));
                    }
                    method = Some(m);
                }
                _ => {
                    if let Some(rest) = lower.strip_prefix("timestamp") {
                        parse_timestamp_word(rest, &mut patch).ok_or_else(unknown)?;
                    } else if let Some(rest) = lower.strip_prefix("timezone:") {
                        parse_timezone_word(rest, &mut patch).ok_or_else(unknown)?;
                    } else if lower.starts_with("attrib:") {
                        let letters = word.get("attrib:".len()..).unwrap_or("");
                        let attrs = DosAttrs::parse_letters(letters).map_err(|e| {
                            FolderError::InvalidRules(format!("criterion `{word}`: {e}"))
                        })?;
                        patch.dos_attributes = Some(attrs);
                    } else {
                        return Err(unknown());
                    }
                }
            }
        }
        patch.unix = Some(unix);
        patch.content_method = Some(method.unwrap_or(ContentMethod::None));
        Ok(CriteriaSpec {
            patch,
            ignore_unimportant,
        })
    }
}

/// `timestamp[:<n>sec][;IgnoreDST]` after the word `timestamp` (lower case). `None` when malformed.
fn parse_timestamp_word(rest: &str, patch: &mut FolderRulesPatch) -> Option<()> {
    let (tolerance, flags) = match rest.strip_prefix(':') {
        Some(spec) => {
            let (number, tail) = spec
                .split_once(';')
                .map_or((spec, None), |(a, b)| (a, Some(b)));
            let (digits, unit) = split_digits(number);
            // Only the documented unit (§2.6.6, docs/12 §3.1): one vocabulary for the CLI and scripts (T11).
            if digits.is_empty() || unit != "sec" {
                return None;
            }
            (Some(digits.parse::<u32>().ok()?), tail)
        }
        None if rest.is_empty() => (None, None),
        None => (None, Some(rest.strip_prefix(';')?)),
    };
    let ignore_dst = match flags {
        None => false,
        Some("ignoredst") => true,
        Some(_) => return None,
    };
    patch.timestamps_enabled = Some(true);
    if let Some(tolerance) = tolerance {
        patch.tolerance_secs = Some(tolerance);
    }
    if ignore_dst {
        patch.ignore_dst = Some(true);
    }
    Some(())
}

/// `timezone:(ignore|(left|right)(+|-)<0..12>)` after `timezone:` (lower case). `None` when malformed.
fn parse_timezone_word(rest: &str, patch: &mut FolderRulesPatch) -> Option<()> {
    if rest == "ignore" {
        patch.timestamps_enabled = Some(true);
        patch.ignore_timezone = Some(true);
        return Some(());
    }
    let (side, signed) = [("left", Side::Left), ("right", Side::Right)]
        .into_iter()
        .find_map(|(word, side)| rest.strip_prefix(word).map(|r| (side, r)))?;
    let (negative, digits) = if let Some(d) = signed.strip_prefix('+') {
        (false, d)
    } else {
        (true, signed.strip_prefix('-')?)
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let hours: i8 = digits.parse().ok()?;
    if hours > MAX_OFFSET_HOURS {
        return None;
    }
    patch.timestamps_enabled = Some(true);
    patch.tz_offset = Some(Change::Set(TzOffset {
        side,
        hours: if negative { -hours } else { hours },
    }));
    Some(())
}

/// Serde for `u64` values in TOML: an integer up to `i64::MAX`, a decimal string above it; both forms (and
/// unsigned integers from JSON) are read back.
mod u64_text {
    use super::*;

    pub(super) fn serialize<S: Serializer>(value: &u64, s: S) -> Result<S::Ok, S::Error> {
        match i64::try_from(*value) {
            Ok(v) => s.serialize_i64(v),
            Err(_) => s.serialize_str(&value.to_string()),
        }
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
        d.deserialize_any(U64Visitor)
    }

    struct U64Visitor;

    impl Visitor<'_> for U64Visitor {
        type Value = u64;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a non-negative integer (or its decimal text)")
        }

        fn visit_u64<E: de::Error>(self, v: u64) -> Result<u64, E> {
            Ok(v)
        }

        fn visit_i64<E: de::Error>(self, v: i64) -> Result<u64, E> {
            u64::try_from(v).map_err(|_| E::custom("negative size"))
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<u64, E> {
            v.parse().map_err(|_| E::custom("invalid size text"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn patch_of(list: &str) -> FolderRulesPatch {
        Criteria::parse_list(list).unwrap().patch
    }

    #[test]
    fn defaults() {
        let r = FolderRules::default();
        assert_eq!(r.schema_version, 1);
        assert!(r.criteria.size);
        assert!(r.criteria.timestamps.enabled);
        assert_eq!(r.criteria.timestamps.tolerance_secs, 2);
        assert!(!r.criteria.timestamps.ignore_dst && !r.criteria.timestamps.ignore_timezone);
        assert_eq!(r.criteria.timestamps.offset, None);
        assert!(!r.criteria.filename_case);
        assert!(r.criteria.dos_attributes.is_empty());
        assert_eq!(r.criteria.unix, UnixCriteria::default());
        assert_eq!(r.criteria.content.method, ContentMethod::None);
        assert!(!r.criteria.content.skip_if_quick_same, "D-34");
        assert!(r.criteria.content.override_quick);
        assert_eq!(r.alignment, NameAlignment::default());
        assert_eq!(r.alignment.case, CaseMode::Auto);
        assert!(r.filters.other.exclude_protected);
        assert!(!r.handling.follow_symlinks && !r.handling.flatten);
        assert!(r.handling.scan_orphan_folders);
    }

    /// §2.6.6 table, rows 1–7.
    #[test]
    fn criteria_table() {
        // 1
        let p = patch_of("size");
        assert_eq!(p.size, Some(true));
        assert_eq!(p.timestamps_enabled, Some(false));
        assert_eq!(p.content_method, Some(ContentMethod::None));
        assert_eq!(p.follow_symlinks, None);
        // 2
        let p = patch_of("size,timestamp:3sec;IgnoreDST");
        assert_eq!(p.size, Some(true));
        assert_eq!(p.timestamps_enabled, Some(true));
        assert_eq!(p.tolerance_secs, Some(3));
        assert_eq!(p.ignore_dst, Some(true));
        // 3
        let p = patch_of("timestamp timezone:left+2 crc");
        assert_eq!(p.size, Some(false));
        assert_eq!(p.timestamps_enabled, Some(true));
        assert_eq!(p.tolerance_secs, Some(2));
        assert_eq!(
            p.tz_offset,
            Some(Change::Set(TzOffset {
                side: Side::Left,
                hours: 2
            }))
        );
        assert_eq!(p.content_method, Some(ContentMethod::Crc));
        // 4
        let spec = Criteria::parse_list("rules-based ignore-unimportant").unwrap();
        assert_eq!(spec.patch.content_method, Some(ContentMethod::Rules));
        assert_eq!(spec.patch.timestamps_enabled, Some(false));
        assert!(spec.ignore_unimportant);
        // 5
        let p = patch_of("attrib:HRS,owner");
        assert_eq!(
            p.dos_attributes,
            Some(DosAttrs::HIDDEN | DosAttrs::READONLY | DosAttrs::SYSTEM)
        );
        assert_eq!(
            p.unix,
            Some(UnixCriteria {
                permissions: false,
                owner: true,
                group: false
            })
        );
        // 6
        assert!(matches!(
            Criteria::parse_list("crc,binary"),
            Err(FolderError::InvalidRules(_))
        ));
        // 7
        let err = Criteria::parse_list("version").unwrap_err();
        assert_eq!(
            err.to_string(),
            "rules: criterion `version` (FC-028) is not available in this version"
        );
    }

    #[test]
    fn criteria_vocabulary_details() {
        let p = patch_of("");
        assert_eq!(p.size, Some(false));
        assert_eq!(p.tz_offset, Some(Change::Clear));
        assert_eq!(p.filename_case, Some(false));
        assert_eq!(p.skip_if_quick_same, None);
        let p = patch_of("TIMEZONE:Right-12, Filename-Case  Follow-Symlinks,PERMISSIONS group");
        assert_eq!(
            p.tz_offset,
            Some(Change::Set(TzOffset {
                side: Side::Right,
                hours: -12
            }))
        );
        assert_eq!(p.filename_case, Some(true));
        assert_eq!(p.follow_symlinks, Some(true));
        assert_eq!(
            p.unix,
            Some(UnixCriteria {
                permissions: true,
                owner: false,
                group: true
            })
        );
        let p = patch_of("timezone:ignore");
        assert_eq!(p.ignore_timezone, Some(true));
        assert_eq!(p.timestamps_enabled, Some(true));
        let p = patch_of("timestamp;ignoredst");
        assert_eq!(p.ignore_dst, Some(true));
        assert_eq!(p.tolerance_secs, Some(2));
        assert_eq!(
            patch_of("binary").content_method,
            Some(ContentMethod::Binary)
        );
        assert_eq!(patch_of("rules").content_method, Some(ContentMethod::Rules));
        for bad in [
            "timestamps",
            "timestamp:sec",
            "timestamp:3min",
            // Undocumented spellings are rejected (one vocabulary, T11).
            "timestamp:3",
            "timestamp:3s",
            "timestamp:3secs",
            "timezone:lt+2",
            "timezone:rt-1",
            "timestamp;dst",
            "timezone:left+13",
            "timezone:up+1",
            "timezone:left",
            "attrib:Q",
            "md5",
            "crc rules",
        ] {
            assert!(
                matches!(Criteria::parse_list(bad), Err(FolderError::InvalidRules(_))),
                "{bad}"
            );
        }
        assert_eq!(
            Criteria::parse_list("frobnicate").unwrap_err().to_string(),
            "rules: unknown criterion `frobnicate`"
        );
    }

    #[test]
    fn size_filter_parsing() {
        let cases = [
            ("<10MB", Cmp::Less, 10 << 20),
            (">1kb", Cmp::Greater, 1024),
            ("5", Cmp::Less, 5),
            (" > 2 GB ", Cmp::Greater, 2 << 30),
            ("3TB", Cmp::Less, 3 << 40),
            ("7b", Cmp::Less, 7),
            ("1KiB", Cmp::Less, 1024),
        ];
        for (text, cmp, bytes) in cases {
            assert_eq!(
                SizeFilter::parse(text).unwrap(),
                SizeFilter { cmp, bytes },
                "{text}"
            );
        }
        for bad in [
            "",
            "<",
            "MB",
            "10XB",
            "-5",
            "99999999999999999999",
            "16777216TB",
        ] {
            assert!(
                matches!(SizeFilter::parse(bad), Err(FolderError::InvalidRules(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn date_filter_parsing() {
        let utc = jiff::tz::TimeZone::UTC;
        assert_eq!(
            DateFilter::parse("<30days", &utc).unwrap(),
            DateFilter {
                cmp: Cmp::Less,
                cutoff: DateCutoff::DaysAgo { days: 30 }
            }
        );
        assert_eq!(
            DateFilter::parse("1DAY", &utc).unwrap().cutoff,
            DateCutoff::DaysAgo { days: 1 }
        );
        assert_eq!(DateFilter::parse("1day", &utc).unwrap().cmp, Cmp::Greater);
        let at = |s: &str| FileTime::parse(s).unwrap();
        assert_eq!(
            DateFilter::parse(">2026-10-19", &utc).unwrap(),
            DateFilter {
                cmp: Cmp::Greater,
                cutoff: DateCutoff::Absolute {
                    at: at("2026-10-19T00:00:00Z")
                }
            }
        );
        // A bare date is local midnight in the injected zone.
        let fixed = jiff::tz::TimeZone::fixed(jiff::tz::offset(2));
        assert_eq!(
            DateFilter::parse("2026-10-19", &fixed).unwrap().cutoff,
            DateCutoff::Absolute {
                at: at("2026-10-18T22:00:00Z")
            }
        );
        // A zone with daylight saving time, built without the system tz database (T1): central European rules.
        let cet = jiff::tz::TimeZone::posix("CET-1CEST,M3.5.0,M10.5.0/3").unwrap();
        assert_eq!(
            DateFilter::parse("2026-07-01", &cet).unwrap().cutoff,
            DateCutoff::Absolute {
                at: at("2026-06-30T22:00:00Z")
            }
        );
        assert_eq!(
            DateFilter::parse("2026-01-01", &cet).unwrap().cutoff,
            DateCutoff::Absolute {
                at: at("2025-12-31T23:00:00Z")
            }
        );
        assert_eq!(
            DateFilter::parse("<2026-10-19T13:12:00.5+02:00", &utc)
                .unwrap()
                .cutoff,
            DateCutoff::Absolute {
                at: at("2026-10-19T11:12:00.5Z")
            }
        );
        for bad in [
            "",
            "<",
            "yesterday",
            "2026-13-01",
            "30weeks",
            "99999999999days",
        ] {
            assert!(
                matches!(
                    DateFilter::parse(bad, &utc),
                    Err(FolderError::InvalidRules(_))
                ),
                "{bad}"
            );
        }
    }

    #[test]
    fn change_json_round_trip() {
        let clear: Change<SizeFilter> = Change::Clear;
        assert_eq!(serde_json::to_string(&clear).unwrap(), "\"clear\"");
        let set = Change::Set(SizeFilter {
            cmp: Cmp::Less,
            bytes: 5,
        });
        let json = serde_json::to_string(&set).unwrap();
        assert_eq!(json, r#"{"set":{"cmp":"less","bytes":5}}"#);
        assert_eq!(
            serde_json::from_str::<Change<SizeFilter>>(&json).unwrap(),
            set
        );
        assert_eq!(
            serde_json::from_str::<Change<SizeFilter>>("\"clear\"").unwrap(),
            clear
        );
        let patch = FolderRulesPatch {
            tz_offset: Some(Change::Clear),
            date_filter: Some(Change::Set(DateFilter {
                cmp: Cmp::Greater,
                cutoff: DateCutoff::DaysAgo { days: 3 },
            })),
            ..FolderRulesPatch::default()
        };
        let json = serde_json::to_string(&patch).unwrap();
        assert_eq!(
            json,
            r#"{"tz_offset":"clear","date_filter":{"set":{"cmp":"greater","cutoff":{"type":"days_ago","days":3}}}}"#
        );
        assert_eq!(
            serde_json::from_str::<FolderRulesPatch>(&json).unwrap(),
            patch
        );
        assert_eq!(
            serde_json::to_string(&FolderRulesPatch::default()).unwrap(),
            "{}"
        );
    }

    #[test]
    fn toml_round_trip_and_shape() {
        let mut rules = FolderRules::default();
        rules.criteria.timestamps.offset = Some(TzOffset {
            side: Side::Right,
            hours: -3,
        });
        rules.criteria.dos_attributes = DosAttrs::HIDDEN | DosAttrs::SYSTEM;
        rules.filters.names = NameFilter::parse(r"*.c;-obj\", SYNTAX_ONLY)
            .unwrap()
            .to_lists();
        rules.filters.other.size = Some(SizeFilter {
            cmp: Cmp::Greater,
            bytes: u64::MAX,
        });
        rules.filters.other.date = Some(DateFilter {
            cmp: Cmp::Less,
            cutoff: DateCutoff::Absolute {
                at: FileTime::parse("2026-10-19T13:12:00.5Z").unwrap(),
            },
        });
        let text = rules.to_toml();
        assert!(text.contains("dos_attributes = \"HS\""), "{text}");
        assert!(text.contains("bytes = \"18446744073709551615\""), "{text}");
        let (back, warnings) = FolderRules::from_toml(&text).unwrap();
        assert_eq!(back, rules);
        assert!(warnings.is_empty());
        let default_text = FolderRules::default().to_toml();
        assert!(!default_text.contains("offset"), "{default_text}");
        assert_eq!(
            FolderRules::from_toml(&default_text).unwrap().0,
            FolderRules::default()
        );
        assert_eq!(
            FolderRules::from_toml("").unwrap().0,
            FolderRules::default()
        );
    }

    #[test]
    fn toml_errors_and_warnings() {
        let (rules, warnings) = FolderRules::from_toml(
            "colour = 1\n[criteria]\nsize = false\nspeed = 3\n[criteria.timestamps]\nwhen = 1\n[x]\ny = 2\n",
        )
        .unwrap();
        assert!(!rules.criteria.size);
        let keys: Vec<&str> = warnings.iter().map(|w| w.message.as_str()).collect();
        assert_eq!(
            keys,
            [
                "folder rules: unknown key `colour`",
                "folder rules: unknown key `criteria.speed`",
                "folder rules: unknown key `criteria.timestamps.when`",
                "folder rules: unknown key `x`",
            ]
        );
        assert!(
            warnings
                .iter()
                .all(|w| w.code == WarningCode::UnknownConfigKey)
        );
        assert!(matches!(
            FolderRules::from_toml("schema_version = 2\nnew_thing = true"),
            Err(FolderError::Schema(_))
        ));
        assert!(matches!(
            FolderRules::from_toml("schema_version = 99999999999"),
            Err(FolderError::Schema(_))
        ));
        assert!(matches!(
            FolderRules::from_toml("[criteria]\nsize = 3"),
            Err(FolderError::InvalidRules(_))
        ));
        assert!(matches!(
            FolderRules::from_toml("= broken"),
            Err(FolderError::InvalidRules(_))
        ));
        assert!(matches!(
            FolderRules::from_toml("[filters.names]\ninclude_files = [\"[a\"]"),
            Err(FolderError::Filter(_))
        ));
        assert!(matches!(
            FolderRules::from_toml("[criteria.timestamps.offset]\nside = \"center\"\nhours = 1"),
            Err(FolderError::InvalidRules(_))
        ));
        assert!(matches!(
            FolderRules::from_toml("[criteria.timestamps.offset]\nside = \"left\"\nhours = 13"),
            Err(FolderError::InvalidRules(_))
        ));
        assert!(matches!(
            FolderRules::from_toml("[criteria]\ndos_attributes = \"Q\""),
            Err(FolderError::InvalidRules(_))
        ));
    }

    #[test]
    fn apply_patches() {
        let mut rules = FolderRules::default();
        let spec = Criteria::parse_list("size,timestamp:5sec timezone:left-1 crc follow-symlinks")
            .unwrap();
        rules.apply(&spec.patch).unwrap();
        assert!(rules.criteria.size);
        assert_eq!(rules.criteria.timestamps.tolerance_secs, 5);
        assert_eq!(
            rules.criteria.timestamps.offset,
            Some(TzOffset {
                side: Side::Left,
                hours: -1
            })
        );
        assert_eq!(rules.criteria.content.method, ContentMethod::Crc);
        assert!(rules.handling.follow_symlinks);
        rules
            .apply(&FolderRulesPatch {
                tz_offset: Some(Change::Clear),
                name_filter: Some(r"*.rs;-target\".into()),
                size_filter: Some(Change::Set(SizeFilter {
                    cmp: Cmp::Less,
                    bytes: 3,
                })),
                flatten: Some(true),
                case: Some(CaseMode::Insensitive),
                ..FolderRulesPatch::default()
            })
            .unwrap();
        assert_eq!(rules.criteria.timestamps.offset, None);
        assert_eq!(rules.filters.names.include_files, ["*.rs"]);
        assert_eq!(rules.filters.names.exclude_folders, [r"target\"]);
        assert!(rules.filters.other.size.is_some());
        assert!(rules.handling.flatten);
        assert_eq!(rules.alignment.case, CaseMode::Insensitive);
        let before = rules.clone();
        let bad = FolderRulesPatch {
            size: Some(false),
            name_filter: Some("[z-a]".into()),
            ..FolderRulesPatch::default()
        };
        assert!(matches!(rules.apply(&bad), Err(FolderError::Filter(_))));
        assert_eq!(rules, before, "a failed patch leaves the rules unchanged");
        let bad = FolderRulesPatch {
            tz_offset: Some(Change::Set(TzOffset {
                side: Side::Output,
                hours: 1,
            })),
            ..FolderRulesPatch::default()
        };
        assert!(matches!(
            rules.apply(&bad),
            Err(FolderError::InvalidRules(_))
        ));
    }

    #[test]
    fn cmp_holds() {
        assert!(Cmp::Less.holds(1, 2));
        assert!(!Cmp::Less.holds(2, 2));
        assert!(Cmp::Greater.holds(3, 2));
        assert!(!Cmp::Greater.holds(2, 2));
    }

    #[test]
    fn file_time_conversion() {
        let ts: jiff::Timestamp = "1969-12-31T23:59:59.5Z".parse().unwrap();
        assert_eq!(file_time(ts), FileTime::from_unix(-1, 500_000_000).unwrap());
        assert_eq!(epoch(), FileTime::from_unix(0, 0).unwrap());
        assert_eq!(epoch_fallback(), epoch());
    }
}
