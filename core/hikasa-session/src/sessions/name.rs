//! Session and workspace names (D-80): [`SessionName`], [`SessionRef`] and the auto-saved slot id [`AutoId`].
//!
//! A name (and a folder name) has 1–100 characters, none of `/ \ : * ? " < > |` or a control character, no
//! leading or trailing space or dot, does not start with `_` (which keeps `_defaults` and `_autosaved` apart) and
//! is not a Windows device name (`CON`, `PRN`, `AUX`, `NUL`, `COM1`…`COM9`, `COM¹`…`COM³`, `LPT1`…`LPT9`,
//! `LPT¹`…`LPT³`, `CONIN$`, `CONOUT$`, in any case, also with an extension). Names are unique per folder case-insensitively ([`SessionName::folded`]).

use std::fmt;
use std::path::PathBuf;

use crate::api::ApiError;

/// Longest name, in characters.
pub const MAX_NAME_CHARS: usize = 100;

/// Characters a name never contains.
const FORBIDDEN: &[char] = &['/', '\\', ':', '*', '?', '"', '<', '>', '|'];

/// Windows device names (compared case-insensitively, before the first `.`).
const DEVICES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "COM¹", "COM²", "COM³", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8",
    "LPT9", "LPT¹", "LPT²", "LPT³", "CONIN$", "CONOUT$",
];

/// A validated session, folder or workspace name (D-80).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SessionName(String);

impl SessionName {
    /// Validates `text` (`invalid_input` naming the rule it breaks).
    pub fn parse(text: &str) -> Result<SessionName, ApiError> {
        check_name(text)
            .map(|()| SessionName(text.to_owned()))
            .map_err(|why| ApiError::InvalidArgument(format!("invalid name {text:?}: {why}")))
    }

    /// The name.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The name for case-insensitive comparison (lower case).
    pub fn folded(&self) -> String {
        self.0.to_lowercase()
    }
}

impl fmt::Display for SessionName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Why `text` is not a valid name (D-80).
pub(crate) fn check_name(text: &str) -> Result<(), &'static str> {
    let chars = text.chars().count();
    if chars == 0 {
        return Err("a name is not empty");
    }
    if chars > MAX_NAME_CHARS {
        return Err("a name has at most 100 characters");
    }
    if text
        .chars()
        .any(|c| FORBIDDEN.contains(&c) || c.is_control())
    {
        return Err(r#"a name contains none of / \ : * ? " < > | or control characters"#);
    }
    if text.starts_with([' ', '.']) || text.ends_with([' ', '.']) {
        return Err("a name does not start or end with a space or a dot");
    }
    if text.starts_with('_') {
        return Err("a name does not start with `_` (reserved)");
    }
    let stem = text.split('.').next().unwrap_or(text).trim_end();
    if DEVICES.iter().any(|d| d.eq_ignore_ascii_case(stem)) {
        return Err("a name is not a reserved device name (CON, NUL, COM1, …)");
    }
    Ok(())
}

/// A named session: its folder path and its name (`work/api-v2`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SessionRef {
    /// The folders, outermost first (empty: the top level).
    pub folder: Vec<SessionName>,
    /// The name.
    pub name: SessionName,
}

impl SessionRef {
    /// Parses `a/b/c` (`invalid_input` for an empty segment or an invalid name).
    pub fn parse(text: &str) -> Result<SessionRef, ApiError> {
        let mut parts = parse_folder(text)?;
        let name = parts
            .pop()
            .ok_or_else(|| ApiError::InvalidArgument("a session reference is not empty".into()))?;
        Ok(SessionRef {
            folder: parts,
            name,
        })
    }

    /// The file path relative to the `sessions` folder (`work/api-v2.toml`).
    pub fn relative_path(&self) -> PathBuf {
        let mut path: PathBuf = self.folder.iter().map(SessionName::as_str).collect();
        path.push(format!("{}.toml", self.name));
        path
    }

    /// The reference folded for case-insensitive comparison.
    pub fn folded(&self) -> String {
        self.to_string().to_lowercase()
    }
}

