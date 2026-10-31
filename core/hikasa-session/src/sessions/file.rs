//! The session file (contract §1.2.1; D-80, D-81): [`SessionType`], [`Session`], [`SessionRules`], [`Specs`],
//! [`ParentLink`], [`Override`], and the on-disk shape [`SessionDoc`] that the D-70 engine loads and saves.
//!
//! In memory a session holds its **full** rules (typed for Folder Compare, Text Compare and Text Edit; an opaque
//! table for the other types until their engine waves). On disk `[rules]` (and `[text_rules]`) hold only what
//! differs from the type's defaults as they are when the file is saved (D-82); a load overlays the file's rules on
//! the defaults as they are when it is loaded, so a session keeps its own differences across a change of the
//! defaults.

use std::fmt;

use hikasa_folder::FolderRules;
use hikasa_text::TextRules;
use serde::{Deserialize, Serialize};
use toml::{Table, Value};

use crate::api::ApiError;
use crate::settings::file::migrate_from_zero;
use crate::settings::keys::KeySegment;
use crate::settings::{FileId, Problem, SettingsSchema, tree};

/// `schema_version` of session files.
pub const SESSION_SCHEMA_VERSION: u32 = 1;

/// The session types of M2 (D-81). P1 types (`text-merge`, `folder-merge`, `media`, `version`, `registry`, …)
/// are refused by [`SessionType::parse`] with `unsupported`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SessionType {
    /// `folder-compare`.
    FolderCompare,
    /// `folder-sync`.
    FolderSync,
    /// `text-compare`.
    TextCompare,
    /// `text-edit`.
    TextEdit,
    /// `table-compare`.
    TableCompare,
    /// `hex-compare`.
    HexCompare,
    /// `picture-compare`.
    PictureCompare,
}

/// Reserved P1 type names (refused with `unsupported`).
const RESERVED_TYPES: &[&str] = &[
    "text-merge",
    "folder-merge",
    "text-patch",
    "media",
    "media-compare",
    "version",
    "version-compare",
    "registry",
    "registry-compare",
];

impl SessionType {
    /// Every type, in declaration order.
    pub const ALL: [SessionType; 7] = [
        SessionType::FolderCompare,
        SessionType::FolderSync,
        SessionType::TextCompare,
        SessionType::TextEdit,
        SessionType::TableCompare,
        SessionType::HexCompare,
        SessionType::PictureCompare,
    ];

    /// The kebab-case name.
    pub const fn as_str(self) -> &'static str {
        match self {
            SessionType::FolderCompare => "folder-compare",
            SessionType::FolderSync => "folder-sync",
            SessionType::TextCompare => "text-compare",
            SessionType::TextEdit => "text-edit",
            SessionType::TableCompare => "table-compare",
            SessionType::HexCompare => "hex-compare",
            SessionType::PictureCompare => "picture-compare",
        }
    }

    /// Parses a type name: a P1 name is `unsupported`, anything else unknown `invalid_input`.
    pub fn parse(text: &str) -> Result<SessionType, ApiError> {
        if let Some(kind) = SessionType::ALL.into_iter().find(|k| k.as_str() == text) {
            return Ok(kind);
        }
        if RESERVED_TYPES.contains(&text) {
            return Err(ApiError::Unsupported(format!(
                "session type `{text}` is not supported in this version"
            )));
        }
        Err(ApiError::InvalidArgument(format!(
            "unknown session type {text:?} (folder-compare, folder-sync, text-compare, text-edit, \
             table-compare, hex-compare, picture-compare)"
        )))
    }

    /// The GUI view kind of the type.
    pub const fn view_kind(self) -> crate::ipc::ViewKind {
        use crate::ipc::ViewKind;
        match self {
            SessionType::FolderCompare => ViewKind::FolderCompare,
            SessionType::FolderSync => ViewKind::FolderSync,
            SessionType::TextCompare => ViewKind::TextCompare,
            SessionType::TextEdit => ViewKind::TextEdit,
            SessionType::TableCompare => ViewKind::TableCompare,
            SessionType::HexCompare => ViewKind::HexCompare,
            SessionType::PictureCompare => ViewKind::PictureCompare,
        }
    }

    /// Folder types (`[text_rules]` and `[[override]]` apply; they can be parents, D-82).
    pub const fn is_folder(self) -> bool {
        matches!(self, SessionType::FolderCompare | SessionType::FolderSync)
    }

    /// The built-in rules of the type (D-81).
    pub fn builtin_rules(self) -> SessionRules {
        match self {
            SessionType::FolderCompare => SessionRules::Folder(FolderRules::default()),
            SessionType::TextCompare | SessionType::TextEdit => {
                SessionRules::Text(TextRules::default())
            }
            SessionType::FolderSync
            | SessionType::TableCompare
            | SessionType::HexCompare
            | SessionType::PictureCompare => SessionRules::Opaque(Table::new()),
        }
    }
}

