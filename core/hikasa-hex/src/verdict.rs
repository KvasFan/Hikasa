//! The verdict and exit code of `hex compare` (HX-009; D-331).
//!
//! | Situation | Result | Exit |
//! |---|---|---|
//! | bytes identical | `binary_same` | 1 |
//! | bytes differ, no conversion on either side | `binary_different` | 11 |
//! | a side converted, raw bytes identical | `binary_same` | 1 |
//! | converted outputs identical | `rules_same` | 2 |
//! | converted outputs differ | `important_differences` | 13 |
//! | any error | `error` | 100 |
//!
//! Hex has no unimportant differences (never 12). The verdict never comes from block hashes: identical means
//! equal lengths and every paired byte same, as the statistics pass counted them.

use hikasa_core::{ContentKind, ExitCode};
use serde::{Deserialize, Serialize};

use crate::align::HexAlignment;

/// The binary answer of a comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HexVerdict {
    /// The content result.
    pub kind: ContentKind,
    /// The exit code of `hex compare`.
    pub exit_code: ExitCode,
}

impl HexVerdict {
    /// The verdict of any error (100).
    pub const ERROR: HexVerdict = HexVerdict {
        kind: ContentKind::Error,
        exit_code: ExitCode::UnknownError,
    };
}

/// The verdict of a comparison without conversion: `binary_same` (1) when the statistics show identical bytes,
/// else `binary_different` (11). It equals `compare_sources`' answer for every mode.
pub fn verdict(alignment: &HexAlignment) -> HexVerdict {
    binary(alignment.stats.identical())
}

fn binary(same: bool) -> HexVerdict {
    if same {
        HexVerdict {
            kind: ContentKind::BinarySame,
            exit_code: ExitCode::BinarySame,
        }
    } else {
        HexVerdict {
            kind: ContentKind::BinaryDifferent,
            exit_code: ExitCode::BinaryDifferent,
        }
    }
}

/// The verdict when a side was converted by its Hex format (D-336): `raw_same` is the raw comparison of the inputs
/// (`compare_sources`), `outputs` the alignment of the converted outputs.
pub fn converted_verdict(raw_same: bool, outputs: &HexAlignment) -> HexVerdict {
    if raw_same {
        binary(true)
    } else if outputs.stats.identical() {
        HexVerdict {
            kind: ContentKind::RulesSame,
            exit_code: ExitCode::RulesSame,
        }
    } else {
        HexVerdict {
            kind: ContentKind::ImportantDifferences,
            exit_code: ExitCode::RulesDifferent,
        }
    }
}
