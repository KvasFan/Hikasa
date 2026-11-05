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
use std::sync::Arc;

use hikasa_core::{JobKind, Side};
use hikasa_folder::ops::{
    Confirmation, FolderOp, OpPlan, OpVerb, PathStructure, QuestionKind, ReadOnlySides, Recycle,
    RenameMask, Reply, TouchFrom,
};
use hikasa_folder::select::{Selection, SideSet};
use hikasa_folder::{DisplayOptions, FolderModel};
use hikasa_session::api::human::TimeZone;
use hikasa_session::api::{
    ExpandWords, FolderOpRequest, OpOverrides, OpRequest, SelectRequest, parse_mkdir_name,
    parse_touch_time,
};
use hikasa_session::ipc::ViewId;
use hikasa_session::{ApiError, Service};
use hikasa_vfs::ops::OverwritePolicy;

use crate::dialog::{ButtonRole, DialogButton, DialogModel, DialogStep, Followup};
use crate::job::Job;
use crate::op_preview::PreviewList;

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
            OpVerb::Rename | OpVerb::Touch | OpVerb::Attrib | OpVerb::Mkdir | OpVerb::Sync => {
                return None;
            }
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
    /// `None`: a script's question, whose operation the dialog does not know (no "Do not ask again").
    verb: Option<OpVerb>,
    /// The status window whose run asked (a script's question): its answers go there and nowhere else.
    script: Option<u64>,
    /// `None` once taken.
    confirmation: Option<Confirmation>,
    counts: Vec<String>,
    /// The plan and its preview (a folder view's operation, M2 W9b): the list the dialog shows, handed back unchanged
    /// with the answers (D-244).
    list: Option<Box<PreviewList>>,
    /// *Include hidden items* (a verb without options, §1.9.12): `None` for a script's question.
    include_hidden: Option<bool>,
    /// The check box was toggled: the view plans again with this value (the dialog closes, nothing runs).
    replan: Option<bool>,
    /// The G15 preview bench scrolls the list here (points).
    pub bench_scroll: Option<f32>,
    /// "Do not ask again" is ticked (the current question's key, when it has one).
    pub ticked: bool,
    /// The keys to set to `false` once the questions are answered.
    keys: Vec<&'static str>,
    /// A Cancel, or a No to the operation.
    cancelled: bool,
}

impl PlanConfirmDialog {
    /// The dialog of `confirmation`, which has a question; `title` is the command's caption, `subject` the row and the
    /// sides it acts on; `list` the plan and its preview (the dialog's body, M2 W9b), whose counts it shows;
    /// `include_hidden` the check box of a verb without options (`None`: not offered).
    pub fn new(
        view: ViewId,
        title: impl Into<String>,
        subject: impl Into<String>,
        list: PreviewList,
        confirmation: Confirmation,
        include_hidden: Option<bool>,
    ) -> PlanConfirmDialog {
        PlanConfirmDialog {
            view,
            title: title.into(),
            subject: subject.into(),
            verb: list.op_plan().map(|p| p.verb),
            script: None,
            confirmation: Some(confirmation),
            counts: list.counts(),
            list: Some(Box::new(list)),
            include_hidden,
            replan: None,
            bench_scroll: None,
            ticked: false,
            keys: Vec::new(),
            cancelled: false,
        }
    }

    /// The preview list (`None`: a script's question).
    pub fn list(&self) -> Option<&PreviewList> {
        self.list.as_deref()
    }

    /// *Include hidden items*, when the dialog offers it.
    pub fn include_hidden(&self) -> Option<bool> {
        self.include_hidden
    }

    /// Toggles *Include hidden items*: the dialog closes and its view plans again (§1.9.12: toggling re-plans).
    pub fn set_include_hidden(&mut self, on: bool) {
        if self.include_hidden.is_some_and(|was| was != on) {
            self.include_hidden = Some(on);
            self.replan = Some(on);
        }
    }

    /// The list scrolled to this offset (the G15 preview bench, D-257).
    pub fn bench_scroll(&self) -> Option<f32> {
        self.bench_scroll
    }

