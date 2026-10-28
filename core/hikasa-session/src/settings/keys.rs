//! Settings keys (D-73): [`FileId`], [`SettingsKey`], [`KeySegment`], [`parse_key`], [`ValueKind`],
//! [`KeyInfo`] and [`key_infos`].
//!
//! ```text
//! key      = file [ "." path ]
//! file     = "options" | "state" | "commands" | "theme" | "filters" | "openwith" | "archives"
//!          | "themes" "." segment                      (a scheme: themes/<name>.toml)
//! path     = segment { "." segment }
//! segment  = bare | quoted | index
//! bare     = 1*( ALPHA / DIGIT / "_" / "-" )          (all digits: an index)
//! quoted   = DQUOTE *( char / "\" DQUOTE / "\\" ) DQUOTE
//! ```
//!
//! An index segment also names the table key with the same digits (a table has no positions). Kinds,
//! OPT ids and priorities come from the rules of each file's schema ([`KindRule`]); a leaf without a rule gets
//! the kind of its default value.

use std::fmt;

use serde::Serialize;
use toml::Value;

use super::tree;
use crate::api::ApiError;

/// One segment of a key path.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum KeySegment {
    /// A table key.
    Name(String),
    /// An array position (also the table key with the same digits).
    Index(usize),
}

impl KeySegment {
    /// The segment as a table key (an index as its digits).
    pub fn as_key(&self) -> String {
        match self {
            KeySegment::Name(name) => name.clone(),
            KeySegment::Index(i) => i.to_string(),
        }
    }
}

impl fmt::Display for KeySegment {
    /// A bare key as it is, an index as its digits, any other key in double quotes.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeySegment::Index(i) => write!(f, "{i}"),
            KeySegment::Name(name) if is_bare(name) => f.write_str(name),
            KeySegment::Name(name) => {
                f.write_str("\"")?;
                for c in name.chars() {
                    match c {
                        '"' => f.write_str("\\\"")?,
                        '\\' => f.write_str("\\\\")?,
                        c => write!(f, "{c}")?,
                    }
                }
                f.write_str("\"")
            }
        }
    }
}

/// True for a TOML bare key: one or more ASCII letters, digits, `_` and `-`.
pub(crate) fn is_bare(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// A settings file (D-70, D-72).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FileId {
    /// `options.toml` (docs/15).
    Options,
    /// `state.toml` (docs/03 §5.2).
    State,
    /// `commands.toml` (OPT-011).
    Commands,
    /// `theme.toml` (docs/14 §2.1).
    Theme,
    /// `themes/<name>.toml`; an empty name means every scheme file (restore defaults only).
    Themes(String),
    /// `filters.toml` (OPT-020).
    Filters,
    /// `openwith.toml` (OPT-012).
    OpenWith,
    /// `archives.toml` (OPT-010).
    Archives,
    /// A session file (`sessions/…/<name>.toml`, D-80; M2 W2). An engine file only: no settings key names it
    /// (`hikasa session edit` addresses its keys).
    Session,
    /// A workspace file (`workspaces/<name>.toml`, D-85; M2 W2). An engine file only, like [`FileId::Session`].
    Workspace,
}

impl FileId {
    /// The fixed files, in list order (the schemes come after `theme`).
    pub const FIXED: [FileId; 7] = [
        FileId::Options,
        FileId::State,
        FileId::Commands,
        FileId::Theme,
        FileId::Filters,
        FileId::OpenWith,
        FileId::Archives,
    ];

    /// Every settings file of `settings restore-defaults --all` (D-77): the fixed files and every scheme
    /// (`Themes` without a name).
    pub fn every() -> Vec<FileId> {
        let mut files = FileId::FIXED.to_vec();
        files.insert(4, FileId::Themes(String::new()));
        files
    }

    /// The key prefix: `options`, …, `themes.<name>` (`themes` for every scheme).
    pub fn name(&self) -> String {
        match self {
            FileId::Options => "options".into(),
            FileId::State => "state".into(),
            FileId::Commands => "commands".into(),
            FileId::Theme => "theme".into(),
            FileId::Themes(name) if name.is_empty() => "themes".into(),
            FileId::Themes(name) => format!("themes.{}", KeySegment::Name(name.clone())),
            FileId::Filters => "filters".into(),
            FileId::OpenWith => "openwith".into(),
            FileId::Archives => "archives".into(),
            FileId::Session => "session".into(),
            FileId::Workspace => "workspace".into(),
        }
    }

