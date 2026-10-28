//! The settings store (M2 W1; contract §1.1, D-70…D-78): one engine for every program-owned settings file
//! ([`SettingsFile`]), change detection by polling ([`ConfigWatch`]), the key grammar of `hikasa settings`
//! ([`SettingsKey`]) and the typed schemas of `options.toml`, `state.toml`, `commands.toml`, `theme.toml`,
//! `themes/<name>.toml`, `filters.toml`, `openwith.toml` and `archives.toml`.
//!
//! | Module | Content |
//! |---|---|
//! | [`file`](mod@file) | [`SettingsSchema`], [`SettingsFile`]: versions, migration, unknown keys, lenient repair, sparse atomic save |
//! | [`stamp`] | [`ChangeStamp`], [`ConfigWatch`] (D-74) |
//! | [`keys`] | [`FileId`], [`SettingsKey`], [`parse_key`], [`ValueKind`], [`KeyInfo`], [`key_infos`] (D-73) |
//! | [`options`] | [`Options`]: docs/15 "Keys" |
//! | [`state`] | [`State`]: MRU lists, window, flags, dialogs |
//! | [`commands`] | [`CommandOverrides`], [`Shortcut`], [`ShortcutConflict`] (UI-003) |
//! | [`theme`] | [`ThemeSelection`], [`Scheme`], [`Colour`] (D-78) |
//! | [`filters`] | [`FilterPresets`] (OPT-020) |
//! | [`openwith`] | [`OpenWith`], [`expand_command`] (OPT-012, D-76) |
//! | [`archives`] | [`ArchiveTypes`] (OPT-010) |

use std::path::{Path, PathBuf};

use hikasa_core::{CaseSensitivity, MaskList, MaskOptions, Warning, WarningCode};
use serde::Serialize;
use toml::Value;

use crate::error::SessionError;
use crate::paths::ConfigLocation;

