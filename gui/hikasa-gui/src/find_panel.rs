//! Find Filename (M2 W9a, FC-061; D-241): the one-line panel above a folder tree (not a dialog: the tree stays
//! usable) and its query. The search itself is core's `Finder` over every row the display filter shows, collapsed or
//! not (`FolderViewModel::find`). The options are remembered in `state.dialogs."find-filename"`, the text in
//! `state.mru.find`. Toolkit-free; `ui/find_panel.rs` draws it.

use hikasa_folder::select::{FindQuery, SideSet};
use hikasa_session::settings::State;
use hikasa_session::settings::state::MruList;

/// The `[dialogs]` id of the options.
pub const FIND_DIALOG: &str = "find-filename";

/// The help topic of the panel.
pub const FIND_TOPIC: &str = "panel-find-filename";

/// The message of a search without a hit.
pub const NOT_FOUND: &str = "Filename not found.";

/// The options' keys in `[dialogs.find-filename]`, with their flags.
const KEYS: [&str; 5] = ["match_case", "whole_word", "regex", "wrap", "up"];

/// The sides' words in `[dialogs.find-filename] sides`.
const SIDES: [(&str, SideSet); 3] = [
    ("left", SideSet::LEFT),
    ("right", SideSet::RIGHT),
    ("both", SideSet::all()),
];

/// The panel: the query being typed and the last search's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindPanel {
    /// The text or the regular expression.
    pub text: String,
    /// *Match case*.
    pub case: bool,
    /// *Whole word*.
    pub whole_word: bool,
    /// *Regular expression*.
    pub regex: bool,
    /// *Wrap around*.
    pub wrap: bool,
    /// *Search up*.
    pub up: bool,
    /// The sides whose names are looked at.
    pub sides: SideSet,
    /// The bad regular expression's message (shown in the panel until the next search).
    pub error: Option<String>,
    /// The recent texts (`state.mru.find`), newest first.
    pub recent: Vec<String>,
    /// The text field asks for the keyboard (the panel was just opened).
    pub focus: bool,
}

impl Default for FindPanel {
    fn default() -> FindPanel {
        FindPanel {
            text: String::new(),
            case: false,
            whole_word: false,
            regex: false,
            wrap: true,
            up: false,
            sides: SideSet::all(),
            error: None,
            recent: Vec::new(),
            focus: false,
        }
    }
}

/// What the search found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FindOutcome {
    /// A row was selected (its index among the shown rows).
    Found(usize),
    /// No row matches.
    NotFound,
    /// The query is not valid (a bad regular expression): its message.
    Invalid(String),
    /// Nothing to look for (empty text, no comparison).
    Nothing,
}

impl FindPanel {
    /// The panel with the remembered options and the most recent text.
    pub fn from_state(state: &State) -> FindPanel {
        let mut panel = FindPanel::default();
        let flag = |key: &str| state.dialog_text(FIND_DIALOG, key).map(|v| v == "true");
        for (key, slot) in KEYS.iter().zip([
            &mut panel.case,
            &mut panel.whole_word,
            &mut panel.regex,
            &mut panel.wrap,
            &mut panel.up,
        ]) {
            if let Some(on) = flag(key) {
                *slot = on;
            }
        }
        if let Some(sides) = state
            .dialog_text(FIND_DIALOG, "sides")
            .and_then(|word| SIDES.iter().find(|(w, _)| *w == word))
        {
            panel.sides = sides.1;
        }
        panel.recent = state.mru(MruList::Find).to_vec();
        if let Some(text) = panel.recent.first() {
            panel.text = text.clone();
        }
        panel
    }

    /// Remembers the options and pushes the text to `mru.find` (capped at `max`).
    pub fn save(&mut self, state: &mut State, max: usize) {
        for (key, on) in
            KEYS.iter()
                .zip([self.case, self.whole_word, self.regex, self.wrap, self.up])
        {
            state.set_dialog_text(FIND_DIALOG, key, if on { "true" } else { "false" });
        }
        if let Some((word, _)) = SIDES.iter().find(|(_, s)| *s == self.sides) {
            state.set_dialog_text(FIND_DIALOG, "sides", word);
        }
        if !self.text.is_empty() {
            state.push_mru(MruList::Find, &self.text, max);
            self.recent = state.mru(MruList::Find).to_vec();
        }
    }

    /// The query; `reverse` searches the other way (Find Previous).
    pub fn query(&self, reverse: bool) -> FindQuery {
        FindQuery {
            text: self.text.clone(),
            case: self.case,
            whole_word: self.whole_word,
            regex: self.regex,
            wrap: self.wrap,
            up: self.up != reverse,
            sides: self.sides,
        }
    }
}