impl fmt::Display for SessionRef {
    /// The folders and the name joined by `/`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for folder in &self.folder {
            write!(f, "{folder}/")?;
        }
        f.write_str(self.name.as_str())
    }
}

/// Parses a folder path `a/b` (every segment a valid name; empty text is the top level).
pub fn parse_folder(text: &str) -> Result<Vec<SessionName>, ApiError> {
    if text.is_empty() {
        return Ok(Vec::new());
    }
    text.split('/').map(SessionName::parse).collect()
}

/// The id of an auto-saved session: `<YYYYMMDD>-<HHMMSS>-<n>` (D-83).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AutoId(String);

impl AutoId {
    /// Parses an id (`invalid_input` when it does not have the D-83 shape).
    pub fn parse(text: &str) -> Result<AutoId, ApiError> {
        let parts: Vec<&str> = text.split('-').collect();
        let digits = |s: &str, n: Option<usize>| {
            !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) && n.is_none_or(|n| s.len() == n)
        };
        match parts.as_slice() {
            [date, time, n]
                if digits(date, Some(8)) && digits(time, Some(6)) && digits(n, None) =>
            {
                Ok(AutoId(text.to_owned()))
            }
            _ => Err(ApiError::InvalidArgument(format!(
                "{text:?} is not an auto-saved session id (YYYYMMDD-HHMMSS-N)"
            ))),
        }
    }

    /// The id for `now` and counter `n`.
    pub(crate) fn at(now: &jiff::Zoned, n: u32) -> AutoId {
        AutoId(format!("{}-{n}", now.strftime("%Y%m%d-%H%M%S")))
    }

    /// The id.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AutoId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_d80() {
        let long = "a".repeat(100);
        let too_long = "a".repeat(101);
        let rows: &[(&str, bool)] = &[
            ("api-v2", true),
            ("Weekly review", true),
            ("v1.2", true),
            ("ünïcode", true),
            (&long, true),
            ("", false),
            (&too_long, false),
            ("a/b", false),
            ("a\\b", false),
            ("a:b", false),
            ("a*b", false),
            ("a?b", false),
            ("a\"b", false),
            ("a<b", false),
            ("a>b", false),
            ("a|b", false),
            ("a\tb", false),
            ("a\u{7}b", false),
            (" lead", false),
            ("trail ", false),
            (".lead", false),
            ("trail.", false),
            ("_defaults", false),
            ("_x", false),
            ("CON", false),
            ("con", false),
            ("Nul.txt", false),
            ("COM1", false),
            ("lpt9", false),
            ("COM10", true),
            ("console", true),
        ];
        for (text, ok) in rows {
            assert_eq!(SessionName::parse(text).is_ok(), *ok, "{text:?}");
        }
    }

    /// The superscript-digit ports and the console devices are reserved too, in any case, with an extension.
    #[test]
    fn superscript_and_console_devices_are_reserved() {
        for text in [
            "COM¹",
            "com²",
            "Com³.toml",
            "LPT¹",
            "lpt²",
            "LPT³.x",
            "CONIN$",
            "conin$",
            "CONOUT$",
            "conout$.txt",
        ] {
            assert!(SessionName::parse(text).is_err(), "{text:?}");
        }
        for text in ["COM⁴", "CONIN", "CONOUT$1", "conin$x"] {
            assert!(SessionName::parse(text).is_ok(), "{text:?}");
        }
    }

    #[test]
    fn references_and_ids() {
        let r = SessionRef::parse("work/api/v2").unwrap();
        assert_eq!(r.folder.len(), 2);
        assert_eq!(r.to_string(), "work/api/v2");
        assert_eq!(r.relative_path(), PathBuf::from("work/api/v2.toml"));
        assert!(SessionRef::parse("work//v2").is_err());
        assert!(SessionRef::parse("").is_err());
        assert!(SessionRef::parse("_autosaved/x").is_err());
        assert!(AutoId::parse("20261028-120000-1").is_ok());
        assert!(AutoId::parse("2026-120000-1").is_err());
        assert!(AutoId::parse("20261028-120000-").is_err());
    }
}
