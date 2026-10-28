//! `manifest.toml` of a settings package (§1.2.6) and the allowed paths of each category (D-86).
//!
//! | Category | Paths (configuration-relative, `/`-separated) | Item name |
//! |---|---|---|
//! | `options` | `options.toml`, `filters.toml`, `openwith.toml`, `archives.toml` | the stem |
//! | `theme` | `theme.toml`, `themes/<scheme>.toml` | `theme`, `themes/<scheme>` |
//! | `commands`, `state` | `commands.toml`, `state.toml` | the stem |
//! | `sessions` | `sessions/<folder>/…/<name>.toml`, `sessions/_defaults/<type>.toml` | the reference, `_defaults/<type>` |
//! | `formats` | `formats/<id>.toml`, `formats/order.toml` | the id, `order` |
//! | `workspaces` | `workspaces/<name>.toml` | the name |
//! | `system_info` | `system-info.txt` | `system-info` |

use hikasa_core::{SchemaCompat, check_schema_version};
use hikasa_formats::FormatId;
use serde::{Deserialize, Serialize};

use super::Category;
use crate::api::{ApiError, HIKASA_VERSION};
use crate::sessions::{SessionName, SessionType};
use crate::settings::keys::is_scheme_name;

/// The manifest's file name inside the zip.
pub const MANIFEST_FILE: &str = "manifest.toml";

/// `schema_version` of manifests.
pub const MANIFEST_SCHEMA_VERSION: u32 = 1;

/// The `format` value of a manifest.
pub const PACKAGE_FORMAT: &str = "hikasa-pkg";

/// The system information file of a package.
pub const SYSTEM_INFO_FILE: &str = "system-info.txt";

/// `manifest.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// `1`.
    pub schema_version: u32,
    /// `hikasa-pkg`.
    pub format: String,
    /// The version that wrote the package.
    pub hikasa_version: String,
    /// When it was written (RFC 3339 with offset, the injected clock).
    pub created: String,
    /// The items (`[[item]]`), sorted by path.
    #[serde(rename = "item", default)]
    pub items: Vec<ManifestItem>,
}

/// One item of a manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestItem {
    /// The category name.
    pub category: String,
    /// The item name ([`item_of`]).
    pub name: String,
    /// The configuration-relative path (`/`-separated).
    pub path: String,
}

impl Manifest {
    /// A manifest of this build.
    pub fn new(created: String, items: Vec<ManifestItem>) -> Manifest {
        Manifest {
            schema_version: MANIFEST_SCHEMA_VERSION,
            format: PACKAGE_FORMAT.to_owned(),
            hikasa_version: HIKASA_VERSION.to_owned(),
            created,
            items,
        }
    }

    /// The TOML of the manifest.
    pub fn to_toml(&self) -> String {
        toml::to_string(self).unwrap_or_default()
    }

    /// Parses and checks a manifest: TOML shape and `format` (`invalid_data`), a newer `schema_version`
    /// (`unsupported`).
    pub fn parse(text: &str) -> Result<Manifest, ApiError> {
        let bad = |m: String| ApiError::InvalidData(format!("{MANIFEST_FILE}: {m}"));
        let table: toml::Table = toml::from_str(text).map_err(|e| bad(e.message().to_owned()))?;
        if let Some(toml::Value::Integer(v)) = table.get("schema_version") {
            let v = u32::try_from(*v).map_err(|_| bad("bad schema_version".into()))?;
            match check_schema_version(v, MANIFEST_SCHEMA_VERSION) {
                Ok(SchemaCompat::Current | SchemaCompat::Older { .. }) => {}
                Err(e) => return Err(ApiError::Unsupported(format!("{MANIFEST_FILE}: {e}"))),
            }
        }
        let manifest: Manifest = toml::Value::Table(table)
            .try_into()
            .map_err(|e: toml::de::Error| bad(e.message().to_owned()))?;
        if manifest.format != PACKAGE_FORMAT {
            return Err(bad(format!(
                "format {:?} is not {PACKAGE_FORMAT:?}",
                manifest.format
            )));
        }
        Ok(manifest)
    }
}

