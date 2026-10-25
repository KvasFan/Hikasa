//! The configuration and cache folders (§2.22, SES-050): [`ConfigLocation`], [`ConfigMode`],
//! [`PORTABLE_MARKER`], [`resolve_config_dir`], [`default_config_dir`].
//!
//! Everything here is pure: the host facts come from [`EnvFacts`] and the one file-system question ("does the
//! portable marker exist?") is an injected predicate. Read-only commands never create either folder (§2.22);
//! missing files mean defaults.
//!
//! ```text
//! resolve_config_dir(facts, explicit, exists):
//!   1. explicit = Some(d) → absolute(d, facts.cwd) (RelativeConfigDir if relative without cwd) → Explicit
//!   2. facts.exe_dir = Some(x) && exists(x / "hikasa-portable.toml") → x → Portable
//!   3. platform default (default_config_dir) → User:
//!        Linux/OtherUnix: (facts.xdg_config_home if absolute, else facts.home/.config) / "hikasa"
//!        MacOs:           facts.home / "Library/Application Support/Hikasa"
//!        Windows:         facts.appdata / "Hikasa"
//!      a missing ingredient → NoConfigDir
//!   cache_dir:  Explicit/Portable → dir / "cache"
//!               User → Linux/OtherUnix: (facts.xdg_cache_home if absolute, else facts.home/.cache) / "hikasa";
//!                      MacOs: facts.home / "Library/Caches/Hikasa"; Windows: facts.local_appdata / "Hikasa" / "cache"
//!               (a missing ingredient → facts.temp_dir / ("hikasa-cache-" + id), id as in §2.23: the uid, else the
//!                sanitised user name, else "user"; wave-4 review)
//!   is_default: dir == default_config_dir(facts)
//! ```

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::SessionError;
use crate::host::{EnvFacts, Platform};

/// Name of the portable-mode marker file (SES-050).
pub const PORTABLE_MARKER: &str = "hikasa-portable.toml";

/// How the configuration folder was chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigMode {
    /// `--config-dir`.
    Explicit,
    /// The portable marker next to the executable.
    Portable,
    /// The platform's per-user default.
    User,
}

/// The configuration folder and the cache folder (§2.22).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigLocation {
    /// Folder (may not exist yet; read paths tolerate absence).
    pub dir: PathBuf,
    /// Mode.
    pub mode: ConfigMode,
    /// Cache folder (spooled stdin for `gui open -`, D-49); created on first write only.
    pub cache_dir: PathBuf,
    /// True when `dir` equals the platform default (no endpoint suffix, D-47).
    pub is_default: bool,
}

/// Resolves the configuration folder (§2.22). Pure: `exists` is injected.
pub fn resolve_config_dir(
    facts: &EnvFacts,
    explicit: Option<&Path>,
    exists: &dyn Fn(&Path) -> bool,
) -> Result<ConfigLocation, SessionError> {
    let default = default_config_dir(facts);
    let (dir, mode) = if let Some(dir) = explicit {
        if dir.as_os_str().is_empty() {
            return Err(SessionError::EmptyConfigDir);
        }
        (absolute(facts, dir)?, ConfigMode::Explicit)
    } else if let Some(exe_dir) = facts
        .exe_dir
        .as_ref()
        .filter(|x| exists(&x.join(PORTABLE_MARKER)))
    {
        (exe_dir.clone(), ConfigMode::Portable)
    } else {
        let dir = default.clone().ok_or(SessionError::NoConfigDir)?;
        (dir, ConfigMode::User)
    };
    let cache_dir = match mode {
        ConfigMode::Explicit | ConfigMode::Portable => dir.join("cache"),
        ConfigMode::User => default_cache_dir(facts).unwrap_or_else(|| {
            facts
                .temp_dir
                .join(format!("hikasa-cache-{}", user_id(facts)))
        }),
    };
    let is_default = default.as_deref() == Some(dir.as_path());
    Ok(ConfigLocation {
        dir,
        mode,
        cache_dir,
        is_default,
    })
}

