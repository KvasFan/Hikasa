//! Program options: the M1 subset of `options.toml` (docs/15; D-25, D-31): [`ProgramOptions`],
//! [`OptionsLoad`], [`load_options`], [`OPTION_SECTIONS`].
//!
//! Loading rules (D-31): a missing file means defaults; a `schema_version` newer than
//! [`OPTIONS_SCHEMA_VERSION`] is an error (SES-052); unknown keys inside the top-level sections of docs/15
//! ([`OPTION_SECTIONS`]) are ignored silently (M2 writes the whole file) and listed in
//! [`OptionsLoad::ignored_keys`] for `--verbose` logging; an unknown top-level section produces warning
//! `unknown_config_key`. The file is never written in M1, so every key Hikasa does not use is preserved.

use hikasa_core::{SchemaCompat, Warning, WarningCode, check_schema_version};
use hikasa_vfs::StreamOptions;
use serde::{Deserialize, Serialize};
use toml::{Table, Value};

use crate::error::SessionError;
use crate::paths::ConfigLocation;

/// File name of the program options inside the configuration folder (SES-051).
pub const OPTIONS_FILE: &str = "options.toml";

/// Newest `schema_version` of `options.toml` this build reads.
pub const OPTIONS_SCHEMA_VERSION: u32 = 1;

/// Top-level sections of `options.toml` (docs/15 pages). Unknown keys inside them are ignored silently;
/// unknown top-level sections produce warning `unknown_config_key` (D-31).
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

/// The dotted key of `context_lines`.
const KEY_CONTEXT_LINES: &str = "text_editing.number_of_context_lines";
/// The dotted key of `binary_buffer_size`.
const KEY_BUFFER_SIZE: &str = "tweaks.file_operations.buffer_size_for_binary_compare";
/// The dotted key of `content_memory_budget_mib`.
const KEY_CONTENT_BUDGET: &str = "tweaks.file_operations.content_memory_budget_mib";

/// M1 subset of `options.toml` (15-options-reference). All other keys are preserved by not touching the file.
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

/// Loads `options.toml` (missing file → defaults; newer `schema_version` → error).
pub fn load_options(config: &ConfigLocation) -> Result<OptionsLoad, SessionError> {
    let path = config.dir.join(OPTIONS_FILE);
    match std::fs::read(&path) {
        Ok(bytes) => {
            let origin = path.display().to_string();
            let text = String::from_utf8(bytes).map_err(|_| SessionError::Config {
                path: origin.clone(),
                message: "the file is not valid UTF-8".into(),
            })?;
            parse_options(&text, &origin)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(OptionsLoad {
            options: ProgramOptions::default(),
            warnings: Vec::new(),
            ignored_keys: Vec::new(),
        }),
        Err(e) => Err(SessionError::io(&path, e)),
    }
}

/// Parses the text of an `options.toml` (`origin` names it in messages). The pure core of [`load_options`]
/// (additive).
pub fn parse_options(text: &str, origin: &str) -> Result<OptionsLoad, SessionError> {
    let config_error = |message: String| SessionError::Config {
        path: origin.to_owned(),
        message,
    };
    let table: Table = toml::from_str(text).map_err(|e| {
        config_error(
            e.to_string()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" "),
        )
    })?;
    if let Some(version) = table.get("schema_version") {
        let found = match version {
            Value::Integer(n) => u32::try_from(*n).unwrap_or(u32::MAX),
            _ => return Err(config_error("`schema_version` must be an integer".into())),
        };
        match check_schema_version(found, OPTIONS_SCHEMA_VERSION) {
            Ok(SchemaCompat::Current | SchemaCompat::Older { .. }) => {}
            Err(source) => {
                return Err(SessionError::Schema {
                    path: origin.to_owned(),
                    source,
                });
            }
        }
    }
    let mut load = OptionsLoad {
        options: ProgramOptions::default(),
        warnings: Vec::new(),
        ignored_keys: Vec::new(),
    };
    for (key, value) in &table {
        if key == "schema_version" {
            continue;
        }
        if !OPTION_SECTIONS.contains(&key.as_str()) {
            load.warnings.push(Warning {
                code: WarningCode::UnknownConfigKey,
                message: format!("{origin}: unknown section `{key}`"),
                spec: None,
            });
        }
        walk(key, value, &mut load, origin)?;
    }
    Ok(load)
}

/// Visits the leaves below `path`: the known M1 keys set their option, every other leaf is ignored.
fn walk(
    path: &str,
    value: &Value,
    load: &mut OptionsLoad,
    origin: &str,
) -> Result<(), SessionError> {
    let target = match path {
        KEY_CONTEXT_LINES => Some(&mut load.options.context_lines),
        KEY_BUFFER_SIZE => Some(&mut load.options.binary_buffer_size),
        KEY_CONTENT_BUDGET => Some(&mut load.options.content_memory_budget_mib),
        _ => None,
    };
    if let Some(slot) = target {
        *slot = match value {
            Value::Integer(n) => u32::try_from(*n).map_err(|_| SessionError::Config {
                path: origin.to_owned(),
                message: format!("`{path}` must be an integer from 0 to {}", u32::MAX),
            })?,
            _ => {
                return Err(SessionError::Config {
                    path: origin.to_owned(),
                    message: format!("`{path}` must be an integer"),
                });
            }
        };
        return Ok(());
    }
    match value {
        Value::Table(table) => {
            for (key, child) in table {
                walk(&format!("{path}.{key}"), child, load, origin)?;
            }
        }
        _ => load.ignored_keys.push(path.to_owned()),
    }
    Ok(())
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