/// A string enum of a settings file: serde names, `ALL` (every spelling), `as_str`, `Default`, `Display`.
macro_rules! settings_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident = $text:literal),+ $(,)? } default $default:ident) => {
        $(#[$meta])*
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
        )]
        pub enum $name {
            $(
                #[doc = concat!("`", $text, "`.")]
                #[serde(rename = $text)]
                $variant,
            )+
        }

        impl $name {
            /// Every spelling, in declaration order.
            pub const ALL: &'static [&'static str] = &[$($text),+];

            /// The spelling in the file.
            pub const fn as_str(self) -> &'static str {
                match self {
                    $($name::$variant => $text,)+
                }
            }
        }

        impl Default for $name {
            fn default() -> $name {
                $name::$default
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

/// A table of a settings file: every field has a default (`#[serde(default)]`), so missing keys read as
/// defaults and unknown keys are ignored by serde (and kept by [`SettingsFile`]).
macro_rules! settings_section {
    ($(#[$meta:meta])* $name:ident {
        $($(#[$fmeta:meta])* $field:ident : $ty:ty = $default:expr),* $(,)?
    }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
        #[serde(default)]
        pub struct $name {
            $($(#[$fmeta])* pub $field: $ty,)*
        }

        impl Default for $name {
            fn default() -> $name {
                $name {
                    $($field: $default,)*
                }
            }
        }
    };
}

pub mod archives;
pub mod commands;
pub mod file;
pub mod filters;
pub mod keys;
pub mod openwith;
pub mod options;
pub mod stamp;
pub mod state;
pub mod theme;
pub(crate) mod tree;

pub use archives::{ArchiveType, ArchiveTypes};
pub use commands::{
    CommandOverride, CommandOverrides, Shortcut, ShortcutConflict, ViewCustomization,
};
pub use file::{LoadMode, Loaded, Overwrite, Problem, SettingsFile, SettingsSchema};
pub use filters::{FilterPreset, FilterPresets};
pub use keys::{FileId, KeyInfo, KeySegment, SettingsKey, ValueKind, key_infos, parse_key};
pub use openwith::{
    ExpandContext, ExpandError, ExpandItem, InstanceMode, OpenWith, OpenWithApp, WorkingFolder,
    expand_command, split_words,
};
pub use options::Options;
pub use stamp::{ChangeKind, ChangeStamp, ConfigChange, ConfigWatch};
pub use state::State;
pub use theme::{
    Colour, FontSpec, Fonts, Scheme, SchemeFile, SyntaxStyle, ThemeMode, ThemeSelection,
};

/// The settings files of one configuration folder.
#[derive(Debug, Clone)]
pub struct SettingsStore {
    dir: PathBuf,
}

impl SettingsStore {
    /// The store of a configuration.
    pub fn new(config: &ConfigLocation) -> SettingsStore {
        SettingsStore {
            dir: config.dir.clone(),
        }
    }

    /// The configuration folder.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The path of a file (`themes` without a name: the `themes` folder).
    pub fn path(&self, file: FileId) -> PathBuf {
        match &file {
            FileId::Themes(name) if name.is_empty() => self.dir.join("themes"),
            other => self.dir.join(other.relative_path()),
        }
    }

    /// Loads a file of a fixed schema.
    pub fn file<T: SettingsSchema>(&self, mode: LoadMode) -> Result<SettingsFile<T>, SessionError> {
        SettingsFile::load(&self.path(T::FILE), mode)
    }

    /// Loads `themes/<name>.toml`.
    pub fn scheme_file(
        &self,
        name: &str,
        mode: LoadMode,
    ) -> Result<SettingsFile<SchemeFile>, SessionError> {
        SettingsFile::load(&self.path(FileId::Themes(name.to_owned())), mode)
    }

    /// `options.toml`.
    pub fn options(&self, mode: LoadMode) -> Result<Loaded<Options>, SessionError> {
        Ok(self.file::<Options>(mode)?.loaded().clone())
    }

    /// `state.toml`.
    pub fn state(&self, mode: LoadMode) -> Result<Loaded<State>, SessionError> {
        Ok(self.file::<State>(mode)?.loaded().clone())
    }

    /// `commands.toml`.
    pub fn commands(&self, mode: LoadMode) -> Result<Loaded<CommandOverrides>, SessionError> {
        Ok(self.file::<CommandOverrides>(mode)?.loaded().clone())
    }

    /// `theme.toml`. A selected scheme that is neither built in nor a file of `themes/` is an error (strict) or
    /// falls back to the built-in of its mode with warning `invalid_config_value` (lenient).
    pub fn theme(&self, mode: LoadMode) -> Result<Loaded<ThemeSelection>, SessionError> {
        let mut loaded = self.file::<ThemeSelection>(mode)?.loaded().clone();
        let origin = self.path(FileId::Theme).display().to_string();
        let names = self.scheme_names();
        let value = &mut loaded.value;
        for (key, slot, builtin) in [
            ("light_scheme", &mut value.light_scheme, "light"),
            ("dark_scheme", &mut value.dark_scheme, "dark"),
        ] {
            if names.iter().any(|n| n == slot) {
                continue;
            }
            let message = format!("{key}: the scheme {slot:?} does not exist");
            match mode {
                LoadMode::Strict => {
                    return Err(SessionError::Config {
                        path: origin,
                        message,
                    });
                }
                LoadMode::Lenient => {
                    loaded.warnings.push(Warning {
                        code: WarningCode::InvalidConfigValue,
                        message: format!("{origin}: {message} (the built-in {builtin} is used)"),
                        spec: None,
                    });
                    *slot = builtin.to_owned();
                }
            }
        }
        Ok(loaded)
    }

    /// `themes/<name>.toml` (missing: an empty file).
    pub fn scheme(&self, name: &str, mode: LoadMode) -> Result<Loaded<SchemeFile>, SessionError> {
        Ok(self.scheme_file(name, mode)?.loaded().clone())
    }

    /// The scheme `name` with every element ([`Scheme::resolve`]) and the warnings of its file.
    pub fn resolved_scheme(
        &self,
        name: &str,
        mode: LoadMode,
    ) -> Result<(Scheme, Vec<Warning>), SessionError> {
        let loaded = self.scheme(name, mode)?;
        Ok((Scheme::resolve(name, Some(&loaded.value)), loaded.warnings))
    }

    /// `filters.toml`.
    pub fn filters(&self, mode: LoadMode) -> Result<Loaded<FilterPresets>, SessionError> {
        Ok(self.file::<FilterPresets>(mode)?.loaded().clone())
    }

    /// `openwith.toml`.
    pub fn openwith(&self, mode: LoadMode) -> Result<Loaded<OpenWith>, SessionError> {
        Ok(self.file::<OpenWith>(mode)?.loaded().clone())
    }

    /// `archives.toml`.
    pub fn archives(&self, mode: LoadMode) -> Result<Loaded<ArchiveTypes>, SessionError> {
        Ok(self.file::<ArchiveTypes>(mode)?.loaded().clone())
    }

    /// The scheme names: the built-ins and the stems of `themes/*.toml` that are scheme names, sorted.
    pub fn scheme_names(&self) -> Vec<String> {
        let mut names: Vec<String> = theme::BUILTIN_SCHEMES
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        names.extend(self.scheme_files().into_iter().map(|(name, _)| name));
        names.sort();
        names.dedup();
        names
    }

    /// The scheme files that exist: `(name, path)`, sorted by name.
    pub fn scheme_files(&self) -> Vec<(String, PathBuf)> {
        let Ok(entries) = std::fs::read_dir(self.path(FileId::Themes(String::new()))) else {
            return Vec::new();
        };
        let mut files: Vec<(String, PathBuf)> = entries
            .flatten()
            .filter_map(|e| {
                let path = e.path();
                let is_toml = path.extension().is_some_and(|x| x == "toml");
                let stem = path.file_stem()?.to_str()?.to_owned();
                (is_toml && keys::is_scheme_name(&stem) && path.is_file()).then_some((stem, path))
            })
            .collect();
        files.sort();
        files
    }

    /// A watch over every settings file, the scheme files and the format files (D-74).
    pub fn watch(&self) -> ConfigWatch {
        let mut watch = ConfigWatch::new(FileId::FIXED.into_iter().map(|f| self.path(f)));
        watch.track_folder(self.path(FileId::Themes(String::new())));
        watch.track_folder(self.dir.join("formats"));
        watch
    }
}

/// The kind rules of a file.
pub(crate) fn rules(file: &FileId) -> &'static [keys::KindRule] {
    match file {
        FileId::Options => options::RULES,
        FileId::State => state::RULES,
        FileId::Commands => commands::RULES,
        FileId::Theme => theme::SELECTION_RULES,
        FileId::Themes(_) => theme::SCHEME_RULES,
        FileId::Filters => filters::RULES,
        FileId::OpenWith => openwith::RULES,
        FileId::Archives => archives::RULES,
        FileId::Session | FileId::Workspace => &[],
    }
}

/// The defaults of the entries of a file's maps and arrays of tables (pattern → default table).
pub(crate) fn templates(file: &FileId) -> Vec<(&'static str, Value)> {
    match file {
        FileId::Commands => commands::templates(),
        FileId::Filters => filters::templates(),
        FileId::OpenWith => openwith::templates(),
        FileId::Workspace => crate::workspaces::templates(),
        // A scheme has no templates: its defaults are its built-in, and a syntax entry must keep what it sets.
        FileId::Options
        | FileId::State
        | FileId::Theme
        | FileId::Themes(_)
        | FileId::Archives
        | FileId::Session => Vec::new(),
    }
}

/// The default of a path inside an entry of a map or of an array of tables (its template), if any.
pub(crate) fn template_default(file: &FileId, path: &[KeySegment]) -> Option<Value> {
    templates(file).into_iter().find_map(|(pattern, template)| {
        let n = pattern.split('.').count();
        let rule = keys::KindRule {
            pattern,
            kind: ValueKind::Table,
            opt: None,
            priority: None,
        };
        (path.len() >= n && rule.matches(&path[..n]))
            .then(|| tree::get(&template, &path[n..]).cloned())
            .flatten()
    })
}

/// The defaults of a file as a full tree. A scheme's defaults are its built-in (with `base` for a scheme that is
/// not built in), so a scheme file only holds what differs from it.
pub(crate) fn defaults_tree(file: &FileId, scheme: Option<&SchemeFile>) -> Value {
    let table = match file {
        FileId::Options => file::full_table(&Options::default()),
        FileId::State => file::full_table(&State::default()),
        FileId::Commands => file::full_table(&CommandOverrides::default()),
        FileId::Theme => file::full_table(&ThemeSelection::default()),
        FileId::Themes(name) => {
            let base = Scheme::base_of(name, scheme);
            let builtin = theme::BUILTIN_SCHEMES.contains(&name.as_str());
            file::full_table(&Scheme::builtin(base).to_file((!builtin).then_some(base)))
        }
        FileId::Filters => file::full_table(&FilterPresets::default()),
        FileId::OpenWith => file::full_table(&OpenWith::default()),
        FileId::Archives => file::full_table(&ArchiveTypes::default()),
        FileId::Session | FileId::Workspace => toml::Table::new(),
    };
    Value::Table(table)
}

/// The checks every schema shares, driven by its kind rules: integer and float ranges, masks and shortcuts.
pub(crate) fn validate_by_rules<T: Serialize>(value: &T, file: &FileId) -> Vec<Problem> {
    let table = file::full_table(value);
    let mut problems = Vec::new();
    for (path, leaf) in tree::leaves(&table, true) {
        let Some(rule) = keys::rule_for(file, &path) else {
            continue;
        };
        let key = tree::display_path(&path);
        match (rule.kind, &leaf) {
            (ValueKind::Int { min, max }, Value::Integer(n)) if *n < min || *n > max => {
                problems.push(Problem::new(
                    key,
                    format!("must be from {min} to {max}, found {n}"),
                ));
            }
            (ValueKind::Float { min, max }, Value::Float(x)) if !(*x >= min && *x <= max) => {
                problems.push(Problem::new(
                    key,
                    format!("must be from {min} to {max}, found {x}"),
                ));
            }
            (ValueKind::Mask, Value::String(mask)) => {
                if let Err(e) = check_mask(mask) {
                    problems.push(Problem::new(key, e));
                }
            }
            (ValueKind::Shortcut, Value::String(text)) if !text.is_empty() => {
                if let Err(e) = Shortcut::parse(text) {
                    problems.push(Problem::new(key, e));
                }
            }
            _ => {}
        }
    }
    problems
}

/// A mask list parses (`hikasa_core::mask`, case-insensitive; empty is valid: disabled).
pub(crate) fn check_mask(mask: &str) -> Result<(), String> {
    if mask.is_empty() {
        return Ok(());
    }
    MaskList::parse(
        mask,
        MaskOptions {
            case: CaseSensitivity::Insensitive,
        },
    )
    .map(|_| ())
    .map_err(|e| e.to_string())
}
