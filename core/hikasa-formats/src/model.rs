//! The file-format model (FF §1), persisted as TOML: one file per format (FF-004).
//!
//! These are configuration types (T5): field order is the canonical TOML key order written by
//! [`crate::to_toml`], maps are `BTreeMap`, and every type round-trips through TOML (INV-FMT-1).

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::grammar::GrammarItem;

/// A string that is not a valid [`FormatId`] or [`ElementName`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid {what} {value:?}: {reason}")]
pub struct InvalidName {
    /// `"format id"` or `"element name"`.
    pub what: &'static str,
    /// The rejected text.
    pub value: String,
    /// Why it was rejected.
    pub reason: &'static str,
}

/// Stable format id: `[a-z0-9][a-z0-9-]{0,63}`. It is also the file stem: `formats/<id>.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct FormatId(String);

impl FormatId {
    /// Validates `id`.
    pub fn new(id: impl Into<String>) -> Result<FormatId, InvalidName> {
        let id = id.into();
        let invalid = |reason| InvalidName {
            what: "format id",
            value: id.clone(),
            reason,
        };
        let bytes = id.as_bytes();
        let Some(&first) = bytes.first() else {
            return Err(invalid("empty"));
        };
        if bytes.len() > 64 {
            return Err(invalid("longer than 64 characters"));
        }
        if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
            return Err(invalid("must start with a-z or 0-9"));
        }
        if !bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
        {
            return Err(invalid("only a-z, 0-9 and '-' are allowed"));
        }
        Ok(FormatId(id))
    }

    /// The id text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for FormatId {
    type Error = InvalidName;
    fn try_from(value: String) -> Result<FormatId, InvalidName> {
        FormatId::new(value)
    }
}

impl From<FormatId> for String {
    fn from(id: FormatId) -> String {
        id.0
    }
}

impl fmt::Display for FormatId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for FormatId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::str::FromStr for FormatId {
    type Err = InvalidName;
    fn from_str(s: &str) -> Result<FormatId, InvalidName> {
        FormatId::new(s)
    }
}

/// Grammar element name: free text, trimmed, 1..=64 chars, no `,`. Built-ins use Comment, String, Keyword,
/// Identifier, Number, Operator, Directive, EnvironmentVar, LineNum, Identification, Indent, Whitespace, Char,
/// Attribute, Annotation, Entity, Variable, RegexLiteral (Appendix A). Names compare exactly (case-sensitive);
/// importance rules reference them by name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ElementName(String);

impl ElementName {
    /// Validates `name` (surrounding whitespace is trimmed first).
    pub fn new(name: impl Into<String>) -> Result<ElementName, InvalidName> {
        let raw = name.into();
        let trimmed = raw.trim();
        let invalid = |reason| InvalidName {
            what: "element name",
            value: raw.clone(),
            reason,
        };
        let chars = trimmed.chars().count();
        if chars == 0 {
            return Err(invalid("empty"));
        }
        if chars > 64 {
            return Err(invalid("longer than 64 characters"));
        }
        if trimmed.contains(',') {
            return Err(invalid("',' is not allowed"));
        }
        if trimmed.chars().any(char::is_control) {
            return Err(invalid("control characters are not allowed"));
        }
        Ok(ElementName(trimmed.to_owned()))
    }

    /// The name text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The name of `ElementId(0)` ("Everything Else"; reserved for it).
    pub(crate) fn everything_else() -> ElementName {
        ElementName(crate::compile::EVERYTHING_ELSE.to_owned())
    }
}

impl TryFrom<String> for ElementName {
    type Error = InvalidName;
    fn try_from(value: String) -> Result<ElementName, InvalidName> {
        ElementName::new(value)
    }
}

impl From<ElementName> for String {
    fn from(name: ElementName) -> String {
        name.0
    }
}

impl fmt::Display for ElementName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for ElementName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Kind of view a format selects (FF §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FormatKind {
    /// Text compare.
    Text,
    /// Table compare (compared as plain text in M1, D-11).
    Table,
    /// Hex compare.
    Hex,
    /// Picture compare (binary comparison in M1).
    Picture,
    /// Media compare (binary comparison in M1).
    Media,
    /// Version compare (binary comparison in M1).
    Version,
    /// Registry compare (binary comparison in M1).
    Registry,
    /// An external program (FF-010, P1).
    External,
}

impl FormatKind {
    /// Stable snake_case name, identical to the serde form.
    pub const fn as_str(self) -> &'static str {
        match self {
            FormatKind::Text => "text",
            FormatKind::Table => "table",
            FormatKind::Hex => "hex",
            FormatKind::Picture => "picture",
            FormatKind::Media => "media",
            FormatKind::Version => "version",
            FormatKind::Registry => "registry",
            FormatKind::External => "external",
        }
    }
}