    /// The file's path relative to the configuration folder (`options.toml`, `themes/dark.toml`; the folder
    /// `sessions` or `workspaces` for the engine-only files).
    pub fn relative_path(&self) -> String {
        match self {
            FileId::Themes(name) => format!("themes/{name}.toml"),
            FileId::Session => "sessions".into(),
            FileId::Workspace => "workspaces".into(),
            other => format!("{}.toml", other.name()),
        }
    }

    /// The file named by a key prefix word (`options`, `themes.dark`, `themes` = every scheme).
    pub fn parse(text: &str) -> Option<FileId> {
        let key = parse_key(text).ok()?;
        key.path.is_empty().then_some(key.file)
    }

    /// OPT id and priority of the whole file, for files that are one option group (docs/15).
    pub(crate) fn opt(&self) -> Option<(&'static str, &'static str)> {
        match self {
            FileId::Archives => Some(("OPT-010", "P0")),
            FileId::Commands => Some(("OPT-011", "P0")),
            FileId::OpenWith => Some(("OPT-012", "P0")),
            FileId::Filters => Some(("OPT-020", "P0")),
            FileId::Options
            | FileId::State
            | FileId::Theme
            | FileId::Themes(_)
            | FileId::Session
            | FileId::Workspace => None,
        }
    }
}

impl fmt::Display for FileId {
    /// [`FileId::name`].
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name())
    }
}

/// A settings key: a file and a key path inside it (D-73).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SettingsKey {
    /// The file.
    pub file: FileId,
    /// The key path (empty: the whole file).
    pub path: Vec<KeySegment>,
}

impl SettingsKey {
    /// The dotted path inside the file (`text_editing.auto_indent`; empty for the whole file).
    pub fn path_text(&self) -> String {
        tree::display_path(&self.path)
    }
}

impl fmt::Display for SettingsKey {
    /// `<file>.<path>` in the spelling of D-73.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.file.name())?;
        for segment in &self.path {
            write!(f, ".{segment}")?;
        }
        Ok(())
    }
}

/// Splits a dotted key path into segments (the grammar of the module documentation, without the file).
pub fn parse_path(text: &str) -> Result<Vec<KeySegment>, String> {
    let mut segments = Vec::new();
    let mut chars = text.chars().peekable();
    if text.is_empty() {
        return Ok(segments);
    }
    loop {
        if chars.peek() == Some(&'"') {
            chars.next();
            let mut name = String::new();
            loop {
                match chars.next() {
                    Some('"') => break,
                    Some('\\') => match chars.next() {
                        Some(c @ ('"' | '\\')) => name.push(c),
                        _ => return Err(format!("{text:?}: only \\\" and \\\\ may be escaped")),
                    },
                    Some(c) => name.push(c),
                    None => return Err(format!("{text:?}: a quoted segment is not closed")),
                }
            }
            segments.push(KeySegment::Name(name));
        } else {
            let mut bare = String::new();
            while let Some(&c) = chars.peek() {
                if c == '.' {
                    break;
                }
                bare.push(c);
                chars.next();
            }
            if !is_bare(&bare) {
                return Err(format!(
                    "{text:?}: {bare:?} is not a key (letters, digits, `_` and `-`, or a quoted name)"
                ));
            }
            match bare.parse::<usize>() {
                Ok(index) if bare.chars().all(|c| c.is_ascii_digit()) => {
                    segments.push(KeySegment::Index(index));
                }
                _ => segments.push(KeySegment::Name(bare)),
            }
        }
        match chars.next() {
            None => return Ok(segments),
            Some('.') if chars.peek().is_some() => {}
            Some(_) => return Err(format!("{text:?}: expected `.` between key segments")),
        }
    }
}

