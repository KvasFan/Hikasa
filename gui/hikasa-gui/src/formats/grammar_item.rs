//! The Grammar Item dialog (M2 W11c; contract §1.11.27, FF-020…FF-025): *Element* (a combo of the format's names, free
//! text; *Everything Else* refused), *Category* (all five: Basic, Delimited, List, Columns, Lines) and the fields of the
//! category — the text to match; from/to with *Stop at end of line* and *Escape character*; the token list one a line;
//! the from/to columns or *to end of line*; the heading text, *next N lines* and *or line 1* — with *Match case*,
//! *Regular expression* where docs/10 allows it and *This element is case sensitive* (FF-025, the element's flag, shared
//! by its items). OK checks the item alone with `format_check` of the format holding it, then hands it to the editor.
//!
//! The fields of every category are kept while another is shown, so switching back loses nothing.

use std::any::Any;

use hikasa_formats::{ElementName, FileFormat, GrammarItem, ItemKind};
use hikasa_session::Service;

use super::FormatsFollowup;
use crate::dialog::{ButtonRole, DialogButton, DialogModel, DialogStep, Followup};

/// The help topic.
pub const TOPIC: &str = "dialog-grammar-item";

/// The reserved element name (element 0).
pub const EVERYTHING_ELSE: &str = "Everything Else";

/// A category (FF-020…FF-024).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    /// FF-020.
    Basic,
    /// FF-021.
    Delimited,
    /// FF-022.
    List,
    /// FF-023.
    Columns,
    /// FF-024.
    Lines,
}

impl Category {
    /// Every category, in the combo's order.
    pub const ALL: [Category; 5] = [
        Category::Basic,
        Category::Delimited,
        Category::List,
        Category::Columns,
        Category::Lines,
    ];

    /// The caption.
    pub fn caption(self) -> &'static str {
        match self {
            Category::Basic => "Basic",
            Category::Delimited => "Delimited",
            Category::List => "List",
            Category::Columns => "Columns",
            Category::Lines => "Lines",
        }
    }

    /// True when the category's texts may be regular expressions (docs/10: not Columns).
    pub fn has_regex(self) -> bool {
        self != Category::Columns
    }
}

/// The fields of every category, as typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemFields {
    /// The category shown.
    pub category: Category,
    /// The element name (free text).
    pub element: String,
    /// Basic: the text.
    pub text: String,
    /// Delimited: from.
    pub from: String,
    /// Delimited: to (`""`: end of line).
    pub to: String,
    /// Delimited: *Stop at end of line*.
    pub stop_at_eol: bool,
    /// Delimited: *Escape character* (`""`: none).
    pub escape: String,
    /// List: the tokens, one a line.
    pub tokens: String,
    /// Columns: from (1-based).
    pub from_column: String,
    /// Columns: to (`""` with `to_end`).
    pub to_column: String,
    /// Columns: *to end of line*.
    pub to_end: bool,
    /// Lines: the heading text.
    pub heading: String,
    /// Lines: *next N lines*.
    pub next_lines: String,
    /// Lines: *or line 1*.
    pub or_line_one: bool,
    /// *Regular expression*.
    pub regex: bool,
    /// *Match case*.
    pub match_case: bool,
    /// *This element is case sensitive* (FF-025).
    pub case_sensitive: bool,
}