/// The platform default configuration folder (§2.22 step 3), or `None` when an ingredient is missing.
pub fn default_config_dir(facts: &EnvFacts) -> Option<PathBuf> {
    match facts.platform {
        Platform::Linux | Platform::OtherUnix => {
            let base = absolute_fact(facts.platform, facts.xdg_config_home.as_deref())
                .or_else(|| facts.home.as_ref().map(|home| home.join(".config")))?;
            Some(base.join("hikasa"))
        }
        Platform::MacOs => facts.home.as_ref().map(|home| {
            home.join("Library")
                .join("Application Support")
                .join("Hikasa")
        }),
        Platform::Windows => facts.appdata.as_ref().map(|appdata| appdata.join("Hikasa")),
    }
}

/// The per-user id of the shared fallback folders (§2.22, §2.23): the uid, else the sanitised user name (every
/// char outside `[A-Za-z0-9_.-]` becomes `_`), else `user`.
pub(crate) fn user_id(facts: &EnvFacts) -> String {
    match (facts.uid, facts.user.as_deref()) {
        (Some(uid), _) => uid.to_string(),
        (None, Some(user)) if !user.is_empty() => sanitize(user),
        _ => "user".to_owned(),
    }
}

/// Every char outside `[A-Za-z0-9_.-]` becomes `_`.
pub(crate) fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// The platform default cache folder (§2.22 `cache_dir` for `User`), or `None` when an ingredient is missing.
fn default_cache_dir(facts: &EnvFacts) -> Option<PathBuf> {
    match facts.platform {
        Platform::Linux | Platform::OtherUnix => {
            let base = absolute_fact(facts.platform, facts.xdg_cache_home.as_deref())
                .or_else(|| facts.home.as_ref().map(|home| home.join(".cache")))?;
            Some(base.join("hikasa"))
        }
        Platform::MacOs => facts
            .home
            .as_ref()
            .map(|home| home.join("Library").join("Caches").join("Hikasa")),
        Platform::Windows => facts
            .local_appdata
            .as_ref()
            .map(|local| local.join("Hikasa").join("cache")),
    }
}

/// `path` when it is absolute for `platform` (XDG variables must be absolute to count).
fn absolute_fact(platform: Platform, path: Option<&Path>) -> Option<PathBuf> {
    path.filter(|p| is_absolute_for(platform, p))
        .map(Path::to_path_buf)
}

/// Absoluteness by the platform's rules, independent of the compilation target (so the Windows rows of §2.22
/// are unit-tested on Linux): Unix paths start with `/`; Windows paths start with a drive and a separator
/// (`C:\`, `C:/`) or are UNC/verbatim paths (`\\server\share`, `\\?\…`).
pub(crate) fn is_absolute_for(platform: Platform, path: &Path) -> bool {
    let text = path.to_string_lossy();
    let bytes = text.as_bytes();
    match platform {
        Platform::Windows => {
            let drive = bytes.len() >= 3
                && bytes[0].is_ascii_alphabetic()
                && bytes[1] == b':'
                && matches!(bytes[2], b'\\' | b'/');
            let unc = bytes.len() >= 2 && matches!(bytes[0], b'\\' | b'/') && bytes[0] == bytes[1];
            drive || unc
        }
        Platform::Linux | Platform::MacOs | Platform::OtherUnix => bytes.first() == Some(&b'/'),
    }
}

/// `--config-dir` made absolute against the captured current directory; `.` components are dropped (`..` is
/// kept, as for local specs, D-06).
fn absolute(facts: &EnvFacts, dir: &Path) -> Result<PathBuf, SessionError> {
    let joined = if is_absolute_for(facts.platform, dir) {
        dir.to_path_buf()
    } else {
        match &facts.cwd {
            Some(cwd) => cwd.join(dir),
            None => return Err(SessionError::RelativeConfigDir(dir.to_path_buf())),
        }
    };
    Ok(joined
        .components()
        .filter(|c| !matches!(c, Component::CurDir))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absoluteness_per_platform() {
        let w = |s: &str| is_absolute_for(Platform::Windows, Path::new(s));
        let u = |s: &str| is_absolute_for(Platform::Linux, Path::new(s));
        assert!(w("C:\\Users") && w("c:/x") && w("\\\\srv\\share") && w("\\\\?\\C:\\x"));
        assert!(!w("C:x") && !w("x\\y") && !w("\\x") && !w(""));
        assert!(u("/x") && !u("x") && !u("") && !u("C:\\x"));
    }
}
