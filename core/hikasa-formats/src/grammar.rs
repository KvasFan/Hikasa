//! Grammar items (FF-020…FF-024) and their TOML shim.
//!
//! A grammar item is written in TOML as one `[[grammar]]` table:
//!
//! ```toml
//! [[grammar]]
//! element = "Comment"
//! kind = "delimited"
//! from = "/*"
//! to = "*/"
//! ```
//!
//! It is deserialised through [`RawGrammarItem`] (every field optional) and validated by `TryFrom`, so that a
//! field that does not belong to the item's kind is a precise error ("field `tokens` is not allowed for kind
//! `basic`") instead of being silently ignored.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::model::ElementName;

/// A grammar item: an element name plus a category (FF-020…FF-024).
/// TOML: `[[grammar]] element = "Comment"  kind = "delimited"  from = "/*"  to = "*/"`.
/// Deserialised through `RawGrammarItem` (all fields optional), then validated by `TryFrom`, so that a field
/// that is not allowed for a kind is a precise error ("field `tokens` is not allowed for kind `basic`").
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "RawGrammarItem", into = "RawGrammarItem")]
pub struct GrammarItem {
    /// Element produced. Names not declared in `FileFormat::elements` get the default `ElementInfo`
    /// (`case_sensitive = true`, `important = true`). Ids follow D-41.
    pub element: ElementName,
    /// Category and parameters.
    pub kind: ItemKind,
}

/// Grammar item categories.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ItemKind {
    /// FF-020: literal or regex, anchored at the scan position.
    Basic {
        /// Literal text or regex.
        text: String,
        /// `text` is a regex.
        regex: bool,
        /// Case-sensitive matching.
        match_case: bool,
    },
    /// FF-021: `from` … `to`, or to end of line when `to` is `None` or `stop_at_eol` is set; `escape` skips the
    /// next char (doubled-delimiter rule, §2.10.3).
    Delimited {
        /// Opening delimiter (literal or regex).
        from: String,
        /// Closing delimiter (literal or regex); `None` = end of line.
        to: Option<String>,
        /// The construct ends at the end of the line even when `to` was not found.
        stop_at_eol: bool,
        /// Escape character.
        escape: Option<char>,
        /// `from` and `to` are regexes.
        regex: bool,
        /// Case-sensitive matching of `from` and `to`.
        match_case: bool,
    },
    /// FF-022: whole-word tokens, longest match wins.
    List {
        /// The tokens.
        tokens: Vec<String>,
        /// The tokens are regexes.
        regex: bool,
        /// Case-sensitive matching.
        match_case: bool,
    },
    /// FF-023: 1-based inclusive columns; `to_column = None` = to end of line.
    Columns {
        /// First column (1-based).
        from_column: u32,
        /// Last column (1-based, inclusive); `None` = to the end of the line.
        to_column: Option<u32>,
    },
    /// FF-024: a heading line and the next `next_lines` lines; `or_line_one` also starts at line 1.
    Lines {
        /// Text or regex searched anywhere in a line (empty: only `or_line_one` starts the item).
        heading: String,
        /// `heading` is a regex.
        regex: bool,
        /// Case-sensitive matching.
        match_case: bool,
        /// Lines covered after the heading line.
        next_lines: u32,
        /// Line 1 also starts the item.
        or_line_one: bool,
    },
}

impl ItemKind {
    /// The TOML `kind` value (`"basic"`, `"delimited"`, `"list"`, `"columns"`, `"lines"`).
    pub const fn name(&self) -> &'static str {
        match self {
            ItemKind::Basic { .. } => "basic",
            ItemKind::Delimited { .. } => "delimited",
            ItemKind::List { .. } => "list",
            ItemKind::Columns { .. } => "columns",
            ItemKind::Lines { .. } => "lines",
        }
    }
}

/// The TOML shape of a grammar item: every field optional, validated into a [`GrammarItem`] by `TryFrom`.
/// Field order is the canonical key order written by `to_toml`; fields at their default value are omitted.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawGrammarItem {
    /// Element name (required).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element: Option<String>,
    /// `basic`, `delimited`, `list`, `columns` or `lines` (required).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Basic: the text or regex.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Lines: the heading text or regex.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heading: Option<String>,
    /// Delimited: the opening delimiter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// Delimited: the closing delimiter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    /// Delimited: the escape character.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub escape: Option<char>,
    /// Delimited: stop at the end of the line (default false).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_at_eol: Option<bool>,
    /// Columns: the first column (1-based).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_column: Option<u32>,
    /// Columns: the last column (1-based, inclusive).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_column: Option<u32>,
    /// Lines: lines covered after the heading (default 0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_lines: Option<u32>,
    /// Lines: line 1 also starts the item (default false).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub or_line_one: Option<bool>,
    /// Basic, Delimited, List, Lines: the texts are regexes (default false).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub regex: Option<bool>,
    /// Basic, Delimited, List, Lines: case-sensitive matching (default true).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub match_case: Option<bool>,
    /// List: the tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<Vec<String>>,
}

