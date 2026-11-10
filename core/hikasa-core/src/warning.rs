//! Non-fatal observations returned inside results (T8: core crates never log or print).

use std::fmt;

use serde::{Deserialize, Serialize};

/// A non-fatal observation returned inside results (T8).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Warning {
    /// Stable code.
    pub code: WarningCode,
    /// Message.
    pub message: String,
    /// Spec concerned, if any.
    pub spec: Option<String>,
}

/// Stable warning codes (JSON `warnings[].code`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum WarningCode {
    /// Content classified binary was compared by bytes (D-12).
    BinaryContent,
    /// Decoding replaced malformed sequences with U+FFFD.
    LossyDecode,
    /// A user regular expression failed at run time (backtracking budget).
    RegexRuntimeError,
    /// A comparison fell back to a binary comparison.
    FellBackToBinary,
    /// A file exceeded the rules-based comparator's size limit and was compared by bytes (D-25).
    RulesSizeLimit,
    /// Closeness matching does not apply to the chosen algorithm (D-17).
    ClosenessIgnored,
    /// The alignment work budget ran out (D-16).
    WorkBudgetExhausted,
    /// The closeness budget ran out.
    ClosenessBudgetExhausted,
    /// The two sides detect different formats (D-15).
    FormatMismatch,
    /// A requested criterion needs a capability the back-end lacks.
    CapabilityMissing,
    /// An unknown key in a configuration file (D-31).
    UnknownConfigKey,
    /// A user format file was skipped.
    UserFormatSkipped,
    /// A CR-only line ending was written as LF in a patch.
    PatchLossyEol,
    /// A symbolic link loop was detected (D-33).
    SymlinkLoop,
    /// One entry of a listing could not be read.
    EntryError,
    /// A legacy command-line switch was accepted but ignored.
    SwitchIgnored,
    /// Standard alignment exceeded its fallback budget and paired part of a gap by position (D-16).
    AlignmentDegraded,
    /// A patch was written in UTF-8 for a file stored in another encoding, so it may not apply to the file's bytes.
    PatchEncoding,
    /// A configuration value of the wrong type or out of range was replaced by its default (D-71; only the
    /// `settings` commands load leniently, every other command fails).
    InvalidConfigValue,
    /// A spec that a session cannot store (`-`, `clipboard://`) was left empty in the saved session (M2 W2,
    /// §1.2.1).
    SpecNotSaved,
    /// A workspace tab names a session that no longer exists; a Home tab takes its place (M2 W2, D-85).
    SessionMissing,
    /// A settings import kept a locked session instead of replacing or deleting it (M2 W2, D-90; `--force`
    /// replaces it).
    SessionLocked,
    /// An archive holds members that were refused and left out of the tree (M2 W5, D-141): one per index, with
    /// the total.
    ArchiveMemberRefused,
    /// An archive holds the same path twice; the last one wins (M2 W5, D-141).
    ArchiveDuplicateMember,
    /// An archive holds encrypted members, which are listed and not read (M2 W5, D-144).
    ArchiveEncrypted,
    /// A copy out of an archive skipped its link members (M2 W5, D-142).
    ArchiveSymlinkSkipped,
    /// A scan stopped opening archives: later ones stay files (M2 W5, D-151).
    ArchiveScanLimit,
    /// NUL bytes followed a compressed stream and were ignored, as `gzip` does (M2 W5, §1.5.7 rule 9).
    ArchiveTrailingZeros,
    /// The recycle bin failed and the items were deleted permanently, as chosen beforehand (M2 W6, §1.6.12).
    TrashFallback,
    /// A sync left a row alone because its side is read-only (M2 W6, D-184).
    ReadOnlySide,
    /// A sync ran on a comparison that did not finish (M2 W6, §1.6.14).
    SyncIncomplete,
    /// A report request held words its layout does not use (M2 W7, §1.7.4).
    ReportOptionUnused,
    /// A linked report's files folder was not this report's, so the next free `_files-<n>` was used (M2 W7, §1.7.9).
    ReportFolderRenamed,
    /// The previous files folder of a linked report could not be removed after the new one took its place (M2 W7,
    /// §1.7.9 step 7).
    ReportOldFolderKept,
    /// A pair report of a linked folder report failed; the folder report was written (M2 W7, §1.7.8).
    FileReportFailed,
    /// `SOURCE_DATE_EPOCH` is not a decimal count of seconds jiff can hold; the clock gave the report time (M2 W7,
    /// §1.7.12).
    SourceDateEpochInvalid,
    /// A script's `select` selected nothing (M2 W8a, §1.8.6).
    NothingSelected,
    /// `attrib` named attributes the back-end does not have (`a`, `s`, `h` on Unix modes): those items are skipped (M2
    /// W8a, D-220).
    AttribUnsupported,
    /// A script's log file is not written in a dry run (M2 W8a, §1.8.9).
    LogNotWritten,
    /// An alignment anchor overlapped or crossed an earlier one, left the lines, or lost every line to edits, and
    /// was dropped (M2 W10a, §1.10.8).
    AnchorDropped,
    /// A session's ignored lines of one side were dropped: the marked lines' text no longer has the saved CRC
    /// (the file changed; line numbers would point elsewhere) (M2 W10b, TC-024, §1.10.17).
    IgnoredLinesDropped,
    /// The file a save writes over has a new time (or size) but the content it was loaded from: it is written without a
    /// question (M2 W10b, §1.10.13).
    SourceTouched,
    /// A save encodes the whole text instead of splicing unchanged lines from the file (an encoding change, no source, an
    /// overwritten changed source, an edited stateful document; M2 W10b, §1.10.12).
    SaveReencoded,
    /// A save rewrote lines whose load replaced malformed bytes by U+FFFD: those bytes are lost (M2 W10b, §1.10.12).
    LossyLineRewritten,
    /// A save replaced characters the encoding cannot write by its substitute (`UnmappablePolicy::Replace`; M2 W10b,
    /// §1.10.12).
    UnmappableReplaced,
    /// A Find in Files stopped at its `max_matches`: the rest was not searched (M2 W10c, §1.10.19).
    FindLimitReached,
    /// A table's key occurs more than once on a side; the duplicates were paired in their order (Table Compare T-8).
    DuplicateKeys,
    /// A qualified field of a delimited table was not closed before the end of the data (Table Compare §2.2).
    UnterminatedQualifier,
    /// Characters followed a closing qualifier before the next delimiter and were kept as data (Table Compare §2.2).
    StrayAfterQualifier,
    /// A sheet of a workbook was skipped: a chart or macro sheet, or a hidden sheet left out (Table Compare §3).
    SheetSkipped,
    /// A sheet holds merged cells, compared at their top-left cell (Table Compare T-14; informational, `--verbose`).
    MergedCells,
    /// A workbook cell's formula has no cached value and compares as empty (Table Compare T-13).
    FormulaWithoutValue,
    /// A rule given for every sheet names no column of one sheet and was left out there (Table Compare T-30).
    RuleNotApplied,
    /// A table's keys do not all name columns of both sides of a sheet: it was compared without keys (Table Compare
    /// T-30, T-43).
    KeysNotApplied,
    /// Every row of a delimited table has one field and another delimiter would split it (Table Compare T-44).
    DelimiterMismatch,
}