/// Parses `<file>.<path>` (D-73). Bad syntax or an unknown file is `invalid_input`.
pub fn parse_key(text: &str) -> Result<SettingsKey, ApiError> {
    let segments = parse_path(text).map_err(ApiError::InvalidArgument)?;
    let mut rest = segments.into_iter();
    let first = match rest.next() {
        Some(KeySegment::Name(name)) => name,
        _ => {
            return Err(ApiError::InvalidArgument(format!(
                "{text:?}: a settings key starts with a file: options, state, commands, theme, \
                 themes.<scheme>, filters, openwith or archives"
            )));
        }
    };
    let file = match first.as_str() {
        "options" => FileId::Options,
        "state" => FileId::State,
        "commands" => FileId::Commands,
        "theme" => FileId::Theme,
        "filters" => FileId::Filters,
        "openwith" => FileId::OpenWith,
        "archives" => FileId::Archives,
        "themes" => match rest.next() {
            Some(segment) => {
                let name = segment.as_key();
                if !is_scheme_name(&name) {
                    return Err(ApiError::InvalidArgument(format!(
                        "{text:?}: {name:?} is not a scheme name (letters, digits, `_` and `-`)"
                    )));
                }
                FileId::Themes(name)
            }
            None => FileId::Themes(String::new()),
        },
        other => {
            return Err(ApiError::InvalidArgument(format!(
                "{text:?}: unknown settings file `{other}` (options, state, commands, theme, \
                 themes.<scheme>, filters, openwith, archives)"
            )));
        }
    };
    Ok(SettingsKey {
        file,
        path: rest.collect(),
    })
}

/// True for a scheme name usable as a file name (`themes/<name>.toml`): a bare key.
pub fn is_scheme_name(name: &str) -> bool {
    is_bare(name)
}

/// The type of a settings value (docs/15 "Keys").
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ValueKind {
    /// `true`/`false`.
    Bool,
    /// An integer in `min..=max`.
    Int {
        /// Smallest value.
        min: i64,
        /// Largest value.
        max: i64,
    },
    /// A float in `min..=max`.
    Float {
        /// Smallest value.
        min: f64,
        /// Largest value.
        max: f64,
    },
    /// One of the listed strings.
    Enum(&'static [&'static str]),
    /// Free text.
    String,
    /// A path (empty: unset).
    Path,
    /// A mask list (empty: disabled).
    Mask,
    /// `#rrggbb` or `#rrggbbaa`.
    Colour,
    /// A shortcut in the reference spelling (UI-003).
    Shortcut,
    /// An array (set as a whole).
    List,
    /// A table (get and list only).
    Table,
}

impl ValueKind {
    /// The kind as written in docs/15: `bool`, `int 0..20` (`int` without a range), `float 6..72`, `enum a|b`, `string`, `path`,
    /// `mask`, `colour`, `shortcut`, `list`, `table`.
    pub fn describe(&self) -> String {
        match self {
            ValueKind::Bool => "bool".into(),
            ValueKind::Int {
                min: i64::MIN,
                max: i64::MAX,
            } => "int".into(),
            ValueKind::Int { min, max } => format!("int {min}..{max}"),
            ValueKind::Float { min, max } if *min == f64::MIN && *max == f64::MAX => "float".into(),
            ValueKind::Float { min, max } => format!("float {min}..{max}"),
            ValueKind::Enum(values) => format!("enum {}", values.join("|")),
            ValueKind::String => "string".into(),
            ValueKind::Path => "path".into(),
            ValueKind::Mask => "mask".into(),
            ValueKind::Colour => "colour".into(),
            ValueKind::Shortcut => "shortcut".into(),
            ValueKind::List => "list".into(),
            ValueKind::Table => "table".into(),
        }
    }

    /// True for the kinds whose value text may be given without TOML quotes (D-73, `settings set`).
    pub fn is_text(&self) -> bool {
        matches!(
            self,
            ValueKind::String
                | ValueKind::Path
                | ValueKind::Mask
                | ValueKind::Colour
                | ValueKind::Shortcut
                | ValueKind::Enum(_)
        )
    }

    /// The kind of a value without a rule.
    pub(crate) fn of_value(value: &Value) -> ValueKind {
        match value {
            Value::Boolean(_) => ValueKind::Bool,
            Value::Integer(_) => ValueKind::Int {
                min: i64::MIN,
                max: i64::MAX,
            },
            Value::Float(_) => ValueKind::Float {
                min: f64::MIN,
                max: f64::MAX,
            },
            Value::String(_) | Value::Datetime(_) => ValueKind::String,
            Value::Array(_) => ValueKind::List,
            Value::Table(_) => ValueKind::Table,
        }
    }
}

impl Serialize for ValueKind {
    /// [`ValueKind::describe`].
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.describe())
    }
}

/// The kind, OPT id and priority of the keys matching `pattern` (dotted; `*` matches any one segment).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KindRule {
    /// The key pattern inside the file.
    pub pattern: &'static str,
    /// The kind.
    pub kind: ValueKind,
    /// OPT id (docs/15).
    pub opt: Option<&'static str>,
    /// Priority (`P0`, `P1`, `P2`).
    pub priority: Option<&'static str>,
}