    /// The value to plan again with, once the check box was toggled.
    pub fn replan(&self) -> Option<bool> {
        self.replan
    }

    /// The answers and the plan they answer, once the dialog is done with them (`None` while a question is open, or
    /// after a toggle that plans again): the plan is the previewed one, unchanged.
    pub fn take_answered(&mut self) -> Option<(Confirmation, PreviewList)> {
        if self.current().is_some() || self.replan.is_some() {
            return None;
        }
        let confirmation = self.confirmation.take()?;
        let list = self.list.take()?;
        Some((confirmation, *list))
    }

    /// The questions of a script's command (M2 W8b, §1.8.14): stacked on the status window, its answers go back to the
    /// run of the status window `script` (`for_script`); no counts and no "Do not ask again" (a script line means the
    /// same whatever a dialog did).
    pub fn for_script(
        script: u64,
        title: impl Into<String>,
        subject: impl Into<String>,
        confirmation: Confirmation,
    ) -> PlanConfirmDialog {
        PlanConfirmDialog {
            view: ViewId(0),
            title: title.into(),
            subject: subject.into(),
            verb: None,
            script: Some(script),
            confirmation: Some(confirmation),
            counts: Vec::new(),
            list: None,
            include_hidden: None,
            replan: None,
            bench_scroll: None,
            ticked: false,
            keys: Vec::new(),
            cancelled: false,
        }
    }

    /// True for the questions of a script (their answers go to the status window).
    pub fn for_a_script(&self) -> bool {
        self.script.is_some()
    }

    /// The status window whose run asked ([`ScriptDialog::id`](crate::script_dialog::ScriptDialog::id)).
    pub fn script(&self) -> Option<u64> {
        self.script
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
        let verb = self.verb?;
        self.kind().and_then(|kind| option_key(kind, verb))
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

    fn size(&self) -> Option<(f32, f32)> {
        self.list.as_ref().map(|_| (620.0, 360.0))
    }

    fn take_requests(&mut self) -> Vec<crate::dialog::DialogRequest> {
        // A toggle of *Include hidden items* closes the dialog: its view plans again (once).
        match self.replan {
            Some(_) if !self.cancelled => {
                self.cancelled = true;
                vec![crate::dialog::DialogRequest::Close]
            }
            _ => Vec::new(),
        }
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

// ---- M2 W9b: the verb dialogs over a core preview (D-244, D-245) ---------------------------------------------------

/// The help topics of the verb dialogs.
pub const COPY_TO_TOPIC: &str = "dialog-copy-to-folder";
/// Rename.
pub const RENAME_TOPIC: &str = "dialog-rename";
/// Touch.
pub const TOUCH_TOPIC: &str = "dialog-touch";
/// New Folder.
pub const NEW_FOLDER_TOPIC: &str = "dialog-new-folder";

/// The role of *Plan again* (a plan whose comparison changed, §1.9.12).
pub const PLAN_AGAIN: ButtonRole = ButtonRole::Other(3);

/// What a verb dialog plans on: the view's comparison when it opened, its selection and what the view may write.
#[derive(Clone)]
pub struct OpContext {
    /// The view.
    pub view: ViewId,
    /// The command (*Plan again* runs it once more).
    pub command: crate::commands::CommandId,
    /// The command's caption (the title).
    pub caption: String,
    /// The comparison the plan reads.
    pub model: Arc<FolderModel>,
    /// Its generation when the dialog opened: a plan runs only on it unchanged.
    pub generation: u64,
    /// The selection (D-238; never the root).
    pub selection: Selection,
    /// The view's display (a plan acts on what is shown).
    pub display: DisplayOptions,
    /// The read-only sides (FC-003): their choices are disabled, and the Service refuses them again (D-184).
    pub read_only: ReadOnlySides,
    /// `--nobackups` and the like.
    pub overrides: OpOverrides,
    /// The selection has items on the left, on the right.
    pub on: [bool; 2],
    /// The zone of the Service's clock (Touch's local time).
    pub zone: TimeZone,
    /// `windows` rules for names (New Folder).
    pub windows: bool,
    /// How Copy/Move to Folder reads its target: relative to the source side's base folder (the focus side's, else
    /// the side that has the items), `~` the home folder (review F13); `None` when that base is not a local folder.
    pub target_context: Option<hikasa_core::SpecContext>,
    /// The path style of the platform (a target typed absolute).
    pub style: hikasa_core::PathStyle,
}

impl std::fmt::Debug for OpContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpContext")
            .field("view", &self.view)
            .field("caption", &self.caption)
            .field("rows", &self.selection.len())
            .finish()
    }
}

/// How Rename makes the names of several rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenameMode {
    /// *DOS compatible* (a mask).
    Dos,
    /// *Regular expressions* (old, new).
    Regex,
}

