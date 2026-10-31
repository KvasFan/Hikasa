//! [`SettingsFile`]: the one engine that loads and saves every program-owned settings file (D-70, T13):
//! `schema_version`, the migration chain, unknown-key preservation (Q11), lenient repair (D-71), sparse atomic
//! writes and the conflict check (D-74).
//!
//! ```text
//! load(path, mode):
//!   1. missing → T::default(), stamp Absent; other read errors → Io; not UTF-8 → Config
//!   2. parse to a table (syntax error → Config, one line)
//!   3. schema_version: absent → current; negative or not an integer → Config; newer → Schema (both modes)
//!   4. older: T::migrate(v) for v = found … VERSION-1 on the table; migrated_from = found (not written now);
//!      T::set_aside: entries this build cannot judge leave the table for the kept keys, each with a warning
//!   5. deserialize T (unknown keys are ignored by serde); a type error is found leaf by leaf: Strict → Config
//!      naming the key; Lenient → the leaf is removed (its default applies), warning invalid_config_value, retry
//!   6. unknown keys = the paths of the table that the full serialization of the value lacks: top-level ones
//!      (except T::known_sections) warn unknown_config_key; all are listed in ignored_keys and kept for save
//!   7. validate(): Strict → the first problem is Config; Lenient → repair(), one warning per problem (array
//!      elements are never dropped or reordered: an element that fails a check is kept as it is)
//! save(overwrite):
//!   1. IfUnchanged and the disk stamp differs from the load stamp → Conflict
//!   2. document = schema_version, then the value without the leaves equal to save_defaults() (T::default();
//!      a scheme: the built-in of its base; fields in declaration order, empty tables dropped), with the kept
//!      unknown keys merged in
//!   3. equal to the file's bytes → Ok(false); only schema_version and no file → Ok(false)
//!   4. create the folder, write_atomic, new stamp → Ok(true)
//! ```

use std::path::{Path, PathBuf};

use hikasa_core::{SchemaCompat, Warning, WarningCode, check_schema_version};
use serde::Serialize;
use serde::de::DeserializeOwned;
use toml::{Table, Value};

use super::keys::{FileId, KeySegment};
use super::stamp::ChangeStamp;
use super::tree::{self, Ordered};
use crate::atomic::write_atomic;
use crate::error::SessionError;

/// The schema of one settings file.
pub trait SettingsSchema:
    Serialize + DeserializeOwned + Default + Clone + PartialEq + std::fmt::Debug
{
    /// Which file (`FileId::Themes(String::new())` for the scheme files, whose name comes from the path).
    const FILE: FileId;
    /// The current `schema_version` (all files: 1 in W1).
    const VERSION: u32;

    /// Rewrites a raw table of version `from` (< `VERSION`) into the shape of `from + 1`. Called once per step.
    fn migrate(from: u32, table: &mut Table) -> Result<(), String> {
        let _ = table;
        Err(format!("no migration from schema_version {from}"))
    }

    /// Range, enum, mask and reference checks after deserialization; each [`Problem`] names a dotted key.
    fn validate(&self) -> Vec<Problem>;

    /// Repairs for [`LoadMode::Lenient`]: replaces each invalid value by its default; an element of an array
    /// is never removed (a problem naming a whole element keeps it as it is). The default implementation works
    /// on the serialized tree.
    fn repair(&mut self, problems: &[Problem]) {
        repair_by_path(self, problems);
    }

    /// What a save leaves out of the file at `path` (D-70 sparse writes): the default value. A scheme file is
    /// sparse against the built-in of its base instead (D-78), whatever changed the value.
    fn save_defaults(&self, path: &Path) -> Option<Value> {
        let _ = path;
        Value::try_from(Self::default()).ok()
    }

    /// Takes out of the raw `table` the entries this build must keep without judging them (T13): they go to the
    /// kept unknown keys with a warning, in both load modes. For the entries of a map keyed by ids that a later
    /// build may know (`commands.toml`).
    fn set_aside(table: &mut Table, origin: &str) -> Vec<SetAside> {
        let _ = (table, origin);
        Vec::new()
    }

    /// Top-level keys that are not fields of this schema but are allowed (kept without a warning).
    fn known_sections() -> &'static [&'static str] {
        &[]
    }
}

