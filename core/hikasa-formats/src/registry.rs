//! The format registry (FF-001…FF-004, §2.12): [`FormatRegistry`], [`RegistryEntry`], [`Origin`],
//! [`EnabledOverrides`], [`Detection`], [`DetectReason`], [`ShadowStatus`], and the `order.toml` model
//! ([`FormatOrder`]) with the pure merge steps of registry loading (§2.12.4), so that the session's
//! `FormatStore` only reads files.
//!
//! Detection: the first enabled entry (in order) whose mask has an include item matching the file **name**
//! and no exclude item matching it wins; masks are always case-insensitive (D-10). No match → "Everything
//! Else". The winning format is compiled lazily, once per entry (value-owned `OnceLock`, T7); a compile error
//! is returned, never skipped (T9).

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use hikasa_core::case::eq_folded;
use hikasa_core::mask::{MaskList, Polarity};
use hikasa_core::warning::{Warning, WarningCode};
use serde::{Deserialize, Serialize};

use crate::compile::{CompiledFormat, compile_mask};
use crate::error::FormatError;
use crate::model::{FileFormat, FormatId};
use crate::regex_engine::RegexLimits;
use crate::toml_io::check_schema_integer;

/// Where a registry entry came from.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Origin {
    /// Embedded preset.
    BuiltIn,
    /// User file (overrides a built-in with the same id, or adds a new format).
    User {
        /// The format file.
        path: PathBuf,
    },
}

/// One registry entry. Compiling happens lazily and once per entry (value-owned cache, T7).
#[derive(Debug)]
pub struct RegistryEntry {
    /// The format definition.
    pub format: Arc<FileFormat>,
    /// Globally enabled (FF-001).
    pub enabled: bool,
    /// Origin.
    pub origin: Origin,
    compiled: OnceLock<Result<Arc<CompiledFormat>, FormatError>>,
    /// Compiled by [`FormatRegistry::from_entries`] (case-insensitive, names only; D-10).
    mask: Option<MaskList>,
}

impl RegistryEntry {
    /// A new entry; its mask is compiled when the entry joins a registry ([`FormatRegistry::from_entries`]).
    pub fn new(format: Arc<FileFormat>, enabled: bool, origin: Origin) -> RegistryEntry {
        RegistryEntry {
            format,
            enabled,
            origin,
            compiled: OnceLock::new(),
            mask: None,
        }
    }

    /// The format id.
    pub fn id(&self) -> &FormatId {
        &self.format.id
    }

    /// The compiled mask (`None` until the entry is part of a registry).
    pub fn mask(&self) -> Option<&MaskList> {
        self.mask.as_ref()
    }

    /// Enabled flag after the session overrides (FF-002).
    pub fn is_enabled(&self, overrides: &EnabledOverrides) -> bool {
        overrides
            .0
            .get(&self.format.id)
            .copied()
            .unwrap_or(self.enabled)
    }

    fn compiled(&self, limits: &RegexLimits) -> Result<Arc<CompiledFormat>, FormatError> {
        self.compiled
            .get_or_init(|| CompiledFormat::compile(Arc::clone(&self.format), limits).map(Arc::new))
            .clone()
    }

    /// The same definition with a fresh (empty) compile cache.
    fn rebuilt(&self, format: Arc<FileFormat>, origin: Origin) -> RegistryEntry {
        RegistryEntry::new(format, self.enabled, origin)
    }
}

/// Per-session enabled overrides (FF-002). Missing ids use the global flag.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EnabledOverrides(pub BTreeMap<FormatId, bool>);

/// Why a format was chosen (FF-003 "Detected (C/C++ Source)").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum DetectReason {
    /// The first enabled entry whose mask item matched.
    Mask {
        /// The entry.
        id: FormatId,
        /// The first include item that matched (as written).
        mask: String,
    },
    /// Chosen explicitly (`--format`).
    Forced {
        /// The entry.
        id: FormatId,
    },
    /// No mask matched: "Everything Else".
    Fallback,
}

