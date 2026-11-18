//! `state.toml` (SES-051; docs/03 §5.2): the last session, the most-recently-used lists, the last main window,
//! first-run and hint flags, the free per-dialog tables and the per-view-type layout tables (M2 W4a, D-108).

use std::collections::BTreeMap;

use toml::Table;

use super::file::{Problem, SettingsSchema, migrate_from_zero};
use super::keys::{FileId, KindRule, ValueKind};

settings_section! {
    /// `state.toml`.
    State {
        /// Name of the session last used (W2).
        last_session: String = String::new(),
        /// `[mru]`: each list newest first.
        mru: Mru = Mru::default(),
        /// `[window]`: the last main window.
        window: WindowState = WindowState::default(),
        /// `[flags]`.
        flags: Flags = Flags::default(),
        /// `[dialogs]`: free tables per dialog id, kept as they are (RPT-003).
        dialogs: Table = Table::new(),
        /// `[layout.<view type>]` (M2 W4a, D-108): an opaque table per view type (`text-compare`, …) whose keys
        /// the views own (W4: `toolbar`, `log_panel`, `log_height`, `font_size`, `thumbnail`), kept as they are.
        layout: BTreeMap<String, Table> = BTreeMap::new(),
    }
}

settings_section! {
    /// `[mru]` (each list newest first, capped at `max_mru_items` on push).
    Mru {
        /// Left paths.
        left: Vec<String> = Vec::new(),
        /// Right paths.
        right: Vec<String> = Vec::new(),
        /// Center (base) paths.
        center: Vec<String> = Vec::new(),
        /// Folders.
        folders: Vec<String> = Vec::new(),
        /// Files.
        files: Vec<String> = Vec::new(),
        /// Find texts.
        find: Vec<String> = Vec::new(),
        /// Replace texts.
        replace: Vec<String> = Vec::new(),
        /// Name filters.
        name_filters: Vec<String> = Vec::new(),
        /// Workspaces.
        workspaces: Vec<String> = Vec::new(),
        /// The *Look in* folders and files of Find in Files (M2 W11b, D-308).
        find_in_files: Vec<String> = Vec::new(),
        /// The Go To addresses of the Hex Compare view (M2 W13a, D-358).
        hex_goto: Vec<String> = Vec::new(),
        /// The Find panel patterns of the Hex Compare view (M2 W13a, D-359).
        hex_find: Vec<String> = Vec::new(),
    }
}

settings_section! {
    /// `[window]`: the last main window (a workspace stores its own, W2).
    WindowState {
        /// Width (200..=32767).
        width: i32 = 1200,
        /// Height (200..=32767).
        height: i32 = 800,
        /// Left edge (-1: the platform places the window).
        x: i32 = -1,
        /// Top edge (-1: the platform places the window).
        y: i32 = -1,
        /// Maximized.
        maximized: bool = false,
    }
}

settings_section! {
    /// `[flags]`.
    Flags {
        /// Set by the GUI's first start (D-21 materialization).
        first_run_done: bool = false,
        /// Ids of the one-time hints already shown.
        hints_seen: Vec<String> = Vec::new(),
    }
}

/// One most-recently-used list of `[mru]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MruList {
    /// `left`.
    Left,
    /// `right`.
    Right,
    /// `center`.
    Center,
    /// `folders`.
    Folders,
    /// `files`.
    Files,
    /// `find`.
    Find,
    /// `replace`.
    Replace,
    /// `name_filters`.
    NameFilters,
    /// `workspaces`.
    Workspaces,
    /// `find_in_files` (M2 W11b).
    FindInFiles,
    /// `hex_goto` (M2 W13a).
    HexGoTo,
    /// `hex_find` (M2 W13a).
    HexFind,
}

/// A value of a `[layout.<view type>]` key (D-108; the W4 keys are booleans and point sizes; M2 W9a adds the folder
/// views' column lists, sort words and width tables, D-234).
#[derive(Debug, Clone, PartialEq)]
pub enum LayoutValue {
    /// A flag (`toolbar`, `log_panel`, `thumbnail`).
    Bool(bool),
    /// A size in points (`log_height`, `font_size`).
    Float(f64),
    /// A word (`sort`, `sort_side`).
    Text(String),
    /// A list of words (`columns`).
    Texts(Vec<String>),
    /// Points by name (`widths`).
    Numbers(BTreeMap<String, f64>),
}

