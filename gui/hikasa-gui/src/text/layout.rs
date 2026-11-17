//! The display flags of a text view that W11a adds (M2 W11a; contract §1.11.4, §1.11.5, §1.11.11): *Word Wrap*,
//! *Overwrite Mode* (per view, not persisted, §1.11.5) and *File Info* (`[layout] header`, on by default). Word wrap
//! and the header are layout keys of the view type (`[layout.text-compare]`, D-108); W11b adds the other display
//! options of §1.11.14 here.

use hikasa_session::settings::State;
use hikasa_session::settings::state::LayoutValue;

/// The display flags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Display {
    /// Long lines wrap at the pane's width (`WordWrap`).
    pub word_wrap: bool,
    /// Typing replaces characters (`Overwriting`, Ins; per view, not persisted).
    pub overwrite: bool,
    /// The File Info header above each pane (`ShowSpecInfo`).
    pub header: bool,
}

impl Default for Display {
    fn default() -> Display {
        Display {
            word_wrap: false,
            overwrite: false,
            header: true,
        }
    }
}

impl Display {
    /// Reads `word_wrap` and `header` of the view type's layout (a value of the wrong type is ignored).
    pub fn apply(&mut self, layout: &State, view: &str) {
        if let Some(on) = layout
            .layout_value(view, "word_wrap")
            .and_then(LayoutValue::as_bool)
        {
            self.word_wrap = on;
        }
        if let Some(on) = layout
            .layout_value(view, "header")
            .and_then(LayoutValue::as_bool)
        {
            self.header = on;
        }
    }

    /// The layout keys these flags own.
    pub fn keys(&self) -> Vec<(&'static str, LayoutValue)> {
        vec![
            ("word_wrap", LayoutValue::Bool(self.word_wrap)),
            ("header", LayoutValue::Bool(self.header)),
        ]
    }
}