/// Detection result.
#[derive(Debug, Clone)]
pub struct Detection {
    /// The compiled format.
    pub format: Arc<CompiledFormat>,
    /// Why.
    pub reason: DetectReason,
}

/// Shadowing status of an entry (FF-001: shadowed entries are shown in red).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ShadowStatus {
    /// Reachable by at least one name, as far as the sound test can tell.
    Visible,
    /// Every mask item is covered by earlier enabled entries (sound, may miss some cases; §2.12.3).
    Shadowed {
        /// The covering entries, in registry order.
        by: Vec<FormatId>,
    },
    /// Empty mask: manual selection only (also a mask with exclude items only, which can never match).
    ManualOnly,
    /// Disabled.
    Disabled,
}

/// Ordered list of formats (first enabled match wins).
#[derive(Debug)]
pub struct FormatRegistry {
    entries: Vec<RegistryEntry>,
    limits: RegexLimits,
    everything_else: Arc<CompiledFormat>,
}

/// The file name part of `file_name` (after the last `/` or `\`): format masks match names only (D-10).
fn name_part(file_name: &str) -> &str {
    file_name.rsplit(['/', '\\']).next().unwrap_or(file_name)
}

impl FormatRegistry {
    /// From explicit entries (validates unique ids and compiles masks).
    pub fn from_entries(
        entries: Vec<RegistryEntry>,
        limits: RegexLimits,
    ) -> Result<FormatRegistry, FormatError> {
        let mut seen = BTreeSet::new();
        let mut out = Vec::with_capacity(entries.len());
        for mut entry in entries {
            if !seen.insert(entry.format.id.clone()) {
                return Err(FormatError::DuplicateId {
                    id: entry.format.id.to_string(),
                });
            }
            entry.mask = Some(compile_mask(entry.format.id.as_str(), &entry.format.mask)?);
            out.push(entry);
        }
        Ok(FormatRegistry {
            entries: out,
            limits,
            everything_else: Arc::new(CompiledFormat::everything_else()),
        })
    }

    /// Entries in order.
    pub fn entries(&self) -> &[RegistryEntry] {
        &self.entries
    }

    /// The entries, for building a modified registry.
    pub fn into_entries(self) -> Vec<RegistryEntry> {
        self.entries
    }

    /// The regex limits every format is compiled with.
    pub fn limits(&self) -> &RegexLimits {
        &self.limits
    }

    /// The implicit "Everything Else" format.
    pub fn everything_else(&self) -> &Arc<CompiledFormat> {
        &self.everything_else
    }

    /// By exact id.
    pub fn get(&self, id: &FormatId) -> Option<&RegistryEntry> {
        self.entries.iter().find(|e| &e.format.id == id)
    }

    /// By id, then by case-insensitive name (`--format`).
    pub fn find(&self, id_or_name: &str) -> Option<&RegistryEntry> {
        let wanted = id_or_name.trim();
        self.entries
            .iter()
            .find(|e| e.format.id.as_str() == wanted)
            .or_else(|| {
                self.entries
                    .iter()
                    .find(|e| eq_folded(e.format.name.trim(), wanted))
            })
    }

    /// Compiled format of an entry (cached in the entry).
    pub fn compiled(&self, id: &FormatId) -> Result<Arc<CompiledFormat>, FormatError> {
        match self.get(id) {
            Some(entry) => entry.compiled(&self.limits),
            None => Err(FormatError::UnknownFormat {
                name: id.to_string(),
            }),
        }
    }

    /// `--format <x>` (§2.12.2): an exact id, else a case-insensitive name, else `UnknownFormat`. The reason
    /// is `Forced`.
    pub fn force(&self, id_or_name: &str) -> Result<Detection, FormatError> {
        let entry = self
            .find(id_or_name)
            .ok_or_else(|| FormatError::UnknownFormat {
                name: id_or_name.to_owned(),
            })?;
        Ok(Detection {
            format: entry.compiled(&self.limits)?,
            reason: DetectReason::Forced {
                id: entry.format.id.clone(),
            },
        })
    }