impl fmt::Display for FormatKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Per-element settings (FF-025).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct ElementInfo {
    /// Changes in letter case inside this element are important (FF-025). Default true.
    pub case_sensitive: bool,
    /// Default importance of the element's text; `ImportanceRules::elements` overrides it by name (D-04).
    /// Default true. The Python `Whitespace` and COBOL `Identification` elements ship with `false`.
    pub important: bool,
}

impl Default for ElementInfo {
    /// `case_sensitive = true`, `important = true`.
    fn default() -> ElementInfo {
        ElementInfo {
            case_sensitive: true,
            important: true,
        }
    }
}

/// Text settings (Text and Table kinds).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct TextSettings {
    /// Tab stop (1..=32). Default 4.
    pub tab_size: u8,
    /// Insert spaces instead of tabs (editing; unused in M1 engines).
    pub tab_as_spaces: bool,
    /// Characters-per-line limit (TC-003); `None` = unlimited.
    pub chars_per_line_limit: Option<u32>,
    /// Lines are independent: every differing row is its own section (FF Misc).
    pub lines_independent: bool,
    /// Column-based data (reserved for M2 table compare).
    pub column_based: bool,
    /// Treat the first Ctrl-Z (0x1A) as end of file (ASCII-compatible encodings only).
    pub ctrl_z_eof: bool,
    /// Save options (TC-005; unused in M1).
    pub save_trim_trailing_ws: bool,
    /// Save options (TC-005; unused in M1).
    pub save_leading_spaces_to_tabs: bool,
    /// Extra characters that count as word characters for List items (word boundaries and keyword lookup,
    /// §2.10.2; D-58). Default empty. COBOL: `"-"`.
    pub word_chars: String,
}

impl Default for TextSettings {
    /// Tab stop 4; every flag off; no limit; no extra word characters.
    fn default() -> TextSettings {
        TextSettings {
            tab_size: 4,
            tab_as_spaces: false,
            chars_per_line_limit: None,
            lines_independent: false,
            column_based: false,
            ctrl_z_eof: false,
            save_trim_trailing_ws: false,
            save_leading_spaces_to_tabs: false,
            word_chars: String::new(),
        }
    }
}

fn yes() -> bool {
    true
}

fn sixty() -> u32 {
    60
}

fn auto_encoding() -> EncodingSetting {
    EncodingSetting::Auto { fallback: None }
}

/// A line weight (TC-012): lines matching `pattern` get `priority` (1..=4) as anchor preference.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LineWeight {
    /// Text or regex searched anywhere in the raw line (no EOL).
    pub pattern: String,
    /// `pattern` is a regex.
    #[serde(default)]
    pub regex: bool,
    /// Case-sensitive matching. Default true.
    #[serde(default = "yes")]
    pub match_case: bool,
    /// 1..=4.
    pub priority: u8,
}

/// Encoding setting (FF-032).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum EncodingSetting {
    /// BOM → UTF-16/32 pattern → pure ASCII (`fallback`, then locale) → UTF-8 validity → detector (§2.8, D-42).
    Auto {
        /// Label used for content without non-ASCII evidence (pure ASCII); `None` → the adapter's locale
        /// encoding. Ignored when the label's encoding is not ASCII-compatible.
        #[serde(default)]
        fallback: Option<String>,
    },
    /// Fixed code page (a BOM still wins; see §2.8 step 2).
    Fixed {
        /// WHATWG label or "UTF-32LE"/"UTF-32BE".
        label: String,
    },
}

impl Default for EncodingSetting {
    /// `Auto` without a fallback.
    fn default() -> EncodingSetting {
        auto_encoding()
    }
}

/// Pre-load conversion (FF-030). M1 implements None, Sorted and External.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum Conversion {
    /// Plain text.
    #[default]
    None,
    /// Sort lines before comparing (text stage, D-38). The document is read-only.
    Sorted,
    /// External program (byte stage, D-39).
    External(ExternalConversion),
}

