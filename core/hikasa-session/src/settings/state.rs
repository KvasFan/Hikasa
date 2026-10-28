//! `state.toml` (SES-051; docs/03 §5.2): the last session, the most-recently-used lists, the last main window,
//! first-run and hint flags, and the free per-dialog tables.

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

impl State {
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
    fn window_ranges() {
        let mut state = State::default();
        state.window.width = 199;
        state.window.x = -2;
        let keys: Vec<String> = state.validate().into_iter().map(|p| p.key).collect();
        assert_eq!(keys, vec!["window.width", "window.x"]);
    }
}
