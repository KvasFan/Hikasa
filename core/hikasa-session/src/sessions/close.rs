//! Close prompts (D-84, SES-004, SES-005): the pure function [`close_action`] of the table of contract §1.2.4.
//!
//! | origin | locked | dirty | explicit save | action |
//! |---|---|---|---|---|
//! | Unnamed / AutoSaved | — | any | — | `AutoSave` (`Nothing` when auto-save is off) |
//! | Named | no | no | — | `SaveViewState` |
//! | Named | no | yes | — | `AskSaveChanges` |
//! | Named | yes | any | no | `Nothing` |
//! | Named | yes | any | yes | `AskSaveLocked` |

use super::file::Session;

/// Where the view's session came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Origin {
    /// Never saved.
    Unnamed,
    /// An auto-saved session.
    AutoSaved,
    /// A named session.
    Named,
}

/// What closing a view does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CloseAction {
    /// Nothing is written.
    Nothing,
    /// The view is auto-saved (D-83).
    AutoSave,
    /// Only `view`/`display` are written, silently.
    SaveViewState,
    /// "Save changes to session?" (Yes → save, No → nothing, Cancel → stay open).
    AskSaveChanges,
    /// "Save changes to locked session?" (writes only after Yes).
    AskSaveLocked,
}

/// The action of closing a view (D-84). `auto_save` is false when `max_auto_saved_sessions` is 0 (W2 note: the
/// first row of the table depends on it, so it is a parameter).
pub fn close_action(
    origin: Origin,
    locked: bool,
    dirty: bool,
    explicit_save: bool,
    auto_save: bool,
) -> CloseAction {
    match origin {
        Origin::Unnamed | Origin::AutoSaved => {
            if auto_save {
                CloseAction::AutoSave
            } else {
                CloseAction::Nothing
            }
        }
        Origin::Named => match (locked, dirty, explicit_save) {
            (false, false, _) => CloseAction::SaveViewState,
            (false, true, _) => CloseAction::AskSaveChanges,
            (true, _, false) => CloseAction::Nothing,
            (true, _, true) => CloseAction::AskSaveLocked,
        },
    }
}

/// True when the specs or rules of `current` differ from `loaded` (`view` and `display` do not count, D-84).
pub fn is_dirty(loaded: &Session, current: &Session) -> bool {
    loaded.specs != current.specs
        || loaded.rules != current.rules
        || loaded.text_rules != current.text_rules
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sessions::SessionType;

    #[test]
    fn the_d84_table() {
        use CloseAction as A;
        use Origin as O;
        let rows = [
            // origin, locked, dirty, explicit, auto_save → action
            (O::Unnamed, false, false, false, true, A::AutoSave),
            (O::Unnamed, false, true, true, true, A::AutoSave),
            (O::AutoSaved, true, true, false, true, A::AutoSave),
            (O::AutoSaved, false, false, false, true, A::AutoSave),
            (O::Unnamed, false, true, false, false, A::Nothing),
            (O::AutoSaved, false, true, false, false, A::Nothing),
            (O::Named, false, false, false, true, A::SaveViewState),
            (O::Named, false, false, true, true, A::SaveViewState),
            (O::Named, false, true, false, true, A::AskSaveChanges),
            (O::Named, false, true, true, true, A::AskSaveChanges),
            (O::Named, true, false, false, true, A::Nothing),
            (O::Named, true, true, false, true, A::Nothing),
            (O::Named, true, false, true, true, A::AskSaveLocked),
            (O::Named, true, true, true, true, A::AskSaveLocked),
            (O::Named, true, true, true, false, A::AskSaveLocked),
        ];
        for (origin, locked, dirty, explicit, auto, action) in rows {
            assert_eq!(
                close_action(origin, locked, dirty, explicit, auto),
                action,
                "{origin:?} locked={locked} dirty={dirty} explicit={explicit} auto={auto}"
            );
        }
    }

    #[test]
    fn view_state_is_not_dirty() {
        let a = Session::builtin(SessionType::TextCompare);
        let mut b = a.clone();
        b.view.insert("scroll".into(), toml::Value::Integer(3));
        b.display
            .insert("filter".into(), toml::Value::String("x".into()));
        assert!(!is_dirty(&a, &b));
        b.specs.left = "/x".into();
        assert!(is_dirty(&a, &b));
    }
}