impl WarningCode {
    /// Stable snake_case name, identical to the serde form (`"binary_content"`, …).
    pub const fn as_str(self) -> &'static str {
        match self {
            WarningCode::BinaryContent => "binary_content",
            WarningCode::LossyDecode => "lossy_decode",
            WarningCode::RegexRuntimeError => "regex_runtime_error",
            WarningCode::FellBackToBinary => "fell_back_to_binary",
            WarningCode::RulesSizeLimit => "rules_size_limit",
            WarningCode::ClosenessIgnored => "closeness_ignored",
            WarningCode::WorkBudgetExhausted => "work_budget_exhausted",
            WarningCode::ClosenessBudgetExhausted => "closeness_budget_exhausted",
            WarningCode::FormatMismatch => "format_mismatch",
            WarningCode::CapabilityMissing => "capability_missing",
            WarningCode::UnknownConfigKey => "unknown_config_key",
            WarningCode::UserFormatSkipped => "user_format_skipped",
            WarningCode::PatchLossyEol => "patch_lossy_eol",
            WarningCode::SymlinkLoop => "symlink_loop",
            WarningCode::EntryError => "entry_error",
            WarningCode::SwitchIgnored => "switch_ignored",
            WarningCode::AlignmentDegraded => "alignment_degraded",
            WarningCode::PatchEncoding => "patch_encoding",
            WarningCode::InvalidConfigValue => "invalid_config_value",
            WarningCode::SpecNotSaved => "spec_not_saved",
            WarningCode::SessionMissing => "session_missing",
            WarningCode::SessionLocked => "session_locked",
            WarningCode::ArchiveMemberRefused => "archive_member_refused",
            WarningCode::ArchiveDuplicateMember => "archive_duplicate_member",
            WarningCode::ArchiveEncrypted => "archive_encrypted",
            WarningCode::ArchiveSymlinkSkipped => "archive_symlink_skipped",
            WarningCode::ArchiveScanLimit => "archive_scan_limit",
            WarningCode::ArchiveTrailingZeros => "archive_trailing_zeros",
            WarningCode::TrashFallback => "trash_fallback",
            WarningCode::ReadOnlySide => "read_only_side",
            WarningCode::SyncIncomplete => "sync_incomplete",
            WarningCode::ReportOptionUnused => "report_option_unused",
            WarningCode::ReportFolderRenamed => "report_folder_renamed",
            WarningCode::ReportOldFolderKept => "report_old_folder_kept",
            WarningCode::FileReportFailed => "file_report_failed",
            WarningCode::SourceDateEpochInvalid => "source_date_epoch_invalid",
            WarningCode::NothingSelected => "nothing_selected",
            WarningCode::AttribUnsupported => "attrib_unsupported",
            WarningCode::LogNotWritten => "log_not_written",
            WarningCode::AnchorDropped => "anchor_dropped",
            WarningCode::IgnoredLinesDropped => "ignored_lines_dropped",
            WarningCode::SourceTouched => "source_touched",
            WarningCode::SaveReencoded => "save_reencoded",
            WarningCode::LossyLineRewritten => "lossy_line_rewritten",
            WarningCode::UnmappableReplaced => "unmappable_replaced",
            WarningCode::FindLimitReached => "find_limit_reached",
            WarningCode::DuplicateKeys => "duplicate_keys",
            WarningCode::UnterminatedQualifier => "unterminated_qualifier",
            WarningCode::StrayAfterQualifier => "stray_after_qualifier",
            WarningCode::SheetSkipped => "sheet_skipped",
            WarningCode::MergedCells => "merged_cells",
            WarningCode::FormulaWithoutValue => "formula_without_value",
            WarningCode::RuleNotApplied => "rule_not_applied",
            WarningCode::KeysNotApplied => "keys_not_applied",
            WarningCode::DelimiterMismatch => "delimiter_mismatch",
        }
    }
}