/// The options of a verb dialog (§1.9.13's table).
#[derive(Debug, Clone, PartialEq)]
pub enum OpForm {
    /// *Copy to Folder* / *Move to Folder*.
    CopyTo {
        /// Move to Folder.
        moving: bool,
        /// The target folder as typed (an archive or a missing path is created, D-150).
        target: String,
        /// *Keep relative folder structure*, *Keep base folder structure*, *Don't keep folder structure*.
        structure: PathStructure,
        /// The sides whose items go.
        sides: SideChoice,
        /// What an existing target means.
        overwrite: OverwritePolicy,
    },
    /// *Rename* of several rows.
    Rename {
        /// DOS mask or regular expressions.
        mode: RenameMode,
        /// The mask (prefilled with `base_mask` of the names).
        dos: String,
        /// The expression.
        old: String,
        /// What replaces it.
        new: String,
        /// The sides.
        sides: SideChoice,
    },
    /// *Touch*.
    Touch {
        /// *Copy timestamps from other side* (else *Set to*).
        from_other: bool,
        /// The side the other side's times go to.
        to: Side,
        /// The date and time as typed (`YYYY-MM-DD HH:MM:SS`, the local zone).
        text: String,
        /// The sides *Set to* touches.
        sides: SideChoice,
    },
    /// *New Folder*.
    NewFolder {
        /// The name.
        name: String,
        /// The sides.
        sides: SideChoice,
        /// The row it goes into, by its path (`""`: the base).
        parent: String,
    },
}

/// The plan of a verb dialog.
enum PlanState {
    /// Nothing planned yet (the next poll plans).
    Idle,
    /// A plan job and its generation (a newer one cancels it, its answer is dropped).
    Planning(
        Job<(OpPlan, hikasa_session::api::OpPreview, Confirmation)>,
        u64,
    ),
    /// The plan, its preview and its questions.
    Ready(Box<PreviewList>, Box<Confirmation>),
    /// The options do not make a plan: the message replaces the preview, Start is off.
    Failed(String),
}

/// A verb dialog (D-245): the options of [`OpForm`], *Include hidden items*, and the preview of the plan they make,
/// planned again on every change; Start hands the previewed plan and its questions to the view unchanged.
pub struct OpDialog {
    ctx: OpContext,
    form: OpForm,
    include_hidden: bool,
    plan: PlanState,
    /// The options changed since the last plan job.
    dirty: bool,
    generation: u64,
    /// The comparison changed under the dialog: Start is replaced by *Plan again*.
    stale: bool,
    started: Option<(PreviewList, Confirmation)>,
    again: bool,
    wake: Arc<dyn Fn() + Send + Sync>,
    /// The recent folders (`state.mru.folders`), offered for Copy/Move to Folder's target.
    pub recent: Vec<String>,
    /// How many plan jobs started (tests: an option change re-plans, the older job is cancelled).
    pub plans: u64,
}

impl std::fmt::Debug for OpDialog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpDialog")
            .field("ctx", &self.ctx)
            .field("form", &self.form)
            .finish()
    }
}