/// A grammar item that does not validate (the message names the field and the kind).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrammarItemError(pub String);

impl fmt::Display for GrammarItemError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for GrammarItemError {}

/// The fields a kind accepts, in canonical order.
const BASIC_FIELDS: &[&str] = &["text", "regex", "match_case"];
const DELIMITED_FIELDS: &[&str] = &["from", "to", "escape", "stop_at_eol", "regex", "match_case"];
const LIST_FIELDS: &[&str] = &["tokens", "regex", "match_case"];
const COLUMNS_FIELDS: &[&str] = &["from_column", "to_column"];
const LINES_FIELDS: &[&str] = &[
    "heading",
    "next_lines",
    "or_line_one",
    "regex",
    "match_case",
];

impl RawGrammarItem {
    /// Names of the fields that are set, in canonical order (without `element` and `kind`).
    fn present(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        let flags = [
            ("text", self.text.is_some()),
            ("heading", self.heading.is_some()),
            ("from", self.from.is_some()),
            ("to", self.to.is_some()),
            ("escape", self.escape.is_some()),
            ("stop_at_eol", self.stop_at_eol.is_some()),
            ("from_column", self.from_column.is_some()),
            ("to_column", self.to_column.is_some()),
            ("next_lines", self.next_lines.is_some()),
            ("or_line_one", self.or_line_one.is_some()),
            ("regex", self.regex.is_some()),
            ("match_case", self.match_case.is_some()),
            ("tokens", self.tokens.is_some()),
        ];
        for (name, set) in flags {
            if set {
                out.push(name);
            }
        }
        out
    }
}

impl TryFrom<RawGrammarItem> for GrammarItem {
    type Error = GrammarItemError;

    fn try_from(raw: RawGrammarItem) -> Result<GrammarItem, GrammarItemError> {
        let Some(element) = raw.element.clone() else {
            return Err(GrammarItemError(
                "grammar item: missing field `element`".into(),
            ));
        };
        let element = ElementName::new(element)
            .map_err(|e| GrammarItemError(format!("grammar item: {e}")))?;
        let Some(kind) = raw.kind.clone() else {
            return Err(GrammarItemError(format!(
                "grammar item `{element}`: missing field `kind`"
            )));
        };
        let allowed = match kind.as_str() {
            "basic" => BASIC_FIELDS,
            "delimited" => DELIMITED_FIELDS,
            "list" => LIST_FIELDS,
            "columns" => COLUMNS_FIELDS,
            "lines" => LINES_FIELDS,
            other => {
                return Err(GrammarItemError(format!(
                    "grammar item `{element}`: unknown kind `{other}` (expected basic, delimited, list, columns or \
                     lines)"
                )));
            }
        };
        if let Some(field) = raw.present().into_iter().find(|f| !allowed.contains(f)) {
            return Err(GrammarItemError(format!(
                "grammar item `{element}`: field `{field}` is not allowed for kind `{kind}`"
            )));
        }
        let missing = |field: &str| {
            GrammarItemError(format!(
                "grammar item `{element}`: missing field `{field}` (required for kind `{kind}`)"
            ))
        };
        let regex = raw.regex.unwrap_or(false);
        let match_case = raw.match_case.unwrap_or(true);
        let kind = match kind.as_str() {
            "basic" => ItemKind::Basic {
                text: raw.text.ok_or_else(|| missing("text"))?,
                regex,
                match_case,
            },
            "delimited" => ItemKind::Delimited {
                from: raw.from.ok_or_else(|| missing("from"))?,
                to: raw.to,
                stop_at_eol: raw.stop_at_eol.unwrap_or(false),
                escape: raw.escape,
                regex,
                match_case,
            },
            "list" => ItemKind::List {
                tokens: raw.tokens.ok_or_else(|| missing("tokens"))?,
                regex,
                match_case,
            },
            "columns" => ItemKind::Columns {
                from_column: raw.from_column.ok_or_else(|| missing("from_column"))?,
                to_column: raw.to_column,
            },
            _ => {
                let or_line_one = raw.or_line_one.unwrap_or(false);
                let heading = match raw.heading {
                    Some(heading) => heading,
                    None if or_line_one => String::new(),
                    None => return Err(missing("heading")),
                };
                ItemKind::Lines {
                    heading,
                    regex,
                    match_case,
                    next_lines: raw.next_lines.unwrap_or(0),
                    or_line_one,
                }
            }
        };
        Ok(GrammarItem { element, kind })
    }
}

