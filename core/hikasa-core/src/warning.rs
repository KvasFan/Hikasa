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