impl OpDialog {
    /// The dialog of `form` on `ctx`; `include_hidden` is the check box's preset (`include_hidden_by_default`).
    pub fn new(
        ctx: OpContext,
        form: OpForm,
        include_hidden: bool,
        wake: Arc<dyn Fn() + Send + Sync>,
    ) -> OpDialog {
        OpDialog {
            ctx,
            form,
            include_hidden,
            plan: PlanState::Idle,
            dirty: true,
            generation: 0,
            stale: false,
            started: None,
            again: false,
            wake,
            recent: Vec::new(),
            plans: 0,
        }
    }

    /// The target folder of Copy/Move to Folder as it is read (`None` for the other verbs and an empty field): an
    /// absolute spec, a relative text taken in the source side's base folder (review F13). The dialog shows it before
    /// Start; it joins `mru.folders` once started.
    pub fn resolved_target(&self) -> Option<Result<hikasa_core::Spec, String>> {
        let OpForm::CopyTo { target, .. } = &self.form else {
            return None;
        };
        let text = target.trim();
        if text.is_empty() {
            return None;
        }
        let absolute = hikasa_core::SpecContext::absolute_only(self.ctx.style);
        let typed = std::ffi::OsStr::new(text);
        Some(match hikasa_core::Spec::parse_os(typed, &absolute) {
            Ok(spec) => Ok(spec),
            Err(_) => match &self.ctx.target_context {
                Some(context) => {
                    hikasa_core::Spec::parse_os(typed, context).map_err(|e| e.to_string())
                }
                None => Err(
                    "Type an absolute folder: the source side's base is not a local folder."
                        .to_owned(),
                ),
            },
        })
    }

    /// The target folder of Copy/Move to Folder, resolved (`None` for the other verbs): it joins `mru.folders` once
    /// started.
    pub fn target(&self) -> Option<String> {
        match self.resolved_target()? {
            Ok(spec) => Some(spec.canonical()),
            Err(_) => None,
        }
    }

    /// The view.
    pub fn view(&self) -> ViewId {
        self.ctx.view
    }

    /// The context it plans on.
    pub fn context(&self) -> &OpContext {
        &self.ctx
    }

    /// The options.
    pub fn form(&self) -> &OpForm {
        &self.form
    }

    /// Changes the options; the next poll plans again (the running job is cancelled).
    pub fn edit(&mut self, change: impl FnOnce(&mut OpForm)) {
        let before = self.form.clone();
        change(&mut self.form);
        if self.form != before {
            self.dirty = true;
        }
    }

    /// *Include hidden items*.
    pub fn include_hidden(&self) -> bool {
        self.include_hidden
    }

    /// Toggles *Include hidden items* (re-plans).
    pub fn set_include_hidden(&mut self, on: bool) {
        if self.include_hidden != on {
            self.include_hidden = on;
            self.dirty = true;
        }
    }

    /// True when `side` may be written (Disable editing, FC-003).
    pub fn writable(&self, side: Side) -> bool {
        !self.ctx.read_only.get(side)
    }

    /// True when the side choice can be picked: the selection has items there and, for a verb that writes the
    /// selected items (Rename, Touch, New Folder, Move to Folder), the side is writable.
    pub fn side_enabled(&self, choice: SideChoice) -> bool {
        let writes_items = !matches!(self.form, OpForm::CopyTo { moving: false, .. });
        let new_folder = matches!(self.form, OpForm::NewFolder { .. });
        let ok = |k: usize, side: Side| {
            (new_folder || self.ctx.on[k]) && (!writes_items || self.writable(side))
        };
        match choice {
            SideChoice::Left => ok(0, Side::Left),
            SideChoice::Right => ok(1, Side::Right),
            SideChoice::Both => ok(0, Side::Left) && ok(1, Side::Right),
        }
    }

    /// The tooltip of a disabled side choice that would write a read-only side.
    pub fn read_only_tip(side: Side) -> String {
        format!("Disable editing is on for the {} side", side.as_str())
    }

    /// The preview list, once planned.
    pub fn list(&self) -> Option<&PreviewList> {
        match &self.plan {
            PlanState::Ready(list, _) => Some(list),
            _ => None,
        }
    }

    /// "Planning…" while the plan job runs.
    pub fn planning(&self) -> bool {
        self.dirty || matches!(self.plan, PlanState::Planning(..) | PlanState::Idle)
    }

