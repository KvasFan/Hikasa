//! `filters.toml` (OPT-020): the name-filter presets.

use serde::{Deserialize, Serialize};
use toml::Table;

use super::file::{Problem, SettingsSchema, migrate_from_zero};
use super::keys::{FileId, KindRule, ValueKind};

/// `filters.toml`: `[[preset]]` entries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FilterPresets {
    /// The presets, in list order.
    pub preset: Vec<FilterPreset>,
}

/// One preset: a name and a mask list.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FilterPreset {
    /// Unique, non-empty name.
    pub name: String,
    /// Mask list (non-empty).
    pub mask: String,
    /// Keys of the entry this version does not know (kept, Q11).
    #[serde(flatten)]
    pub extra: Table,
}

impl FilterPreset {
    /// A preset without unknown keys.
    pub fn new(name: &str, mask: &str) -> FilterPreset {
        FilterPreset {
            name: name.to_owned(),
            mask: mask.to_owned(),
            extra: Table::new(),
        }
    }
}

impl Default for FilterPresets {
    /// `Source files`, `Web files`, `Documents` (docs/15).
    fn default() -> FilterPresets {
        FilterPresets {
            preset: vec![
                FilterPreset::new(
                    "Source files",
                    "*.c;*.cpp;*.h;*.hpp;*.rs;*.py;*.java;*.cs;*.go;*.js;*.ts",
                ),
                FilterPreset::new("Web files", "*.htm;*.html;*.css;*.js"),
                FilterPreset::new("Documents", "*.txt;*.md;*.rst;*.adoc"),
            ],
        }
    }
}

impl SettingsSchema for FilterPresets {
    const FILE: FileId = FileId::Filters;
    const VERSION: u32 = 1;

    fn migrate(from: u32, _table: &mut Table) -> Result<(), String> {
        migrate_from_zero(from)
    }

    /// Names non-empty and unique; masks non-empty and valid. A bad entry is a problem of the whole entry
    /// (`preset.<n>`; lenient loading keeps it as it is, with a warning).
    fn validate(&self) -> Vec<Problem> {
        let mut problems = Vec::new();
        for (k, preset) in self.preset.iter().enumerate() {
            let key = format!("preset.{k}");
            if preset.name.trim().is_empty() {
                problems.push(Problem::new(key, "the name is empty"));
            } else if self.preset[..k].iter().any(|p| p.name == preset.name) {
                problems.push(Problem::new(
                    key,
                    format!("the name {:?} is used twice", preset.name),
                ));
            } else if preset.mask.trim().is_empty() {
                problems.push(Problem::new(key, "the mask is empty"));
            } else if let Err(e) = super::check_mask(&preset.mask) {
                problems.push(Problem::new(key, e));
            }
        }
        problems
    }
}

/// Kinds of the keys of `filters.toml`.
pub(crate) const RULES: &[KindRule] = &[
    KindRule {
        pattern: "preset.*.name",
        kind: ValueKind::String,
        opt: Some("OPT-020"),
        priority: Some("P0"),
    },
    KindRule {
        pattern: "preset.*.mask",
        kind: ValueKind::Mask,
        opt: Some("OPT-020"),
        priority: Some("P0"),
    },
    KindRule {
        pattern: "preset.*",
        kind: ValueKind::Table,
        opt: Some("OPT-020"),
        priority: Some("P0"),
    },
];

/// The defaults of a new preset entry.
pub(crate) fn templates() -> Vec<(&'static str, toml::Value)> {
    vec![(
        "preset.*",
        super::file::full_table(&FilterPreset::default()).into(),
    )]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_are_checked() {
        let mut presets = FilterPresets::default();
        assert!(presets.validate().is_empty());
        presets.preset.push(FilterPreset::new("Web files", "*.x"));
        presets.preset.push(FilterPreset::new("", "*.x"));
        presets.preset.push(FilterPreset::new("Bad", "[a"));
        presets.preset.push(FilterPreset::new("Empty", ""));
        let keys: Vec<String> = presets.validate().into_iter().map(|p| p.key).collect();
        assert_eq!(
            keys,
            vec!["preset.3", "preset.4", "preset.5", "preset.6"],
            "{:?}",
            presets.validate()
        );
        // Lenient repair never drops or reorders entries: a bad entry is kept as it is.
        let problems = presets.validate();
        let before = presets.clone();
        presets.repair(&problems);
        assert_eq!(presets, before);
    }
}
