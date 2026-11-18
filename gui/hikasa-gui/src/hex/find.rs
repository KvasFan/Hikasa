//! The hex view's Find panel (M2 W13a, contract §1.13.7, D-359), toolkit-free: a one-line panel above the panes with
//! the pattern (MRU `mru.hex_find`), *Text* / *Hex bytes*, the encoding, *Match case*, *Wrap around* and the side. The
//! pattern is compiled by the Service on each edit (`Service::hex_finder`, core's parsers): a refused pattern is the
//! panel's message and greys its buttons.

use hikasa_core::Side;
use hikasa_hex::HexFinder;
use hikasa_session::Service;
use hikasa_session::api::HexFindQuery;

/// The help topic of the panel.
pub const FIND_TOPIC: &str = "panel-hex-find";

/// The encodings a text pattern offers: the core's words with their captions.
pub const ENCODINGS: [(&str, &str); 4] = [
    ("utf-8", "UTF-8"),
    ("latin1", "Latin-1"),
    ("utf-16le", "UTF-16 LE"),
    ("utf-16be", "UTF-16 BE"),
];

/// The Find panel.
#[derive(Debug, Clone)]
pub struct FindPanel {
    /// Shown.
    pub open: bool,
    /// The pattern as typed.
    pub pattern: String,
    /// Hex bytes instead of text.
    pub hex: bool,
    /// The text's encoding word (one of [`ENCODINGS`]).
    pub encoding: &'static str,
    /// Match the case of ASCII letters (text only).
    pub match_case: bool,
    /// Continue from the other end.
    pub wrap: bool,
    /// The side searched (the focus side when the panel opens).
    pub side: Side,
    /// The refusal of the pattern (the core's message), or `None`.
    pub message: Option<String>,
    finder: Option<HexFinder>,
    /// Patterns searched since the last call (the window pushes them to `mru.hex_find`).
    searched: Vec<String>,
}

impl Default for FindPanel {
    fn default() -> FindPanel {
        FindPanel {
            open: false,
            pattern: String::new(),
            hex: false,
            encoding: ENCODINGS[0].0,
            match_case: false,
            wrap: true,
            side: Side::Left,
            message: None,
            finder: None,
            searched: Vec::new(),
        }
    }
}

impl FindPanel {
    /// The query of the fields.
    pub fn query(&self) -> HexFindQuery {
        HexFindQuery {
            pattern: self.pattern.clone(),
            hex: self.hex,
            encoding: Some(self.encoding.to_owned()),
            match_case: self.match_case,
        }
    }

    /// Compiles the pattern after an edit (pure: the UI thread).
    pub fn validate(&mut self, service: &Service) {
        if self.pattern.is_empty() {
            self.finder = None;
            self.message = None;
            return;
        }
        match service.hex_finder(&self.query()) {
            Ok(f) => {
                self.finder = Some(f);
                self.message = None;
            }
            Err(e) => {
                self.finder = None;
                self.message = Some(e.to_string());
            }
        }
    }

    /// The compiled pattern (`None`: empty or refused; the buttons are grey).
    pub fn finder(&self) -> Option<&HexFinder> {
        self.finder.as_ref()
    }

    /// A search starts: the pattern goes to the MRU.
    pub(super) fn remember(&mut self) {
        if !self.pattern.is_empty() {
            self.searched.push(self.pattern.clone());
        }
    }

    /// The patterns searched since the last call.
    pub fn take_searched(&mut self) -> Vec<String> {
        std::mem::take(&mut self.searched)
    }
}