impl fmt::Display for SessionType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The rules of a session (D-81).
#[derive(Debug, Clone, PartialEq)]
pub enum SessionRules {
    /// Folder Compare (the TOML of `--rules-file`).
    Folder(FolderRules),
    /// Text Compare and Text Edit.
    Text(TextRules),
    /// Folder Sync, Table, Hex and Picture until their engine waves: kept as it is.
    Opaque(Table),
}

impl SessionRules {
    /// The rules as a TOML tree.
    pub fn to_value(&self) -> Value {
        let value = match self {
            SessionRules::Folder(rules) => Value::try_from(rules),
            SessionRules::Text(rules) => Value::try_from(rules),
            SessionRules::Opaque(table) => Ok(Value::Table(table.clone())),
        };
        value.unwrap_or_else(|_| Value::Table(Table::new()))
    }

    /// Reads a full tree back into the variant of `like` (`Err`: the serde message).
    pub fn from_value(like: &SessionRules, value: Value) -> Result<SessionRules, String> {
        match like {
            SessionRules::Folder(_) => value
                .try_into()
                .map(SessionRules::Folder)
                .map_err(|e: toml::de::Error| one_line(e.message())),
            SessionRules::Text(_) => value
                .try_into()
                .map(SessionRules::Text)
                .map_err(|e: toml::de::Error| one_line(e.message())),
            SessionRules::Opaque(_) => match value {
                Value::Table(table) => Ok(SessionRules::Opaque(table)),
                _ => Err("rules must be a table".into()),
            },
        }
    }
}

/// The inputs of a session (canonical spec strings; `""` = not set).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Specs {
    /// Left spec (Text Edit: the file).
    pub left: String,
    /// Right spec.
    pub right: String,
    /// File types: the format id (`""` = detect).
    pub format: String,
    /// Left encoding label (`""` = detect).
    pub left_encoding: String,
    /// Right encoding label.
    pub right_encoding: String,
}

/// The parent link of a child session (TC-052).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ParentLink {
    /// The parent: a named session reference, or `_autosaved/<id>`.
    pub session: String,
    /// The child's left path relative to the parent's left base (`/`-separated).
    pub left: String,
    /// The child's right path relative to the parent's right base.
    pub right: String,
}

impl ParentLink {
    /// True when nothing is set (no parent).
    pub fn is_empty(&self) -> bool {
        *self == ParentLink::default()
    }
}

/// The scope of a stored override (D-82).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverrideScope {
    /// One pair of files within the parent.
    #[default]
    Pair,
    /// Every child of a type within the parent.
    Type,
}

/// A per-pair or per-type override stored in a folder session (D-82).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Override {
    /// `pair` or `type`.
    pub scope: OverrideScope,
    /// Pair: the child's left relative path.
    #[serde(default)]
    pub left: String,
    /// Pair: the child's right relative path.
    #[serde(default)]
    pub right: String,
    /// The child session type the override applies to.
    #[serde(rename = "type")]
    pub kind: SessionType,
    /// The rules, sparse against the defaults of `kind`.
    #[serde(default)]
    pub rules: Table,
}

