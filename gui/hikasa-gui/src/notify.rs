//! Notifications (M2 W4a, D-117; UI-031, SES-053): non-modal notices in the window concerned, at most three
//! visible, newest on top; info notices hide after 8 s, warnings, errors and notices with actions stay until
//! dismissed; a notice replaces an earlier one with the same key. Times are the frame times
//! (`ctx.input(|i| i.time)`), so tests pass explicit times.

use hikasa_session::ipc::ViewId;

/// How long an info notice without actions stays (seconds).
pub const INFO_SECONDS: f64 = 8.0;
/// Notices visible at once.
pub const MAX_VISIBLE: usize = 3;

/// The kind of a notice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NoticeKind {
    /// Information.
    Info,
    /// A warning.
    Warning,
    /// An error.
    Error,
}

/// A button of a notice.
#[allow(dead_code)] // variants for W4b/W4c (contract seams)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NoticeAction {
    /// Reloads a view (its input changed on disk).
    ReloadView(ViewId),
    /// Reloads the settings (discards the dialog's edits, W4c).
    ReloadSettings,
    /// Keeps the dialog's edits (W4c).
    KeepMine,
    /// Shows the tab of a view (a background job failed).
    ShowTab(ViewId),
    /// Opens the Options dialog (W4c).
    OpenOptions,
    /// Hides the notice.
    Dismiss,
}

impl NoticeAction {
    /// The button's label.
    pub fn label(self) -> &'static str {
        match self {
            NoticeAction::ReloadView(_) => "Reload",
            NoticeAction::ReloadSettings => "Reload (discard my edits)",
            NoticeAction::KeepMine => "Keep my edits",
            NoticeAction::ShowTab(_) => "Show",
            NoticeAction::OpenOptions => "Options…",
            NoticeAction::Dismiss => "Dismiss",
        }
    }
}

/// One notice.
#[derive(Debug, Clone, PartialEq)]
pub struct Notice {
    /// Equal keys replace each other (`settings:options.toml`, `input:<path>`).
    pub key: String,
    /// Kind.
    pub kind: NoticeKind,
    /// Text.
    pub text: String,
    /// Buttons (besides the close button).
    pub actions: Vec<NoticeAction>,
    /// When it was pushed (frame time, seconds).
    pub since: f64,
}

impl Notice {
    /// A notice without actions (its time is set by [`Notices::push`]).
    pub fn new(key: impl Into<String>, kind: NoticeKind, text: impl Into<String>) -> Notice {
        Notice {
            key: key.into(),
            kind,
            text: text.into(),
            actions: Vec::new(),
            since: 0.0,
        }
    }

    /// The notice with buttons.
    pub fn with_actions(mut self, actions: Vec<NoticeAction>) -> Notice {
        self.actions = actions;
        self
    }

    /// True when it hides by itself at `now`: an info notice without actions older than [`INFO_SECONDS`].
    pub fn expired(&self, now: f64) -> bool {
        self.kind == NoticeKind::Info && self.actions.is_empty() && now - self.since >= INFO_SECONDS
    }
}

/// The notices of one window (oldest first).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Notices {
    items: Vec<Notice>,
}

impl Notices {
    /// Adds a notice at `now`; one with the same key goes away.
    pub fn push(&mut self, mut notice: Notice, now: f64) {
        self.items.retain(|n| n.key != notice.key);
        notice.since = now;
        self.items.push(notice);
    }

    /// The visible notices at `now`, newest first, at most [`MAX_VISIBLE`]. Expired ones are dropped.
    pub fn visible(&mut self, now: f64) -> Vec<&Notice> {
        self.items.retain(|n| !n.expired(now));
        self.items.iter().rev().take(MAX_VISIBLE).collect()
    }

    /// Every notice kept (oldest first), expired or not.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn all(&self) -> &[Notice] {
        &self.items
    }

    /// The notice with `key`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn get(&self, key: &str) -> Option<&Notice> {
        self.items.iter().find(|n| n.key == key)
    }

    /// Removes the notices about a view that closed: its keys (`input:<view>:…`, `job:<view>`) and every notice
    /// whose actions name it.
    pub fn forget_view(&mut self, view: hikasa_session::ipc::ViewId) {
        let job = format!("job:{}", view.0);
        let input = format!("input:{}:", view.0);
        self.items.retain(|n| {
            n.key != job
                && !n.key.starts_with(&input)
                && !n.actions.iter().any(|a| {
                    matches!(a, NoticeAction::ShowTab(v) | NoticeAction::ReloadView(v) if *v == view)
                })
        });
    }

    /// Removes the notice `key`.
    pub fn dismiss(&mut self, key: &str) {
        self.items.retain(|n| n.key != key);
    }

    /// A button of the notice `key` was pressed: the notice goes away and its action is returned (when it has
    /// that action).
    pub fn act(&mut self, key: &str, action: NoticeAction) -> Option<NoticeAction> {
        let found = self.items.iter().any(|n| {
            n.key == key && (action == NoticeAction::Dismiss || n.actions.contains(&action))
        });
        if found {
            self.dismiss(key);
            Some(action)
        } else {
            None
        }
    }

    /// Number of notices kept.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// The time of the next expiry after `now` (the window repaints then), if any.
    pub fn next_expiry(&self, now: f64) -> Option<f64> {
        self.items
            .iter()
            .filter(|n| n.kind == NoticeKind::Info && n.actions.is_empty())
            .map(|n| n.since + INFO_SECONDS)
            .filter(|t| *t > now)
            .reduce(f64::min)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A4.8: dedupe by key, at most three visible, info hidden after 8 s, actions persist.
    #[test]
    fn notices_dedupe_cap_and_expire() {
        let mut n = Notices::default();
        n.push(Notice::new("a", NoticeKind::Info, "one"), 0.0);
        n.push(Notice::new("a", NoticeKind::Info, "two"), 1.0);
        assert_eq!(n.len(), 1);
        assert_eq!(n.get("a").map(|x| x.text.as_str()), Some("two"));
        n.push(Notice::new("b", NoticeKind::Warning, "w"), 2.0);
        n.push(
            Notice::new("c", NoticeKind::Info, "act")
                .with_actions(vec![NoticeAction::ReloadSettings]),
            3.0,
        );
        n.push(Notice::new("d", NoticeKind::Error, "e"), 4.0);
        let keys: Vec<&str> = n.visible(4.0).iter().map(|x| x.key.as_str()).collect();
        assert_eq!(keys, ["d", "c", "b"]);
        assert_eq!(n.next_expiry(4.0), Some(9.0));
        // At 9 s the plain info notice is gone; the others stay.
        let keys: Vec<&str> = n.visible(9.0).iter().map(|x| x.key.as_str()).collect();
        assert_eq!(keys, ["d", "c", "b"]);
        assert!(n.get("a").is_none());
        let keys: Vec<&str> = n.visible(1000.0).iter().map(|x| x.key.as_str()).collect();
        assert_eq!(keys, ["d", "c", "b"]);
        // Acting: only an action the notice has; it then goes away.
        assert_eq!(n.act("c", NoticeAction::KeepMine), None);
        assert_eq!(
            n.act("c", NoticeAction::ReloadSettings),
            Some(NoticeAction::ReloadSettings)
        );
        assert!(n.get("c").is_none());
        assert_eq!(
            n.act("b", NoticeAction::Dismiss),
            Some(NoticeAction::Dismiss)
        );
        assert_eq!(n.len(), 1);
    }
}