    /// Detection by file name (§2.12). Compiles the winner; a compile error is returned, not skipped.
    ///
    /// Only the name part of `file_name` is matched (anything up to the last `/` or `\` is ignored).
    pub fn detect(
        &self,
        file_name: &str,
        overrides: &EnabledOverrides,
    ) -> Result<Detection, FormatError> {
        let name = name_part(file_name);
        for entry in &self.entries {
            if !entry.is_enabled(overrides) {
                continue;
            }
            let Some(mask) = entry.mask.as_ref().filter(|m| !m.is_empty()) else {
                continue;
            };
            let include = mask
                .masks()
                .iter()
                .find(|m| m.polarity() == Polarity::Include && m.matches_name(name));
            let Some(include) = include else {
                continue;
            };
            let excluded = mask
                .masks()
                .iter()
                .any(|m| m.polarity() == Polarity::Exclude && m.matches_name(name));
            if excluded {
                continue;
            }
            return Ok(Detection {
                format: entry.compiled(&self.limits)?,
                reason: DetectReason::Mask {
                    id: entry.format.id.clone(),
                    mask: include.source().to_owned(),
                },
            });
        }
        Ok(Detection {
            format: Arc::clone(&self.everything_else),
            reason: DetectReason::Fallback,
        })
    }

    /// Shadowing analysis for every entry (§2.12.3).
    ///
    /// An enabled entry with a non-empty mask is `Shadowed` iff every include item is covered (a sound
    /// language-inclusion proof, `Mask::is_covered_by`) by an include item of an earlier enabled entry that has
    /// no exclude items.
    pub fn shadow_report(&self, overrides: &EnabledOverrides) -> Vec<(FormatId, ShadowStatus)> {
        let mut out = Vec::with_capacity(self.entries.len());
        for (i, entry) in self.entries.iter().enumerate() {
            let status = if !entry.is_enabled(overrides) {
                ShadowStatus::Disabled
            } else {
                self.shadow_status(i, entry, overrides)
            };
            out.push((entry.format.id.clone(), status));
        }
        out
    }

    fn shadow_status(
        &self,
        i: usize,
        entry: &RegistryEntry,
        overrides: &EnabledOverrides,
    ) -> ShadowStatus {
        let Some(mask) = entry.mask.as_ref() else {
            return ShadowStatus::ManualOnly;
        };
        let includes: Vec<_> = mask
            .masks()
            .iter()
            .filter(|m| m.polarity() == Polarity::Include)
            .collect();
        if includes.is_empty() {
            return ShadowStatus::ManualOnly;
        }
        let mut by = BTreeSet::new();
        for item in includes {
            let cover = self.entries[..i].iter().position(|earlier| {
                earlier.is_enabled(overrides)
                    && earlier.mask.as_ref().is_some_and(|m| {
                        !m.is_empty()
                            && m.masks().iter().all(|j| j.polarity() == Polarity::Include)
                            && m.masks().iter().any(|j| item.is_covered_by(j))
                    })
            });
            match cover {
                Some(k) => {
                    by.insert(k);
                }
                None => return ShadowStatus::Visible,
            }
        }
        ShadowStatus::Shadowed {
            by: by
                .into_iter()
                .map(|k| self.entries[k].format.id.clone())
                .collect(),
        }
    }