/// The category and item name of a configuration-relative path, or `None` when no category allows it.
pub fn item_of(path: &str) -> Option<(Category, String)> {
    if path == SYSTEM_INFO_FILE {
        return Some((Category::SystemInfo, "system-info".into()));
    }
    let stem_path = path.strip_suffix(".toml")?;
    let parts: Vec<&str> = stem_path.split('/').collect();
    match parts.as_slice() {
        [name @ ("options" | "filters" | "openwith" | "archives")] => {
            Some((Category::Options, (*name).to_owned()))
        }
        ["theme"] => Some((Category::Theme, "theme".into())),
        ["themes", scheme] if is_scheme_name(scheme) => {
            Some((Category::Theme, format!("themes/{scheme}")))
        }
        ["commands"] => Some((Category::Commands, "commands".into())),
        ["state"] => Some((Category::State, "state".into())),
        ["sessions", "_defaults", kind] if SessionType::parse(kind).is_ok() => {
            Some((Category::Sessions, format!("_defaults/{kind}")))
        }
        ["sessions", rest @ ..]
            if !rest.is_empty() && rest.iter().all(|s| SessionName::parse(s).is_ok()) =>
        {
            Some((Category::Sessions, rest.join("/")))
        }
        ["formats", "order"] => Some((Category::Formats, "order".into())),
        ["formats", id] if FormatId::new(*id).is_ok() => {
            Some((Category::Formats, (*id).to_owned()))
        }
        ["workspaces", name] if SessionName::parse(name).is_ok() => {
            Some((Category::Workspaces, (*name).to_owned()))
        }
        _ => None,
    }
}

/// Checks the spelling of an entry path (D-86): not empty, `/`-separated, no backslash, not absolute (no
/// leading `/`, no drive letter), no empty, `.` or `..` segment, no control character.
pub fn check_entry_path(path: &str) -> Result<(), String> {
    if path.is_empty() {
        return Err("an empty path".into());
    }
    if path.contains('\\') {
        return Err("a backslash in the path".into());
    }
    if path.starts_with('/') || path.as_bytes().get(1) == Some(&b':') {
        return Err("an absolute path".into());
    }
    if path.chars().any(char::is_control) {
        return Err("a control character in the path".into());
    }
    for segment in path.trim_end_matches('/').split('/') {
        match segment {
            "" => return Err("an empty path segment".into()),
            "." | ".." => return Err("a `.` or `..` segment".into()),
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_map_to_items() {
        let rows = [
            ("options.toml", Some((Category::Options, "options"))),
            ("archives.toml", Some((Category::Options, "archives"))),
            ("theme.toml", Some((Category::Theme, "theme"))),
            ("themes/mine.toml", Some((Category::Theme, "themes/mine"))),
            ("commands.toml", Some((Category::Commands, "commands"))),
            ("state.toml", Some((Category::State, "state"))),
            (
                "sessions/work/api.toml",
                Some((Category::Sessions, "work/api")),
            ),
            (
                "sessions/_defaults/text-compare.toml",
                Some((Category::Sessions, "_defaults/text-compare")),
            ),
            ("sessions/_defaults/nope.toml", None),
            ("sessions/_autosaved/20261028-120000-1.toml", None),
            ("formats/order.toml", Some((Category::Formats, "order"))),
            ("formats/my-c.toml", Some((Category::Formats, "my-c"))),
            ("workspaces/last.toml", Some((Category::Workspaces, "last"))),
            ("workspaces/a/b.toml", None),
            (
                "system-info.txt",
                Some((Category::SystemInfo, "system-info")),
            ),
            ("other.toml", None),
            ("options.txt", None),
        ];
        for (path, want) in rows {
            assert_eq!(
                item_of(path),
                want.map(|(c, n)| (c, n.to_owned())),
                "{path}"
            );
        }
    }

    #[test]
    fn entry_paths_are_checked() {
        for good in ["options.toml", "sessions/a/b.toml", "themes/"] {
            assert!(check_entry_path(good).is_ok(), "{good}");
        }
        for bad in [
            "",
            "/etc/passwd",
            "C:/x.toml",
            "a\\b.toml",
            "../x.toml",
            "a/../b.toml",
            "a//b.toml",
            "./a.toml",
            "a/\u{1}.toml",
        ] {
            assert!(check_entry_path(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn manifests_round_trip_and_check() {
        let m = Manifest::new(
            "2026-10-28T12:00:00+01:00".into(),
            vec![ManifestItem {
                category: "sessions".into(),
                name: "work/api-v2".into(),
                path: "sessions/work/api-v2.toml".into(),
            }],
        );
        assert_eq!(Manifest::parse(&m.to_toml()).unwrap(), m);
        let newer = m
            .to_toml()
            .replace("schema_version = 1", "schema_version = 9");
        assert!(matches!(
            Manifest::parse(&newer),
            Err(ApiError::Unsupported(_))
        ));
        let other = m.to_toml().replace("hikasa-pkg", "zip");
        assert!(matches!(
            Manifest::parse(&other),
            Err(ApiError::InvalidData(_))
        ));
    }
}