/// The M1 files had no versions before 1: a `schema_version = 0` file reads as version 1 (the identity step).
pub(crate) fn migrate_from_zero(from: u32) -> Result<(), String> {
    if from == 0 {
        Ok(())
    } else {
        Err(format!("no migration from schema_version {from}"))
    }
}

/// The default [`SettingsSchema::repair`]: on the serialized tree, each problem key is set to the default's
/// value at the same path (inside an entry of a map or of an array of tables: the entry template's), or removed
/// when there is none and the key is not inside an array of tables. Lenient loading never drops or reorders the
/// elements of an array (the positions in memory are the positions in the file): a problem that names a whole
/// element (`app.1`) leaves it as it is. The tree is then read back (a value that still does not deserialize
/// keeps `T::default()`).
pub(crate) fn repair_by_path<T: SettingsSchema>(value: &mut T, problems: &[Problem]) {
    let Ok(mut tree) = Value::try_from(&*value) else {
        return;
    };
    let defaults = Value::try_from(T::default()).unwrap_or(Value::Table(Table::new()));
    let mut paths: Vec<Vec<KeySegment>> = problems
        .iter()
        .filter_map(|p| super::keys::parse_path(&p.key).ok())
        .collect();
    paths.sort();
    paths.dedup();
    for path in &paths {
        if matches!(path.last(), Some(KeySegment::Index(_))) {
            continue;
        }
        let default = tree::get(&defaults, path)
            .cloned()
            .or_else(|| super::template_default(&T::FILE, path));
        match default {
            Some(default) => {
                let _ = tree::set(&mut tree, path, default);
            }
            None if path.iter().any(|s| matches!(s, KeySegment::Index(_))) => {}
            None => {
                tree::remove(&mut tree, path);
            }
        }
    }
    *value = tree.try_into().unwrap_or_default();
}

/// An entry [`SettingsSchema::set_aside`] took out of a file.
#[derive(Debug, Clone, PartialEq)]
pub struct SetAside {
    /// Where it is in the file.
    pub path: Vec<KeySegment>,
    /// The entry.
    pub value: Value,
    /// What the load reports.
    pub warning: Warning,
}

/// How strictly a file is read (D-71).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LoadMode {
    /// A wrong type, a value out of range or a failed check is an error.
    #[default]
    Strict,
    /// Bad values are replaced by their defaults with warning `invalid_config_value` (the `settings` commands).
    Lenient,
}

/// Whether a save may replace a file changed on disk since it was read (D-74).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Overwrite {
    /// A changed file is `SessionError::Conflict`.
    IfUnchanged,
    /// Write anyway.
    Force,
}

/// One failed check of a value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// The dotted key inside the file (`tweaks.metrics.max_mru_items`, `app.0`).
    pub key: String,
    /// What is wrong.
    pub message: String,
}

impl Problem {
    /// A problem at `key`.
    pub fn new(key: impl Into<String>, message: impl Into<String>) -> Problem {
        Problem {
            key: key.into(),
            message: message.into(),
        }
    }
}

/// A loaded value with what the load found.
#[derive(Debug, Clone, PartialEq)]
pub struct Loaded<T> {
    /// The value (defaults where missing; repaired in lenient mode).
    pub value: T,
    /// The stamp of the bytes that were read.
    pub stamp: ChangeStamp,
    /// The file's `schema_version` when it was migrated (written back on the next save, SES-052).
    pub migrated_from: Option<u32>,
    /// `unknown_config_key` and (lenient) `invalid_config_value` warnings.
    pub warnings: Vec<Warning>,
    /// Every unknown key kept for the next save (dotted), for `--verbose` logging.
    pub ignored_keys: Vec<String>,
}

/// One settings file: where it lives, its kept unknown keys and its loaded value.
#[derive(Debug, Clone)]
pub struct SettingsFile<T> {
    path: PathBuf,
    /// The unknown keys of the file (everything the value does not hold), kept for the next save.
    raw: Table,
    loaded: Loaded<T>,
}

