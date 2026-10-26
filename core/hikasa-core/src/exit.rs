//! Process exit codes (CLI-004) and the verdict rows of the mapping table §2.21 (D-01, D-02, D-03).

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::content::{ContentKind, ContentResult};
use crate::error::{Classify, ErrorKind};

/// Process exit codes (CLI-004), shared by every subcommand. Mapping rules: §2.21.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(into = "u8", try_from = "u8")]
#[repr(u8)]
pub enum ExitCode {
    /// 0: success.
    Success = 0,
    /// 1: binary same.
    BinarySame = 1,
    /// 2: rules-based same.
    RulesSame = 2,
    /// 11: binary differences.
    BinaryDifferent = 11,
    /// 12: similar (only unimportant differences).
    Similar = 12,
    /// 13: rules-based differences.
    RulesDifferent = 13,
    /// 14: conflicts detected.
    Conflicts = 14,
    /// 100: unknown error (any error of a file comparison, D-01).
    UnknownError = 100,
    /// 101: conflicts detected, merge output not written.
    ConflictsNotWritten = 101,
    /// 102: the CLI helper is unable to wait for the GUI.
    CannotWaitForGui = 102,
    /// 103: the CLI helper cannot find the GUI executable.
    GuiNotFound = 103,
    /// 105: error loading the script file.
    ScriptLoadError = 105,
    /// 106: script syntax error.
    ScriptSyntaxError = 106,
    /// 107: failed to load folders or files (a base folder that cannot be loaded).
    LoadFailed = 107,
}

/// A number that is not one of the [`ExitCode`] values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("unknown exit code {0}")]
pub struct UnknownExitCode(pub u8);

impl Classify for UnknownExitCode {
    /// Always `invalid_data`.
    fn kind(&self) -> ErrorKind {
        ErrorKind::InvalidData
    }
}

impl ExitCode {
    /// Every exit code, in numeric order.
    pub const ALL: [ExitCode; 14] = [
        ExitCode::Success,
        ExitCode::BinarySame,
        ExitCode::RulesSame,
        ExitCode::BinaryDifferent,
        ExitCode::Similar,
        ExitCode::RulesDifferent,
        ExitCode::Conflicts,
        ExitCode::UnknownError,
        ExitCode::ConflictsNotWritten,
        ExitCode::CannotWaitForGui,
        ExitCode::GuiNotFound,
        ExitCode::ScriptLoadError,
        ExitCode::ScriptSyntaxError,
        ExitCode::LoadFailed,
    ];

    /// Numeric value.
    pub const fn code(self) -> u8 {
        self as u8
    }

    /// Inverse of `code`.
    pub const fn from_code(code: u8) -> Option<ExitCode> {
        Some(match code {
            0 => ExitCode::Success,
            1 => ExitCode::BinarySame,
            2 => ExitCode::RulesSame,
            11 => ExitCode::BinaryDifferent,
            12 => ExitCode::Similar,
            13 => ExitCode::RulesDifferent,
            14 => ExitCode::Conflicts,
            100 => ExitCode::UnknownError,
            101 => ExitCode::ConflictsNotWritten,
            102 => ExitCode::CannotWaitForGui,
            103 => ExitCode::GuiNotFound,
            105 => ExitCode::ScriptLoadError,
            106 => ExitCode::ScriptSyntaxError,
            107 => ExitCode::LoadFailed,
            _ => return None,
        })
    }

    /// Verdict mapping for `quick`, `text compare` and GUI file views (§2.21, D-01): BinarySame 1, RulesSame 2,
    /// UnimportantOnly 12 (2 with `ignore_unimportant`), ImportantDifferences 13, BinaryDifferent 11, Error 100.
    pub fn for_content(result: &ContentResult, ignore_unimportant: bool) -> ExitCode {
        ExitCode::for_kind(result.kind(), ignore_unimportant)
    }