/// External conversion command lines (`%s` source, `%t` target, `%o` original).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ExternalConversion {
    /// Load command line (required).
    pub load: String,
    /// Save command line (`None` = converted documents are read-only, FF-031).
    #[serde(default)]
    pub save: Option<String>,
    /// How file names are passed. M1: `Unicode` only; `Ansi` is accepted and treated as `Unicode`.
    #[serde(default)]
    pub filename_encoding: FilenameEncoding,
    /// Seconds before the program is killed (default 60; 0 = no limit).
    #[serde(default = "sixty")]
    pub timeout_secs: u32,
}

/// File name encoding for external programs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilenameEncoding {
    /// UTF-8 / UTF-16 native.
    #[default]
    Unicode,
    /// Legacy code page (Windows).
    Ansi,
}

/// Delimited-table settings (parsed and validated now; used by M2 table compare).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TableSettings {
    /// Field delimiter (`,` or `\t` …).
    pub delimiter: char,
    /// Text qualifier (`"`).
    #[serde(default)]
    pub qualifier: Option<char>,
    /// Treat consecutive delimiters as one.
    #[serde(default)]
    pub consecutive_as_one: bool,
}

/// A file format (FF §1). Field order is the canonical TOML key order written by `to_toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileFormat {
    /// Must be ≤ FORMAT_SCHEMA_VERSION (SES-052).
    pub schema_version: u32,
    /// Stable id.
    pub id: FormatId,
    /// Display name ("C, C++, C#, ObjC Source").
    pub name: String,
    /// Description.
    #[serde(default)]
    pub description: String,
    /// Kind.
    pub kind: FormatKind,
    /// `;`-separated name masks; empty = manual selection only (FF-001).
    #[serde(default)]
    pub mask: String,
    /// Conversion.
    #[serde(default)]
    pub conversion: Conversion,
    /// Encoding.
    #[serde(default = "auto_encoding")]
    pub encoding: EncodingSetting,
    /// Text settings.
    #[serde(default)]
    pub text: TextSettings,
    /// Element settings by name (an unordered map: `ElementId`s follow first use in `grammar`, D-41).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub elements: BTreeMap<ElementName, ElementInfo>,
    /// Grammar items, ordered (first match wins, FF-026).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grammar: Vec<GrammarItem>,
    /// Line weights.
    #[serde(default, rename = "line_weight", skip_serializing_if = "Vec::is_empty")]
    pub line_weights: Vec<LineWeight>,
    /// Table settings (Table kind).
    #[serde(default)]
    pub table: Option<TableSettings>,
}

impl FileFormat {
    /// A format with the given identity and every other field at its default (current schema version, no
    /// mask, no conversion, Auto encoding, default text settings, no elements, no grammar, no weights).
    pub fn new(id: FormatId, name: impl Into<String>, kind: FormatKind) -> FileFormat {
        FileFormat {
            schema_version: crate::toml_io::FORMAT_SCHEMA_VERSION,
            id,
            name: name.into(),
            description: String::new(),
            kind,
            mask: String::new(),
            conversion: Conversion::None,
            encoding: auto_encoding(),
            text: TextSettings::default(),
            elements: BTreeMap::new(),
            grammar: Vec::new(),
            line_weights: Vec::new(),
            table: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_ids() {
        assert!(FormatId::new("c-cpp").is_ok());
        assert!(FormatId::new("0x").is_ok());
        assert!(FormatId::new("a".repeat(64)).is_ok());
        for bad in ["", "-a", "A", "a_b", "a b", "é", &"a".repeat(65)] {
            assert!(FormatId::new(bad).is_err(), "{bad:?}");
        }
        let err = FormatId::new("C").unwrap_err();
        assert_eq!(
            err.to_string(),
            "invalid format id \"C\": must start with a-z or 0-9"
        );
        assert_eq!("rust".parse::<FormatId>().unwrap().as_str(), "rust");
    }

    #[test]
    fn element_names() {
        assert_eq!(ElementName::new("  Comment ").unwrap().as_str(), "Comment");
        assert!(ElementName::new("Line Num").is_ok());
        assert!(ElementName::new("é".repeat(64)).is_ok());
        for bad in ["", "   ", "a,b", "a\tb", &"x".repeat(65)] {
            assert!(ElementName::new(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn defaults() {
        assert_eq!(
            ElementInfo::default(),
            ElementInfo {
                case_sensitive: true,
                important: true
            }
        );
        assert_eq!(TextSettings::default().tab_size, 4);
        assert_eq!(
            EncodingSetting::default(),
            EncodingSetting::Auto { fallback: None }
        );
        assert_eq!(Conversion::default(), Conversion::None);
        assert_eq!(FormatKind::Registry.to_string(), "registry");
    }
}