impl KindRule {
    /// True when `path` matches the pattern segment by segment.
    pub fn matches(&self, path: &[KeySegment]) -> bool {
        let parts: Vec<&str> = self.pattern.split('.').collect();
        parts.len() == path.len()
            && parts
                .iter()
                .zip(path)
                .all(|(part, segment)| *part == "*" || *part == segment.as_key())
    }
}

/// A leaf key of a file: its key (`options.text_editing.auto_indent`), kind, default and OPT id.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyInfo {
    /// The full key (`<file>.<path>`).
    pub key: String,
    /// The kind.
    pub kind: ValueKind,
    /// The default value.
    pub default: Value,
    /// OPT id (docs/15).
    pub opt: Option<&'static str>,
    /// Priority.
    pub priority: Option<&'static str>,
}

/// The rule of `path` in `file`, if any.
pub(crate) fn rule_for(file: &FileId, path: &[KeySegment]) -> Option<KindRule> {
    let rules = super::rules(file);
    rules.iter().find(|r| r.matches(path)).copied()
}

/// Every leaf key of the file's schema with its default (the default value of the schema; for a scheme,
/// the built-in it is based on: `light`, `dark`, else `light`), in key order.
pub fn key_infos(file: &FileId) -> Vec<KeyInfo> {
    let defaults = super::defaults_tree(file, None);
    let Value::Table(table) = &defaults else {
        return Vec::new();
    };
    tree::leaves(table, false)
        .into_iter()
        .map(|(path, value)| info_for(file, &path, value))
        .collect()
}

/// The [`KeyInfo`] of one path whose default is `default`.
pub(crate) fn info_for(file: &FileId, path: &[KeySegment], default: Value) -> KeyInfo {
    let rule = rule_for(file, path);
    let file_opt = file.opt();
    KeyInfo {
        key: SettingsKey {
            file: file.clone(),
            path: path.to_vec(),
        }
        .to_string(),
        kind: rule.map_or_else(|| ValueKind::of_value(&default), |r| r.kind),
        opt: rule.and_then(|r| r.opt).or(file_opt.map(|o| o.0)),
        priority: rule.and_then(|r| r.priority).or(file_opt.map(|o| o.1)),
        default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(s: &str) -> KeySegment {
        KeySegment::Name(s.into())
    }

    #[test]
    fn keys_parse_and_print() {
        let key = parse_key("options.text_editing.auto_indent").unwrap();
        assert_eq!(key.file, FileId::Options);
        assert_eq!(key.path, vec![n("text_editing"), n("auto_indent")]);
        assert_eq!(key.to_string(), "options.text_editing.auto_indent");
        let key = parse_key("openwith.app.0.command").unwrap();
        assert_eq!(key.path, vec![n("app"), KeySegment::Index(0), n("command")]);
        let key = parse_key("themes.dark.file.syntax.\"a.b\\\"c\".bold").unwrap();
        assert_eq!(key.file, FileId::Themes("dark".into()));
        assert_eq!(key.path[2], n("a.b\"c"));
        assert_eq!(key.to_string(), "themes.dark.file.syntax.\"a.b\\\"c\".bold");
        assert_eq!(
            parse_key("themes").unwrap().file,
            FileId::Themes(String::new())
        );
        assert!(parse_key("options").unwrap().path.is_empty());
        for bad in [
            "",
            "nope.x",
            "options..x",
            "options.",
            "options.a b",
            "options.\"open",
            "options.\"a\"b",
            "themes.\"a b\"",
            "\"options\"x",
        ] {
            assert!(parse_key(bad).is_err(), "{bad:?}");
        }
        assert_eq!(FileId::parse("themes.x"), Some(FileId::Themes("x".into())));
        assert_eq!(FileId::parse("options.x"), None);
        assert_eq!(FileId::Themes("x".into()).relative_path(), "themes/x.toml");
        assert_eq!(FileId::OpenWith.relative_path(), "openwith.toml");
    }

    #[test]
    fn rules_match_segments() {
        let rule = KindRule {
            pattern: "view.*.command.*.shortcuts",
            kind: ValueKind::List,
            opt: None,
            priority: None,
        };
        let key = parse_key("commands.view.home.command.Exit.shortcuts").unwrap();
        assert!(rule.matches(&key.path));
        assert!(!rule.matches(&key.path[..4]));
        assert_eq!(ValueKind::Int { min: 0, max: 20 }.describe(), "int 0..20");
        assert_eq!(ValueKind::Enum(&["a", "b"]).describe(), "enum a|b");
    }
}
