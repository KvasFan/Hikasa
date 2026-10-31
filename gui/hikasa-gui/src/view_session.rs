//! The session of a comparison view (M2 W4b, §1.4.9; D-120, D-121, SES-001…005): where it came from, the session
//! as loaded, the lock, the auto-save slot and timer. [`crate::app::AppCore`] runs the close flow around it; this
//! file only holds the state and the rules that need nothing but that state. Toolkit-free (§1.4.16).

use hikasa_core::Warning;
use hikasa_session::api::ViewRequest;
use hikasa_session::sessions::{
    AutoId, CloseAction, OpaqueTable, Origin, ParentLink, Session, SessionRef, close_action,
};
use hikasa_session::{ApiError, Service};

/// The period of the auto-save timer in seconds (D-121; a constant, no option exists).
pub const AUTOSAVE_PERIOD: f64 = 300.0;

/// Where a view's session came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewOrigin {
    /// Never saved.
    Unnamed,
    /// An auto-saved session.
    AutoSaved(AutoId),
    /// A named session.
    Named(SessionRef),
}

/// The session state of one view.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewSession {
    /// Where it came from.
    pub origin: ViewOrigin,
    /// The session as loaded (the effective one, D-82); `None` for a view never saved.
    pub loaded: Option<Session>,
    /// The file is locked.
    pub locked: bool,
    /// The user chose Save on a locked session and has not confirmed yet (D-84).
    pub explicit_save: bool,
    /// The session of the last auto-save.
    pub last_autosaved: Option<Session>,
    /// The child link (TC-052).
    pub parent: Option<ParentLink>,
    /// When the timer looks next (frame time, seconds).
    pub next_autosave_at: f64,
    /// `is_dirty(loaded, current)`, computed off the UI thread when the view's specs or rules change (the only
    /// way they change): a close decides without disk access.
    pub dirty: bool,
    /// The description (the Session Settings dialog changes it).
    pub description: String,
    /// The close action of this view was taken: nothing more is written when its tab goes.
    pub decided: bool,
    /// An auto-save of this view is queued (at most one per view, D-121).
    pub autosaving: bool,
    /// The file as read when the view loaded it, or as this view last saved it: a Save never overwrites a file
    /// that is no longer the session the view holds (a session put under the old name after an outside rename).
    pub seen: Option<Session>,
    /// The auto-saved file of this view is locked: auto-save cannot write it, and a dirty view asks at its close.
    pub slot_locked: bool,
    /// The dialog's OK is being applied: `dirty` is provisional and a close writes only after the result.
    pub settings_pending: bool,
    /// The inline table of the workspace tab this view was restored from (D-90 for Save Workspace As).
    pub inline: Option<OpaqueTable>,
}

impl ViewSession {
    /// A view that was never saved.
    pub fn unnamed(now: f64) -> ViewSession {
        ViewSession {
            origin: ViewOrigin::Unnamed,
            loaded: None,
            locked: false,
            explicit_save: false,
            last_autosaved: None,
            parent: None,
            next_autosave_at: now + AUTOSAVE_PERIOD,
            dirty: false,
            description: String::new(),
            decided: false,
            autosaving: false,
            seen: None,
            slot_locked: false,
            settings_pending: false,
            inline: None,
        }
    }

    /// The origin as the D-84 table names it.
    pub fn origin_kind(&self) -> Origin {
        match self.origin {
            ViewOrigin::Unnamed => Origin::Unnamed,
            ViewOrigin::AutoSaved(_) => Origin::AutoSaved,
            ViewOrigin::Named(_) => Origin::Named,
        }
    }

    /// The named session of the view.
    pub fn reference(&self) -> Option<&SessionRef> {
        match &self.origin {
            ViewOrigin::Named(r) => Some(r),
            _ => None,
        }
    }

    /// True for a view of a named session.
    pub fn named(&self) -> bool {
        self.reference().is_some()
    }

    /// The action of closing this view (D-84).
    pub fn close_action(&self, auto_save: bool) -> CloseAction {
        if self.slot_locked && self.dirty && auto_save && !self.named() {
            // D-84 auto-save never writes a locked file, so "auto-save" here would drop the changes.
            return CloseAction::AskSaveChanges;
        }
        close_action(
            self.origin_kind(),
            self.locked,
            self.dirty,
            self.explicit_save,
            auto_save,
        )
    }

    /// True when the timer may look at this view now (D-121): not a named view, not while an auto-save of it is
    /// queued, and the period has passed. Whether the session changed is decided by the job.
    pub fn timer_due(&self, now: f64, auto_save: bool) -> bool {
        auto_save
            && !self.named()
            && !self.autosaving
            && !self.decided
            && now >= self.next_autosave_at
    }

    /// The parent link's text of a child of this view: `<reference>` or `_autosaved/<id>`; `None` for a view that
    /// has no file (TC-052 needs a named or auto-saved parent).
    pub fn link_text(&self, slot: Option<&AutoId>) -> Option<String> {
        match (&self.origin, slot) {
            (ViewOrigin::Named(r), _) => Some(r.to_string()),
            (ViewOrigin::AutoSaved(id), _) | (ViewOrigin::Unnamed, Some(id)) => {
                Some(format!("_autosaved/{id}"))
            }
            (ViewOrigin::Unnamed, None) => None,
        }
    }
}

