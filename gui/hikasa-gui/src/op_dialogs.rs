//! The dialogs of folder operations (M2 W6d, §1.6.20, D-189): [`PlanConfirmDialog`] asks the questions of a plan's
//! [`Confirmation`] one at a time, [`DeleteDialog`] picks the sides and the recycle bin of a delete. Both are toolkit-free
//! models (§1.4.16 rule 4, architecture check 7): the view keeps the plan and runs the jobs (D-105), a dialog only holds
//! the answers, which the window hands back to the view when the dialog leaves its stack.
//!
//! *W6d notes:* "Do not ask again" counts with Yes and Yes to All only, and is written once the questions are all
//! answered and the operation was not cancelled: the key's `false` means "proceed without asking", which a No does not
//! say. It takes effect from the next operation; the questions of this plan stay as they were asked. A dialog that
//! leaves with a question still open (the window's close button) is a Cancel: nothing runs.

use std::any::Any;

use hikasa_core::Side;
use hikasa_folder::ops::{Confirmation, OpPlan, OpVerb, QuestionKind, Recycle, Reply};
use hikasa_folder::select::SideSet;
use hikasa_session::Service;
use hikasa_session::ipc::ViewId;
use hikasa_vfs::ops::Action;

use crate::dialog::{ButtonRole, DialogButton, DialogModel, DialogStep, Followup};

/// The role of *Yes to All*.
pub const YES_TO_ALL: ButtonRole = ButtonRole::Other(1);
/// The role of *No to All*.
pub const NO_TO_ALL: ButtonRole = ButtonRole::Other(2);

/// The help topic of the confirmation dialog.
pub const CONFIRM_TOPIC: &str = "dialog-confirm";
/// The help topic of the Delete dialog.
pub const DELETE_TOPIC: &str = "dialog-delete";

/// The `options.toml` key (below `options`) that drops the question of `kind` in an operation of `verb`, the key
/// "Do not ask again" sets to `false`; `None` where no key may answer: the recycle bin is always asked, and a sync's
/// question follows `sync_confirmations`, which is no yes-or-no key (§1.6.20).
pub fn option_key(kind: QuestionKind, verb: OpVerb) -> Option<&'static str> {
    Some(match kind {
        QuestionKind::Operation => match verb {
            OpVerb::Copy | OpVerb::CopyTo => "file_operations.confirm_copy",
            OpVerb::Move | OpVerb::Exchange | OpVerb::MoveTo => "file_operations.confirm_move",
            OpVerb::Delete => "file_operations.confirm_delete",
            OpVerb::Rename | OpVerb::Touch | OpVerb::Mkdir | OpVerb::Sync => return None,
        },
        QuestionKind::ReadOnly => "file_operations.confirm_read_only",
        QuestionKind::System => "file_operations.confirm_system",
        QuestionKind::OverwriteNewer | QuestionKind::OverwriteBackup => {
            "file_operations.confirm_overwrite_newer"
        }
        QuestionKind::ReplaceDuringMove => "file_operations.confirm_replace_during_move",
        QuestionKind::NoRecycleBin => return None,
    })
}