impl LayoutValue {
    /// The flag, if this is one.
    pub fn as_bool(self) -> Option<bool> {
        match self {
            LayoutValue::Bool(b) => Some(b),
            _ => None,
        }
    }

    /// The size, if this is one.
    pub fn as_float(self) -> Option<f64> {
        match self {
            LayoutValue::Float(f) => Some(f),
            _ => None,
        }
    }

    /// The word, if this is one.
    pub fn as_text(self) -> Option<String> {
        match self {
            LayoutValue::Text(t) => Some(t),
            _ => None,
        }
    }

    /// The list of words, if this is one.
    pub fn as_texts(self) -> Option<Vec<String>> {
        match self {
            LayoutValue::Texts(t) => Some(t),
            _ => None,
        }
    }

    /// The table of points, if this is one.
    pub fn as_numbers(self) -> Option<BTreeMap<String, f64>> {
        match self {
            LayoutValue::Numbers(n) => Some(n),
            _ => None,
        }
    }

    /// The TOML form.
    pub fn to_toml(&self) -> toml::Value {
        match self {
            LayoutValue::Bool(b) => toml::Value::Boolean(*b),
            LayoutValue::Float(f) => toml::Value::Float(*f),
            LayoutValue::Text(t) => toml::Value::String(t.clone()),
            LayoutValue::Texts(t) => {
                toml::Value::Array(t.iter().cloned().map(toml::Value::String).collect())
            }
            LayoutValue::Numbers(n) => toml::Value::Table(
                n.iter()
                    .map(|(k, v)| (k.clone(), toml::Value::Float(*v)))
                    .collect(),
            ),
        }
    }

    /// The layout value of a TOML value: a boolean, a finite number, a string, an array of strings, a table of
    /// numbers; anything else is none.
    pub fn of_toml(value: &toml::Value) -> Option<LayoutValue> {
        let number = |v: &toml::Value| match v {
            toml::Value::Float(f) if f.is_finite() => Some(*f),
            toml::Value::Integer(i) => Some(*i as f64),
            _ => None,
        };
        Some(match value {
            toml::Value::Boolean(b) => LayoutValue::Bool(*b),
            toml::Value::String(t) => LayoutValue::Text(t.clone()),
            toml::Value::Array(items) => LayoutValue::Texts(
                items
                    .iter()
                    .map(|v| v.as_str().map(str::to_owned))
                    .collect::<Option<Vec<_>>>()?,
            ),
            toml::Value::Table(table) => LayoutValue::Numbers(
                table
                    .iter()
                    .map(|(k, v)| number(v).map(|n| (k.clone(), n)))
                    .collect::<Option<BTreeMap<_, _>>>()?,
            ),
            other => LayoutValue::Float(number(other)?),
        })
    }
}

impl LayoutValue {
    /// The value of `value` keeping the good entries of a list or a table (M2 W9a after review): a list keeps its
    /// words, a table its numbers; what was left out comes back as text (an entry's value, a table's key), so the
    /// reader warns once and keeps the rest. A value of no layout type is `None`.
    pub fn of_toml_kept(value: &toml::Value) -> Option<(LayoutValue, Vec<String>)> {
        match value {
            toml::Value::Array(items) => {
                let mut kept = Vec::new();
                let mut dropped = Vec::new();
                for item in items {
                    match item.as_str() {
                        Some(word) => kept.push(word.to_owned()),
                        None => dropped.push(item.to_string()),
                    }
                }
                Some((LayoutValue::Texts(kept), dropped))
            }
            toml::Value::Table(table) => {
                let mut kept = BTreeMap::new();
                let mut dropped = Vec::new();
                for (key, item) in table {
                    match item {
                        toml::Value::Float(f) if f.is_finite() => {
                            kept.insert(key.clone(), *f);
                        }
                        toml::Value::Integer(i) => {
                            kept.insert(key.clone(), *i as f64);
                        }
                        _ => dropped.push(key.clone()),
                    }
                }
                Some((LayoutValue::Numbers(kept), dropped))
            }
            other => LayoutValue::of_toml(other).map(|v| (v, Vec::new())),
        }
    }
}