impl<T: SettingsSchema> SettingsFile<T> {
    /// Loads `path` (missing: defaults).
    pub fn load(path: &Path, mode: LoadMode) -> Result<SettingsFile<T>, SessionError> {
        let origin = path.display().to_string();
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(SettingsFile {
                    path: path.to_path_buf(),
                    raw: Table::new(),
                    loaded: Loaded {
                        value: T::default(),
                        stamp: ChangeStamp::Absent,
                        migrated_from: None,
                        warnings: Vec::new(),
                        ignored_keys: Vec::new(),
                    },
                });
            }
            Err(e) => return Err(SessionError::io(path, e)),
        };
        let modified_ns = super::stamp::modified_ns(
            &std::fs::metadata(path).map_err(|e| SessionError::io(path, e))?,
        );
        let text = String::from_utf8(bytes).map_err(|_| SessionError::Config {
            path: origin.clone(),
            message: "the file is not valid UTF-8".into(),
        })?;
        let mut file = SettingsFile::parse(&text, &origin, mode)?;
        file.path = path.to_path_buf();
        file.loaded.stamp = ChangeStamp::of_bytes(text.as_bytes(), modified_ns);
        Ok(file)
    }

    /// Parses the text of a file (`origin` names it in messages and is the path a later save writes to).
    /// The pure core of [`SettingsFile::load`].
    pub fn parse(
        text: &str,
        origin: &str,
        mode: LoadMode,
    ) -> Result<SettingsFile<T>, SessionError> {
        let config_error = |message: String| SessionError::Config {
            path: origin.to_owned(),
            message,
        };
        let mut table: Table =
            toml::from_str(text).map_err(|e| config_error(one_line(&e.to_string())))?;
        // Step 3: the version.
        let found = match table.remove("schema_version") {
            None => T::VERSION,
            Some(Value::Integer(n)) if n < 0 => {
                return Err(config_error(
                    "`schema_version` must be a non-negative integer".into(),
                ));
            }
            Some(Value::Integer(n)) => u32::try_from(n).unwrap_or(u32::MAX),
            Some(_) => return Err(config_error("`schema_version` must be an integer".into())),
        };
        let migrated_from = match check_schema_version(found, T::VERSION) {
            Ok(SchemaCompat::Current) => None,
            Ok(SchemaCompat::Older { found }) => {
                // Step 4: the migration chain.
                for v in found..T::VERSION {
                    T::migrate(v, &mut table).map_err(|message| {
                        config_error(format!("migration from schema_version {v}: {message}"))
                    })?;
                }
                Some(found)
            }
            Err(source) => {
                return Err(SessionError::Schema {
                    path: origin.to_owned(),
                    source,
                });
            }
        };
        let aside = T::set_aside(&mut table, origin);
        let mut warnings = Vec::new();
        // Step 5: the value, type errors located leaf by leaf.
        let defaults = full_table(&T::default());
        let mut value = loop {
            match Value::Table(table.clone()).try_into::<T>() {
                Ok(value) => break value,
                Err(error) => {
                    let Some(bad) = first_bad_leaf::<T>(&table) else {
                        return Err(config_error(one_line(error.message())));
                    };
                    let key = tree::display_path(&bad);
                    let message = type_message::<T>(&table, &defaults, &bad, &key);
                    match mode {
                        LoadMode::Strict => return Err(config_error(message)),
                        LoadMode::Lenient => {
                            let mut root = Value::Table(table);
                            tree::remove(&mut root, &bad);
                            table = match root {
                                Value::Table(t) => t,
                                _ => Table::new(),
                            };
                            warnings.push(invalid(origin, &message));
                        }
                    }
                }
            }
        };
        // Step 6: unknown keys.
        let known = full_table(&value);
        let mut raw = Table::new();
        let mut ignored_keys = Vec::new();
        let mut unknown_warnings = Vec::new();
        for path in tree::unknown_paths(&table, &known) {
            let Some(found) = tree::get_in(&table, &path).cloned() else {
                continue;
            };
            let name = tree::display_path(&path);
            if path.len() == 1 && !T::known_sections().contains(&path[0].as_key().as_str()) {
                unknown_warnings.push(Warning {
                    code: WarningCode::UnknownConfigKey,
                    message: format!("{origin}: unknown section `{name}`"),
                    spec: None,
                });
            }
            match &found {
                Value::Table(inner) => {
                    for (sub, _) in tree::leaves(inner, false) {
                        let mut full = path.clone();
                        full.extend(sub);
                        ignored_keys.push(tree::display_path(&full));
                    }
                }
                _ => ignored_keys.push(name),
            }
            let mut root = Value::Table(raw);
            let _ = tree::set(&mut root, &path, found);
            raw = match root {
                Value::Table(t) => t,
                _ => Table::new(),
            };
        }
        let mut aside_warnings = Vec::new();
        for entry in aside {
            match &entry.value {
                Value::Table(inner) if !inner.is_empty() => {
                    for (sub, _) in tree::leaves(inner, false) {
                        let mut full = entry.path.clone();
                        full.extend(sub);
                        ignored_keys.push(tree::display_path(&full));
                    }
                }
                _ => ignored_keys.push(tree::display_path(&entry.path)),
            }
            let mut root = Value::Table(raw);
            let _ = tree::set(&mut root, &entry.path, entry.value);
            raw = match root {
                Value::Table(t) => t,
                _ => Table::new(),
            };
            aside_warnings.push(entry.warning);
        }
        unknown_warnings.extend(aside_warnings);
        ignored_keys.sort();
        // Step 7: checks.
        let problems = value.validate();
        if let Some(first) = problems.first() {
            match mode {
                LoadMode::Strict => {
                    return Err(config_error(format!("`{}`: {}", first.key, first.message)));
                }
                LoadMode::Lenient => {
                    for problem in &problems {
                        let message = format!("`{}`: {}", problem.key, problem.message);
                        let entry = super::keys::parse_path(&problem.key)
                            .is_ok_and(|p| matches!(p.last(), Some(KeySegment::Index(_))));
                        warnings.push(if entry {
                            Warning {
                                code: WarningCode::InvalidConfigValue,
                                message: format!(
                                    "{origin}: {message} (the entry is kept as it is)"
                                ),
                                spec: None,
                            }
                        } else {
                            invalid(origin, &message)
                        });
                    }
                    value.repair(&problems);
                }
            }
        }
        warnings.splice(0..0, unknown_warnings);
        Ok(SettingsFile {
            path: PathBuf::from(origin),
            raw,
            loaded: Loaded {
                value,
                stamp: ChangeStamp::of_bytes(text.as_bytes(), None),
                migrated_from,
                warnings,
                ignored_keys,
            },
        })
    }

    /// A file of defaults at `path` that was never read (its stamp is `Absent`).
    pub fn new_default(path: &Path) -> SettingsFile<T> {
        SettingsFile {
            path: path.to_path_buf(),
            raw: Table::new(),
            loaded: Loaded {
                value: T::default(),
                stamp: ChangeStamp::Absent,
                migrated_from: None,
                warnings: Vec::new(),
                ignored_keys: Vec::new(),
            },
        }
    }

    /// The file's path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The value.
    pub fn value(&self) -> &T {
        &self.loaded.value
    }

    /// The value, to change before [`SettingsFile::save`].
    pub fn value_mut(&mut self) -> &mut T {
        &mut self.loaded.value
    }

    /// What the load found (warnings, ignored keys, migration).
    pub fn loaded(&self) -> &Loaded<T> {
        &self.loaded
    }

    /// The kept unknown keys.
    pub fn unknown(&self) -> &Table {
        &self.raw
    }

    /// The stamp of the bytes last read or written.
    pub fn stamp(&self) -> &ChangeStamp {
        &self.loaded.stamp
    }

    /// The bytes [`SettingsFile::save`] would write: `schema_version`, the sparse value in declaration order,
    /// the kept unknown keys.
    pub fn to_toml(&self) -> String {
        document(&self.loaded.value, &self.raw, &self.path)
    }

    /// Writes the file if its document differs from the bytes on disk (D-70, D-74). `Ok(false)`: nothing to
    /// write (equal bytes, or only defaults and no file).
    pub fn save(&mut self, overwrite: Overwrite) -> Result<bool, SessionError> {
        if overwrite == Overwrite::IfUnchanged && self.changed_on_disk()? {
            return Err(SessionError::Conflict {
                path: self.path.display().to_string(),
            });
        }
        let doc = self.to_toml();
        let current = match std::fs::read(&self.path) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(SessionError::io(&self.path, e)),
        };
        match &current {
            Some(bytes) if bytes.as_slice() == doc.as_bytes() => return Ok(false),
            None if doc == version_line::<T>() => return Ok(false),
            _ => {}
        }
        if let Some(parent) = self.path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|e| SessionError::io(parent, e))?;
        }
        write_atomic(&self.path, doc.as_bytes()).map_err(|e| SessionError::io(&self.path, e))?;
        self.loaded.stamp = ChangeStamp::of(&self.path)?;
        self.loaded.migrated_from = None;
        Ok(true)
    }

    /// True when the file on disk no longer has the stamp of the load (or of the last save).
    pub fn changed_on_disk(&self) -> Result<bool, SessionError> {
        Ok(ChangeStamp::of(&self.path)? != self.loaded.stamp)
    }
}

