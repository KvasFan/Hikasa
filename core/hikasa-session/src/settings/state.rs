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
}

/// A value of a `[layout.<view type>]` key (D-108; the W4 keys are booleans and point sizes).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LayoutValue {
    /// A flag (`toolbar`, `log_panel`, `thumbnail`).
    Bool(bool),
    /// A size in points (`log_height`, `font_size`).
    Float(f64),
}

impl LayoutValue {
    /// The flag, if this is one.
    pub fn as_bool(self) -> Option<bool> {
        match self {
            LayoutValue::Bool(b) => Some(b),
            LayoutValue::Float(_) => None,
        }
    }

    /// The size, if this is one.
    pub fn as_float(self) -> Option<f64> {
        match self {
            LayoutValue::Float(f) => Some(f),
            LayoutValue::Bool(_) => None,
        }
    }
}

impl State {
    /// The value of `key` in `[layout.<view>]`: a boolean, or a finite number (an integer counts); anything else
    /// is `None` (the view's default applies).
    pub fn layout_value(&self, view: &str, key: &str) -> Option<LayoutValue> {
        match self.layout.get(view)?.get(key)? {
            toml::Value::Boolean(b) => Some(LayoutValue::Bool(*b)),
            toml::Value::Float(f) if f.is_finite() => Some(LayoutValue::Float(*f)),
            toml::Value::Integer(i) => Some(LayoutValue::Float(*i as f64)),
            _ => None,
        }
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
        let v = match value {
            LayoutValue::Bool(b) => toml::Value::Boolean(b),
            LayoutValue::Float(f) => toml::Value::Float(f),
        };
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
        assert_eq!(state.layout_value("text-compare", "other"), None);
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