    /// The message that replaces the preview (a refused plan, an invalid option, a changed comparison).
    pub fn message(&self) -> Option<String> {
        if self.stale {
            return Some(format!(
                "{}: the comparison changed while the operation waited; nothing was written. Plan again.",
                self.ctx.caption
            ));
        }
        match &self.plan {
            PlanState::Failed(message) => Some(message.clone()),
            _ => None,
        }
    }

    /// The plan, its preview and its questions after Start (taken by the view).
    pub fn take_started(&mut self) -> Option<(PreviewList, Confirmation)> {
        self.started.take()
    }

    /// True when *Plan again* was pressed (the window runs the command again).
    pub fn plan_again(&self) -> bool {
        self.again
    }

    /// The browse dialog's answer for the target (Copy/Move to Folder).
    pub fn set_target(&mut self, spec: &str) {
        self.edit(|form| {
            if let OpForm::CopyTo { target, .. } = form {
                *target = spec.to_owned();
            }
        });
    }

    /// The choices to remember in `state.dialogs."<id>"` (§1.9.13's last column): `(dialog, key, value)`.
    pub fn remembered(&self) -> Vec<(&'static str, &'static str, String)> {
        match &self.form {
            OpForm::CopyTo {
                structure,
                overwrite,
                ..
            } => vec![
                (
                    COPY_TO_DIALOG,
                    structure_key(),
                    structure.as_str().to_owned(),
                ),
                (
                    COPY_TO_DIALOG,
                    overwrite_key(),
                    overwrite.as_str().to_owned(),
                ),
            ],
            OpForm::Rename {
                mode,
                dos,
                old,
                new,
                ..
            } => vec![
                (RENAME_DIALOG, "mode", mode_word(*mode).to_owned()),
                (RENAME_DIALOG, "dos", dos.clone()),
                (RENAME_DIALOG, "old", old.clone()),
                (RENAME_DIALOG, "new", new.clone()),
            ],
            OpForm::Touch {
                from_other, text, ..
            } => {
                let mut out = vec![(
                    TOUCH_DIALOG,
                    "mode",
                    if *from_other { OTHER_SIDE } else { SET_TO }.to_owned(),
                )];
                if !*from_other {
                    out.push((TOUCH_DIALOG, "last", text.clone()));
                }
                out
            }
            OpForm::NewFolder { sides, .. } => {
                vec![(NEW_FOLDER_DIALOG, "sides", side_word(*sides).to_owned())]
            }
        }
    }

    /// The request of the options, or the message of an option that makes none.
    fn request(&self) -> Result<FolderOpRequest, String> {
        let op = match &self.form {
            OpForm::CopyTo {
                moving,
                target,
                structure,
                sides,
                overwrite,
            } => {
                if target.trim().is_empty() {
                    return Err("Choose the folder the items go to.".to_owned());
                }
                // A relative text is the source side's base folder's, never the program's current folder (F13).
                let target = match self.resolved_target() {
                    Some(Ok(spec)) => hikasa_session::api::spec_os_string(&spec),
                    Some(Err(message)) => return Err(message),
                    None => return Err("Choose the folder the items go to.".to_owned()),
                };
                if *moving {
                    OpRequest::MoveTo {
                        sides: sides.sides(),
                        target,
                        structure: *structure,
                        overwrite: *overwrite,
                    }
                } else {
                    OpRequest::CopyTo {
                        sides: sides.sides(),
                        target,
                        structure: *structure,
                        overwrite: *overwrite,
                    }
                }
            }
            OpForm::Rename {
                mode,
                dos,
                old,
                new,
                sides,
            } => {
                let mask = match mode {
                    RenameMode::Dos if dos.is_empty() => {
                        return Err("Type the mask of the new names.".to_owned());
                    }
                    RenameMode::Dos => RenameMask::Dos(dos.clone()),
                    RenameMode::Regex => RenameMask::Regex {
                        old: old.clone(),
                        new: new.clone(),
                    },
                };
                OpRequest::Op(FolderOp::Rename {
                    sides: sides.sides(),
                    mask,
                })
            }
            OpForm::Touch {
                from_other,
                to,
                text,
                sides,
            } => {
                if *from_other {
                    OpRequest::Op(FolderOp::Touch {
                        sides: SideSet::of(*to),
                        from: TouchFrom::OtherSide { to: *to },
                    })
                } else {
                    let at = parse_touch_time(text, &self.ctx.zone).map_err(|e| e.to_string())?;
                    OpRequest::Op(FolderOp::Touch {
                        sides: sides.sides(),
                        from: TouchFrom::At(at),
                    })
                }
            }
            OpForm::NewFolder {
                name,
                sides,
                parent,
            } => {
                let (name, folder) =
                    parse_mkdir_name(name, self.ctx.windows).map_err(|e| e.to_string())?;
                OpRequest::MkdirIn {
                    sides: sides.sides(),
                    parent: parent.clone(),
                    name,
                    folder,
                }
            }
        };
        let mut select = select_request_of(&self.ctx, self.include_hidden);
        if matches!(self.form, OpForm::NewFolder { .. }) {
            // New Folder plans no selected item: its parent is named by path.
            select.selection = None;
        }
        Ok(FolderOpRequest {
            op,
            select,
            read_only: self.ctx.read_only,
            overrides: self.ctx.overrides,
        })
    }

