//! Toolbars, Shortcuts, Menus (M2 W4c, D-129; UI-003, OPT-011): a view selector, a search and the list of the
//! view's `implemented` commands, over the pure `CommandOverrides` helpers of core. Conflicts are computed live on
//! the working copy and shown on every row they concern; they never block Apply.

use hikasa_session::commands::{catalogue, effective};
use hikasa_session::settings::commands::DisplayFilterMode;
use hikasa_session::settings::{CommandOverrides, Shortcut};

use crate::commands::{CommandId, Scope, lookup};

use super::OptionsDialog;

/// The view types the GUI shows.
pub const SHOWN_VIEWS: [(&str, &str); 3] = [
    ("home", "Home"),
    ("folder-compare", "Folder Compare"),
    ("text-compare", "Text Compare"),
];

/// One row of the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandRow {
    /// The id.
    pub id: String,
    /// The caption (the id when the catalogue has none).
    pub caption: String,
    /// The description.
    pub description: String,
    /// The Menu check box; `None`: a command without a menu.
    pub menu: Option<bool>,
    /// The Toolbar check box; `None`: a command without an icon.
    pub toolbar: Option<bool>,
    /// The effective shortcuts.
    pub shortcuts: Vec<Shortcut>,
    /// Its shortcuts are overridden (what `Default` drops).
    pub customized: bool,
    /// The conflicts of its shortcuts: "Ctrl+F12 is also About".
    pub conflicts: Vec<String>,
}

/// The state of the page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandsPage {
    /// The view type.
    pub view: &'static str,
    /// The search text.
    pub search: String,
    /// The selected command.
    pub selected: Option<String>,
    /// The selected shortcut of the selected command.
    pub shortcut: Option<usize>,
    /// Add waits for a key.
    pub capturing: bool,
    /// The rows (rebuilt by an edit, never per frame).
    pub rows: Vec<CommandRow>,
}

impl Default for CommandsPage {
    fn default() -> CommandsPage {
        CommandsPage {
            view: SHOWN_VIEWS[0].0,
            search: String::new(),
            selected: None,
            shortcut: None,
            capturing: false,
            rows: Vec::new(),
        }
    }
}

fn scope_of(view: &str) -> Scope {
    match view {
        "folder-compare" => Scope::Folder,
        "text-compare" => Scope::Text,
        _ => Scope::Home,
    }
}

/// True when the command has an icon in the view (only those can be on a toolbar).
fn has_icon(view: &str, id: &str) -> bool {
    CommandId::from_name(id)
        .and_then(|c| lookup(scope_of(view), c))
        .is_some_and(|r| r.icon.is_some())
}

/// The rows of `view` matching `search` (id, caption, description, shortcut; case-insensitive).
pub fn rows(overrides: &CommandOverrides, view: &'static str, search: &str) -> Vec<CommandRow> {
    let table = effective(catalogue(), view, overrides);
    let conflicts = overrides.user_conflicts(view);
    let needle = search.trim().to_lowercase();
    table
        .entries
        .iter()
        .filter(|e| e.implemented())
        .filter_map(|e| {
            let id = e.id().to_owned();
            let shortcuts = e.shortcuts.clone();
            let texts: Vec<String> = shortcuts.iter().map(ToString::to_string).collect();
            let found = needle.is_empty()
                || [&id, &e.caption.to_owned(), &e.description.to_owned()]
                    .iter()
                    .chain(texts.iter().collect::<Vec<_>>().iter())
                    .any(|t| t.to_lowercase().contains(&needle));
            if !found {
                return None;
            }
            let row_conflicts = conflicts
                .iter()
                .filter(|c| c.commands.contains(&id))
                .map(|c| {
                    let others: Vec<&str> = c
                        .commands
                        .iter()
                        .filter(|o| **o != id)
                        .map(|o| {
                            table
                                .get(o)
                                .map_or(o.as_str(), |x| non_empty(x.caption, o.as_str()))
                        })
                        .collect();
                    format!("{} is also {}", c.shortcut, others.join(", "))
                })
                .collect();
            let customized = table.custom.command.get(&id);
            let own_shortcuts = customized.is_some_and(|c| c.shortcuts.is_some());
            Some(CommandRow {
                caption: non_empty(e.caption, &id).to_owned(),
                description: e.description.to_owned(),
                menu: (!e.menu.is_empty() && !e.caption.is_empty())
                    .then(|| customized.and_then(|c| c.menu).unwrap_or(true)),
                toolbar: has_icon(view, &id).then_some(
                    customized
                        .and_then(|c| c.toolbar)
                        .unwrap_or(e.entry.toolbar),
                ),
                shortcuts,
                customized: own_shortcuts,
                conflicts: row_conflicts,
                id,
            })
        })
        .collect()
}

fn non_empty<'a>(text: &'a str, other: &'a str) -> &'a str {
    if text.is_empty() { other } else { text }
}

/// One of the view's flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewFlag {
    /// Large buttons.
    LargeButtons,
    /// Show all in menu.
    ShowAllInMenu,
    /// Show all on toolbar.
    ShowAllOnToolbar,
}