    /// §2.12.4 step 2: user formats replace the built-in with the same id in place (origin `User`, the
    /// built-in's enabled flag kept); new ids are appended, sorted by id, enabled.
    ///
    /// A user format is skipped with warning `user_format_skipped` (the built-ins and every other format stay
    /// usable) when its id is not its file stem, when its mask does not compile (syntax, or an item with a
    /// separator: masks match names only, D-10), or when its id is already taken by another user format.
    pub fn with_user_formats(
        self,
        user: Vec<(Arc<FileFormat>, PathBuf)>,
    ) -> (FormatRegistry, Vec<Warning>) {
        let FormatRegistry {
            mut entries,
            limits,
            everything_else,
        } = self;
        let mut added: BTreeMap<FormatId, RegistryEntry> = BTreeMap::new();
        let mut warnings = Vec::new();
        for (format, path) in user {
            let skip = |reason: String| Warning {
                code: WarningCode::UserFormatSkipped,
                message: format!("{}: skipped: {reason}", path.display()),
                spec: None,
            };
            let stem = path.file_stem().and_then(|s| s.to_str());
            if stem != Some(format.id.as_str()) {
                warnings.push(skip(format!(
                    "the format id `{}` is not the file name `{}.toml`",
                    format.id,
                    stem.unwrap_or("")
                )));
                continue;
            }
            let mask = match compile_mask(format.id.as_str(), &format.mask) {
                Ok(mask) => mask,
                Err(e) => {
                    warnings.push(skip(e.to_string()));
                    continue;
                }
            };
            let taken = added.contains_key(&format.id)
                || entries
                    .iter()
                    .any(|e| e.format.id == format.id && e.origin != Origin::BuiltIn);
            if taken {
                warnings.push(skip(format!(
                    "another user format already has the id `{}`",
                    format.id
                )));
                continue;
            }
            let origin = Origin::User { path };
            match entries.iter().position(|e| e.format.id == format.id) {
                Some(k) => {
                    let mut entry = entries[k].rebuilt(format, origin);
                    entry.mask = Some(mask);
                    entries[k] = entry;
                }
                None => {
                    let mut entry = RegistryEntry::new(format, true, origin);
                    entry.mask = Some(mask);
                    added.insert(entry.format.id.clone(), entry);
                }
            }
        }
        entries.extend(added.into_values());
        (
            FormatRegistry {
                entries,
                limits,
                everything_else,
            },
            warnings,
        )
    }

    /// §2.12.4 steps 3–4: the final order is the ids listed in `order` (unknown ids → warning), followed by the
    /// unlisted entries in their current order; enabled flags come from the list where it gives one. An id
    /// listed twice is a `DuplicateId` error.
    pub fn with_order(
        self,
        order: &FormatOrder,
    ) -> Result<(FormatRegistry, Vec<Warning>), FormatError> {
        let limits = self.limits;
        let mut pool: Vec<Option<RegistryEntry>> = self.entries.into_iter().map(Some).collect();
        let mut listed = BTreeSet::new();
        let mut entries = Vec::with_capacity(pool.len());
        let mut warnings = Vec::new();
        for item in &order.formats {
            if !listed.insert(item.id.clone()) {
                return Err(FormatError::DuplicateId {
                    id: item.id.to_string(),
                });
            }
            let slot = pool
                .iter_mut()
                .find(|e| e.as_ref().is_some_and(|e| e.format.id == item.id));
            match slot.and_then(Option::take) {
                Some(mut entry) => {
                    if let Some(enabled) = item.enabled {
                        entry.enabled = enabled;
                    }
                    entries.push(entry);
                }
                None => warnings.push(Warning {
                    code: WarningCode::UnknownConfigKey,
                    message: format!("order.toml: unknown format id `{}`", item.id),
                    spec: None,
                }),
            }
        }
        entries.extend(pool.into_iter().flatten());
        Ok((FormatRegistry::from_entries(entries, limits)?, warnings))
    }
}

/// Schema version of `formats/order.toml`.
pub const ORDER_SCHEMA_VERSION: u32 = 1;

/// `formats/order.toml` (SES-051): the user's format order and enabled flags.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormatOrder {
    /// Must be ≤ [`ORDER_SCHEMA_VERSION`].
    pub schema_version: u32,
    /// `[[format]]` tables, in order.
    #[serde(default, rename = "format")]
    pub formats: Vec<OrderItem>,
}

