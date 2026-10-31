//! `hikasa settings get|set|unset|list|restore-defaults` and the `format` write commands (M2 W1; contract §1.1.5,
//! D-73, D-75, D-77): the [`Service`] calls and their DTOs.
//!
//! Every call reads the configuration fresh (no caching across calls). The settings calls load leniently (D-71):
//! a bad value is reported as warning `invalid_config_value` and replaced by its default, so `settings set` can
//! repair the file. A file is handled through its *effective* tree (every key with its value; a scheme overlays
//! its built-in) and its *defaults* tree; `set` writes the new value into the effective tree, reads the schema
//! back from it (type errors and failed checks are `invalid_input`) and saves the file sparsely.

use std::any::Any;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use hikasa_core::Warning;
use hikasa_formats::{
    CompiledFormat, FileFormat, FormatId, FormatKind, FormatRegistry, Origin, RegexLimits,
    parse_format, to_toml,
};
use serde::Serialize;
use toml::{Table, Value};

use super::{ApiError, Service};
use crate::atomic::write_atomic;
use crate::commands::{VIEWS, catalogue, view_sections};
use crate::error::SessionError;
use crate::formats::{FormatStore, ORDER_FILE};
use crate::help::edit_distance;
use crate::jobs::run_guarded;
use crate::settings::file::full_table;
use crate::settings::keys::{info_for, parse_path, rule_for};
use crate::settings::theme::{BUILTIN_SCHEMES, SchemeBase};
use crate::settings::{
    ArchiveTypes, CommandOverrides, FileId, FileOutcome, FilterPresets, KeySegment, LoadMode,
    OpenWith, Options, Overwrite, SchemeFile, SettingsChanges, SettingsFile, SettingsKey,
    SettingsSaveDto, SettingsSchema, SettingsSnapshot, SettingsStore, ShortcutConflict, State,
    ThemeSelection, ValueKind, defaults_tree, key_infos, template_default, tree,
};
use crate::watch::{InputStamp, InputState};

/// `hikasa.settings_get.v1`: one key.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SettingsValueDto {
    /// The key as given (`options.text_editing.auto_indent`).
    pub key: String,
    /// The file (`options`, `themes.dark`).
    pub file: String,
    /// The path inside the file.
    pub path: String,
    /// The effective value (a subtree as an object; `null`: not set and no default).
    pub value: serde_json::Value,
    /// The default value.
    pub default: serde_json::Value,
    /// The value equals the default.
    pub is_default: bool,
    /// The kind (`bool`, `int 0..20`, …).
    pub kind: String,
    /// The OPT id (docs/15).
    pub opt: Option<&'static str>,
    /// The load warnings of the file (for the envelope).
    #[serde(skip)]
    pub warnings: Vec<Warning>,
    /// The effective value as TOML (for human output).
    #[serde(skip)]
    pub toml: Option<Value>,
}

/// `hikasa.settings_set.v1` (`set` and `unset`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SettingsSetDto {
    /// The key.
    pub key: String,
    /// The file.
    pub file: String,
    /// The path inside the file.
    pub path: String,
    /// The value before.
    pub old: serde_json::Value,
    /// The value after.
    pub new: serde_json::Value,
    /// False when the file did not change (D-70: equal bytes, or only defaults and no file).
    pub written: bool,
    /// The load warnings of the file.
    #[serde(skip)]
    pub warnings: Vec<Warning>,
    /// The new value as TOML (for human output).
    #[serde(skip)]
    pub toml: Option<Value>,
}

/// One entry of `hikasa.settings_list.v1`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SettingsEntryDto {
    /// The key.
    pub key: String,
    /// The effective value.
    pub value: serde_json::Value,
    /// The default (`null` for an unknown kept key).
    pub default: serde_json::Value,
    /// The value equals the default.
    pub is_default: bool,
    /// The kind.
    pub kind: String,
    /// The OPT id.
    pub opt: Option<&'static str>,
    /// A key this version does not know, kept from the file (Q11).
    pub unknown: bool,
    /// The value as TOML (for human output).
    #[serde(skip)]
    pub toml: Value,
    /// The default as TOML (for human output).
    #[serde(skip)]
    pub default_toml: Option<Value>,
}

/// `hikasa.settings_list.v1`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SettingsListDto {
    /// The keys, in file order then key order.
    pub entries: Vec<SettingsEntryDto>,
    /// Shortcut conflicts that involve a command whose shortcuts `commands.toml` changes (UI-003).
    pub conflicts: Vec<ShortcutConflict>,
    /// The load warnings of the files.
    #[serde(skip)]
    pub warnings: Vec<Warning>,
}

/// One file of `hikasa.settings_restore.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RestoreFileDto {
    /// The file (`options`, `themes.mine`).
    pub file: String,
    /// Its path.
    pub path: String,
    /// It existed.
    pub existed: bool,
    /// It was deleted (false in a dry run).
    pub deleted: bool,
}

/// `hikasa.settings_restore.v1` (D-77).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SettingsRestoreDto {
    /// Only the plan.
    pub dry_run: bool,
    /// The files.
    pub files: Vec<RestoreFileDto>,
    /// Warnings of the plan (`session_locked`: a locked session `--delete-sessions` keeps, D-91).
    #[serde(skip)]
    pub warnings: Vec<hikasa_core::Warning>,
}

/// `hikasa.format_write.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FormatWriteDto {
    /// `new`, `edit`, `delete`, `import`, `enable`, `disable` or `move`.
    pub action: &'static str,
    /// The format id.
    pub id: String,
    /// The files written.
    pub written: Vec<String>,
    /// The files removed.
    pub removed: Vec<String>,
}

/// `hikasa.format_export.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FormatExportDto {
    /// The format id.
    pub id: String,
    /// The file written (`null`: standard output).
    pub path: Option<String>,
    /// The canonical TOML.
    pub toml: String,
}

/// `hikasa.format_restore.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FormatRestoreDto {
    /// Only the plan.
    pub dry_run: bool,
    /// The ids whose user files are (or would be) removed.
    pub ids: Vec<String>,
    /// The files removed (or that would be).
    pub removed: Vec<String>,
    /// The files of the named ids that do not exist (for human output).
    #[serde(skip)]
    pub absent: Vec<String>,
}

/// Where `format move` puts a format.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MoveTo {
    /// One place up.
    Up,
    /// One place down.
    Down,
    /// First.
    Top,
    /// Last.
    Bottom,
    /// Right before another format.
    Before(String),
    /// Right after another format.
    After(String),
}

/// Parses a format kind (`text`, `table`, `hex`, `picture`, `media`, `version`, `registry`, `external`).
pub fn parse_format_kind(text: &str) -> Result<FormatKind, ApiError> {
    let value = Value::String(text.to_owned());
    value.try_into::<FormatKind>().map_err(|_| {
        ApiError::InvalidArgument(format!(
            "unknown format kind {text:?} (text, table, hex, picture, media, version, registry, external)"
        ))
    })
}