/// `schema_version = <VERSION>\n`.
fn version_line<T: SettingsSchema>() -> String {
    format!("schema_version = {}\n", T::VERSION)
}

/// The document of `value` (to be saved at `path`) with the kept unknown keys `raw` (save step 2–3).
pub(crate) fn document<T: SettingsSchema>(value: &T, raw: &Table, path: &Path) -> String {
    let defaults = value.save_defaults(path);
    let sparse = Ordered::of(value)
        .ok()
        .and_then(|o| {
            o.prune(defaults.as_ref(), &mut Vec::new(), &|path| {
                super::template_default(&T::FILE, path)
            })
        })
        .unwrap_or(Ordered::Table(Vec::new()));
    let mut entries = vec![(
        "schema_version".to_owned(),
        Ordered::Scalar(Value::Integer(i64::from(T::VERSION))),
    )];
    if let Ordered::Table(fields) = sparse {
        entries.extend(fields);
    }
    let mut doc = Ordered::Table(entries);
    doc.merge(raw);
    toml::to_string(&doc).unwrap_or_else(|_| version_line::<T>())
}

/// The full serialization of a value as a table (empty when it cannot be serialized).
pub(crate) fn full_table<T: Serialize>(value: &T) -> Table {
    match Value::try_from(value) {
        Ok(Value::Table(table)) => table,
        _ => Table::new(),
    }
}

