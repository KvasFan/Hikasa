//! The built-in presets (FF-040, Appendix A): one TOML file per format under `presets/`, embedded with
//! `include_str!`, in canonical order (the file prefix). Each file starts with a `# Sources:` comment naming
//! the public specifications its keyword lists were written from.

use std::sync::Arc;

use crate::error::FormatError;
use crate::regex_engine::RegexLimits;
use crate::registry::{FormatRegistry, Origin, RegistryEntry};
use crate::toml_io::parse_format;

/// An embedded preset.
#[derive(Debug, Clone, Copy)]
pub struct Preset {
    /// File name under `presets/`.
    pub file: &'static str,
    /// TOML text (`include_str!`).
    pub toml: &'static str,
    /// Enabled by default (the only place built-in enabled flags live besides `order.toml`; D-27).
    pub enabled: bool,
}

macro_rules! preset {
    ($file:literal, $enabled:expr) => {
        Preset {
            file: $file,
            toml: include_str!(concat!("../presets/", $file)),
            enabled: $enabled,
        }
    };
}

/// All presets in canonical order.
pub const PRESETS: &[Preset] = &[
    preset!("01-bash.toml", true),
    preset!("02-c-cpp.toml", true),
    preset!("03-cobol.toml", true),
    preset!("04-pascal-forms.toml", true),
    preset!("05-pascal.toml", true),
    preset!("06-html.toml", true),
    preset!("07-java.toml", true),
    preset!("08-javascript.toml", true),
    preset!("09-perl.toml", true),
    preset!("10-python.toml", true),
    preset!("11-ini-reg.toml", true),
    preset!("12-sql.toml", true),
    preset!("13-visual-basic.toml", true),
    preset!("14-xml.toml", true),
    preset!("15-rust.toml", true),
    preset!("16-sorted.toml", true),
    preset!("17-csv.toml", true),
    preset!("18-pictures.toml", true),
    preset!("19-media.toml", true),
    preset!("20-executables.toml", true),
    // D-27: fully shadowed by "Registry Dumps / INI" (`*.reg`) and its view is P1, so it ships disabled.
    preset!("21-registry-files.toml", false),
];

impl Preset {
    /// The origin string used in messages: `built-in:<file>`.
    pub fn origin(&self) -> String {
        format!("built-in:{}", self.file)
    }
}

impl FormatRegistry {
    /// Built-in presets in canonical order (fresh value on each call; no global cache).
    pub fn builtin() -> Result<FormatRegistry, FormatError> {
        FormatRegistry::builtin_with_limits(RegexLimits::default())
    }

    /// [`FormatRegistry::builtin`] with explicit regex limits.
    pub fn builtin_with_limits(limits: RegexLimits) -> Result<FormatRegistry, FormatError> {
        let mut entries = Vec::with_capacity(PRESETS.len());
        for preset in PRESETS {
            let (format, _warnings) = parse_format(preset.toml, &preset.origin())?;
            entries.push(RegistryEntry::new(
                Arc::new(format),
                preset.enabled,
                Origin::BuiltIn,
            ));
        }
        FormatRegistry::from_entries(entries, limits)
    }
}