/// A loaded settings file behind a uniform interface.
trait Handle {
    /// The file.
    fn id(&self) -> &FileId;
    /// The load warnings.
    fn warnings(&self) -> Vec<Warning>;
    /// The kept unknown keys.
    fn unknown(&self) -> &Table;
    /// Drops a kept unknown key.
    fn forget_unknown(&mut self, path: &[KeySegment]) -> bool;
    /// Every key with its value (a scheme: its built-in overlaid by the file).
    fn effective(&self) -> Value;
    /// The defaults.
    fn defaults(&self) -> Value;
    /// Reads the schema back from a changed effective tree.
    fn replace(&mut self, effective: Value) -> Result<(), Rejected>;
    /// Saves the file.
    fn save(&mut self, overwrite: Overwrite) -> Result<bool, SessionError>;
    /// The value, for the files with extra behaviour (`commands.toml`).
    fn value_any(&self) -> &dyn Any;
}

/// Why a changed tree was not taken.
enum Rejected {
    /// It does not deserialize (serde's message).
    Type(String),
    /// A check failed: the problem's key inside the file and its message.
    Problem(String, String),
}

/// A [`Handle`] over one typed file.
struct Typed<T> {
    id: FileId,
    file: SettingsFile<T>,
}

/// The defaults of a file whose full tree is `full` (a scheme's base comes from its `base` key).
fn defaults_for(id: &FileId, full: &Value) -> Value {
    match id {
        FileId::Themes(_) => {
            let base = match full.get("base").and_then(Value::as_str) {
                Some("dark") => SchemeBase::Dark,
                _ => SchemeBase::Light,
            };
            let probe = SchemeFile {
                base: Some(base),
                ..SchemeFile::default()
            };
            defaults_tree(id, Some(&probe))
        }
        other => defaults_tree(other, None),
    }
}

impl<T: SettingsSchema + 'static> Handle for Typed<T> {
    fn id(&self) -> &FileId {
        &self.id
    }

    fn warnings(&self) -> Vec<Warning> {
        self.file.loaded().warnings.clone()
    }

    fn unknown(&self) -> &Table {
        self.file.unknown()
    }

    fn forget_unknown(&mut self, path: &[KeySegment]) -> bool {
        self.file.forget_unknown(path)
    }

    fn effective(&self) -> Value {
        let full = Value::Table(full_table(self.file.value()));
        match self.id {
            FileId::Themes(_) => {
                let mut tree = defaults_for(&self.id, &full);
                tree::overlay(&mut tree, &full);
                tree
            }
            _ => full,
        }
    }

    fn defaults(&self) -> Value {
        defaults_for(&self.id, &Value::Table(full_table(self.file.value())))
    }

    fn replace(&mut self, effective: Value) -> Result<(), Rejected> {
        let tree = match self.id {
            // The elements the scheme sets: what differs from the built-in of its current base (so a change of
            // `base` keeps only the elements the file set). The save prunes the rest against the built-in of
            // the new base (`SchemeFile::save_defaults`).
            FileId::Themes(_) => {
                let mut tree = tree::sparse(&effective, &self.defaults());
                if full_table(self.file.value()).contains_key("base")
                    && let (Some(base), Value::Table(table)) = (effective.get("base"), &mut tree)
                {
                    table.insert("base".into(), base.clone());
                }
                tree
            }
            _ => effective,
        };
        let value: T = tree.try_into().map_err(|e: toml::de::Error| {
            Rejected::Type(crate::settings::file::one_line(e.message()))
        })?;
        // A problem the file already had (an entry kept as it is by the lenient load) does not block a change
        // elsewhere; a new one does.
        let before = self.file.value().validate();
        if let Some(problem) = value.validate().iter().find(|p| !before.contains(p)) {
            return Err(Rejected::Problem(
                problem.key.clone(),
                problem.message.clone(),
            ));
        }
        *self.file.value_mut() = value;
        Ok(())
    }

    fn save(&mut self, overwrite: Overwrite) -> Result<bool, SessionError> {
        self.file.save(overwrite)
    }

    fn value_any(&self) -> &dyn Any {
        self.file.value()
    }
}

/// Opens one file leniently (D-71).
fn open(store: &SettingsStore, id: &FileId) -> Result<Box<dyn Handle>, ApiError> {
    fn typed<T: SettingsSchema + 'static>(
        store: &SettingsStore,
        id: &FileId,
    ) -> Result<Box<dyn Handle>, ApiError> {
        let file = SettingsFile::<T>::load(&store.path(id.clone()), LoadMode::Lenient)?;
        Ok(Box::new(Typed {
            id: id.clone(),
            file,
        }))
    }
    match id {
        FileId::Options => typed::<Options>(store, id),
        FileId::State => typed::<State>(store, id),
        FileId::Commands => typed::<CommandOverrides>(store, id),
        FileId::Theme => typed::<ThemeSelection>(store, id),
        FileId::Themes(name) if name.is_empty() => Err(ApiError::InvalidArgument(
            "name a scheme: themes.<name> (`hikasa settings list themes` lists them)".into(),
        )),
        FileId::Themes(_) => typed::<SchemeFile>(store, id),
        FileId::Filters => typed::<FilterPresets>(store, id),
        FileId::OpenWith => typed::<OpenWith>(store, id),
        FileId::Archives => typed::<ArchiveTypes>(store, id),
        FileId::Session | FileId::Workspace => Err(ApiError::InvalidArgument(format!(
            "`{id}` files are changed with `hikasa {id} …`, not with `hikasa settings`"
        ))),
    }
}

/// What a key path names in a file.
struct Found {
    /// The effective value (an unknown kept key's value; `None`: an optional key that is not set).
    value: Option<Value>,
    /// The default.
    default: Option<Value>,
    /// The kind.
    kind: ValueKind,
    /// The OPT id.
    opt: Option<&'static str>,
    /// An unknown kept key.
    unknown: bool,
}

/// Looks a path up in a file (`None`: the file has no such key).
fn find(handle: &dyn Handle, path: &[KeySegment]) -> Option<Found> {
    let id = handle.id();
    let effective = handle.effective();
    let defaults = handle.defaults();
    if path.is_empty() {
        return Some(Found {
            value: Some(effective),
            default: Some(defaults),
            kind: ValueKind::Table,
            opt: id.opt().map(|o| o.0),
            unknown: false,
        });
    }
    let kept = || {
        tree::get_in(handle.unknown(), path)
            .cloned()
            .map(|v| Found {
                kind: ValueKind::of_value(&v),
                value: Some(v),
                default: None,
                opt: None,
                unknown: true,
            })
    };
    if !names_an_entry(id, &effective, path) {
        return kept();
    }
    let value = tree::get(&effective, path).cloned();
    // An entry `set_aside` keeps (the override of a known command that this build cannot read) is what the file
    // says, not the default the key would have.
    if value.is_none()
        && let Some(found) = kept()
    {
        return Some(found);
    }
    let default = tree::get(&defaults, path)
        .cloned()
        .or_else(|| template_default(id, path));
    // A rule whose last segment is a wildcard (`tabs.*`) gives the kind of the keys a table has; it does not make
    // any other name a key. A rule that names its key (`app.*.command`) covers an optional key that is not set.
    let rule = rule_for(id, path).filter(|r| !r.pattern.ends_with('*'));
    if value.is_none() && default.is_none() && rule.is_none() {
        return kept();
    }
    let sample = value
        .clone()
        .or_else(|| default.clone())
        .unwrap_or(Value::String(String::new()));
    let info = info_for(id, path, sample);
    Some(Found {
        value: value.or_else(|| default.clone()),
        default,
        kind: info.kind,
        opt: info.opt,
        unknown: false,
    })
}