impl From<GrammarItem> for RawGrammarItem {
    fn from(item: GrammarItem) -> RawGrammarItem {
        let kind_name = item.kind.name();
        let mut raw = RawGrammarItem {
            element: Some(item.element.into()),
            kind: Some(kind_name.to_owned()),
            ..RawGrammarItem::default()
        };
        let flags = |raw: &mut RawGrammarItem, regex: bool, match_case: bool| {
            raw.regex = regex.then_some(true);
            raw.match_case = (!match_case).then_some(false);
        };
        match item.kind {
            ItemKind::Basic {
                text,
                regex,
                match_case,
            } => {
                raw.text = Some(text);
                flags(&mut raw, regex, match_case);
            }
            ItemKind::Delimited {
                from,
                to,
                stop_at_eol,
                escape,
                regex,
                match_case,
            } => {
                raw.from = Some(from);
                raw.to = to;
                raw.escape = escape;
                raw.stop_at_eol = stop_at_eol.then_some(true);
                flags(&mut raw, regex, match_case);
            }
            ItemKind::List {
                tokens,
                regex,
                match_case,
            } => {
                raw.tokens = Some(tokens);
                flags(&mut raw, regex, match_case);
            }
            ItemKind::Columns {
                from_column,
                to_column,
            } => {
                raw.from_column = Some(from_column);
                raw.to_column = to_column;
            }
            ItemKind::Lines {
                heading,
                regex,
                match_case,
                next_lines,
                or_line_one,
            } => {
                raw.heading = (!heading.is_empty() || !or_line_one).then_some(heading);
                raw.next_lines = (next_lines > 0).then_some(next_lines);
                raw.or_line_one = or_line_one.then_some(true);
                flags(&mut raw, regex, match_case);
            }
        }
        raw
    }
}

/// The field names a grammar item table may contain (used by the unknown-key scan).
pub(crate) const RAW_FIELDS: &[&str] = &[
    "element",
    "kind",
    "text",
    "heading",
    "from",
    "to",
    "escape",
    "stop_at_eol",
    "from_column",
    "to_column",
    "next_lines",
    "or_line_one",
    "regex",
    "match_case",
    "tokens",
];

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(element: &str, kind: &str) -> RawGrammarItem {
        RawGrammarItem {
            element: Some(element.into()),
            kind: Some(kind.into()),
            ..RawGrammarItem::default()
        }
    }

    #[test]
    fn precise_errors() {
        let err = |r: RawGrammarItem| GrammarItem::try_from(r).unwrap_err().0;
        let mut r = raw("Keyword", "basic");
        r.text = Some("x".into());
        r.tokens = Some(vec!["a".into()]);
        assert_eq!(
            err(r),
            "grammar item `Keyword`: field `tokens` is not allowed for kind `basic`"
        );
        assert_eq!(
            err(raw("Comment", "delimited")),
            "grammar item `Comment`: missing field `from` (required for kind `delimited`)"
        );
        assert_eq!(
            err(raw("X", "word")),
            "grammar item `X`: unknown kind `word` (expected basic, delimited, list, columns or lines)"
        );
        assert_eq!(
            err(RawGrammarItem::default()),
            "grammar item: missing field `element`"
        );
        let mut r = raw("Num", "columns");
        r.from_column = Some(1);
        r.escape = Some('\\');
        assert_eq!(
            err(r),
            "grammar item `Num`: field `escape` is not allowed for kind `columns`"
        );
        assert!(err(raw("a,b", "basic")).contains("',' is not allowed"));
        assert_eq!(
            err(raw("Head", "lines")),
            "grammar item `Head`: missing field `heading` (required for kind `lines`)"
        );
    }

    #[test]
    fn round_trip_through_raw() {
        let items = [
            ItemKind::Basic {
                text: "x".into(),
                regex: true,
                match_case: false,
            },
            ItemKind::Delimited {
                from: "/*".into(),
                to: Some("*/".into()),
                stop_at_eol: false,
                escape: None,
                regex: false,
                match_case: true,
            },
            ItemKind::Delimited {
                from: "\"".into(),
                to: Some("\"".into()),
                stop_at_eol: true,
                escape: Some('\\'),
                regex: false,
                match_case: true,
            },
            ItemKind::List {
                tokens: vec!["if".into()],
                regex: false,
                match_case: true,
            },
            ItemKind::Columns {
                from_column: 1,
                to_column: Some(6),
            },
            ItemKind::Lines {
                heading: String::new(),
                regex: false,
                match_case: true,
                next_lines: 2,
                or_line_one: true,
            },
            ItemKind::Lines {
                heading: "Page".into(),
                regex: false,
                match_case: true,
                next_lines: 0,
                or_line_one: false,
            },
        ];
        for kind in items {
            let item = GrammarItem {
                element: ElementName::new("E").unwrap(),
                kind,
            };
            let back = GrammarItem::try_from(RawGrammarItem::from(item.clone())).unwrap();
            assert_eq!(back, item);
        }
    }
}