impl State {
    /// `key` of `[layout.<view>]` with the good entries of a list or a table kept and the others named
    /// ([`LayoutValue::of_toml_kept`]).
    pub fn layout_value_kept(&self, view: &str, key: &str) -> Option<(LayoutValue, Vec<String>)> {
        LayoutValue::of_toml_kept(self.layout.get(view)?.get(key)?)
    }

    /// A text remembered for a dialog (`[dialogs.<dialog>] <key>`; another type reads as none).
    pub fn dialog_text(&self, dialog: &str, key: &str) -> Option<&str> {
        self.dialogs.get(dialog)?.get(key)?.as_str()
    }

    /// Remembers a text for a dialog; the table's other keys stay.
    pub fn set_dialog_text(&mut self, dialog: &str, key: &str, value: &str) {
        let entry = self
            .dialogs
            .entry(dialog.to_owned())
            .or_insert_with(|| toml::Value::Table(Table::new()));
        if !entry.is_table() {
            *entry = toml::Value::Table(Table::new());
        }
        if let Some(table) = entry.as_table_mut() {
            table.insert(key.to_owned(), toml::Value::String(value.to_owned()));
        }
    }

    /// The value of `key` in `[layout.<view>]`: a boolean, or a finite number (an integer counts); anything else
    /// is `None` (the view's default applies).
    pub fn layout_value(&self, view: &str, key: &str) -> Option<LayoutValue> {
        LayoutValue::of_toml(self.layout.get(view)?.get(key)?)
    }

    /// Overlays a session's `[view]` keys on `[layout.<view>]` in this copy (the precedence of D-108: the
    /// session wins over the layout of the view type; keys of the wrong type are simply read past later).
    pub fn overlay_view(&mut self, view: &str, keys: &Table) {
        let table = self.layout.entry(view.to_owned()).or_default();
        for (key, value) in keys {
            table.insert(key.clone(), value.clone());
        }
    }

    /// Sets `key` of `[layout.<view>]` (the table is created on demand; its other keys stay).
    pub fn set_layout_value(&mut self, view: &str, key: &str, value: LayoutValue) {
        let v = value.to_toml();
        self.layout
            .entry(view.to_owned())
            .or_default()
            .insert(key.to_owned(), v);
    }

    /// The list `list`.
    pub fn mru(&self, list: MruList) -> &[String] {
        let m = &self.mru;
        match list {
            MruList::Left => &m.left,
            MruList::Right => &m.right,
            MruList::Center => &m.center,
            MruList::Folders => &m.folders,
            MruList::Files => &m.files,
            MruList::Find => &m.find,
            MruList::Replace => &m.replace,
            MruList::NameFilters => &m.name_filters,
            MruList::Workspaces => &m.workspaces,
            MruList::FindInFiles => &m.find_in_files,
            MruList::HexGoTo => &m.hex_goto,
            MruList::HexFind => &m.hex_find,
        }
    }

    /// Puts `value` at the front of `list` (an equal entry moves there) and keeps at most `max` entries.
    pub fn push_mru(&mut self, list: MruList, value: &str, max: usize) {
        let m = &mut self.mru;
        let items = match list {
            MruList::Left => &mut m.left,
            MruList::Right => &mut m.right,
            MruList::Center => &mut m.center,
            MruList::Folders => &mut m.folders,
            MruList::Files => &mut m.files,
            MruList::Find => &mut m.find,
            MruList::Replace => &mut m.replace,
            MruList::NameFilters => &mut m.name_filters,
            MruList::Workspaces => &mut m.workspaces,
            MruList::FindInFiles => &mut m.find_in_files,
            MruList::HexGoTo => &mut m.hex_goto,
            MruList::HexFind => &mut m.hex_find,
        };
        items.retain(|item| item != value);
        items.insert(0, value.to_owned());
        items.truncate(max);
    }
}