/// False when `path` goes through an entry that does not exist: an array position past the end, a view that
/// is not a view type, or a command id that is not in the sections of its view (`commands.toml`).
fn names_an_entry(id: &FileId, effective: &Value, path: &[KeySegment]) -> bool {
    let indexes_exist = path.iter().enumerate().all(|(k, segment)| {
        !matches!(segment, KeySegment::Index(_)) || tree::get(effective, &path[..=k]).is_some()
    });
    if !indexes_exist {
        return false;
    }
    if *id == FileId::Commands
        && let [KeySegment::Name(top), KeySegment::Name(view), rest @ ..] = path
        && top == "view"
    {
        if !VIEWS.contains(&view.as_str()) {
            return false;
        }
        if let [KeySegment::Name(command), KeySegment::Name(cmd), ..] = rest
            && command == "command"
            && catalogue().find(view_sections(view), cmd).is_none()
        {
            return false;
        }
    }
    true
}

/// A TOML value as JSON (`None` → `null`).
fn json(value: Option<&Value>) -> serde_json::Value {
    value.map_or(serde_json::Value::Null, tree::to_json)
}

/// Parses the text of `settings set` (D-73): a TOML value; for the text kinds, any text that is not a TOML
/// string is the string itself.
pub(crate) fn parse_value(text: &str, kind: ValueKind) -> Result<Value, ApiError> {
    let parsed = toml::from_str::<Table>(&format!("v = {text}"))
        .ok()
        .filter(|t| t.len() == 1)
        .and_then(|mut t| t.remove("v"));
    match parsed {
        Some(Value::String(s)) => Ok(Value::String(s)),
        Some(_) | None if kind.is_text() => Ok(Value::String(text.to_owned())),
        Some(value) => Ok(value),
        None => Err(ApiError::InvalidArgument(format!(
            "{text:?} is not a TOML value (expected: {})",
            kind.describe()
        ))),
    }
}

/// The keys of a file that are near `key` (edit distance ≤ 2), at most five.
fn suggestions(store: &SettingsStore, key: &SettingsKey) -> Vec<String> {
    let text = key.to_string();
    let mut candidates: Vec<String> = key_infos(&key.file).into_iter().map(|i| i.key).collect();
    if let Ok(handle) = open(store, &key.file)
        && let Value::Table(table) = handle.effective()
    {
        for (path, _) in tree::leaves(&table, false) {
            candidates.push(
                SettingsKey {
                    file: key.file.clone(),
                    path,
                }
                .to_string(),
            );
        }
    }
    candidates.sort();
    candidates.dedup();
    candidates
        .into_iter()
        .filter(|c| edit_distance(c, &text) <= 2)
        .take(5)
        .collect()
}

/// `not_found` for an unknown key, with suggestions.
fn unknown_key(store: &SettingsStore, key: &SettingsKey) -> ApiError {
    let near = suggestions(store, key);
    let hint = if near.is_empty() {
        "; `hikasa settings list` lists the keys".to_owned()
    } else {
        format!("; did you mean: {}?", near.join(", "))
    };
    ApiError::NotFound(format!("unknown settings key `{key}`{hint}"))
}

/// Every file `settings list` and `restore-defaults --all` cover, in list order (the schemes after `theme`).
fn all_files(store: &SettingsStore) -> Vec<FileId> {
    let mut files = Vec::new();
    for file in FileId::FIXED {
        let theme = file == FileId::Theme;
        files.push(file);
        if theme {
            files.extend(store.scheme_names().into_iter().map(FileId::Themes));
        }
    }
    files
}

/// The scheme files of `themes/` as file ids.
fn scheme_ids(store: &SettingsStore) -> Vec<FileId> {
    store
        .scheme_files()
        .into_iter()
        .map(|(name, _)| FileId::Themes(name))
        .collect()
}

impl Service {
    /// The settings store of this configuration.
    pub fn settings_store(&self) -> SettingsStore {
        SettingsStore::new(self.config())
    }

    /// `settings get`: the effective value of a leaf or subtree (file or default). An unknown key is
    /// `not_found` with up to five suggestions.
    pub fn settings_get(&self, key: &SettingsKey) -> Result<SettingsValueDto, ApiError> {
        run_guarded(|| {
            let store = self.settings_store();
            let handle = open(&store, &key.file)?;
            let found = find(&*handle, &key.path).ok_or_else(|| unknown_key(&store, key))?;
            Ok(SettingsValueDto {
                key: key.to_string(),
                file: key.file.name(),
                path: key.path_text(),
                value: json(found.value.as_ref()),
                default: json(found.default.as_ref()),
                is_default: found.value == found.default,
                kind: found.kind.describe(),
                opt: found.opt,
                warnings: handle.warnings(),
                toml: found.value,
            })
        })
    }

    /// `settings set`: parses `value_text` (D-73), checks it with the file's schema (`invalid_input`), and
    /// saves the file sparsely (a value equal to its default removes the key). A file changed on disk since it
    /// was read is `Conflict` unless `force`.
    pub fn settings_set(
        &self,
        key: &SettingsKey,
        value_text: &str,
        force: bool,
    ) -> Result<SettingsSetDto, ApiError> {
        run_guarded(|| {
            let store = self.settings_store();
            let handle = open(&store, &key.file)?;
            let found = match find(&*handle, &key.path) {
                Some(found) if !found.unknown && !key.path.is_empty() => found,
                _ => return Err(unknown_key(&store, key)),
            };
            if found.kind == ValueKind::Table {
                return Err(ApiError::InvalidArgument(format!(
                    "`{key}` is a table: set one of its keys (`hikasa settings list {key}`)"
                )));
            }
            let new = parse_value(value_text, found.kind)?;
            let mut effective = handle.effective();
            tree::set(&mut effective, &key.path, new)
                .map_err(|e| ApiError::InvalidArgument(format!("{}: {e}", key.file.name())))?;
            self.finish_change(&store, handle, key, found, effective, force)
        })
    }

    /// `settings unset`: the key back to its default (an entry of a map is removed); a kept unknown key, or a
    /// whole kept view or command of `commands.toml`, leaves the file (§1.4.20).
    pub fn settings_unset(&self, key: &SettingsKey) -> Result<SettingsSetDto, ApiError> {
        run_guarded(|| {
            let store = self.settings_store();
            let mut handle = open(&store, &key.file)?;
            let found = match find(&*handle, &key.path) {
                Some(found) if !key.path.is_empty() => found,
                _ => return Err(unknown_key(&store, key)),
            };
            if found.unknown {
                handle.forget_unknown(&key.path);
                let effective = handle.effective();
                return self.finish_change(&store, handle, key, found, effective, false);
            }
            let mut effective = handle.effective();
            match tree::get(&handle.defaults(), &key.path) {
                Some(default) => {
                    let _ = tree::set(&mut effective, &key.path, default.clone());
                }
                None => {
                    tree::remove(&mut effective, &key.path);
                }
            }
            self.finish_change(&store, handle, key, found, effective, false)
        })
    }

