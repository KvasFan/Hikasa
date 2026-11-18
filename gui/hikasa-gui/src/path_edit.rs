//! The path edits above the panes of a folder view (M2 W9a, FC-002; D-239): the typed text, the committed spec, the
//! completion list and its debounce, the MRU list. Toolkit-free: the frame time decides when a completion is due (no
//! timer thread); the view model runs `Service::path_completions` in a job and drops an answer of an older
//! generation; `ui/folder.rs` draws the fields.

use std::time::{Duration, Instant};

use hikasa_core::Side;
use hikasa_session::api::paths::{Completion, CompletionDto};

/// How long after the last key a completion is asked for (D-239).
pub const DEBOUNCE: Duration = Duration::from_millis(150);

/// The most completions one answer holds (D-239).
pub const LIMIT: usize = 200;

/// The tooltip and message of a completion whose name is not valid Unicode.
pub const LOSSY_ITEM: &str = "This name is not valid Unicode: it cannot be typed or taken here.";

/// A key the field gives the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathKey {
    /// Up in the list.
    Up,
    /// Down in the list.
    Down,
    /// Tab: the highlighted item into the field.
    Tab,
    /// Enter: the highlighted item, else Accept.
    Enter,
    /// Esc: close the list, else Cancel.
    Esc,
}

/// What a key did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathKeyOutcome {
    /// The list or the text changed.
    Handled,
    /// Accept both fields.
    Accept,
    /// The text went back to the committed spec.
    Cancelled,
    /// Not the field's key.
    Ignored,
}

/// One path edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathEdit {
    /// The side it edits.
    pub side: Side,
    /// The text in the field.
    pub text: String,
    /// The spec the view runs with.
    pub committed: String,
    /// The last answer's items.
    pub completions: Vec<Completion>,
    /// The list is shown.
    pub open: bool,
    /// The highlighted item of the list.
    pub highlighted: Option<usize>,
    /// Counts the edits: an answer for an older one is dropped.
    pub generation: u64,
    /// The message under the field (a spec that cannot be used).
    pub error: Option<String>,
    /// The recent specs of the side (`state.mru.left` / `.right`), newest first.
    pub recent: Vec<String>,
    /// When the text last changed, while a completion is still to be asked for.
    typed_at: Option<Instant>,
}

impl PathEdit {
    /// A field showing `committed`.
    pub fn new(side: Side, committed: &str) -> PathEdit {
        PathEdit {
            side,
            text: committed.to_owned(),
            committed: committed.to_owned(),
            completions: Vec::new(),
            open: false,
            highlighted: None,
            generation: 0,
            error: None,
            recent: Vec::new(),
            typed_at: None,
        }
    }

    /// The text differs from the committed spec (the Accept and Cancel buttons show).
    pub fn changed(&self) -> bool {
        self.text != self.committed
    }

    /// The text changed at frame time `now` (a key, a paste, an item taken): a completion is due after
    /// [`DEBOUNCE`].
    pub fn edited(&mut self, now: Instant) {
        self.generation += 1;
        self.typed_at = Some(now);
        self.error = None;
    }

    /// Replaces the text (a pick from the MRU list or the browse dialog): no completion is asked for.
    pub fn set_text(&mut self, text: &str) {
        self.text = text.to_owned();
        self.generation += 1;
        self.typed_at = None;
        self.open = false;
        self.error = None;
    }

    /// A frame at `now`: `Ok(generation)` when a completion is due now (once per edit), `Err(Some(wait))` while
    /// waiting, `Err(None)` when nothing is to be asked for.
    pub fn due(&mut self, now: Instant) -> Result<u64, Option<Duration>> {
        let Some(at) = self.typed_at else {
            return Err(None);
        };
        let still = now.saturating_duration_since(at);
        if still < DEBOUNCE {
            return Err(Some(DEBOUNCE - still));
        }
        self.typed_at = None;
        Ok(self.generation)
    }

    /// The answer of the completion job of `generation`; true when it was taken (not an older one's).
    pub fn answer(&mut self, generation: u64, dto: CompletionDto) -> bool {
        if generation != self.generation {
            return false;
        }
        self.completions = dto.items;
        self.open = !self.completions.is_empty();
        self.highlighted = None;
        true
    }

    /// True when the field takes `key` now (the window gives the others on: Tab with nothing highlighted moves the
    /// focus as usual).
    pub fn handles(&self, key: PathKey) -> bool {
        let n = self.completions.len();
        match key {
            PathKey::Down | PathKey::Up => self.open && n > 0,
            PathKey::Tab => self.open && self.highlighted.is_some(),
            PathKey::Enter | PathKey::Esc => true,
        }
    }

    /// A key of the field at `now`.
    pub fn key(&mut self, key: PathKey, now: Instant) -> PathKeyOutcome {
        let n = self.completions.len();
        match key {
            PathKey::Down if self.open && n > 0 => {
                self.highlighted = Some(self.highlighted.map_or(0, |i| (i + 1).min(n - 1)));
                PathKeyOutcome::Handled
            }
            PathKey::Up if self.open && n > 0 => {
                self.highlighted = Some(self.highlighted.map_or(n - 1, |i| i.saturating_sub(1)));
                PathKeyOutcome::Handled
            }
            PathKey::Tab | PathKey::Enter if self.open && self.highlighted.is_some() => {
                let index = self.highlighted.unwrap_or(0);
                self.take(index, now);
                PathKeyOutcome::Handled
            }
            PathKey::Enter => {
                self.open = false;
                PathKeyOutcome::Accept
            }
            PathKey::Esc if self.open => {
                self.open = false;
                PathKeyOutcome::Handled
            }
            PathKey::Esc => {
                self.cancel();
                PathKeyOutcome::Cancelled
            }
            _ => PathKeyOutcome::Ignored,
        }
    }

    /// Puts item `index` of the list in the field: a folder's text ends with its separator, so the next completion
    /// lists its entries. An item whose name is not valid Unicode is not taken (it cannot be typed back; the list
    /// says so).
    pub fn take(&mut self, index: usize, now: Instant) {
        let Some(item) = self.completions.get(index) else {
            return;
        };
        if item.lossy {
            self.error = Some(LOSSY_ITEM.to_owned());
            return;
        }
        self.text = item.text.clone();
        self.open = false;
        self.highlighted = None;
        if item.folder {
            self.edited(now);
        } else {
            self.generation += 1;
            self.typed_at = None;
        }
    }

    /// Cancel: the committed spec again.
    pub fn cancel(&mut self) {
        self.set_text(&self.committed.clone());
    }
}