impl SettingsSchema for State {
    const FILE: FileId = FileId::State;
    const VERSION: u32 = 1;

    fn migrate(from: u32, _table: &mut Table) -> Result<(), String> {
        migrate_from_zero(from)
    }

    fn validate(&self) -> Vec<Problem> {
        super::validate_by_rules(self, &FileId::State)
    }
}

/// Kinds of the keys of `state.toml`.
pub(crate) const RULES: &[KindRule] = &[
    KindRule {
        pattern: "window.width",
        kind: ValueKind::Int {
            min: 200,
            max: 32767,
        },
        opt: None,
        priority: None,
    },
    KindRule {
        pattern: "window.height",
        kind: ValueKind::Int {
            min: 200,
            max: 32767,
        },
        opt: None,
        priority: None,
    },
    KindRule {
        pattern: "window.x",
        kind: ValueKind::Int {
            min: -1,
            max: 32767,
        },
        opt: None,
        priority: None,
    },
    KindRule {
        pattern: "window.y",
        kind: ValueKind::Int {
            min: -1,
            max: 32767,
        },
        opt: None,
        priority: None,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dialogs_remember_texts() {
        let mut state = State::default();
        assert_eq!(state.dialog_text("options", "page"), None);
        state.set_dialog_text("options", "page", "tabs");
        state.set_dialog_text("options", "other", "x");
        assert_eq!(state.dialog_text("options", "page"), Some("tabs"));
        assert_eq!(state.dialog_text("options", "other"), Some("x"));
        state
            .dialogs
            .insert("broken".into(), toml::Value::Integer(1));
        assert_eq!(state.dialog_text("broken", "page"), None);
        state.set_dialog_text("broken", "page", "y");
        assert_eq!(state.dialog_text("broken", "page"), Some("y"));
    }

    #[test]
    fn mru_push_moves_and_caps() {
        let mut state = State::default();
        for value in ["a", "b", "c", "a"] {
            state.push_mru(MruList::Left, value, 3);
        }
        assert_eq!(state.mru(MruList::Left), ["a", "c", "b"]);
        state.push_mru(MruList::Left, "d", 2);
        assert_eq!(state.mru(MruList::Left), ["d", "a"]);
        assert!(state.mru(MruList::Right).is_empty());
    }

    #[test]
    fn layout_values_read_and_write() {
        let mut state = State::default();
        assert_eq!(state.layout_value("text-compare", "font_size"), None);
        state.set_layout_value("text-compare", "font_size", LayoutValue::Float(15.0));
        state.set_layout_value("text-compare", "thumbnail", LayoutValue::Bool(false));
        assert_eq!(
            state.layout_value("text-compare", "font_size"),
            Some(LayoutValue::Float(15.0))
        );
        assert_eq!(
            state
                .layout_value("text-compare", "thumbnail")
                .and_then(LayoutValue::as_bool),
            Some(false)
        );
        let table = state.layout.get_mut("text-compare").unwrap();
        table.insert("font_size".into(), toml::Value::Integer(12));
        table.insert("other".into(), toml::Value::String("kept".into()));
        assert_eq!(
            state.layout_value("text-compare", "font_size"),
            Some(LayoutValue::Float(12.0))
        );
        // A word reads as one (M2 W9a: the folder views' `sort`); a date does not.
        assert_eq!(
            state.layout_value("text-compare", "other"),
            Some(LayoutValue::Text("kept".into()))
        );
        let table = state.layout.get_mut("text-compare").unwrap();
        table.insert(
            "when".into(),
            toml::Value::Datetime("1979-05-27T07:32:00Z".parse().unwrap()),
        );
        assert_eq!(state.layout_value("text-compare", "when"), None);
        state.set_layout_value("text-compare", "font_size", LayoutValue::Float(9.0));
        assert!(state.layout["text-compare"].contains_key("other"));
    }

    #[test]
    fn window_ranges() {
        let mut state = State::default();
        state.window.width = 199;
        state.window.x = -2;
        let keys: Vec<String> = state.validate().into_iter().map(|p| p.key).collect();
        assert_eq!(keys, vec!["window.width", "window.x"]);
    }
}