    /// The common end of `set` and `unset`: read back, check, save, report.
    fn finish_change(
        &self,
        store: &SettingsStore,
        mut handle: Box<dyn Handle>,
        key: &SettingsKey,
        before: Found,
        effective: Value,
        force: bool,
    ) -> Result<SettingsSetDto, ApiError> {
        handle.replace(effective).map_err(|rejected| {
            ApiError::InvalidArgument(match rejected {
                Rejected::Type(message) => format!("{key}: {message}"),
                Rejected::Problem(at, message) => format!("{}.{at}: {message}", key.file.name()),
            })
        })?;
        if key.file == FileId::Theme
            && let Some(KeySegment::Name(name)) = key.path.first()
            && (name == "light_scheme" || name == "dark_scheme")
            && let Some(Value::String(scheme)) = tree::get(&handle.effective(), &key.path)
            && !store.scheme_names().contains(scheme)
        {
            return Err(ApiError::InvalidArgument(format!(
                "{key}: the scheme {scheme:?} does not exist (one of {})",
                store.scheme_names().join(", ")
            )));
        }
        let overwrite = if force {
            Overwrite::Force
        } else {
            Overwrite::IfUnchanged
        };
        let written = handle.save(overwrite)?;
        let after = find(&*handle, &key.path);
        let new = after.and_then(|f| f.value);
        Ok(SettingsSetDto {
            key: key.to_string(),
            file: key.file.name(),
            path: key.path_text(),
            old: json(before.value.as_ref()),
            new: json(new.as_ref()),
            written,
            warnings: handle.warnings(),
            toml: new,
        })
    }

    /// `settings list`: every key (and every unknown kept key, flagged) of the files under `prefix` (all files
    /// without one), in file order then key order; with `changed_only`, the keys that differ from their
    /// defaults. Shortcut conflicts of `commands.toml` are attached.
    pub fn settings_list(
        &self,
        prefix: Option<&SettingsKey>,
        changed_only: bool,
    ) -> Result<SettingsListDto, ApiError> {
        run_guarded(|| {
            let store = self.settings_store();
            let files = match prefix.map(|p| &p.file) {
                None => all_files(&store),
                Some(FileId::Themes(name)) if name.is_empty() => store
                    .scheme_names()
                    .into_iter()
                    .map(FileId::Themes)
                    .collect(),
                Some(file) => vec![file.clone()],
            };
            let under: &[KeySegment] = prefix.map_or(&[], |p| p.path.as_slice());
            let mut dto = SettingsListDto {
                entries: Vec::new(),
                conflicts: Vec::new(),
                warnings: Vec::new(),
            };
            for id in files {
                let handle = open(&store, &id)?;
                dto.warnings.extend(handle.warnings());
                let effective = handle.effective();
                let defaults = handle.defaults();
                let mut leaves: BTreeMap<Vec<KeySegment>, (Option<Value>, Option<Value>, bool)> =
                    BTreeMap::new();
                if let Value::Table(table) = &effective {
                    for (path, value) in tree::leaves(table, false) {
                        leaves.entry(path).or_default().0 = Some(value);
                    }
                }
                if let Value::Table(table) = &defaults {
                    for (path, value) in tree::leaves(table, false) {
                        leaves.entry(path).or_default().1 = Some(value);
                    }
                }
                for (path, value) in tree::leaves(handle.unknown(), false) {
                    let entry = leaves.entry(path).or_default();
                    entry.0 = Some(value);
                    entry.2 = true;
                }
                for (path, (value, default, unknown)) in leaves {
                    if !path.starts_with(under) {
                        continue;
                    }
                    let default = if unknown {
                        default
                    } else {
                        default.or_else(|| template_default(&id, &path))
                    };
                    let Some(value) = value.or_else(|| default.clone()) else {
                        continue;
                    };
                    let is_default = default.as_ref() == Some(&value);
                    if changed_only && is_default {
                        continue;
                    }
                    let info = info_for(&id, &path, value.clone());
                    dto.entries.push(SettingsEntryDto {
                        key: info.key,
                        value: tree::to_json(&value),
                        default: json(default.as_ref()),
                        is_default,
                        kind: if unknown {
                            ValueKind::of_value(&value).describe()
                        } else {
                            info.kind.describe()
                        },
                        opt: if unknown { None } else { info.opt },
                        unknown,
                        toml: value,
                        default_toml: default,
                    });
                }
                if let Some(overrides) = handle.value_any().downcast_ref::<CommandOverrides>() {
                    for view in VIEWS {
                        dto.conflicts.extend(overrides.user_conflicts(view));
                    }
                }
            }
            Ok(dto)
        })
    }

    /// `settings restore-defaults` (D-77): the plan (each file: path, whether it exists) and, unless `dry_run`,
    /// the deletion. `FileId::Themes` without a name means every scheme file.
    pub fn settings_restore_defaults(
        &self,
        files: &[FileId],
        dry_run: bool,
    ) -> Result<SettingsRestoreDto, ApiError> {
        run_guarded(|| {
            let store = self.settings_store();
            let mut ids: Vec<FileId> = Vec::new();
            for file in files {
                let expanded = match file {
                    FileId::Themes(name) if name.is_empty() => scheme_ids(&store),
                    other => vec![other.clone()],
                };
                for id in expanded {
                    if !ids.contains(&id) {
                        ids.push(id);
                    }
                }
            }
            let mut dto = SettingsRestoreDto {
                dry_run,
                files: Vec::new(),
                warnings: Vec::new(),
            };
            for id in ids {
                let path = store.path(id.clone());
                let existed = path.is_file();
                let deleted = existed && !dry_run;
                if deleted {
                    match std::fs::remove_file(&path) {
                        Ok(()) => {}
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(e) => return Err(SessionError::io(&path, e).into()),
                    }
                }
                dto.files.push(RestoreFileDto {
                    file: id.name(),
                    path: path.display().to_string(),
                    existed,
                    deleted,
                });
            }
            Ok(dto)
        })
    }

    /// The format store and a registry read now (the format write commands never use the startup registry).
    fn fresh_formats(&self) -> Result<(FormatStore, FormatRegistry), ApiError> {
        let store = FormatStore::new(self.config());
        let (registry, _) = store.load(RegexLimits::default())?;
        Ok((store, registry))
    }