/// The first leaf of `table` (in key order, array-of-table elements by index) whose presence makes `T` fail to
/// deserialize: the table holding only the leaves up to it fails, the one before it does not.
fn first_bad_leaf<T: SettingsSchema>(table: &Table) -> Option<Vec<KeySegment>> {
    let mut prefix = Value::Table(Table::new());
    for (path, value) in tree::leaves(table, true) {
        pad_elements(&mut prefix, &path);
        if tree::set(&mut prefix, &path, value).is_err() {
            return Some(path);
        }
        if prefix.clone().try_into::<T>().is_err() {
            return Some(path);
        }
    }
    None
}

/// Gives every array of tables on `path` its elements before the indexed one (as empty tables), so that the
/// prefix of [`first_bad_leaf`] keeps the positions of the file (an element without leaves is still there).
fn pad_elements(prefix: &mut Value, path: &[KeySegment]) {
    for (k, segment) in path.iter().enumerate() {
        let KeySegment::Index(i) = segment else {
            continue;
        };
        if tree::get(prefix, &path[..k]).is_none()
            && tree::set(prefix, &path[..k], Value::Array(Vec::new())).is_err()
        {
            return;
        }
        let Some(Value::Array(items)) = tree::get_mut(prefix, &path[..k]) else {
            return;
        };
        while items.len() < *i {
            items.push(Value::Table(Table::new()));
        }
    }
}

/// The message of a type error at `bad`: a known section written as a plain value says so (as in M1); any other
/// value names its key and serde's complaint.
fn type_message<T: SettingsSchema>(
    table: &Table,
    defaults: &Table,
    bad: &[KeySegment],
    key: &str,
) -> String {
    let default = tree::get_in(defaults, bad);
    let found = tree::get_in(table, bad);
    if default.is_some_and(Value::is_table) && found.is_some_and(|v| !v.is_table()) {
        return format!("`{key}` must be a table (a [{key}] section)");
    }
    let mut single = Value::Table(Table::new());
    if let Some(found) = found {
        pad_elements(&mut single, bad);
        let _ = tree::set(&mut single, bad, found.clone());
    }
    match single.try_into::<T>() {
        Err(e) => format!("`{key}`: {}", one_line(e.message())),
        Ok(_) => format!("`{key}` has a value of the wrong type or range"),
    }
}

/// An `invalid_config_value` warning.
fn invalid(origin: &str, message: &str) -> Warning {
    Warning {
        code: WarningCode::InvalidConfigValue,
        message: format!("{origin}: {message} (the default is used)"),
        spec: None,
    }
}

/// A parser message on one line.
pub(crate) fn one_line(message: &str) -> String {
    message.split_whitespace().collect::<Vec<_>>().join(" ")
}