impl fmt::Display for WarningCode {
    /// Writes [`WarningCode::as_str`].
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_match_serde() {
        let all = [
            WarningCode::BinaryContent,
            WarningCode::LossyDecode,
            WarningCode::RegexRuntimeError,
            WarningCode::FellBackToBinary,
            WarningCode::RulesSizeLimit,
            WarningCode::ClosenessIgnored,
            WarningCode::WorkBudgetExhausted,
            WarningCode::ClosenessBudgetExhausted,
            WarningCode::FormatMismatch,
            WarningCode::CapabilityMissing,
            WarningCode::UnknownConfigKey,
            WarningCode::UserFormatSkipped,
            WarningCode::PatchLossyEol,
            WarningCode::SymlinkLoop,
            WarningCode::EntryError,
            WarningCode::SwitchIgnored,
            WarningCode::AlignmentDegraded,
            WarningCode::PatchEncoding,
            WarningCode::InvalidConfigValue,
            WarningCode::SpecNotSaved,
            WarningCode::SessionMissing,
            WarningCode::SessionLocked,
            WarningCode::ArchiveMemberRefused,
            WarningCode::ArchiveDuplicateMember,
            WarningCode::ArchiveEncrypted,
            WarningCode::ArchiveSymlinkSkipped,
            WarningCode::ArchiveScanLimit,
            WarningCode::ArchiveTrailingZeros,
            WarningCode::TrashFallback,
            WarningCode::ReadOnlySide,
            WarningCode::SyncIncomplete,
            WarningCode::ReportOptionUnused,
            WarningCode::ReportFolderRenamed,
            WarningCode::ReportOldFolderKept,
            WarningCode::FileReportFailed,
            WarningCode::SourceDateEpochInvalid,
            WarningCode::NothingSelected,
            WarningCode::AttribUnsupported,
            WarningCode::LogNotWritten,
            WarningCode::AnchorDropped,
            WarningCode::IgnoredLinesDropped,
            WarningCode::SourceTouched,
            WarningCode::SaveReencoded,
            WarningCode::LossyLineRewritten,
            WarningCode::UnmappableReplaced,
            WarningCode::FindLimitReached,
            WarningCode::DuplicateKeys,
            WarningCode::UnterminatedQualifier,
            WarningCode::StrayAfterQualifier,
            WarningCode::SheetSkipped,
            WarningCode::MergedCells,
            WarningCode::FormulaWithoutValue,
            WarningCode::RuleNotApplied,
            WarningCode::KeysNotApplied,
            WarningCode::DelimiterMismatch,
        ];
        for code in all {
            assert_eq!(
                serde_json::to_string(&code).unwrap(),
                format!("\"{}\"", code.as_str())
            );
            assert_eq!(code.to_string(), code.as_str());
        }
    }
}