    /// `format new` (D-75): `formats/<id>.toml` from a copy of `from` (named `<name> (copy)` unless `name`
    /// is given) or an empty format of `kind` (text); appended to `order.toml` as enabled.
    pub fn format_new(
        &self,
        id: &str,
        from: Option<&str>,
        name: Option<&str>,
        mask: Option<&str>,
        kind: Option<FormatKind>,
    ) -> Result<FormatWriteDto, ApiError> {
        run_guarded(|| {
            let fid = writable_format_id(id)?;
            let (store, registry) = self.fresh_formats()?;
            if registry.get(&fid).is_some() {
                return Err(ApiError::AlreadyExists(format!(
                    "the format `{id}` already exists"
                )));
            }
            check_no_skipped_file(&store, id, "")?;
            let mut format = match from {
                Some(source) => {
                    let entry = registry
                        .find(source)
                        .ok_or_else(|| ApiError::NotFound(format!("unknown format {source:?}")))?;
                    let mut format = (*entry.format).clone();
                    format.id = fid.clone();
                    format.name =
                        name.map_or_else(|| format!("{} (copy)", format.name), str::to_owned);
                    if let Some(kind) = kind {
                        format.kind = kind;
                    }
                    format
                }
                None => FileFormat::new(
                    fid.clone(),
                    name.unwrap_or(id),
                    kind.unwrap_or(FormatKind::Text),
                ),
            };
            if let Some(mask) = mask {
                format.mask = mask.to_owned();
            }
            check_compiles(&format)?;
            let mut order = store.editable_order(&registry)?;
            order.set_enabled(&fid, true);
            let (path, order_path) = write_both(
                || store.write_user_file(id, to_toml(&format).as_bytes()),
                || store.write_order(&order),
            )?;
            Ok(write_dto("new", id, vec![path, order_path], Vec::new()))
        })
    }

    /// `format edit` (D-75): the format replaced by `file` (its id must be `id`; the file is copied), or the
    /// dotted `key=value` edits of `set` applied to the canonical TOML. The result must parse, know every key
    /// and compile, else `invalid_input` and nothing is written. A built-in gets its override file.
    pub fn format_edit(
        &self,
        id: &str,
        set: &[(String, String)],
        file: Option<&Path>,
    ) -> Result<FormatWriteDto, ApiError> {
        run_guarded(|| {
            let fid = writable_format_id(id)?;
            let (store, registry) = self.fresh_formats()?;
            let entry = registry
                .get(&fid)
                .ok_or_else(|| ApiError::NotFound(format!("unknown format `{id}`")))?;
            let (format, bytes) =
                match file {
                    Some(path) => {
                        let text = read_text(path)?;
                        let (format, _) = parse_format(&text, &path.display().to_string())
                            .map_err(|e| ApiError::InvalidArgument(e.to_string()))?;
                        (format, text.into_bytes())
                    }
                    None => {
                        let mut root: Value = toml::from_str::<Table>(&to_toml(&entry.format))
                            .map_err(|e| ApiError::Internal(e.to_string()))?
                            .into();
                        for (key, value) in set {
                            let path = parse_path(key).ok().filter(|p| !p.is_empty()).ok_or_else(
                                || ApiError::InvalidArgument(format!("{key:?} is not a key path")),
                            )?;
                            tree::set(&mut root, &path, loose_value(value))
                                .map_err(|e| ApiError::InvalidArgument(e.to_string()))?;
                        }
                        let text = toml::to_string(&root)
                            .map_err(|e| ApiError::Internal(e.to_string()))?;
                        let (format, warnings) = parse_format(&text, &format!("format {id}"))
                            .map_err(|e| ApiError::InvalidArgument(e.to_string()))?;
                        if let Some(warning) = warnings.first() {
                            return Err(ApiError::InvalidArgument(warning.message.clone()));
                        }
                        let bytes = to_toml(&format).into_bytes();
                        (format, bytes)
                    }
                };
            if format.id != fid {
                return Err(ApiError::InvalidArgument(format!(
                    "the format id is `{}`, not `{id}`",
                    format.id
                )));
            }
            check_compiles(&format)?;
            let path = store.write_user_file(id, &bytes)?;
            Ok(write_dto("edit", id, vec![path], Vec::new()))
        })
    }

    /// `format delete` (D-75): a user format's file and its `order.toml` entry. A built-in (or a user file
    /// overriding one) is `invalid_input`: disable it, or `format restore` it.
    pub fn format_delete(&self, id: &str) -> Result<FormatWriteDto, ApiError> {
        run_guarded(|| {
            let fid = format_id(id)?;
            let (store, registry) = self.fresh_formats()?;
            let entry = registry
                .get(&fid)
                .ok_or_else(|| ApiError::NotFound(format!("unknown format `{id}`")))?;
            if FormatRegistry::builtin()?.get(&fid).is_some() {
                return Err(ApiError::InvalidArgument(format!(
                    "`{id}` is a built-in format: disable it instead (`hikasa format disable {id}`); \
                     `hikasa format restore {id}` removes a file that overrides it"
                )));
            }
            let mut removed = Vec::new();
            if let Origin::User { path } = &entry.origin {
                remove_file(path)?;
                removed.push(path.clone());
            }
            let mut written = Vec::new();
            if let Some(mut order) = store.read_order()?
                && order.remove(&fid)
            {
                written.push(store.write_order(&order)?);
            }
            Ok(write_dto("delete", id, written, removed))
        })
    }

    /// `format import` (D-75): validates `path` and copies it to `formats/<id>.toml`; an existing id without
    /// `replace` is `already_exists`.
    pub fn format_import(&self, path: &Path, replace: bool) -> Result<FormatWriteDto, ApiError> {
        run_guarded(|| {
            let text = read_text(path)?;
            let (format, _) = parse_format(&text, &path.display().to_string())
                .map_err(|e| ApiError::InvalidArgument(e.to_string()))?;
            let id = format.id.as_str();
            writable_format_id(id)?;
            check_compiles(&format)?;
            let (store, registry) = self.fresh_formats()?;
            if !replace {
                if registry.get(&format.id).is_some() {
                    return Err(ApiError::AlreadyExists(format!(
                        "the format `{id}` already exists (`--replace` replaces it)"
                    )));
                }
                check_no_skipped_file(&store, id, " (`--replace` replaces it)")?;
            }
            let target = store.write_user_file(id, text.as_bytes())?;
            Ok(write_dto("import", id, vec![target], Vec::new()))
        })
    }

    /// `format export`: the canonical TOML of a format (by id or name), written to `out` (atomically) or
    /// returned for standard output.
    pub fn format_export(&self, id: &str, out: Option<&Path>) -> Result<FormatExportDto, ApiError> {
        run_guarded(|| {
            let (_, registry) = self.fresh_formats()?;
            let entry = registry
                .find(id)
                .ok_or_else(|| ApiError::NotFound(format!("unknown format {id:?}")))?;
            let toml = to_toml(&entry.format);
            if let Some(out) = out {
                write_atomic(out, toml.as_bytes()).map_err(|e| SessionError::io(out, e))?;
            }
            Ok(FormatExportDto {
                id: entry.id().as_str().to_owned(),
                path: out.map(|p| p.display().to_string()),
                toml,
            })
        })
    }

    /// `format enable|disable` (D-75): writes `order.toml` only (created from the registry order when absent).
    pub fn format_enable(&self, id: &str, enabled: bool) -> Result<FormatWriteDto, ApiError> {
        run_guarded(|| {
            let fid = format_id(id)?;
            let (store, registry) = self.fresh_formats()?;
            if registry.get(&fid).is_none() {
                return Err(ApiError::NotFound(format!("unknown format `{id}`")));
            }
            let mut order = store.editable_order(&registry)?;
            order.set_enabled(&fid, enabled);
            let path = store.write_order(&order)?;
            let action = if enabled { "enable" } else { "disable" };
            Ok(write_dto(action, id, vec![path], Vec::new()))
        })
    }