/// What a plan will do, one line per action, then what it leaves: the counts the dialog shows under its question.
pub fn plan_counts(plan: &OpPlan) -> Vec<String> {
    let mut per_action: Vec<(&'static str, u64, u64)> = Vec::new();
    let (mut skipped, mut failed) = (0u64, 0u64);
    for item in &plan.plan.items {
        if item.error.is_some() {
            failed += 1;
        } else if item.skip.is_some() || item.action == Action::Skip {
            skipped += 1;
        } else {
            let word = item.action.as_str();
            match per_action.iter_mut().find(|(w, ..)| *w == word) {
                Some(row) => {
                    row.1 += 1;
                    row.2 += item.size.unwrap_or(0);
                }
                None => per_action.push((word, 1, item.size.unwrap_or(0))),
            }
        }
    }
    let mut lines: Vec<String> = per_action
        .iter()
        .map(|(word, items, bytes)| {
            if *bytes > 0 {
                format!("{word}: {items} items, {bytes} bytes")
            } else {
                format!("{word}: {items} items")
            }
        })
        .collect();
    if skipped > 0 {
        lines.push(format!("left as they are: {skipped} items"));
    }
    if failed > 0 {
        lines.push(format!("cannot run: {failed} items"));
    }
    lines
}

/// The questions of one plan (§1.6.11): the row and the sides it acts on, the text of each
/// [`hikasa_folder::ops::Question`], the plan's counts, and Yes, No, Yes to All, No to All, Cancel. Esc is Cancel; Enter
/// is Yes for the operation question and No for an item's.
#[derive(Debug)]
pub struct PlanConfirmDialog {
    /// The view that asked.
    pub view: ViewId,
    title: String,
    /// The row and its sides (`a/b.txt: from the left to the right`).
    subject: String,
    verb: OpVerb,
    /// `None` once taken.
    confirmation: Option<Confirmation>,
    counts: Vec<String>,
    /// "Do not ask again" is ticked (the current question's key, when it has one).
    pub ticked: bool,
    /// The keys to set to `false` once the questions are answered.
    keys: Vec<&'static str>,
    /// A Cancel, or a No to the operation.
    cancelled: bool,
}

impl PlanConfirmDialog {
    /// The dialog of `confirmation`, which has a question; `title` is the command's caption, `subject` the row and the
    /// sides it acts on.
    pub fn new(
        view: ViewId,
        title: impl Into<String>,
        subject: impl Into<String>,
        plan: &OpPlan,
        confirmation: Confirmation,
    ) -> PlanConfirmDialog {
        PlanConfirmDialog {
            view,
            title: title.into(),
            subject: subject.into(),
            verb: plan.verb,
            confirmation: Some(confirmation),
            counts: plan_counts(plan),
            ticked: false,
            keys: Vec::new(),
            cancelled: false,
        }
    }

    fn current(&self) -> Option<&hikasa_folder::ops::Question> {
        self.confirmation.as_ref().and_then(Confirmation::next)
    }

    /// The row and the sides the operation acts on.
    pub fn subject(&self) -> &str {
        &self.subject
    }

    /// The question asked now (`None`: every question is answered).
    pub fn question(&self) -> Option<&str> {
        self.current().map(|q| q.text.as_str())
    }

    /// The kind of the question asked now.
    pub fn kind(&self) -> Option<QuestionKind> {
        self.current().map(|q| q.kind)
    }

    /// The questions left, this one included.
    pub fn remaining(&self) -> u64 {
        self.current().map_or(0, |q| q.remaining)
    }

    /// The plan's counts.
    pub fn counts(&self) -> &[String] {
        &self.counts
    }

    /// The key "Do not ask again" sets for the question asked now (`None`: the check box is not shown).
    pub fn do_not_ask_key(&self) -> Option<&'static str> {
        self.kind().and_then(|kind| option_key(kind, self.verb))
    }

    /// The answers, once the dialog is done with them: `None` while a question is open (a dialog closed then is a
    /// Cancel). A declined operation comes back too, and `Confirmation::finish` says so.
    pub fn take_confirmation(&mut self) -> Option<Confirmation> {
        if self.current().is_some() {
            return None;
        }
        self.confirmation.take()
    }

    /// The reply of a button.
    fn reply_of(role: ButtonRole) -> Option<Reply> {
        Some(match role {
            ButtonRole::Yes => Reply::Yes,
            ButtonRole::No => Reply::No,
            YES_TO_ALL => Reply::YesToAll,
            NO_TO_ALL => Reply::NoToAll,
            ButtonRole::Cancel => Reply::Cancel,
            _ => return None,
        })
    }
}

impl DialogModel for PlanConfirmDialog {
    fn title(&self) -> String {
        self.title.clone()
    }

