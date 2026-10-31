//! Settings packages (M2 W2; contract §1.2.6, D-86, D-87, SES-060…062): a `.hikasa-pkg` is a zip (stored or
//! deflate, no encryption) holding `manifest.toml` and the files at their configuration-relative paths.
//!
//! | Module | Content |
//! |---|---|
//! | [`manifest`] | [`Manifest`], [`ManifestItem`], the allowed path of each [`Category`] ([`item_of`]) |
//! | [`export`](mod@export) | [`export()`]: the zip, written atomically and deterministically |
//! | [`import`](mod@import) | [`plan_import`] (validates everything first), [`execute_import`], [`plan_restore`] |

pub mod export;
pub mod import;
pub mod manifest;

use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Serialize, Serializer};

pub use export::{ExportDto, ExportItem, ExportRequest, export};
pub use import::{
    CategoryCounts, ImportChoices, ImportPlan, ImportReport, ItemResult, MAX_ENTRIES,
    MAX_ENTRY_BYTES, MAX_TOTAL_BYTES, PlanAction, PlanItem, execute_import, plan_import,
    plan_restore,
};
pub use manifest::{MANIFEST_FILE, MANIFEST_SCHEMA_VERSION, Manifest, ManifestItem, item_of};

use crate::api::ApiError;
use crate::sessions::store::entries;

/// The extension of settings packages.
pub const PACKAGE_EXTENSION: &str = "hikasa-pkg";

/// The file a typed package path names: only an absolute path is taken. Nothing is expanded (`~`) and core has no
/// current folder to resolve against (the GUI's would be a place the user never sees).
pub fn package_path(text: &str) -> Result<std::path::PathBuf, String> {
    let path = std::path::PathBuf::from(text.trim());
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(format!(
            "{:?} is not a full path: enter the whole path of the package file.",
            text.trim()
        ))
    }
}

/// A category of a package (D-86).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Category {
    /// `options.toml`, `filters.toml`, `openwith.toml`, `archives.toml`.
    Options,
    /// `theme.toml`, `themes/*.toml`.
    Theme,
    /// `commands.toml`.
    Commands,
    /// `state.toml`.
    State,
    /// Named sessions and `_defaults` (never `_autosaved`).
    Sessions,
    /// `formats/*.toml` and `order.toml`.
    Formats,
    /// `workspaces/*.toml`.
    Workspaces,
    /// `system-info.txt` (export only).
    SystemInfo,
}

impl Category {
    /// Every category, in package order.
    pub const ALL: [Category; 8] = [
        Category::Options,
        Category::Theme,
        Category::Commands,
        Category::State,
        Category::Sessions,
        Category::Formats,
        Category::Workspaces,
        Category::SystemInfo,
    ];

    /// The name (`options`, …, `system_info`).
    pub const fn as_str(self) -> &'static str {
        match self {
            Category::Options => "options",
            Category::Theme => "theme",
            Category::Commands => "commands",
            Category::State => "state",
            Category::Sessions => "sessions",
            Category::Formats => "formats",
            Category::Workspaces => "workspaces",
            Category::SystemInfo => "system_info",
        }
    }

    /// Parses a name; `profiles` is reserved (P1, `unsupported`), anything else unknown `invalid_input`.
    pub fn parse(text: &str) -> Result<Category, ApiError> {
        if let Some(c) = Category::ALL.into_iter().find(|c| c.as_str() == text) {
            return Ok(c);
        }
        if text == "profiles" {
            return Err(ApiError::Unsupported(
                "the category `profiles` is not supported in this version".into(),
            ));
        }
        Err(ApiError::InvalidArgument(format!(
            "unknown category {text:?} (options, theme, commands, state, sessions, formats, workspaces, \
             system_info)"
        )))
    }

    /// Categories whose items are chosen one by one (and that "delete existing" applies to).
    pub const fn per_item(self) -> bool {
        matches!(
            self,
            Category::Sessions | Category::Formats | Category::Workspaces
        )
    }
}

impl fmt::Display for Category {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for Category {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

/// Parses `<CAT>:<NAME>` (`--item`).
pub fn parse_item(text: &str) -> Result<(Category, String), ApiError> {
    let (category, name) = text.split_once(':').ok_or_else(|| {
        ApiError::InvalidArgument(format!("{text:?}: an item is <category>:<name>"))
    })?;
    Ok((Category::parse(category)?, name.to_owned()))
}

/// The files of a category in a configuration folder: `(name, relative path)`, sorted by path. System info is
/// generated, so it has none here.
pub fn category_files(config_dir: &Path, category: Category) -> Vec<(String, String)> {
    let fixed = |names: &[&str]| -> Vec<(String, String)> {
        names
            .iter()
            .filter(|n| config_dir.join(format!("{n}.toml")).is_file())
            .map(|n| ((*n).to_owned(), format!("{n}.toml")))
            .collect()
    };
    let mut out = match category {
        Category::Options => fixed(&["archives", "filters", "openwith", "options"]),
        Category::Theme => {
            let mut out = fixed(&["theme"]);
            out.extend(folder_items(config_dir, "themes", false));
            out
        }
        Category::Commands => fixed(&["commands"]),
        Category::State => fixed(&["state"]),
        Category::Sessions => folder_items(config_dir, "sessions", true),
        Category::Formats => folder_items(config_dir, "formats", false),
        Category::Workspaces => folder_items(config_dir, "workspaces", false),
        Category::SystemInfo => Vec::new(),
    };
    out.retain(|(_, path)| item_of(path).is_some_and(|(c, _)| c == category));
    out.sort_by(|a, b| a.1.cmp(&b.1));
    out
}

/// The `*.toml` files below `<config>/<folder>` whose paths are items ([`item_of`]); with `recursive`, also in
/// sub-folders (never `_autosaved`).
fn folder_items(config_dir: &Path, folder: &str, recursive: bool) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut stack: Vec<String> = vec![folder.to_owned()];
    while let Some(rel) = stack.pop() {
        let dir: PathBuf = config_dir.join(&rel);
        for (name, is_dir) in entries(&dir) {
            let path = format!("{rel}/{name}");
            if is_dir {
                if recursive && name != crate::sessions::store::AUTOSAVED_FOLDER {
                    stack.push(path);
                }
            } else if let Some((_, item)) = item_of(&path) {
                out.push((item, path));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories_and_items_parse() {
        for c in Category::ALL {
            assert_eq!(Category::parse(c.as_str()).unwrap(), c);
        }
        assert!(matches!(
            Category::parse("profiles"),
            Err(ApiError::Unsupported(_))
        ));
        assert!(matches!(
            Category::parse("x"),
            Err(ApiError::InvalidArgument(_))
        ));
        assert_eq!(
            parse_item("sessions:work/a").unwrap(),
            (Category::Sessions, "work/a".to_owned())
        );
        assert!(parse_item("sessions").is_err());
    }
}