    /// `format move` (D-75): writes `order.toml` only.
    pub fn format_move(&self, id: &str, to: &MoveTo) -> Result<FormatWriteDto, ApiError> {
        run_guarded(|| {
            let fid = format_id(id)?;
            let (store, registry) = self.fresh_formats()?;
            if registry.get(&fid).is_none() {
                return Err(ApiError::NotFound(format!("unknown format `{id}`")));
            }
            let mut order = store.editable_order(&registry)?;
            let from = order
                .position(&fid)
                .ok_or_else(|| ApiError::Internal(format!("`{id}` is not in the order")))?;
            let other_index = |other: &str| -> Result<usize, ApiError> {
                let oid = format_id(other)?;
                if oid == fid {
                    return Err(ApiError::InvalidArgument(format!(
                        "cannot move `{id}` relative to itself"
                    )));
                }
                let k = order
                    .position(&oid)
                    .ok_or_else(|| ApiError::NotFound(format!("unknown format `{other}`")))?;
                // The position in the list without the moved format.
                Ok(if k > from { k - 1 } else { k })
            };
            let at = match to {
                MoveTo::Up => from.saturating_sub(1),
                MoveTo::Down => from + 1,
                MoveTo::Top => 0,
                MoveTo::Bottom => usize::MAX,
                MoveTo::Before(other) => other_index(other)?,
                MoveTo::After(other) => other_index(other)? + 1,
            };
            order.move_to(&fid, at);
            let path = store.write_order(&order)?;
            Ok(write_dto("move", id, vec![path], Vec::new()))
        })
    }

    /// `format restore` (D-75): removes the user files that override the built-ins `ids` (each must be a
    /// built-in id), or with `all` every user format file and `order.toml`. A plan with `dry_run` (D-77).
    pub fn format_restore(
        &self,
        ids: &[String],
        all: bool,
        dry_run: bool,
    ) -> Result<FormatRestoreDto, ApiError> {
        run_guarded(|| {
            let store = FormatStore::new(self.config());
            let mut dto = FormatRestoreDto {
                dry_run,
                ids: Vec::new(),
                removed: Vec::new(),
                absent: Vec::new(),
            };
            let mut paths: Vec<PathBuf> = Vec::new();
            if all {
                for path in store.user_file_paths() {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        dto.ids.push(stem.to_owned());
                    }
                    paths.push(path);
                }
                let order = store.order_path();
                if order.is_file() {
                    paths.push(order);
                }
            } else {
                let builtin = FormatRegistry::builtin()?;
                for id in ids {
                    let fid = format_id(id)?;
                    if builtin.get(&fid).is_none() {
                        return Err(ApiError::InvalidArgument(format!(
                            "`{id}` is not a built-in format (`hikasa format delete {id}` removes a user format)"
                        )));
                    }
                    let path = store.user_path(id);
                    if path.is_file() {
                        dto.ids.push(id.clone());
                        paths.push(path);
                    } else {
                        dto.absent.push(path.display().to_string());
                    }
                }
            }
            for path in paths {
                if !dry_run {
                    remove_file(&path)?;
                }
                dto.removed.push(path.display().to_string());
            }
            Ok(dto)
        })
    }
}

/// A format id (`invalid_input` when malformed).
fn format_id(id: &str) -> Result<FormatId, ApiError> {
    FormatId::new(id).map_err(|e| ApiError::InvalidArgument(e.to_string()))
}

/// A format id that a write may create or replace (`invalid_input` when malformed). `order` is reserved in any
/// case: its file would be `formats/order.toml`, which the loader reads as the order (D-75).
fn writable_format_id(id: &str) -> Result<FormatId, ApiError> {
    let reserved = ORDER_FILE.strip_suffix(".toml").unwrap_or(ORDER_FILE);
    if id.eq_ignore_ascii_case(reserved) {
        return Err(ApiError::InvalidArgument(format!(
            "the format id `{id}` is reserved (`formats/{ORDER_FILE}` is the order of the formats)"
        )));
    }
    format_id(id)
}

/// `already_exists` when `formats/<id>.toml` exists although the registry has no such format (the load skipped
/// it: it could not be read or parsed); a write must not replace it unasked.
fn check_no_skipped_file(store: &FormatStore, id: &str, hint: &str) -> Result<(), ApiError> {
    let path = store.user_path(id);
    if path.exists() {
        return Err(ApiError::AlreadyExists(format!(
            "{} already exists (it is not a format the registry could load; fix or remove it){hint}",
            path.display()
        )));
    }
    Ok(())
}

/// The format compiles (patterns, masks, ranges, encodings), else `invalid_input`.
fn check_compiles(format: &FileFormat) -> Result<(), ApiError> {
    CompiledFormat::compile(Arc::new(format.clone()), &RegexLimits::default())
        .map(|_| ())
        .map_err(|e| ApiError::InvalidArgument(e.to_string()))
}

/// The text of a file (`invalid_input` when it is not UTF-8).
fn read_text(path: &Path) -> Result<String, ApiError> {
    let bytes = std::fs::read(path).map_err(|e| SessionError::io(path, e))?;
    String::from_utf8(bytes)
        .map_err(|_| ApiError::InvalidArgument(format!("{}: not valid UTF-8", path.display())))
}

/// Removes a file (a missing file is fine).
fn remove_file(path: &Path) -> Result<(), ApiError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(SessionError::io(path, e).into()),
    }
}

/// Writes a new file with `first`, then another with `second`; when the second write fails the first file is
/// removed again (no half state), and the error is the second one.
fn write_both(
    first: impl FnOnce() -> Result<PathBuf, SessionError>,
    second: impl FnOnce() -> Result<PathBuf, SessionError>,
) -> Result<(PathBuf, PathBuf), ApiError> {
    let created = first()?;
    match second() {
        Ok(other) => Ok((created, other)),
        Err(e) => {
            let _ = remove_file(&created);
            Err(e.into())
        }
    }
}

/// A `key=value` value of `format edit --set`: a TOML value, else the text as a string.
fn loose_value(text: &str) -> Value {
    toml::from_str::<Table>(&format!("v = {text}"))
        .ok()
        .filter(|t| t.len() == 1)
        .and_then(|mut t| t.remove("v"))
        .unwrap_or_else(|| Value::String(text.to_owned()))
}

/// A [`FormatWriteDto`].
fn write_dto(
    action: &'static str,
    id: &str,
    written: Vec<PathBuf>,
    removed: Vec<PathBuf>,
) -> FormatWriteDto {
    let text = |paths: Vec<PathBuf>| paths.iter().map(|p| p.display().to_string()).collect();
    FormatWriteDto {
        action,
        id: id.to_owned(),
        written: text(written),
        removed: text(removed),
    }
}

// ---- M2 W4a: the GUI's settings calls (§1.4.15, D-105, D-108) ------------------------------------------------

