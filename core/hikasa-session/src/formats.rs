//! [`FormatStore`]: the user formats folder and registry loading (§2.12.4; SES-051, FF-004, D-21, D-31).
//!
//! ```text
//! FormatStore::load(limits):
//!   1. the built-ins in PRESETS order, with their default enabled flags (FormatRegistry::builtin_with_limits)
//!   2. formats/*.toml sorted by file name (order.toml skipped), each through parse_format; a file that cannot
//!      be read or parsed is skipped with `user_format_skipped`; FormatRegistry::with_user_formats merges them
//!      (a user id equal to a built-in replaces it in place; new ids are appended sorted by id)
//!   3. formats/order.toml (FormatOrder::parse; a newer schema_version is an error) through
//!      FormatRegistry::with_order (unknown ids → warning; an id listed twice → DuplicateId)
//! ```
//!
//! Reading never writes: read-only commands never materialise the built-ins (D-21). [`FormatStore::materialize`]
//! runs only on the GUI's first start and on explicit `format` write commands (M2).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use hikasa_core::{Warning, WarningCode};
use hikasa_formats::{FormatOrder, FormatRegistry, RegexLimits, parse_format, to_toml};

use crate::atomic::write_atomic;
use crate::error::SessionError;
use crate::paths::ConfigLocation;

/// File name of the order list inside the formats folder.
pub const ORDER_FILE: &str = "order.toml";

/// User formats folder (`<config>/formats`).
#[derive(Debug, Clone)]
pub struct FormatStore {
    dir: PathBuf,
}

impl FormatStore {
    /// Store for a configuration.
    pub fn new(config: &ConfigLocation) -> FormatStore {
        FormatStore {
            dir: config.dir.join("formats"),
        }
    }

    /// The formats folder (`<config>/formats`; it may not exist).
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Built-ins merged with user files (§2.12.4). Broken user files are skipped with `user_format_skipped`.
    pub fn load(
        &self,
        limits: RegexLimits,
    ) -> Result<(FormatRegistry, Vec<Warning>), SessionError> {
        let builtin = FormatRegistry::builtin_with_limits(limits)?;
        let mut warnings = Vec::new();
        let mut user = Vec::new();
        for path in self.user_files(&mut warnings) {
            let origin = path.display().to_string();
            let text = match std::fs::read(&path) {
                Ok(bytes) => match String::from_utf8(bytes) {
                    Ok(text) => text,
                    Err(_) => {
                        warnings.push(skipped(&origin, "the file is not valid UTF-8"));
                        continue;
                    }
                },
                Err(e) => {
                    warnings.push(skipped(&origin, &e.to_string()));
                    continue;
                }
            };
            match parse_format(&text, &origin) {
                Ok((format, mut file_warnings)) => {
                    warnings.append(&mut file_warnings);
                    user.push((Arc::new(format), path));
                }
                Err(e) => warnings.push(skipped(&origin, &e.to_string())),
            }
        }
        let (mut registry, mut merge_warnings) = builtin.with_user_formats(user);
        warnings.append(&mut merge_warnings);
        let order_path = self.dir.join(ORDER_FILE);
        match std::fs::read(&order_path) {
            Ok(bytes) => {
                let origin = order_path.display().to_string();
                let text = String::from_utf8(bytes).map_err(|_| SessionError::Config {
                    path: origin.clone(),
                    message: "the file is not valid UTF-8".into(),
                })?;
                let (order, mut order_warnings) = FormatOrder::parse(&text, &origin)?;
                warnings.append(&mut order_warnings);
                let (ordered, mut apply_warnings) = registry.with_order(&order)?;
                warnings.append(&mut apply_warnings);
                registry = ordered;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(SessionError::io(&order_path, e)),
        }
        Ok((registry, warnings))
    }

    /// Writes the built-ins and `order.toml` when absent (GUI first run; D-21). Never overwrites.
    ///
    /// Each built-in is written as `formats/<id>.toml` (its canonical TOML, `to_toml`); `order.toml` lists the
    /// loaded registry's order and enabled flags (so a disabled built-in stays disabled, D-27).
    pub fn materialize(&self) -> Result<(), SessionError> {
        let builtin = FormatRegistry::builtin()?;
        std::fs::create_dir_all(&self.dir).map_err(|e| SessionError::io(&self.dir, e))?;
        for entry in builtin.entries() {
            let path = self.dir.join(format!("{}.toml", entry.id()));
            if !exists(&path)? {
                write_atomic(&path, to_toml(&entry.format).as_bytes())
                    .map_err(|e| SessionError::io(&path, e))?;
            }
        }
        let order_path = self.dir.join(ORDER_FILE);
        if !exists(&order_path)? {
            let (registry, _) = self.load(RegexLimits::default())?;
            let order = FormatOrder::from_registry(&registry);
            write_atomic(&order_path, order.to_toml().as_bytes())
                .map_err(|e| SessionError::io(&order_path, e))?;
        }
        Ok(())
    }

    /// `formats/<id>.toml` (it may not exist). Additive (M2 W1, D-75).
    pub fn user_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.toml"))
    }