    fn help_topic(&self) -> &'static str {
        CONFIRM_TOPIC
    }

    fn buttons(&self) -> Vec<DialogButton> {
        let operation = self.kind() == Some(QuestionKind::Operation);
        [
            (ButtonRole::Yes, "Yes"),
            (ButtonRole::No, "No"),
            (YES_TO_ALL, "Yes to All"),
            (NO_TO_ALL, "No to All"),
            (ButtonRole::Cancel, "Cancel"),
        ]
        .into_iter()
        .map(|(role, label)| DialogButton {
            label: label.to_owned(),
            role,
            enabled: true,
            default: if operation {
                role == ButtonRole::Yes
            } else {
                role == ButtonRole::No
            },
        })
        .collect()
    }

    fn on_button(&mut self, role: ButtonRole, _svc: &Service) -> DialogStep {
        let Some(reply) = PlanConfirmDialog::reply_of(role) else {
            return DialogStep::Stay;
        };
        let key = self.do_not_ask_key();
        let operation = self.kind() == Some(QuestionKind::Operation);
        let Some(confirmation) = self.confirmation.as_mut() else {
            return DialogStep::Close;
        };
        confirmation.reply(reply);
        if reply == Reply::Cancel || (operation && matches!(reply, Reply::No | Reply::NoToAll)) {
            self.cancelled = true;
        }
        if self.ticked
            && matches!(reply, Reply::Yes | Reply::YesToAll)
            && let Some(key) = key
            && !self.keys.contains(&key)
        {
            self.keys.push(key);
        }
        self.ticked = false;
        if self.current().is_some() {
            return DialogStep::Stay;
        }
        if self.cancelled || self.keys.is_empty() {
            return DialogStep::Close;
        }
        DialogStep::CloseWith(
            self.keys
                .iter()
                .map(|key| Followup::DisableOption(key))
                .collect(),
        )
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// The sides a Delete dialog offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SideChoice {
    /// The left side.
    Left,
    /// The right side.
    Right,
    /// Both sides.
    Both,
}

impl SideChoice {
    /// Every choice, in the dialog's order.
    pub const ALL: [SideChoice; 3] = [SideChoice::Left, SideChoice::Right, SideChoice::Both];

    /// The label.
    pub fn label(self) -> &'static str {
        match self {
            SideChoice::Left => "Left",
            SideChoice::Right => "Right",
            SideChoice::Both => "Both",
        }
    }

    /// The sides.
    pub fn sides(self) -> SideSet {
        match self {
            SideChoice::Left => SideSet::LEFT,
            SideChoice::Right => SideSet::RIGHT,
            SideChoice::Both => SideSet::LEFT | SideSet::RIGHT,
        }
    }
}

/// What a Delete dialog chose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeleteChoice {
    /// The sides.
    pub sides: SideSet,
    /// The recycle bin.
    pub recycle: Recycle,
}

/// The Delete dialog (FC-073): Left, Right, Both; *Use recycle bin if possible*; *If the recycle bin fails, delete
/// permanently*; the row's path and what it holds on each side. A side the row has nothing on, or that is read-only, is
/// greyed. A row on one side has that side preset when it is writable; a row on both sides (a pair) never has a preset,
/// whatever is read-only, nor does any row with `require_side_selection_in_delete`: OK stays off until a side is
/// chosen. *Delete Permanently* has the bin off and greyed, and no fallback.
#[derive(Debug)]
pub struct DeleteDialog {
    /// The view that asked.
    pub view: ViewId,
    permanent: bool,
    /// The sides that can be chosen: the row has an item there and it is writable.
    allowed: [bool; 2],
    /// What the row holds, per side (`None`: nothing).
    holds: [Option<String>; 2],
    /// The row's path.
    path: String,
    choice: Option<SideChoice>,
    use_bin: bool,
    fallback: bool,
    chosen: Option<DeleteChoice>,
}