/// Saves one file of a [`crate::settings::SettingsChanges`]: equal to the snapshot → `Unchanged`; checks fail →
/// `Invalid`; else the snapshot's file with the new value, saved sparsely (`IfUnchanged` unless `force`). The
/// outcome is pushed on `files`; `slot` takes the saved file (its new stamp).
fn save_one<T: SettingsSchema + Clone + PartialEq>(
    files: &mut Vec<(FileId, PathBuf, FileOutcome)>,
    slot: &mut SettingsFile<T>,
    id: FileId,
    value: &T,
    overwrite: Overwrite,
) {
    let (outcome, saved) = save_copy(slot, value, overwrite);
    files.push((id, slot.path().to_path_buf(), outcome));
    if let Some(saved) = saved {
        *slot = saved;
    }
}

fn save_copy<T: SettingsSchema + Clone + PartialEq>(
    file: &SettingsFile<T>,
    value: &T,
    overwrite: Overwrite,
) -> (FileOutcome, Option<SettingsFile<T>>) {
    if file.value() == value && file.loaded().migrated_from.is_none() {
        return (FileOutcome::Unchanged, None);
    }
    let problems = value.validate();
    if !problems.is_empty() {
        return (FileOutcome::Invalid(problems), None);
    }
    // `Force` writes over the file as it is now: its unknown keys are today's, not the snapshot's, and one that
    // cannot be read is not replaced. `IfUnchanged` after that read: a change in between is a `Conflict` again.
    let mut copy = match overwrite {
        Overwrite::IfUnchanged => file.clone(),
        Overwrite::Force => match read_current::<T>(file.path()) {
            Ok(current) => current,
            Err(outcome) => return (outcome, None),
        },
    };
    *copy.value_mut() = value.clone();
    match copy.save(Overwrite::IfUnchanged) {
        Ok(true) => (FileOutcome::Written, Some(copy)),
        Ok(false) => (FileOutcome::Unchanged, Some(copy)),
        Err(SessionError::Conflict { .. }) => {
            let outcome = read_current::<T>(file.path())
                .err()
                .unwrap_or(FileOutcome::Conflict);
            (outcome, None)
        }
        Err(e) => (FileOutcome::Failed(e.to_string()), None),
    }
}

/// The file as it is on disk now; `Unreadable` when it cannot be read.
fn read_current<T: SettingsSchema>(path: &Path) -> Result<SettingsFile<T>, FileOutcome> {
    SettingsFile::<T>::load(path, LoadMode::Lenient)
        .map_err(|e| FileOutcome::Unreadable(e.to_string()))
}

/// Deletes the file of a scheme change of `None`.
fn delete_scheme_file(file: &SettingsFile<SchemeFile>, force: bool) -> FileOutcome {
    let path = file.path();
    if !path.exists() {
        return FileOutcome::Unchanged;
    }
    if (force || !matches!(file.changed_on_disk(), Ok(false)))
        && let Err(outcome) = read_current::<SchemeFile>(path)
    {
        return outcome;
    }
    match file.changed_on_disk() {
        Ok(true) if !force => return FileOutcome::Conflict,
        Err(e) if !force => return FileOutcome::Failed(e.to_string()),
        _ => {}
    }
    match std::fs::remove_file(path) {
        Ok(()) => FileOutcome::Deleted,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => FileOutcome::Unchanged,
        Err(e) => FileOutcome::Failed(SessionError::io(path, e).to_string()),
    }
}

impl Service {
    /// Every settings file the window and the settings dialogs read, loaded leniently (bad values are their
    /// defaults with warnings). A file of a newer `schema_version` is an error (SES-052).
    pub fn settings_snapshot(&self) -> Result<SettingsSnapshot, ApiError> {
        run_guarded(|| {
            let store = self.settings_store();
            let mode = LoadMode::Lenient;
            let mut schemes = BTreeMap::new();
            for name in store.scheme_names() {
                let file = store.scheme_file(&name, mode)?;
                schemes.insert(name, file);
            }
            Ok(SettingsSnapshot {
                options: store.file::<Options>(mode)?,
                commands: store.file::<CommandOverrides>(mode)?,
                theme: store.file::<ThemeSelection>(mode)?,
                schemes,
                filters: store.file::<FilterPresets>(mode)?,
                openwith: store.file::<OpenWith>(mode)?,
                archives: store.file::<ArchiveTypes>(mode)?,
                unreadable: BTreeMap::new(),
            })
        })
    }

    /// [`Service::settings_snapshot`] for a window that starts whatever one file holds: each file is read on its
    /// own, and one that cannot be read (a newer `schema_version`, broken TOML, no access) is its defaults and is
    /// named in [`SettingsSnapshot::unreadable`]. The other files stay what they are, and a save over the
    /// unreadable one is refused ([`FileOutcome::Unreadable`]).
    pub fn settings_snapshot_partial(&self) -> Result<SettingsSnapshot, ApiError> {
        fn read<T: SettingsSchema>(
            store: &SettingsStore,
            id: FileId,
            unreadable: &mut BTreeMap<FileId, String>,
        ) -> SettingsFile<T> {
            let path = store.path(id.clone());
            SettingsFile::load(&path, LoadMode::Lenient).unwrap_or_else(|e| {
                unreadable.insert(id, e.to_string());
                SettingsFile::new_default(&path)
            })
        }
        run_guarded(|| {
            let store = self.settings_store();
            let mut bad = BTreeMap::new();
            let mut schemes = BTreeMap::new();
            for name in store.scheme_names() {
                let id = FileId::Themes(name.clone());
                schemes.insert(name, read(&store, id, &mut bad));
            }
            Ok(SettingsSnapshot {
                options: read(&store, FileId::Options, &mut bad),
                commands: read(&store, FileId::Commands, &mut bad),
                theme: read(&store, FileId::Theme, &mut bad),
                schemes,
                filters: read(&store, FileId::Filters, &mut bad),
                openwith: read(&store, FileId::OpenWith, &mut bad),
                archives: read(&store, FileId::Archives, &mut bad),
                unreadable: bad,
            })
        })
    }