/// A session (§1.2.1).
#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    /// The type.
    pub kind: SessionType,
    /// The description (Specs tab).
    pub description: String,
    /// Locked (SES-004).
    pub locked: bool,
    /// Pinned to the top of the tree (SES-031).
    pub favorite: bool,
    /// When it was auto-saved (auto-saved files only).
    pub saved_at: Option<jiff::Timestamp>,
    /// The parent link (child sessions only).
    pub parent: Option<ParentLink>,
    /// The inputs.
    pub specs: Specs,
    /// The full rules.
    pub rules: SessionRules,
    /// Folder types: the rules of rules-based content tests.
    pub text_rules: Option<TextRules>,
    /// The display filter state (SES-010), opaque.
    pub display: Table,
    /// The view layout and state, opaque.
    pub view: Table,
    /// Folder types: per-pair and per-type overrides of children (D-82).
    pub overrides: Vec<Override>,
}

impl Session {
    /// The built-in defaults of a type (what a missing `_defaults/<type>.toml` means).
    pub fn builtin(kind: SessionType) -> Session {
        Session {
            kind,
            description: String::new(),
            locked: false,
            favorite: false,
            saved_at: None,
            parent: None,
            specs: Specs::default(),
            rules: kind.builtin_rules(),
            text_rules: kind.is_folder().then(TextRules::default),
            display: Table::new(),
            view: Table::new(),
            overrides: Vec::new(),
        }
    }

    /// A new session of `kind` with `defaults` (the type's defaults) as template (SES-001).
    pub fn new(kind: SessionType, defaults: &Session) -> Session {
        let mut session = if defaults.kind == kind {
            defaults.clone()
        } else {
            Session::builtin(kind)
        };
        session.locked = false;
        session.favorite = false;
        session.saved_at = None;
        session.parent = None;
        session.overrides.clear();
        session
    }

    /// Clear Session (SES-008): a new session of the same type.
    pub fn clear(&self, defaults: &Session) -> Session {
        Session::new(self.kind, defaults)
    }

    /// Swap Sides (SES-008): specs, encodings, parent paths, pair overrides and the `view` keys ending in
    /// `_left`/`_right` (at any depth) are exchanged.
    pub fn swap_sides(&mut self) {
        let s = &mut self.specs;
        std::mem::swap(&mut s.left, &mut s.right);
        std::mem::swap(&mut s.left_encoding, &mut s.right_encoding);
        if let Some(parent) = &mut self.parent {
            std::mem::swap(&mut parent.left, &mut parent.right);
        }
        for o in &mut self.overrides {
            if o.scope == OverrideScope::Pair {
                std::mem::swap(&mut o.left, &mut o.right);
            }
        }
        swap_side_keys(&mut self.view);
    }

    /// The session as a script loads it (SES-010): `display` cleared.
    pub fn for_script(&self) -> Session {
        Session {
            display: Table::new(),
            ..self.clone()
        }
    }

    /// The on-disk shape: rules sparse against `defaults` (D-82) with no earlier file (D-90: see
    /// [`Session::to_doc_over`]).
    pub(crate) fn to_doc(&self, defaults: &Session) -> SessionDoc {
        self.to_doc_over(defaults, None)
    }

