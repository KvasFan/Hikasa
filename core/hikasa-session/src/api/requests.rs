//! Service requests (§1.8.4): [`QuickRequest`], [`QuickMethod`], [`TextCompareRequest`],
//! [`FolderCompareRequest`], [`FormatTokensRequest`].
//!
//! Requests carry the user's words as they were typed (raw specs as `OsString`, rules file paths, vocabulary
//! words); the service parses them with the core parsers (T11), so the CLI and the GUI build the same requests.

use std::ffi::OsString;
use std::ops::RangeInclusive;

use hikasa_core::{Pair, Spec};
use hikasa_folder::FolderRulesPatch;
use hikasa_text::TextRulesPatch;
use serde::{Deserialize, Serialize};

/// Quick compare method (`--size|--crc|--binary|--rules`; default Rules, CLI-003 `--qc`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuickMethod {
    /// Sizes only (`stat`).
    Size,
    /// CRC32.
    Crc,
    /// Byte by byte.
    Binary,
    /// Rules-based (the shared quick-compare core of `RulesComparator`, §2.19).
    #[default]
    Rules,
}

impl QuickMethod {
    /// Stable snake_case name, identical to the serde form.
    pub const fn as_str(self) -> &'static str {
        match self {
            QuickMethod::Size => "size",
            QuickMethod::Crc => "crc",
            QuickMethod::Binary => "binary",
            QuickMethod::Rules => "rules",
        }
    }

    /// Parses a CLI-003 `--qc=<type>` value (`size`, `crc`, `binary`; `rules`/`rules-based`; ASCII-case-insensitive;
    /// an empty value is Rules). Additive.
    pub fn parse(word: &str) -> Option<QuickMethod> {
        match word.trim().to_ascii_lowercase().as_str() {
            "size" => Some(QuickMethod::Size),
            "crc" => Some(QuickMethod::Crc),
            "binary" => Some(QuickMethod::Binary),
            "" | "rules" | "rules-based" | "rules_based" => Some(QuickMethod::Rules),
            _ => None,
        }
    }
}

/// `hikasa quick`.
#[derive(Debug, Clone, Default)]
pub struct QuickRequest {
    /// Left spec as typed.
    pub left: OsString,
    /// Right spec as typed.
    pub right: OsString,
    /// Method.
    pub method: QuickMethod,
    /// `--ignore-unimportant`: UnimportantOnly maps to exit code 2 instead of 12.
    pub ignore_unimportant: bool,
    /// `--format <id|name>` (Rules method).
    pub format: Option<String>,
    /// `--rules-file <toml>` (Rules method).
    pub rules_file: Option<OsString>,
    /// Rule overrides from flags (Rules method), applied after the rules file.
    pub rules_patch: TextRulesPatch,
}

/// `hikasa text compare`.
#[derive(Debug, Clone, Default)]
pub struct TextCompareRequest {
    /// Left spec as typed.
    pub left: OsString,
    /// Right spec as typed.
    pub right: OsString,
    /// `--format <id|name>`.
    pub format: Option<String>,
    /// Per-side encoding overrides (TC-001): `--encoding` fills both, `--left-encoding`/`--right-encoding` one.
    pub encodings: Pair<Option<String>>,
    /// `--rules-file <toml>`.
    pub rules_file: Option<OsString>,
    /// Rule overrides from flags, applied after the rules file.
    pub rules_patch: TextRulesPatch,
    /// `--ignore-unimportant` (verdict and exit code).
    pub ignore_unimportant: bool,
}

impl TextCompareRequest {
    /// A text comparison of two specs that were already parsed, e.g. a file pair of a folder view
    /// (`TreeRead::spec(id, side)`, FC-062). Exact for local paths that are not valid UTF-8
    /// ([`spec_os_string`]). Additive (wave-4 review).
    pub fn for_specs(left: &Spec, right: &Spec) -> TextCompareRequest {
        TextCompareRequest {
            left: spec_os_string(left),
            right: spec_os_string(right),
            ..TextCompareRequest::default()
        }
    }
}

/// A spec as a request argument that the service parses back to the same spec (additive, FC-062): the exact OS
/// path of a local spec (also when it is not valid UTF-8, where `canonical()` is lossy), the canonical string
/// otherwise.
pub fn spec_os_string(spec: &Spec) -> OsString {
    match spec {
        Spec::Local(path) => path.as_path().as_os_str().to_os_string(),
        other => OsString::from(other.canonical()),
    }
}

/// `hikasa folder compare`.
#[derive(Debug, Clone, Default)]
pub struct FolderCompareRequest {
    /// Left base folder as typed.
    pub left: OsString,
    /// Right base folder as typed.
    pub right: OsString,
    /// `--rules-file <toml>`.
    pub rules_file: Option<OsString>,
    /// `--criteria` (D-57), applied after the rules file and before `patch`.
    pub criteria: Option<String>,
    /// Folder rule overrides from flags.
    pub patch: FolderRulesPatch,
    /// Rules for `--content rules` (the injected RulesComparator).
    pub text_rules_file: Option<OsString>,
    /// Text rule overrides for `--content rules`.
    pub text_patch: TextRulesPatch,
    /// `--ignore-unimportant` (verdict and exit code; also set by the criteria word `ignore-unimportant`).
    pub ignore_unimportant: bool,
}

/// `hikasa format tokens`.
#[derive(Debug, Clone, Default)]
pub struct FormatTokensRequest {
    /// The file (a spec; `-` uses the stdin name hint, D-15).
    pub file: OsString,
    /// `--format <id|name>`.
    pub format: Option<String>,
    /// `--encoding <label>`.
    pub encoding: Option<String>,
    /// 1-based inclusive line range filter for output (all lines are still tokenised for state).
    pub lines: Option<RangeInclusive<u32>>,
}