impl ItemFields {
    /// The fields of `item`, with its element's flag.
    pub fn of(item: &GrammarItem, case_sensitive: bool) -> ItemFields {
        let mut f = ItemFields {
            category: Category::Basic,
            element: item.element.as_str().to_owned(),
            text: String::new(),
            from: String::new(),
            to: String::new(),
            stop_at_eol: false,
            escape: String::new(),
            tokens: String::new(),
            from_column: "1".to_owned(),
            to_column: String::new(),
            to_end: true,
            heading: String::new(),
            next_lines: "0".to_owned(),
            or_line_one: false,
            regex: false,
            match_case: true,
            case_sensitive,
        };
        match &item.kind {
            ItemKind::Basic {
                text,
                regex,
                match_case,
            } => {
                f.category = Category::Basic;
                f.text.clone_from(text);
                (f.regex, f.match_case) = (*regex, *match_case);
            }
            ItemKind::Delimited {
                from,
                to,
                stop_at_eol,
                escape,
                regex,
                match_case,
            } => {
                f.category = Category::Delimited;
                f.from.clone_from(from);
                f.to = to.clone().unwrap_or_default();
                f.stop_at_eol = *stop_at_eol;
                f.escape = escape.map(String::from).unwrap_or_default();
                (f.regex, f.match_case) = (*regex, *match_case);
            }
            ItemKind::List {
                tokens,
                regex,
                match_case,
            } => {
                f.category = Category::List;
                f.tokens = tokens.join("\n");
                (f.regex, f.match_case) = (*regex, *match_case);
            }
            ItemKind::Columns {
                from_column,
                to_column,
            } => {
                f.category = Category::Columns;
                f.from_column = from_column.to_string();
                f.to_column = to_column.map(|n| n.to_string()).unwrap_or_default();
                f.to_end = to_column.is_none();
            }
            ItemKind::Lines {
                heading,
                regex,
                match_case,
                next_lines,
                or_line_one,
            } => {
                f.category = Category::Lines;
                f.heading.clone_from(heading);
                f.next_lines = next_lines.to_string();
                f.or_line_one = *or_line_one;
                (f.regex, f.match_case) = (*regex, *match_case);
            }
        }
        f
    }

    /// The item of the fields, or why it cannot be one.
    pub fn item(&self) -> Result<GrammarItem, String> {
        let name = self.element.trim();
        if name == EVERYTHING_ELSE {
            return Err(format!(
                "\"{EVERYTHING_ELSE}\" is reserved: it is the text no item matches."
            ));
        }
        let element = ElementName::new(name).map_err(|e| e.to_string())?;
        let number = |text: &str, what: &str| -> Result<u32, String> {
            text.trim()
                .parse()
                .map_err(|_| format!("{what} must be a whole number, not `{}`.", text.trim()))
        };
        let kind = match self.category {
            Category::Basic => ItemKind::Basic {
                text: self.text.clone(),
                regex: self.regex,
                match_case: self.match_case,
            },
            Category::Delimited => {
                let mut chars = self.escape.chars();
                let escape = match (chars.next(), chars.next()) {
                    (None, _) => None,
                    (Some(c), None) => Some(c),
                    _ => return Err("The escape character must be one character.".to_owned()),
                };
                ItemKind::Delimited {
                    from: self.from.clone(),
                    to: (!self.to.is_empty()).then(|| self.to.clone()),
                    stop_at_eol: self.stop_at_eol,
                    escape,
                    regex: self.regex,
                    match_case: self.match_case,
                }
            }
            Category::List => ItemKind::List {
                tokens: self
                    .tokens
                    .split('\n')
                    .map(|t| t.strip_suffix('\r').unwrap_or(t))
                    .filter(|t| !t.is_empty())
                    .map(str::to_owned)
                    .collect(),
                regex: self.regex,
                match_case: self.match_case,
            },
            Category::Columns => ItemKind::Columns {
                from_column: number(&self.from_column, "The first column")?,
                to_column: if self.to_end {
                    None
                } else {
                    Some(number(&self.to_column, "The last column")?)
                },
            },
            Category::Lines => ItemKind::Lines {
                heading: self.heading.clone(),
                regex: self.regex,
                match_case: self.match_case,
                next_lines: number(&self.next_lines, "The number of lines")?,
                or_line_one: self.or_line_one,
            },
        };
        Ok(GrammarItem { element, kind })
    }
}

/// What the dialog hands back to the editor.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemDone {
    /// The item's place (`None`: a new item).
    pub index: Option<usize>,
    /// The item.
    pub item: GrammarItem,
    /// The element's *case sensitive* flag.
    pub case_sensitive: bool,
}

/// The Grammar Item dialog.
pub struct GrammarItemDialog {
    /// The fields.
    pub fields: ItemFields,
    /// The names the element combo offers (the format's).
    pub names: Vec<String>,
    /// Core's message or a field that does not read.
    pub error: Option<String>,
    index: Option<usize>,
    format: FileFormat,
}