    /// `formats/order.toml` (it may not exist). Additive (M2 W1).
    pub fn order_path(&self) -> PathBuf {
        self.dir.join(ORDER_FILE)
    }

    /// The order to change (D-75): `order.toml` when it exists, without the ids the registry does not know and
    /// with the unlisted formats appended in registry order (so the effective order is unchanged); else the
    /// registry's order and flags. Additive (M2 W1).
    pub fn editable_order(&self, registry: &FormatRegistry) -> Result<FormatOrder, SessionError> {
        let Some(mut order) = self.read_order()? else {
            return Ok(FormatOrder::from_registry(registry));
        };
        order
            .formats
            .retain(|item| registry.get(&item.id).is_some());
        for entry in registry.entries() {
            if order.position(entry.id()).is_none() {
                order.formats.push(hikasa_formats::OrderItem {
                    id: entry.id().clone(),
                    enabled: None,
                });
            }
        }
        Ok(order)
    }

    /// `order.toml` as it is (`None` when it does not exist). Additive (M2 W1).
    pub fn read_order(&self) -> Result<Option<FormatOrder>, SessionError> {
        let path = self.order_path();
        match std::fs::read(&path) {
            Ok(bytes) => {
                let origin = path.display().to_string();
                let text = String::from_utf8(bytes).map_err(|_| SessionError::Config {
                    path: origin.clone(),
                    message: "the file is not valid UTF-8".into(),
                })?;
                Ok(Some(FormatOrder::parse(&text, &origin)?.0))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(SessionError::io(&path, e)),
        }
    }

    /// Writes `order.toml` (creating the folder). Additive (M2 W1).
    pub fn write_order(&self, order: &FormatOrder) -> Result<PathBuf, SessionError> {
        let path = self.order_path();
        self.write(&path, order.to_toml().as_bytes())?;
        Ok(path)
    }

    /// Writes `formats/<id>.toml` with `bytes` (creating the folder). Additive (M2 W1).
    pub fn write_user_file(&self, id: &str, bytes: &[u8]) -> Result<PathBuf, SessionError> {
        let path = self.user_path(id);
        self.write(&path, bytes)?;
        Ok(path)
    }

    /// Creates the folder and writes `path` atomically.
    fn write(&self, path: &Path, bytes: &[u8]) -> Result<(), SessionError> {
        std::fs::create_dir_all(&self.dir).map_err(|e| SessionError::io(&self.dir, e))?;
        write_atomic(path, bytes).map_err(|e| SessionError::io(path, e))
    }

    /// Every user file: `formats/*.toml` except `order.toml`, sorted (unreadable folders: none). Additive (M2 W1).
    pub fn user_file_paths(&self) -> Vec<PathBuf> {
        self.user_files(&mut Vec::new())
    }

    /// `formats/*.toml` except `order.toml`, sorted by file name. A folder that cannot be read (other than a
    /// missing one) adds one `user_format_skipped` warning: the built-ins stay usable.
    fn user_files(&self, warnings: &mut Vec<Warning>) -> Vec<PathBuf> {
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
            Err(e) => {
                warnings.push(skipped(&self.dir.display().to_string(), &e.to_string()));
                return Vec::new();
            }
        };
        let mut files: Vec<(OsString, PathBuf)> = entries
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                let name = entry.file_name();
                let is_toml = path
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("toml"));
                let is_order = name
                    .to_str()
                    .is_some_and(|n| n.eq_ignore_ascii_case(ORDER_FILE));
                (is_toml && !is_order && path.is_file()).then_some((name, path))
            })
            .collect();
        files.sort_by(|a, b| a.0.as_encoded_bytes().cmp(b.0.as_encoded_bytes()));
        files.into_iter().map(|(_, path)| path).collect()
    }
}

/// A `user_format_skipped` warning.
fn skipped(origin: &str, reason: &str) -> Warning {
    Warning {
        code: WarningCode::UserFormatSkipped,
        message: format!("{origin}: skipped: {reason}"),
        spec: None,
    }
}

/// `path` exists (a metadata error other than "not found" is an I/O error).
fn exists(path: &Path) -> Result<bool, SessionError> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(SessionError::io(path, e)),
    }
}