    /// Starts a plan job for the options (the running one is cancelled).
    fn replan(&mut self, svc: &Service) {
        self.dirty = false;
        if let PlanState::Planning(job, _) = &self.plan {
            job.cancel();
        }
        let request = match self.request() {
            Ok(request) => request,
            Err(message) => {
                self.plan = PlanState::Failed(message);
                return;
            }
        };
        self.generation += 1;
        self.plans += 1;
        let model = Arc::clone(&self.ctx.model);
        let job = Job::spawn(
            svc,
            JobKind::FolderOp,
            format!("plan: {}", self.ctx.caption),
            &self.wake,
            move |svc, job| {
                let plan = svc.folder_op_plan(&model, &request, job)?;
                if job.cancel.is_cancelled() {
                    return Err(ApiError::Cancelled);
                }
                let preview = svc.folder_op_preview(&plan);
                let confirmation = svc.folder_confirmation(&plan, None);
                Ok((plan, preview, confirmation))
            },
        );
        self.plan = PlanState::Planning(job, self.generation);
    }

    fn start_enabled(&self) -> bool {
        !self.stale && !self.dirty && self.list().is_some_and(PreviewList::acts)
    }
}

/// The keys of Copy/Move to Folder's remembered choices (`state.dialogs."copy-to-folder"`).
pub fn structure_key() -> &'static str {
    "structure"
}

/// The key of the remembered overwrite choice.
pub fn overwrite_key() -> &'static str {
    "overwrite"
}

/// The `state.dialogs` ids of the verb dialogs (§1.9.13).
pub const COPY_TO_DIALOG: &str = "copy-to-folder";
/// Rename.
pub const RENAME_DIALOG: &str = "rename";
/// Touch.
pub const TOUCH_DIALOG: &str = "touch";
/// New Folder.
pub const NEW_FOLDER_DIALOG: &str = "new-folder";
/// Touch's remembered mode words.
const OTHER_SIDE: &str = "other-side";
const SET_TO: &str = "set-to";

fn mode_word(mode: RenameMode) -> &'static str {
    match mode {
        RenameMode::Dos => "mask",
        RenameMode::Regex => "regex",
    }
}

/// The remembered Rename mode (the dialog's own word, not a CLI word).
pub fn rename_mode_of(word: Option<&str>) -> RenameMode {
    if word == Some(mode_word(RenameMode::Regex)) {
        RenameMode::Regex
    } else {
        RenameMode::Dos
    }
}

/// The remembered Touch mode: true for *Copy timestamps from other side*.
pub fn touch_from_other_of(word: Option<&str>) -> bool {
    word == Some(OTHER_SIDE)
}