    /// The on-disk shape over the file it replaces (`previous`, D-90): `[rules]` and `[text_rules]` hold every
    /// key that differs from `defaults` (the defaults of the type now), plus every key `previous` held (an
    /// explicit choice stays written even when it now equals the defaults); keys of `previous` that the typed
    /// rules do not know are kept as they were (D-70). An optional rule that is off where the defaults set it is
    /// written `false` (it reads back as off).
    pub(crate) fn to_doc_over(
        &self,
        defaults: &Session,
        previous: Option<&SessionDoc>,
    ) -> SessionDoc {
        let text_default = || {
            defaults
                .text_rules
                .as_ref()
                .and_then(|d| Value::try_from(d).ok())
                .unwrap_or_else(|| {
                    Value::try_from(TextRules::default()).unwrap_or(Value::Table(Table::new()))
                })
        };
        let text_rules = match &self.text_rules {
            Some(rules) => written_rules(
                &Value::try_from(rules).unwrap_or(Value::Table(Table::new())),
                &text_default(),
                previous.map(|p| &p.text_rules),
                RulesShape::TEXT,
            ),
            None => Table::new(),
        };
        let shape = match &self.rules {
            SessionRules::Folder(_) => RulesShape::FOLDER,
            SessionRules::Text(_) => RulesShape::TEXT,
            SessionRules::Opaque(_) => RulesShape::OPAQUE,
        };
        let rules = written_rules(
            &self.rules.to_value(),
            &defaults.rules.to_value(),
            previous.map(|p| &p.rules),
            shape,
        );
        SessionDoc {
            kind: self.kind.as_str().to_owned(),
            description: self.description.clone(),
            locked: self.locked,
            favorite: self.favorite,
            saved_at: self.saved_at.map(|t| t.to_string()).unwrap_or_default(),
            parent: self.parent.clone().unwrap_or_default(),
            specs: self.specs.clone(),
            rules,
            text_rules,
            display: self.display.clone(),
            view: self.view.clone(),
            overrides: self.overrides.clone(),
        }
    }

    /// The session of a file: the type checked, the rules overlaid on `defaults` (the defaults of its type, as
    /// they are now). `Err`: what is wrong (the caller adds the file).
    pub(crate) fn from_doc(
        doc: &SessionDoc,
        defaults: Option<&Session>,
    ) -> Result<Session, ApiError> {
        let kind = SessionType::parse(&doc.kind)?;
        let builtin;
        let base = match defaults {
            Some(d) if d.kind == kind => d,
            _ => {
                builtin = Session::builtin(kind);
                &builtin
            }
        };
        let mut rules = base.rules.to_value();
        tree::overlay(&mut rules, &Value::Table(doc.rules.clone()));
        let rules = SessionRules::from_value(&base.rules, rules)
            .map_err(|e| ApiError::InvalidData(format!("rules: {e}")))?;
        let text_rules = if kind.is_folder() {
            let mut value = Value::try_from(base.text_rules.clone().unwrap_or_default())
                .unwrap_or(Value::Table(Table::new()));
            tree::overlay(&mut value, &Value::Table(doc.text_rules.clone()));
            Some(value.try_into::<TextRules>().map_err(|e| {
                ApiError::InvalidData(format!("text_rules: {}", one_line(e.message())))
            })?)
        } else {
            None
        };
        let saved_at = match doc.saved_at.as_str() {
            "" => None,
            text => Some(text.parse::<jiff::Timestamp>().map_err(|e| {
                ApiError::InvalidData(format!("saved_at: {text:?} is not an RFC 3339 time ({e})"))
            })?),
        };
        Ok(Session {
            kind,
            description: doc.description.clone(),
            locked: doc.locked,
            favorite: doc.favorite,
            saved_at,
            parent: (!doc.parent.is_empty()).then(|| doc.parent.clone()),
            specs: doc.specs.clone(),
            rules,
            text_rules,
            display: doc.display.clone(),
            view: doc.view.clone(),
            overrides: if kind.is_folder() {
                doc.overrides.clone()
            } else {
                Vec::new()
            },
        })
    }

    /// The whole session as one TOML table (every rule; for `session edit` and `show`).
    pub(crate) fn full_doc(&self) -> SessionDoc {
        let mut doc = self.to_doc(&Session::builtin(self.kind));
        doc.rules = match self.rules.to_value() {
            Value::Table(t) => t,
            _ => Table::new(),
        };
        if let Some(text) = &self.text_rules
            && let Ok(Value::Table(t)) = Value::try_from(text)
        {
            doc.text_rules = t;
        }
        doc
    }
}