/// One `[[format]]` table of `order.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderItem {
    /// Format id.
    pub id: FormatId,
    /// Enabled flag; absent = keep the entry's flag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

impl FormatOrder {
    /// The order and enabled flags of a registry (what `FormatStore::materialize` writes).
    pub fn from_registry(registry: &FormatRegistry) -> FormatOrder {
        FormatOrder {
            schema_version: ORDER_SCHEMA_VERSION,
            formats: registry
                .entries
                .iter()
                .map(|e| OrderItem {
                    id: e.format.id.clone(),
                    enabled: Some(e.enabled),
                })
                .collect(),
        }
    }

    /// Parses `order.toml`. Unknown keys become warnings (D-31); a newer schema version is an error.
    pub fn parse(
        toml_text: &str,
        origin: &str,
    ) -> Result<(FormatOrder, Vec<Warning>), FormatError> {
        let table: toml::Table = toml::from_str(toml_text).map_err(|e| FormatError::Toml {
            origin: origin.to_owned(),
            message: e
                .to_string()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" "),
        })?;
        if let Some(toml::Value::Integer(found)) = table.get("schema_version") {
            check_schema_integer(*found, ORDER_SCHEMA_VERSION, origin)?;
        }
        let order: FormatOrder = toml::from_str(toml_text).map_err(|e| FormatError::Toml {
            origin: origin.to_owned(),
            message: e
                .to_string()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" "),
        })?;
        let mut unknown = Vec::new();
        for (key, value) in &table {
            match (key.as_str(), value) {
                ("schema_version", _) => {}
                ("format", toml::Value::Array(items)) => {
                    for (k, item) in items.iter().enumerate() {
                        if let toml::Value::Table(t) = item {
                            for key in t.keys().filter(|k| *k != "id" && *k != "enabled") {
                                unknown.push(format!("format[{k}].{key}"));
                            }
                        }
                    }
                }
                ("format", _) => {}
                (other, _) => unknown.push(other.to_owned()),
            }
        }
        let warnings = unknown
            .into_iter()
            .map(|key| Warning {
                code: WarningCode::UnknownConfigKey,
                message: format!("{origin}: unknown key `{key}`"),
                spec: None,
            })
            .collect();
        Ok((order, warnings))
    }

    /// The position of `id` in the list.
    pub fn position(&self, id: &FormatId) -> Option<usize> {
        self.formats.iter().position(|item| item.id == *id)
    }

    /// Sets the enabled flag of `id` (appended when it is not listed). Additive (M2 W1, D-75).
    pub fn set_enabled(&mut self, id: &FormatId, enabled: bool) {
        match self.position(id) {
            Some(k) => self.formats[k].enabled = Some(enabled),
            None => self.formats.push(OrderItem {
                id: id.clone(),
                enabled: Some(enabled),
            }),
        }
    }

    /// Removes `id`; false when it is not listed. Additive (M2 W1).
    pub fn remove(&mut self, id: &FormatId) -> bool {
        let before = self.formats.len();
        self.formats.retain(|item| item.id != *id);
        self.formats.len() != before
    }

    /// Moves `id` to position `index` of the list without it (clamped to the end); false when it is not listed.
    /// Additive (M2 W1).
    pub fn move_to(&mut self, id: &FormatId, index: usize) -> bool {
        let Some(k) = self.position(id) else {
            return false;
        };
        let item = self.formats.remove(k);
        let index = index.min(self.formats.len());
        self.formats.insert(index, item);
        true
    }

    /// Canonical TOML.
    pub fn to_toml(&self) -> String {
        match toml::to_string_pretty(self) {
            Ok(text) => text,
            Err(e) => format!("# cannot serialise order: {e}\n"),
        }
    }
}

#[cfg(test)]
mod tests;