impl OptionsDialog {
    /// Rebuilds the rows of the page.
    pub fn refresh_commands(&mut self) {
        self.commands_page.rows = rows(
            &self.commands,
            self.commands_page.view,
            &self.commands_page.search,
        );
    }

    /// Selects the view type of the page.
    pub fn select_view(&mut self, view: &'static str) {
        self.commands_page.view = view;
        self.commands_page.selected = None;
        self.commands_page.shortcut = None;
        self.commands_page.capturing = false;
        self.refresh_commands();
    }

    /// The search text changed.
    pub fn set_command_search(&mut self, text: &str) {
        text.clone_into(&mut self.commands_page.search);
        self.refresh_commands();
    }

    /// Selects a command.
    pub fn select_command(&mut self, id: &str) {
        self.commands_page.selected = Some(id.to_owned());
        self.commands_page.shortcut = None;
        self.commands_page.capturing = false;
    }

    fn effective_of(&self, id: &str) -> Vec<Shortcut> {
        effective(catalogue(), self.commands_page.view, &self.commands)
            .get(id)
            .map(|e| e.shortcuts.clone())
            .unwrap_or_default()
    }

    /// Add: the next key becomes a shortcut of the selected command.
    pub fn begin_capture(&mut self) {
        self.commands_page.capturing = self.commands_page.selected.is_some();
    }

    /// A capture ends without a key (the page was left, the search took the focus).
    pub fn end_capture(&mut self) {
        self.commands_page.capturing = false;
        self.open_with.capturing = false;
    }

    /// The key a capture waited for. Add: Esc gives up, anything else is added (a duplicate of the command is
    /// ignored).
    pub fn capture(&mut self, key: &Shortcut) {
        if self.open_with.capturing {
            self.open_with_capture(key);
            return;
        }
        self.commands_page.capturing = false;
        let plain = !key.ctrl && !key.alt && !key.shift && !key.cmd;
        if plain && key.key == "Esc" {
            return;
        }
        let Some(id) = self.commands_page.selected.clone() else {
            return;
        };
        let mut list = self.effective_of(&id);
        if !list.contains(key) {
            list.push(key.clone());
            let view = self.commands_page.view;
            self.commands.set_shortcuts(view, &id, Some(list));
            self.commands_edited();
        }
    }

    /// Remove: the selected shortcut (removing the last one writes `shortcuts = []`).
    pub fn remove_shortcut(&mut self) {
        let (Some(id), Some(index)) = (
            self.commands_page.selected.clone(),
            self.commands_page.shortcut,
        ) else {
            return;
        };
        let mut list = self.effective_of(&id);
        if index < list.len() {
            list.remove(index);
            let view = self.commands_page.view;
            self.commands.set_shortcuts(view, &id, Some(list));
            self.commands_page.shortcut = None;
            self.commands_edited();
        }
    }

    /// Default: the override of the selected command's shortcuts goes.
    pub fn default_shortcuts(&mut self) {
        let Some(id) = self.commands_page.selected.clone() else {
            return;
        };
        let view = self.commands_page.view;
        self.commands.set_shortcuts(view, &id, None);
        self.commands_page.shortcut = None;
        self.commands_edited();
    }

    /// The Menu check box of a command.
    pub fn set_command_menu(&mut self, id: &str, shown: bool) {
        let view = self.commands_page.view;
        // A command with a menu shows by default: only "off" is an override.
        self.commands
            .set_in_menu(view, id, (!shown).then_some(false));
        self.commands_edited();
    }

    /// The Toolbar check box of a command.
    pub fn set_command_toolbar(&mut self, id: &str, shown: bool) {
        let view = self.commands_page.view;
        let default = effective(catalogue(), view, &CommandOverrides::default())
            .get(id)
            .is_some_and(|e| e.on_toolbar);
        self.commands
            .set_on_toolbar(view, id, (shown != default).then_some(shown));
        self.commands_edited();
    }

    /// The display filter mode of the view.
    pub fn set_filter_mode(&mut self, mode: DisplayFilterMode) {
        let view = self.commands_page.view;
        self.commands.view_mut(view).display_filter_mode = mode;
        self.commands.prune_view(view);
        self.commands_edited();
    }

    /// A flag of the view.
    pub fn set_view_flag(&mut self, flag: ViewFlag, on: bool) {
        let view = self.commands_page.view;
        let custom = self.commands.view_mut(view);
        match flag {
            ViewFlag::LargeButtons => custom.large_buttons = on,
            ViewFlag::ShowAllInMenu => custom.show_all_in_menu = on,
            ViewFlag::ShowAllOnToolbar => custom.show_all_on_toolbar = on,
        }
        self.commands.prune_view(view);
        self.commands_edited();
    }

    /// The view's customization shown by the page.
    pub fn view_custom(&self) -> hikasa_session::settings::ViewCustomization {
        self.commands
            .view
            .get(self.commands_page.view)
            .cloned()
            .unwrap_or_default()
    }

    /// Back to the view's defaults (`reset_view`).
    pub fn reset_commands(&mut self) {
        let view = self.commands_page.view;
        self.commands.reset_view(view);
        self.commands_edited();
    }

    fn commands_edited(&mut self) {
        self.refresh_commands();
        self.edited();
    }
}
