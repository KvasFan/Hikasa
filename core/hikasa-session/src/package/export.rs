//! [`export`] (SES-060, D-86): the files of the chosen categories and items, plus `manifest.toml` and, when
//! asked, `system-info.txt`, as a zip written atomically. Entries are sorted by path and carry the manifest's
//! `created` time, so two exports of the same configuration at the same instant are byte-identical.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

use serde::Serialize;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, System, ZipWriter};

use super::manifest::{MANIFEST_FILE, Manifest, ManifestItem, SYSTEM_INFO_FILE};
use super::{Category, category_files};
use crate::api::{ApiError, HIKASA_VERSION};
use crate::atomic::write_atomic;
use crate::error::SessionError;
use crate::host::EnvFacts;
use crate::paths::ConfigLocation;

/// What to export.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExportRequest {
    /// The categories (with `items`, the categories of the items are added; both empty: every category except
    /// `system_info`).
    pub categories: Vec<Category>,
    /// Per category, the item names to export (absent or empty: all).
    pub items: BTreeMap<Category, Vec<String>>,
}

/// One exported item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExportItem {
    /// The category.
    pub category: Category,
    /// The item name.
    pub name: String,
    /// The path inside the package.
    pub path: String,
}

/// `hikasa.settings_export.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExportDto {
    /// The package written.
    pub path: String,
    /// The items, sorted by path.
    pub items: Vec<ExportItem>,
}

/// Writes a settings package to `out` (atomically). An item named in `request.items` that does not exist is
/// `not_found`.
pub fn export(
    config: &ConfigLocation,
    request: &ExportRequest,
    now: &jiff::Zoned,
    env: &EnvFacts,
    out: &Path,
) -> Result<ExportDto, ApiError> {
    // Item keys select their categories; with neither, every category but system_info.
    let categories: Vec<Category> = if request.categories.is_empty() && request.items.is_empty() {
        Category::ALL
            .into_iter()
            .filter(|c| *c != Category::SystemInfo)
            .collect()
    } else {
        let mut c = request.categories.clone();
        c.extend(request.items.keys().copied());
        c.sort();
        c.dedup();
        c
    };
    let mut files: Vec<(ExportItem, Vec<u8>)> = Vec::new();
    let mut counts: Vec<(Category, usize)> = Vec::new();
    for category in &categories {
        if *category == Category::SystemInfo {
            continue;
        }
        let wanted = request.items.get(category).filter(|v| !v.is_empty());
        let available = category_files(&config.dir, *category);
        if let Some(wanted) = wanted {
            for name in wanted {
                if !available.iter().any(|(n, _)| n == name) {
                    return Err(ApiError::NotFound(format!("no {category} item `{name}`")));
                }
            }
        }
        let mut n = 0;
        for (name, path) in available {
            if wanted.is_some_and(|w| !w.contains(&name)) {
                continue;
            }
            let full = config.dir.join(&path);
            let bytes = std::fs::read(&full).map_err(|e| SessionError::io(&full, e))?;
            files.push((
                ExportItem {
                    category: *category,
                    name,
                    path,
                },
                bytes,
            ));
            n += 1;
        }
        counts.push((*category, n));
    }
    if categories.contains(&Category::SystemInfo) {
        let text = system_info(config, env, &counts);
        files.push((
            ExportItem {
                category: Category::SystemInfo,
                name: "system-info".into(),
                path: SYSTEM_INFO_FILE.into(),
            },
            text.into_bytes(),
        ));
    }
    files.sort_by(|a, b| a.0.path.cmp(&b.0.path));
    let created = now
        .timestamp()
        .display_with_offset(now.offset())
        .to_string();
    let manifest = Manifest::new(
        created,
        files
            .iter()
            .map(|(item, _)| ManifestItem {
                category: item.category.as_str().to_owned(),
                name: item.name.clone(),
                path: item.path.clone(),
            })
            .collect(),
    );
    let mut entries: Vec<(String, Vec<u8>)> = files
        .iter()
        .map(|(item, bytes)| (item.path.clone(), bytes.clone()))
        .collect();
    entries.push((MANIFEST_FILE.to_owned(), manifest.to_toml().into_bytes()));
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let bytes = zip_bytes(&entries, zip_time(now))
        .map_err(|e| ApiError::Internal(format!("cannot build the package: {e}")))?;
    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| SessionError::io(parent, e))?;
    }
    write_atomic(out, &bytes).map_err(|e| SessionError::io(out, e))?;
    Ok(ExportDto {
        path: out.display().to_string(),
        items: files.into_iter().map(|(item, _)| item).collect(),
    })
}

/// The zip of `entries` (in the given order), deflated, every entry with `time`.
pub(crate) fn zip_bytes(
    entries: &[(String, Vec<u8>)],
    time: DateTime,
) -> zip::result::ZipResult<Vec<u8>> {
    let mut zip = ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .last_modified_time(time)
        .system(System::Unix)
        .unix_permissions(0o644);
    for (path, bytes) in entries {
        zip.start_file(path.as_str(), options)?;
        zip.write_all(bytes)?;
    }
    Ok(zip.finish()?.into_inner())
}

/// The zip time of `now` (local civil time; outside the zip range 1980…2107: 1980-01-01).
fn zip_time(now: &jiff::Zoned) -> DateTime {
    let dt = now.datetime();
    let part = |v: i8| u8::try_from(v).unwrap_or(0);
    u16::try_from(dt.year())
        .ok()
        .and_then(|year| {
            DateTime::from_date_and_time(
                year,
                part(dt.month()),
                part(dt.day()),
                part(dt.hour()),
                part(dt.minute()),
                part(dt.second()),
            )
            .ok()
        })
        .unwrap_or_default()
}

/// `system-info.txt`: version, platform, configuration mode and folder (the home folder written `~`), item
/// counts. No user names, no environment values.
fn system_info(config: &ConfigLocation, env: &EnvFacts, counts: &[(Category, usize)]) -> String {
    let folder = match &env.home {
        Some(home) => match config.dir.strip_prefix(home) {
            Ok(rest) => Path::new("~").join(rest).display().to_string(),
            Err(_) => config.dir.display().to_string(),
        },
        None => config.dir.display().to_string(),
    };
    let mode = match config.mode {
        crate::ConfigMode::Explicit => "explicit (--config-dir)",
        crate::ConfigMode::Portable => "portable",
        crate::ConfigMode::User => "per-user",
    };
    let platform = serde_json::to_value(env.platform)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default();
    let mut text = format!(
        "Hikasa {HIKASA_VERSION}\nplatform: {platform}\nconfiguration: {mode}\nconfiguration folder: {folder}\n"
    );
    for (category, n) in counts {
        text.push_str(&format!("{category}: {n} item(s)\n"));
    }
    text
}