impl GrammarItemDialog {
    /// The dialog over `item` of `format` (`index` `None`: a new item).
    pub fn new(format: &FileFormat, index: Option<usize>, item: GrammarItem) -> GrammarItemDialog {
        let case_sensitive = format
            .elements
            .get(&item.element)
            .is_none_or(|i| i.case_sensitive);
        let mut names: Vec<String> = Vec::new();
        for n in format
            .grammar
            .iter()
            .map(|g| g.element.as_str().to_owned())
            .chain(format.elements.keys().map(|k| k.as_str().to_owned()))
        {
            if !names.contains(&n) {
                names.push(n);
            }
        }
        GrammarItemDialog {
            fields: ItemFields::of(&item, case_sensitive),
            names,
            error: None,
            index,
            format: format.clone(),
        }
    }

    /// The format with this item at its place (what OK checks).
    pub fn format_with(&self, item: &GrammarItem) -> FileFormat {
        let mut f = self.format.clone();
        match self.index {
            Some(k) if k < f.grammar.len() => f.grammar[k] = item.clone(),
            _ => f.grammar.push(item.clone()),
        }
        f
    }
}

impl DialogModel for GrammarItemDialog {
    fn title(&self) -> String {
        if self.index.is_some() {
            "Edit Grammar Item".to_owned()
        } else {
            "Add Grammar Item".to_owned()
        }
    }

    fn help_topic(&self) -> &'static str {
        TOPIC
    }

    fn buttons(&self) -> Vec<DialogButton> {
        vec![
            DialogButton {
                label: "OK".to_owned(),
                role: ButtonRole::Ok,
                enabled: !self.fields.element.trim().is_empty(),
                default: true,
            },
            DialogButton {
                label: "Cancel".to_owned(),
                role: ButtonRole::Cancel,
                enabled: true,
                default: false,
            },
        ]
    }

    fn on_button(&mut self, role: ButtonRole, svc: &Service) -> DialogStep {
        if role != ButtonRole::Ok {
            return DialogStep::Close;
        }
        let item = match self.fields.item() {
            Ok(item) => item,
            Err(e) => {
                self.error = Some(e);
                return DialogStep::Stay;
            }
        };
        // The item alone, in its format (core names it: *Grammar item 3: …*).
        if let Err(e) = svc.format_check(&self.format_with(&item)) {
            self.error = Some(e.to_string());
            return DialogStep::Stay;
        }
        DialogStep::CloseWith(vec![Followup::Formats(FormatsFollowup::Item {
            id: self.format.id.as_str().to_owned(),
            done: Box::new(ItemDone {
                index: self.index,
                item,
                case_sensitive: self.fields.case_sensitive,
            }),
        })])
    }

    fn size(&self) -> Option<(f32, f32)> {
        Some((520.0, 360.0))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(n: &str) -> ElementName {
        ElementName::new(n).unwrap()
    }

    /// Each category's fields to the model and back (A11.25).
    #[test]
    fn every_category_round_trips() {
        let kinds = [
            ItemKind::Basic {
                text: "#!".into(),
                regex: false,
                match_case: false,
            },
            ItemKind::Delimited {
                from: "\"".into(),
                to: Some("\"".into()),
                stop_at_eol: true,
                escape: Some('\\'),
                regex: false,
                match_case: true,
            },
            ItemKind::Delimited {
                from: "//".into(),
                to: None,
                stop_at_eol: false,
                escape: None,
                regex: true,
                match_case: false,
            },
            ItemKind::List {
                tokens: vec!["if".into(), "else".into()],
                regex: false,
                match_case: true,
            },
            ItemKind::Columns {
                from_column: 1,
                to_column: Some(6),
            },
            ItemKind::Columns {
                from_column: 73,
                to_column: None,
            },
            ItemKind::Lines {
                heading: "^Section".into(),
                regex: true,
                match_case: true,
                next_lines: 3,
                or_line_one: true,
            },
        ];
        for kind in kinds {
            let item = GrammarItem {
                element: name("Thing"),
                kind,
            };
            for case in [true, false] {
                let f = ItemFields::of(&item, case);
                assert_eq!(f.item().unwrap(), item);
                assert_eq!(f.case_sensitive, case);
            }
        }
    }

    #[test]
    fn everything_else_is_refused() {
        let item = GrammarItem {
            element: name("Comment"),
            kind: ItemKind::Basic {
                text: "x".into(),
                regex: false,
                match_case: true,
            },
        };
        let mut f = ItemFields::of(&item, true);
        f.element = EVERYTHING_ELSE.into();
        assert!(f.item().is_err());
        f.element = "  ".into();
        assert!(f.item().is_err());
    }
}