/// What the writer of a rules table knows about its shape (D-90).
#[derive(Debug, Clone, Copy)]
struct RulesShape {
    /// The optional rules (an `Option` of the typed rules): written whole, or `false` for "off".
    optional: &'static [&'static [&'static str]],
    /// Maps of the typed rules: their entries are known keys (an entry removed in memory is gone, not unknown).
    maps: &'static [&'static [&'static str]],
    /// Opaque rules: every key is known (nothing is kept beyond the table in memory).
    opaque: bool,
}

impl RulesShape {
    /// `hikasa_folder::FolderRules`.
    const FOLDER: RulesShape = RulesShape {
        optional: &[
            &["criteria", "timestamps", "offset"],
            &["filters", "other", "size"],
            &["filters", "other", "date"],
        ],
        maps: &[],
        opaque: false,
    };
    /// `hikasa_text::TextRules`.
    const TEXT: RulesShape = RulesShape {
        optional: &[],
        maps: &[&["importance", "elements"]],
        opaque: false,
    };
    /// An opaque table (W6/W12 types).
    const OPAQUE: RulesShape = RulesShape {
        optional: &[],
        maps: &[],
        opaque: true,
    };
}

/// The key path of `names`.
fn key_path(names: &[&str]) -> Vec<KeySegment> {
    names
        .iter()
        .map(|n| KeySegment::Name((*n).to_owned()))
        .collect()
}

/// The `[rules]` (or `[text_rules]`) table a save writes (D-82, D-90): what differs from `defaults`, every key
/// `previous` held (with its value now; a key the typed rules do not know with its old value), optional rules
/// whole or `false` for "off".
fn written_rules(
    current: &Value,
    defaults: &Value,
    previous: Option<&Table>,
    shape: RulesShape,
) -> Table {
    let mut out = tree::sparse(current, defaults);
    if !out.is_table() {
        out = Value::Table(Table::new());
    }
    let optional: Vec<Vec<KeySegment>> = shape.optional.iter().map(|p| key_path(p)).collect();
    let maps: Vec<Vec<KeySegment>> = shape.maps.iter().map(|p| key_path(p)).collect();
    let whole = |out: &mut Value, path: &[KeySegment]| {
        let value = tree::get(current, path)
            .cloned()
            .unwrap_or(Value::Boolean(false));
        let _ = tree::set(out, path, value);
    };
    for path in &optional {
        if tree::get(current, path) == tree::get(defaults, path) {
            tree::remove(&mut out, path);
        } else {
            whole(&mut out, path);
        }
    }
    if let Some(previous) = previous {
        for (path, old) in tree::leaves(previous, false) {
            if let Some(opt) = optional.iter().find(|o| path.starts_with(o)) {
                if tree::get(&out, opt).is_none() {
                    whole(&mut out, opt);
                }
                continue;
            }
            match tree::get(current, &path) {
                Some(now) => {
                    let _ = tree::set(&mut out, &path, now.clone());
                }
                None => {
                    let known = shape.opaque || maps.iter().any(|m| path.starts_with(m));
                    if !known {
                        let _ = tree::set(&mut out, &path, old);
                    }
                }
            }
        }
    }
    match out {
        Value::Table(t) => t,
        _ => Table::new(),
    }
}

/// Exchanges the keys ending in `_left` and `_right` of a table and of the tables below it.
fn swap_side_keys(table: &mut Table) {
    let stems: std::collections::BTreeSet<String> = table
        .keys()
        .filter_map(|k| k.strip_suffix("_left").or_else(|| k.strip_suffix("_right")))
        .map(str::to_owned)
        .collect();
    for stem in stems {
        let (l, r) = (format!("{stem}_left"), format!("{stem}_right"));
        let left = table.remove(&l);
        let right = table.remove(&r);
        if let Some(v) = left {
            table.insert(r, v);
        }
        if let Some(v) = right {
            table.insert(l, v);
        }
    }
    for (_, value) in table.iter_mut() {
        if let Value::Table(inner) = value {
            swap_side_keys(inner);
        }
    }
}

/// A parser message on one line.
fn one_line(message: &str) -> String {
    message.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The on-disk shape of a session file (§1.2.1), loaded and saved by the D-70 engine.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionDoc {
    /// `type` (always written: its default is empty).
    #[serde(rename = "type")]
    pub kind: String,
    /// `description`.
    pub description: String,
    /// `locked`.
    pub locked: bool,
    /// `favorite`.
    pub favorite: bool,
    /// `saved_at` (RFC 3339; auto-saved files).
    pub saved_at: String,
    /// `[parent]`.
    pub parent: ParentLink,
    /// `[specs]`.
    pub specs: Specs,
    /// `[rules]`, sparse against the type's defaults.
    pub rules: Table,
    /// `[text_rules]`, sparse.
    pub text_rules: Table,
    /// `[display]`.
    pub display: Table,
    /// `[view]`.
    pub view: Table,
    /// `[[override]]`.
    #[serde(rename = "override")]
    pub overrides: Vec<Override>,
}

impl SettingsSchema for SessionDoc {
    const FILE: FileId = FileId::Session;
    const VERSION: u32 = SESSION_SCHEMA_VERSION;

    fn migrate(from: u32, _table: &mut Table) -> Result<(), String> {
        migrate_from_zero(from)
    }

    fn validate(&self) -> Vec<Problem> {
        // The type is checked by `Session::from_doc` (a P1 type is `unsupported`, not a bad value).
        let mut problems = Vec::new();
        if !self.saved_at.is_empty() && self.saved_at.parse::<jiff::Timestamp>().is_err() {
            problems.push(Problem::new("saved_at", "must be an RFC 3339 time"));
        }
        problems
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types_parse_and_reserve_p1_names() {
        for kind in SessionType::ALL {
            assert_eq!(SessionType::parse(kind.as_str()).unwrap(), kind);
        }
        for p1 in ["text-merge", "folder-merge", "media", "version", "registry"] {
            assert!(
                matches!(SessionType::parse(p1), Err(ApiError::Unsupported(_))),
                "{p1}"
            );
        }
        assert!(matches!(
            SessionType::parse("nope"),
            Err(ApiError::InvalidArgument(_))
        ));
    }

    #[test]
    fn swap_exchanges_every_side() {
        let mut s = Session::builtin(SessionType::FolderCompare);
        s.specs.left = "/a".into();
        s.specs.right = "/b".into();
        s.specs.left_encoding = "utf-8".into();
        s.parent = Some(ParentLink {
            session: "p".into(),
            left: "x".into(),
            right: "y".into(),
        });
        s.view.insert("width_left".into(), Value::Integer(1));
        s.view.insert("width_right".into(), Value::Integer(2));
        s.view.insert("only_left".into(), Value::Integer(3));
        s.swap_sides();
        assert_eq!(
            (s.specs.left.as_str(), s.specs.right.as_str()),
            ("/b", "/a")
        );
        assert_eq!(s.specs.right_encoding, "utf-8");
        let p = s.parent.as_ref().unwrap();
        assert_eq!((p.left.as_str(), p.right.as_str()), ("y", "x"));
        assert_eq!(s.view["width_left"], Value::Integer(2));
        assert_eq!(s.view["width_right"], Value::Integer(1));
        assert_eq!(s.view["only_right"], Value::Integer(3));
        assert!(!s.view.contains_key("only_left"));
    }

    #[test]
    fn script_loads_clear_the_display() {
        let mut s = Session::builtin(SessionType::TextCompare);
        s.display
            .insert("filter".into(), Value::String("diffs".into()));
        assert!(s.for_script().display.is_empty());
        let cleared = s.clear(&Session::builtin(SessionType::TextCompare));
        assert_eq!(cleared, Session::builtin(SessionType::TextCompare));
    }
}