impl DeleteDialog {
    /// The dialog of a row at `path` that holds `holds` per side; `writable` per side; `permanent` for Delete
    /// Permanently; `require_side` is `require_side_selection_in_delete`.
    pub fn new(
        view: ViewId,
        path: impl Into<String>,
        holds: [Option<String>; 2],
        writable: [bool; 2],
        permanent: bool,
        require_side: bool,
    ) -> DeleteDialog {
        let allowed = [
            holds[0].is_some() && writable[0],
            holds[1].is_some() && writable[1],
        ];
        // A preset never surprises: only a row on one side has one, that side, when it is writable and the option does
        // not ask for a choice. A pair is never preset, even when one of its sides is read-only: Enter, Enter must not
        // delete one half of a pair the user did not pick.
        let on = [holds[0].is_some(), holds[1].is_some()];
        let choice = match (require_side, on, allowed) {
            (false, [true, false], [true, _]) => Some(SideChoice::Left),
            (false, [false, true], [_, true]) => Some(SideChoice::Right),
            _ => None,
        };
        DeleteDialog {
            view,
            permanent,
            allowed,
            holds,
            path: path.into(),
            choice,
            use_bin: !permanent,
            fallback: false,
            chosen: None,
        }
    }

    /// True for Delete Permanently.
    pub fn permanent(&self) -> bool {
        self.permanent
    }

    /// The row's path.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// What the row holds on `side` (`None`: nothing).
    pub fn holds(&self, side: Side) -> Option<&str> {
        match side {
            Side::Left => self.holds[0].as_deref(),
            Side::Right => self.holds[1].as_deref(),
            Side::Center | Side::Output => None,
        }
    }

    /// True when `choice` can be picked.
    pub fn enabled(&self, choice: SideChoice) -> bool {
        match choice {
            SideChoice::Left => self.allowed[0],
            SideChoice::Right => self.allowed[1],
            SideChoice::Both => self.allowed[0] && self.allowed[1],
        }
    }

    /// The side picked.
    pub fn choice(&self) -> Option<SideChoice> {
        self.choice
    }

    /// Picks `choice` (a greyed one is ignored).
    pub fn set_choice(&mut self, choice: SideChoice) {
        if self.enabled(choice) {
            self.choice = Some(choice);
        }
    }

    /// *Use recycle bin if possible*.
    pub fn use_bin(&self) -> bool {
        self.use_bin
    }

    /// Sets *Use recycle bin if possible* (greyed in Delete Permanently).
    pub fn set_use_bin(&mut self, on: bool) {
        if !self.permanent {
            self.use_bin = on;
        }
    }

    /// *If the recycle bin fails, delete permanently* (hidden in Delete Permanently, and only with the bin).
    pub fn fallback(&self) -> bool {
        self.fallback
    }

    /// True when the fallback check box is shown.
    pub fn shows_fallback(&self) -> bool {
        !self.permanent
    }

    /// Sets *If the recycle bin fails, delete permanently*.
    pub fn set_fallback(&mut self, on: bool) {
        if !self.permanent {
            self.fallback = on;
        }
    }

    /// The choice once OK was pressed.
    pub fn take_choice(&mut self) -> Option<DeleteChoice> {
        self.chosen.take()
    }

    fn ok_enabled(&self) -> bool {
        self.choice.is_some_and(|c| self.enabled(c))
    }
}

impl DialogModel for DeleteDialog {
    fn title(&self) -> String {
        if self.permanent {
            "Delete Permanently".to_owned()
        } else {
            "Delete".to_owned()
        }
    }

    fn help_topic(&self) -> &'static str {
        DELETE_TOPIC
    }

    fn buttons(&self) -> Vec<DialogButton> {
        vec![
            DialogButton {
                label: "OK".to_owned(),
                role: ButtonRole::Ok,
                enabled: self.ok_enabled(),
                default: true,
            },
            DialogButton {
                label: "Cancel".to_owned(),
                role: ButtonRole::Cancel,
                enabled: true,
                default: false,
            },
        ]
    }

    fn on_button(&mut self, role: ButtonRole, _svc: &Service) -> DialogStep {
        match role {
            ButtonRole::Ok => {
                let Some(choice) = self.choice.filter(|c| self.enabled(*c)) else {
                    return DialogStep::Stay;
                };
                let use_bin = self.use_bin && !self.permanent;
                self.chosen = Some(DeleteChoice {
                    sides: choice.sides(),
                    recycle: Recycle {
                        use_bin,
                        fallback: use_bin && self.fallback,
                    },
                });
                DialogStep::Close
            }
            ButtonRole::Cancel => DialogStep::Close,
            _ => DialogStep::Stay,
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
