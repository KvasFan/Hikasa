//! Program options: the M1 view of `options.toml` (docs/15; D-25, D-31, D-71): [`ProgramOptions`],
//! [`OptionsLoad`], [`load_options`], [`OPTION_SECTIONS`].
//!
//! Since M2 W1 these are thin wrappers over [`crate::settings::options::Options`] read by
//! [`crate::settings::SettingsFile`] (contract §1.1): every key of docs/15 "Keys" is typed and checked; a
//! `schema_version` newer than [`OPTIONS_SCHEMA_VERSION`] is an error (SES-052); unknown keys inside the
//! top-level sections are ignored silently and listed in [`OptionsLoad::ignored_keys`] for `--verbose` logging;
//! an unknown top-level section produces warning `unknown_config_key`; a wrong type or range is an error
//! ([`load_options`]) or, for the `settings` commands, warning `invalid_config_value` ([`load_options_with`]).

use hikasa_core::Warning;
use hikasa_vfs::StreamOptions;
use serde::{Deserialize, Serialize};

use crate::error::SessionError;
use crate::paths::ConfigLocation;
use crate::settings::options::Options;
use crate::settings::{LoadMode, Loaded, SettingsFile};

/// File name of the program options inside the configuration folder (SES-051).
pub const OPTIONS_FILE: &str = "options.toml";

/// Newest `schema_version` of `options.toml` this build reads.
pub const OPTIONS_SCHEMA_VERSION: u32 = 1;

/// Top-level sections of `options.toml` (docs/15 pages; `archive_types`, `commands` and `open_with` are reserved,
/// D-72). Unknown keys inside them are ignored silently; unknown top-level sections produce warning
/// `unknown_config_key` (D-31).
pub const OPTION_SECTIONS: &[&str] = &[
    "startup",
    "tabs",
    "appearance",
    "text_editing",
    "next_difference",
    "backups",
    "file_operations",
    "archive_types",
    "commands",
    "open_with",
    "tweaks",
];

/// M1 subset of `options.toml` (15-options-reference): the view [`Options::program_options`] computes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgramOptions {
    /// `[text_editing] number_of_context_lines` (default 3).
    pub context_lines: u32,
    /// `[tweaks.file_operations] buffer_size_for_binary_compare` (default 1 MiB; clamped per `StreamOptions`).
    pub binary_buffer_size: u32,
    /// `[tweaks.file_operations] content_memory_budget_mib` (default 1024; D-25; added to docs/15 in wave 4).
    pub content_memory_budget_mib: u32,
}

impl Default for ProgramOptions {
    /// 3 context lines, a 1 MiB binary-compare buffer, a 1024 MiB content-test budget.
    fn default() -> ProgramOptions {
        ProgramOptions {
            context_lines: 3,
            binary_buffer_size: 1 << 20,
            content_memory_budget_mib: 1024,
        }
    }
}

impl ProgramOptions {
    /// The binary-compare and CRC stream options: the configured buffer (clamped by `StreamOptions` to
    /// 4 KiB…64 MiB), the default mmap policy, and `threads` mapped-chunk threads (the service passes
    /// `min(4, pool size)`, §5.1).
    pub fn stream_options(&self, threads: usize) -> StreamOptions {
        StreamOptions {
            buffer_size: usize::try_from(self.binary_buffer_size)
                .unwrap_or(StreamOptions::DEFAULT_BUFFER_SIZE),
            threads: threads.max(1),
            ..StreamOptions::default()
        }
    }

    /// The content-test memory budget in bytes (D-25): `content_memory_budget_mib` × 2^20, at least 1 MiB.
    pub fn content_budget_bytes(&self) -> u64 {
        u64::from(self.content_memory_budget_mib.max(1)) << 20
    }
}

/// Result of `load_options`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionsLoad {
    /// Options (defaults where missing).
    pub options: ProgramOptions,
    /// `unknown_config_key` warnings for unknown top-level sections (D-31).
    pub warnings: Vec<Warning>,
    /// Dotted names of every ignored key, for `--verbose` logging (never a warning).
    pub ignored_keys: Vec<String>,
}

/// Loads `options.toml` strictly (missing file → defaults; newer `schema_version`, a wrong type or a value out of
/// range → error; D-71).
pub fn load_options(config: &ConfigLocation) -> Result<OptionsLoad, SessionError> {
    load_options_with(config, LoadMode::Strict)
}

/// Loads `options.toml` in `mode` (`Lenient`: the `settings` commands, D-71). Additive (M2 W1).
pub fn load_options_with(
    config: &ConfigLocation,
    mode: LoadMode,
) -> Result<OptionsLoad, SessionError> {
    let file = SettingsFile::<Options>::load(&config.dir.join(OPTIONS_FILE), mode)?;
    Ok(view(file.loaded()))
}

/// Parses the text of an `options.toml` strictly (`origin` names it in messages). The pure core of
/// [`load_options`] (additive).
pub fn parse_options(text: &str, origin: &str) -> Result<OptionsLoad, SessionError> {
    let file = SettingsFile::<Options>::parse(text, origin, LoadMode::Strict)?;
    Ok(view(file.loaded()))
}

/// The M1 view of a loaded file.
fn view(loaded: &Loaded<Options>) -> OptionsLoad {
    OptionsLoad {
        options: loaded.value.program_options(),
        warnings: loaded.warnings.clone(),
        ignored_keys: loaded.ignored_keys.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_and_budget() {
        let o = ProgramOptions {
            binary_buffer_size: 1,
            content_memory_budget_mib: 0,
            ..ProgramOptions::default()
        };
        assert_eq!(o.stream_options(4).effective_buffer_size(), 4 << 10);
        assert_eq!(o.stream_options(0).threads, 1);
        assert_eq!(o.content_budget_bytes(), 1 << 20);
        assert_eq!(ProgramOptions::default().content_budget_bytes(), 1 << 30);
    }
}