/// What the UI thread takes from a view so that a job can build its current session (D-105).
#[derive(Debug, Clone)]
pub struct ViewCapture {
    /// The request of the view.
    pub request: ViewRequest,
    /// Its layout keys.
    pub view: OpaqueTable,
    /// Its display filter state.
    pub display: OpaqueTable,
    /// Its parent link.
    pub parent: Option<ParentLink>,
    /// Its description.
    pub description: String,
}

impl ViewCapture {
    /// `Service::session_for_view` with the view's description.
    pub fn session(&self, service: &Service) -> Result<(Session, Vec<Warning>), ApiError> {
        let (mut session, warnings) = service.session_for_view(
            &self.request,
            self.view.clone(),
            self.display.clone(),
            self.parent.clone(),
        )?;
        session.description.clone_from(&self.description);
        Ok((session, warnings))
    }
}

/// `session` with the identity its file holds now (lock, favorite, overrides): what a view's Save writes. The
/// view owns specs, rules, `view`, `display`, the description and the parent link.
pub fn merged_with_file(session: Session, file: &Session) -> Session {
    Session {
        locked: file.locked,
        favorite: file.favorite,
        overrides: file.overrides.clone(),
        ..session
    }
}

/// True when `file` is still the session `seen` was read from: the same type, specs, description, parent and rules.
/// Lock, favorite, overrides, `view` and `display` may change under an open view.
pub fn same_session(seen: &Session, file: &Session) -> bool {
    seen.kind == file.kind
        && seen.specs == file.specs
        && seen.description == file.description
        && seen.parent == file.parent
        && seen.rules == file.rules
        && seen.text_rules == file.text_rules
}

/// True when `current` holds what the last auto-save wrote: what the timer compares (D-121). The file's own fields
/// (`saved_at`, lock, favorite, overrides) are not the view's.
pub fn same_autosaved(last: &Session, current: &Session) -> bool {
    let plain = |s: &Session| Session {
        saved_at: None,
        locked: false,
        favorite: false,
        overrides: Vec::new(),
        ..s.clone()
    };
    plain(last) == plain(current)
}

#[cfg(test)]
mod tests {
    use hikasa_session::sessions::SessionType;

    use super::*;

    #[test]
    fn the_timer_rule() {
        let mut v = ViewSession::unnamed(10.0);
        assert_eq!(v.next_autosave_at, 310.0);
        assert!(!v.timer_due(309.9, true), "no save before 300 s");
        assert!(v.timer_due(310.0, true));
        assert!(!v.timer_due(310.0, false), "auto-save off");
        v.autosaving = true;
        assert!(!v.timer_due(400.0, true), "one at a time");
        v.autosaving = false;
        v.origin = ViewOrigin::Named(SessionRef::parse("a").unwrap());
        assert!(!v.timer_due(400.0, true), "never a named view");
    }

    #[test]
    fn close_actions_follow_the_origin() {
        let mut v = ViewSession::unnamed(0.0);
        assert_eq!(v.close_action(true), CloseAction::AutoSave);
        assert_eq!(v.close_action(false), CloseAction::Nothing);
        v.origin = ViewOrigin::Named(SessionRef::parse("a").unwrap());
        assert_eq!(v.close_action(true), CloseAction::SaveViewState);
        v.dirty = true;
        assert_eq!(v.close_action(true), CloseAction::AskSaveChanges);
        v.locked = true;
        assert_eq!(v.close_action(true), CloseAction::Nothing);
        v.explicit_save = true;
        assert_eq!(v.close_action(true), CloseAction::AskSaveLocked);
    }

    #[test]
    fn links_name_a_file() {
        let mut v = ViewSession::unnamed(0.0);
        assert_eq!(v.link_text(None), None);
        let id = AutoId::parse("20260101-000000-1").unwrap();
        assert_eq!(v.link_text(Some(&id)).unwrap(), format!("_autosaved/{id}"));
        v.origin = ViewOrigin::Named(SessionRef::parse("a/b").unwrap());
        assert_eq!(v.link_text(Some(&id)).unwrap(), "a/b");
    }

    #[test]
    fn a_locked_auto_saved_file_makes_a_dirty_view_ask() {
        let mut v = ViewSession::unnamed(0.0);
        v.slot_locked = true;
        assert_eq!(
            v.close_action(true),
            CloseAction::AutoSave,
            "nothing to lose"
        );
        v.dirty = true;
        assert_eq!(v.close_action(true), CloseAction::AskSaveChanges);
        assert_eq!(v.close_action(false), CloseAction::Nothing);
    }

    #[test]
    fn identity_and_autosave_comparisons() {
        let mut file = Session::builtin(SessionType::TextCompare);
        let seen = file.clone();
        file.locked = true;
        file.favorite = true;
        assert!(same_session(&seen, &file), "a lock is not another session");
        assert!(same_autosaved(&seen, &file));
        file.description = "another".into();
        assert!(!same_session(&seen, &file));
        assert!(!same_autosaved(&seen, &file));
    }

    #[test]
    fn a_save_keeps_the_identity_of_the_file() {
        let mut file = Session::builtin(SessionType::TextCompare);
        file.locked = true;
        file.favorite = true;
        let mut current = Session::builtin(SessionType::TextCompare);
        current.description = "mine".into();
        let merged = merged_with_file(current, &file);
        assert!(merged.locked && merged.favorite);
        assert_eq!(merged.description, "mine");
    }
}