fn side_word(choice: SideChoice) -> &'static str {
    match choice {
        SideChoice::Left => Side::Left.as_str(),
        SideChoice::Right => Side::Right.as_str(),
        SideChoice::Both => "both",
    }
}

/// The remembered sides of New Folder.
pub fn side_choice_of(word: Option<&str>) -> Option<SideChoice> {
    SideChoice::ALL
        .into_iter()
        .find(|c| Some(side_word(*c)) == word)
}

/// The request of a plan on the selection of `ctx` (D-238): the selection explicit, the view's display, the dialog's
/// *Include hidden items*.
pub fn select_request_of(ctx: &OpContext, include_hidden: bool) -> SelectRequest {
    SelectRequest {
        selection: Some(ctx.selection.clone()),
        display: Some(ctx.display),
        expand: ExpandWords::None,
        include_hidden: Some(include_hidden),
        iu: ctx.display.ignore_unimportant,
        ..SelectRequest::default()
    }
}

impl DialogModel for OpDialog {
    fn title(&self) -> String {
        self.ctx
            .caption
            .trim_end_matches("...")
            .trim_end_matches('…')
            .to_owned()
    }

    fn help_topic(&self) -> &'static str {
        match self.form {
            OpForm::CopyTo { .. } => COPY_TO_TOPIC,
            OpForm::Rename { .. } => RENAME_TOPIC,
            OpForm::Touch { .. } => TOUCH_TOPIC,
            OpForm::NewFolder { .. } => NEW_FOLDER_TOPIC,
        }
    }

    fn size(&self) -> Option<(f32, f32)> {
        Some((640.0, 420.0))
    }

    fn buttons(&self) -> Vec<DialogButton> {
        let mut buttons = Vec::new();
        if self.stale {
            buttons.push(DialogButton {
                label: "Plan again".to_owned(),
                role: PLAN_AGAIN,
                enabled: true,
                default: true,
            });
        } else {
            buttons.push(DialogButton {
                label: "Start".to_owned(),
                role: ButtonRole::Ok,
                enabled: self.start_enabled(),
                default: true,
            });
        }
        buttons.push(DialogButton {
            label: "Cancel".to_owned(),
            role: ButtonRole::Cancel,
            enabled: true,
            default: false,
        });
        buttons
    }

    fn on_button(&mut self, role: ButtonRole, _svc: &Service) -> DialogStep {
        match role {
            ButtonRole::Ok => {
                if self.ctx.model.generation() != self.ctx.generation {
                    self.stale = true;
                    return DialogStep::Stay;
                }
                if !self.start_enabled() {
                    return DialogStep::Stay;
                }
                match std::mem::replace(&mut self.plan, PlanState::Idle) {
                    PlanState::Ready(list, confirmation) => {
                        self.started = Some((*list, *confirmation));
                        DialogStep::Close
                    }
                    other => {
                        self.plan = other;
                        DialogStep::Stay
                    }
                }
            }
            PLAN_AGAIN => {
                self.again = true;
                DialogStep::Close
            }
            ButtonRole::Cancel => {
                if let PlanState::Planning(job, _) = &self.plan {
                    job.cancel();
                }
                DialogStep::Close
            }
            _ => DialogStep::Stay,
        }
    }

    fn poll(&mut self, svc: &Service) -> bool {
        if self.stale {
            return false;
        }
        if self.ctx.model.generation() != self.ctx.generation {
            self.stale = true;
            return true;
        }
        if self.dirty {
            self.replan(svc);
            return true;
        }
        let PlanState::Planning(job, generation) = &self.plan else {
            return false;
        };
        let Some(result) = job.try_result() else {
            return false;
        };
        if *generation != self.generation {
            return false;
        }
        self.plan = match result {
            Ok((plan, preview, confirmation)) => PlanState::Ready(
                Box::new(PreviewList::op(plan, preview)),
                Box::new(confirmation),
            ),
            Err(ApiError::Cancelled) => PlanState::Idle,
            Err(e) => PlanState::Failed(e.to_string()),
        };
        true
    }

    fn busy(&self) -> bool {
        matches!(self.plan, PlanState::Planning(..))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