    /// Writes the changed files of a settings dialog (D-128): in `FileId::every` order, each independently — a
    /// value equal to its snapshot is `Unchanged`, a value that fails its checks `Invalid` (not written), else
    /// the snapshot's `SettingsFile` saves it sparsely with `IfUnchanged`: `Written` or `Conflict`. With `force`
    /// the file is read again first and saved with the value given and the unknown keys it holds now. A file on
    /// disk that cannot be read is `Unreadable`, `force` or not, and stays as it is. A scheme change of `None`
    /// deletes `themes/<name>.toml` (`Deleted`). The same `SettingsFile` path as `settings set`. Returns the outcomes and a fresh snapshot; files already written are never reported
    /// as a failure of the whole call: when the files cannot be read back, the snapshot is the given one with the
    /// saved files in it and `reread_error` says why.
    pub fn settings_save(
        &self,
        snapshot: &SettingsSnapshot,
        changes: &SettingsChanges,
        force: bool,
    ) -> Result<SettingsSaveDto, ApiError> {
        run_guarded(|| {
            let store = self.settings_store();
            let overwrite = if force {
                Overwrite::Force
            } else {
                Overwrite::IfUnchanged
            };
            let mut files = Vec::new();
            let mut after = snapshot.clone();
            if let Some(v) = &changes.options {
                save_one(
                    &mut files,
                    &mut after.options,
                    FileId::Options,
                    v,
                    overwrite,
                );
            }
            if let Some(v) = &changes.commands {
                save_one(
                    &mut files,
                    &mut after.commands,
                    FileId::Commands,
                    v,
                    overwrite,
                );
            }
            if let Some(v) = &changes.theme {
                save_one(&mut files, &mut after.theme, FileId::Theme, v, overwrite);
            }
            for (name, change) in &changes.schemes {
                let id = FileId::Themes(name.clone());
                let path = store.path(id.clone());
                let file = after
                    .schemes
                    .entry(name.clone())
                    .or_insert_with(|| SettingsFile::new_default(&path));
                match change {
                    Some(v) => save_one(&mut files, file, id, v, overwrite),
                    None => {
                        let outcome = delete_scheme_file(file, force);
                        let deleted = outcome == FileOutcome::Deleted;
                        files.push((id, path.clone(), outcome));
                        if deleted && BUILTIN_SCHEMES.contains(&name.as_str()) {
                            *file = SettingsFile::new_default(&path);
                        } else if deleted {
                            after.schemes.remove(name);
                        }
                    }
                }
            }
            if let Some(v) = &changes.filters {
                save_one(
                    &mut files,
                    &mut after.filters,
                    FileId::Filters,
                    v,
                    overwrite,
                );
            }
            if let Some(v) = &changes.openwith {
                save_one(
                    &mut files,
                    &mut after.openwith,
                    FileId::OpenWith,
                    v,
                    overwrite,
                );
            }
            if let Some(v) = &changes.archives {
                save_one(
                    &mut files,
                    &mut after.archives,
                    FileId::Archives,
                    v,
                    overwrite,
                );
            }
            let (snapshot, reread_error) = match self.settings_snapshot() {
                Ok(fresh) => (fresh, None),
                Err(e) => (after, Some(e.to_string())),
            };
            Ok(SettingsSaveDto {
                files,
                snapshot,
                reread_error,
            })
        })
    }

    /// Changes `state.toml` (D-108): loads it leniently, applies `change`, saves it with `IfUnchanged`; when the
    /// file changed on disk in between (a concurrent `hikasa settings set state…`), reloads it and applies
    /// `change` once more. `Ok(false)`: nothing to write (the change left the value as it was).
    pub fn state_update(&self, change: &mut dyn FnMut(&mut State)) -> Result<bool, ApiError> {
        run_guarded(|| {
            let store = self.settings_store();
            let mut attempt = 0;
            loop {
                attempt += 1;
                let mut file = store.file::<State>(LoadMode::Lenient)?;
                let before = file.value().clone();
                change(file.value_mut());
                if *file.value() == before && file.loaded().migrated_from.is_none() {
                    return Ok(false);
                }
                match file.save(Overwrite::IfUnchanged) {
                    Ok(written) => return Ok(written),
                    Err(SessionError::Conflict { .. }) if attempt < 2 => {}
                    Err(e) => return Err(e.into()),
                }
            }
        })
    }

    /// Appends `lines` (each ended by a line break) to the log file `path` of `[tweaks.log_panel] save_to_file`
    /// (D-116), creating it and its folder. Not atomic: an append.
    pub fn log_append(&self, path: &Path, lines: &[String]) -> Result<(), ApiError> {
        use std::io::Write as _;
        run_guarded(|| {
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent).map_err(|e| SessionError::io(parent, e))?;
            }
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .map_err(|e| SessionError::io(path, e))?;
            let mut text = String::new();
            for line in lines {
                text.push_str(line);
                text.push('\n');
            }
            file.write_all(text.as_bytes())
                .map_err(|e| SessionError::io(path, e))?;
            Ok(())
        })
    }

    /// The stamp of an input of a file view (D-118): length and modification time from the VFS `stat`, never
    /// the content. `-`, `clipboard://`, the spooled copy of standard input (`<cache>/stdin/…`) and specs that do
    /// not name a file system path are `Unwatched` (a spec that does not parse too); a missing input is `Missing` (any other `stat` failure as well: it cannot be
    /// read now).
    pub fn input_stamp(&self, spec: &str) -> InputState {
        use hikasa_vfs::{FollowLinks, Vfs as _};
        // `Spec::parse_os`, not `parse_spec`: a stamp never reads standard input or the clipboard.
        let Ok(parsed) =
            hikasa_core::Spec::parse_os(std::ffi::OsStr::new(spec), &self.spec_context())
        else {
            return InputState::Unwatched;
        };
        let hikasa_core::Spec::Local(path) = &parsed else {
            return InputState::Unwatched;
        };
        // Standard input given to the GUI is spooled to `<cache>/stdin/<n>/…` (a blob that never changes).
        if path
            .as_path()
            .starts_with(self.config().cache_dir.join("stdin"))
        {
            return InputState::Unwatched;
        }
        match self.vfs().stat(&parsed, FollowLinks::Yes) {
            Ok(entry) => InputState::Present(InputStamp {
                len: entry.size,
                modified_ns: entry.mtime.map(|t| {
                    i128::from(t.unix_seconds()) * 1_000_000_000 + i128::from(t.subsec_nanos())
                }),
            }),
            Err(_) => InputState::Missing,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_parse_by_kind() {
        assert_eq!(
            parse_value("true", ValueKind::Bool).unwrap(),
            Value::Boolean(true)
        );
        assert_eq!(
            parse_value("dark", ValueKind::Enum(&["dark"])).unwrap(),
            Value::String("dark".into())
        );
        assert_eq!(
            parse_value("\"x y\"", ValueKind::String).unwrap(),
            Value::String("x y".into())
        );
        assert_eq!(
            parse_value("2024", ValueKind::String).unwrap(),
            Value::String("2024".into())
        );
        assert_eq!(
            parse_value("*.c;*.h", ValueKind::Mask).unwrap(),
            Value::String("*.c;*.h".into())
        );
        assert!(parse_value("yes", ValueKind::Bool).is_err());
        assert!(parse_value("1\nx = 2", ValueKind::Int { min: 0, max: 9 }).is_err());
        assert_eq!(loose_value("*.x"), Value::String("*.x".into()));
        assert_eq!(loose_value("8"), Value::Integer(8));
        assert!(parse_format_kind("hex").is_ok() && parse_format_kind("nope").is_err());
    }

    #[test]
    fn a_failed_second_write_removes_the_first_file() {
        let tmp = tempfile::tempdir().unwrap();
        let first = tmp.path().join("mine.toml");
        let err = write_both(
            || {
                std::fs::write(&first, "x").map_err(|e| SessionError::io(&first, e))?;
                Ok(first.clone())
            },
            || {
                Err(SessionError::io(
                    &tmp.path().join("order.toml"),
                    std::io::Error::other("disk full"),
                ))
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("disk full"), "{err}");
        assert!(!first.exists(), "no half state");
        let (a, b) = write_both(|| Ok(first.clone()), || Ok(tmp.path().join("b"))).unwrap();
        assert_eq!((a, b), (first, tmp.path().join("b")));
    }

    #[test]
    fn the_order_id_is_reserved_in_any_case() {
        for id in ["order", "Order", "ORDER"] {
            assert!(matches!(
                writable_format_id(id),
                Err(ApiError::InvalidArgument(_))
            ));
        }
        assert!(writable_format_id("orders").is_ok());
    }
}