    /// Same mapping on the payload-free kind (`for_content(r, iu) == for_kind(r.kind(), iu)`).
    pub fn for_kind(kind: ContentKind, ignore_unimportant: bool) -> ExitCode {
        match kind {
            ContentKind::BinarySame => ExitCode::BinarySame,
            ContentKind::RulesSame => ExitCode::RulesSame,
            ContentKind::UnimportantOnly => {
                if ignore_unimportant {
                    ExitCode::RulesSame
                } else {
                    ExitCode::Similar
                }
            }
            ContentKind::ImportantDifferences => ExitCode::RulesDifferent,
            ContentKind::BinaryDifferent => ExitCode::BinaryDifferent,
            ContentKind::Error => ExitCode::UnknownError,
        }
    }

    /// Verdict mapping for `folder compare` (§2.21, D-02): NoDifferences 2, UnimportantOnly 12, Differences 13,
    /// Incomplete 100.
    pub fn for_verdict(verdict: CompareVerdict) -> ExitCode {
        match verdict {
            CompareVerdict::NoDifferences => ExitCode::RulesSame,
            CompareVerdict::UnimportantOnly => ExitCode::Similar,
            CompareVerdict::Differences => ExitCode::RulesDifferent,
            CompareVerdict::Incomplete => ExitCode::UnknownError,
        }
    }
}

impl From<ExitCode> for u8 {
    fn from(code: ExitCode) -> u8 {
        code.code()
    }
}

impl TryFrom<u8> for ExitCode {
    type Error = UnknownExitCode;

    fn try_from(code: u8) -> Result<ExitCode, UnknownExitCode> {
        ExitCode::from_code(code).ok_or(UnknownExitCode(code))
    }
}

impl fmt::Display for ExitCode {
    /// Writes the number.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.code())
    }
}

/// Overall verdict of a multi-item comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompareVerdict {
    /// Nothing differs.
    NoDifferences,
    /// Only unimportant differences.
    UnimportantOnly,
    /// At least one important difference.
    Differences,
    /// The comparison could not inspect everything (errors, unknown items or cancellation).
    Incomplete,
}

impl CompareVerdict {
    /// Stable snake_case name, identical to the serde form.
    pub const fn as_str(self) -> &'static str {
        match self {
            CompareVerdict::NoDifferences => "no_differences",
            CompareVerdict::UnimportantOnly => "unimportant_only",
            CompareVerdict::Differences => "differences",
            CompareVerdict::Incomplete => "incomplete",
        }
    }

    /// Inverse of `as_str` (`None` for unknown names).
    pub fn parse(s: &str) -> Option<CompareVerdict> {
        match s {
            "no_differences" => Some(CompareVerdict::NoDifferences),
            "unimportant_only" => Some(CompareVerdict::UnimportantOnly),
            "differences" => Some(CompareVerdict::Differences),
            "incomplete" => Some(CompareVerdict::Incomplete),
            _ => None,
        }
    }
}

impl fmt::Display for CompareVerdict {
    /// Writes [`CompareVerdict::as_str`].
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_round_trip() {
        for code in ExitCode::ALL {
            assert_eq!(ExitCode::from_code(code.code()), Some(code));
            assert_eq!(ExitCode::try_from(code.code()), Ok(code));
            assert_eq!(
                serde_json::to_string(&code).unwrap(),
                code.code().to_string()
            );
            assert_eq!(
                serde_json::from_str::<ExitCode>(&code.code().to_string()).unwrap(),
                code
            );
        }
        let unknown = (0..=u8::MAX)
            .filter(|&n| ExitCode::from_code(n).is_none())
            .count();
        assert_eq!(unknown, 256 - ExitCode::ALL.len());
        assert_eq!(ExitCode::try_from(104), Err(UnknownExitCode(104)));
        assert!(serde_json::from_str::<ExitCode>("104").is_err());
    }

    #[test]
    fn verdict_names() {
        for verdict in [
            CompareVerdict::NoDifferences,
            CompareVerdict::UnimportantOnly,
            CompareVerdict::Differences,
            CompareVerdict::Incomplete,
        ] {
            assert_eq!(CompareVerdict::parse(verdict.as_str()), Some(verdict));
            assert_eq!(
                serde_json::to_string(&verdict).unwrap(),
                format!("\"{}\"", verdict.as_str())
            );
        }
        assert_eq!(CompareVerdict::parse("NoDifferences"), None);
    }
}
