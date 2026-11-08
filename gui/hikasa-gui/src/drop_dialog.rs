//! The prompt of paths dropped on a folder view's pane or pasted into it (M2 W9c, FC-080, D-251, D-252): "Copy (or
//! move) `<n>` items into `<folder>`?" with *Copy*, *Move*, *Cancel* and an overwrite choice (*Fail*, *Skip*,
//! *Replace if newer*, *Replace*: `--overwrite`'s words). A toolkit-free model (check 7): one job asks core for the plan of a copy and of a move into the folder
//! (`vfs_op_plan`), the preview shows core's lines of either (`vfs_op_lines`), and the button pressed hands its plan,
//! unchanged, to the view, which runs it (`vfs_op_execute`, T12). A change of the overwrite choice plans again.
//!
//! After the review: a source on a read-only side (or that side's base itself) offers *Copy* only, *Move* off with its
//! reason (F1); a drop whose place in the window is unknown (winit gives no pointer position during an OS drag) asks
//! which pane it goes into, the focus pane's current folder first, and offers W4's *Open as new comparison* for one or
//! two paths (F4).

use std::any::Any;
use std::ffi::OsString;

use hikasa_core::{JobKind, Side, Spec};
use hikasa_folder::NodeId;
use hikasa_session::api::{VfsOpRequest, human::vfs_op_lines};
use hikasa_session::ipc::ViewId;
use hikasa_session::{ApiError, Service};
use hikasa_vfs::ops::{CopyOptions, MoveOptions, OverwritePolicy, Plan};

use crate::dialog::{ButtonRole, DialogButton, DialogModel, DialogStep};
use crate::job::{Job, Wake};

/// The help topic (the drops of the Folder Compare view).
pub const DROP_TOPIC: &str = "view-folder-compare";
/// The `state.dialogs` id: the remembered overwrite choice.
pub const DROP_DIALOG: &str = "drop";
/// The overwrite choices, in the order shown.
pub const OVERWRITES: [OverwritePolicy; 4] = [
    OverwritePolicy::Fail,
    OverwritePolicy::Skip,
    OverwritePolicy::Newer,
    OverwritePolicy::Always,
];

/// The caption of an overwrite choice.
pub fn overwrite_caption(policy: OverwritePolicy) -> &'static str {
    match policy {
        OverwritePolicy::Fail => "Fail",
        OverwritePolicy::Skip => "Skip",
        OverwritePolicy::Newer => "Replace if newer",
        OverwritePolicy::Always => "Replace",
    }
}

/// The planned copy or move: core's plan, its preview lines (`vfs_op_lines` of the dry run), or why it cannot run.
pub type Planned = Result<(Plan, Vec<String>), String>;

/// Where dropped paths go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropInto {
    /// The view.
    pub view: ViewId,
    /// The pane.
    pub side: Side,
    /// The folder row (`None`: the side's base).
    pub folder: Option<NodeId>,
    /// The folder's spec text as shown.
    pub shown: String,
    /// The target as `vfs cp` takes it: the folder's spec with a trailing separator ("into this folder").
    pub target: OsString,
    /// The folder's spec (a row drag prefills Copy/Move to Folder with it, never with its text).
    pub spec: Spec,
}

/// What the prompt answered: the plan of the button pressed.
pub struct DropAnswer {
    /// Where it goes.
    pub into: DropInto,
    /// A move (else a copy).
    pub moving: bool,
    /// The plan, as previewed.
    pub plan: Plan,
    /// The sources, as dropped (the view refreshes their folders after a move).
    pub sources: Vec<OsString>,
}

/// The drop prompt.
pub struct DropDialog {
    /// The view.
    view: ViewId,
    /// Where the paths go (`None`: no pane can take them; only *Open as new comparison* or *Cancel*).
    pub into: Option<DropInto>,
    /// The panes to choose from, when the drop had no place in the window (F4): per side, its current folder or why
    /// it cannot take the paths.
    pub sides: Option<[Result<DropInto, String>; 2]>,
    /// The paths.
    pub sources: Vec<OsString>,
    /// The overwrite choice.
    pub overwrite: OverwritePolicy,
    /// The preview shows the move (else the copy); Shift at the drop shows the move first.
    pub show_move: bool,
    /// Why *Move* is off: a source lies on a read-only side (*Disable editing*, `--ro`).
    pub move_refused: Option<String>,
    /// *Open as new comparison* is offered (one or two paths dropped where the window could not say).
    pub open_new: bool,
    /// The plans of the last finished job: copy, move.
    pub plans: Option<(Planned, Planned)>,
    dirty: bool,
    job: Option<Job<(Planned, Planned)>>,
    answer: Option<DropAnswer>,
    opened_new: bool,
    wake: Wake,
}

/// The button *Open as new comparison*.
pub const OPEN_NEW: ButtonRole = ButtonRole::Other(3);

impl DropDialog {
    /// A prompt for `sources` dropped into `into`; `moving` shows the move first and makes it the default button
    /// unless `move_refused` says why a move may not run.
    pub fn new(
        into: DropInto,
        sources: Vec<OsString>,
        overwrite: OverwritePolicy,
        moving: bool,
        move_refused: Option<String>,
        wake: Wake,
    ) -> DropDialog {
        DropDialog {
            view: into.view,
            into: Some(into),
            ..DropDialog::new_parts(sources, overwrite, moving, move_refused, wake)
        }
    }

    /// A prompt for `sources` dropped on `view` where the window could not say which pane (F4): `sides` per side,
    /// `first` chosen first (when it can take them, else the other side when it can).
    #[allow(clippy::too_many_arguments)] // the prompt's parts
    pub fn asking(
        view: ViewId,
        sides: [Result<DropInto, String>; 2],
        first: Side,
        sources: Vec<OsString>,
        overwrite: OverwritePolicy,
        moving: bool,
        move_refused: Option<String>,
        wake: Wake,
    ) -> DropDialog {
        let k = usize::from(first == Side::Right);
        let into = sides[k].as_ref().or(sides[1 - k].as_ref()).ok().cloned();
        let open_new = matches!(sources.len(), 1 | 2);
        DropDialog {
            view,
            into,
            sides: Some(sides),
            open_new,
            ..DropDialog::new_parts(sources, overwrite, moving, move_refused, wake)
        }
    }

    fn new_parts(
        sources: Vec<OsString>,
        overwrite: OverwritePolicy,
        moving: bool,
        move_refused: Option<String>,
        wake: Wake,
    ) -> DropDialog {
        DropDialog {
            view: ViewId(0),
            into: None,
            sides: None,
            sources,
            overwrite,
            show_move: moving && move_refused.is_none(),
            move_refused,
            open_new: false,
            plans: None,
            dirty: true,
            job: None,
            answer: None,
            opened_new: false,
            wake,
        }
    }

    /// The question.
    pub fn message(&self) -> String {
        match &self.into {
            Some(into) => format!(
                "Copy (or move) {} items into `{}`?",
                self.sources.len(),
                into.shown
            ),
            None => format!(
                "Neither pane can take the {} dropped items.",
                self.sources.len()
            ),
        }
    }

    /// The pane chosen (the prompt that asks, F4): its current folder; planned again at the next poll.
    pub fn choose_side(&mut self, side: Side) {
        let k = usize::from(side == Side::Right);
        let Some(Ok(into)) = self.sides.as_ref().map(|s| &s[k]) else {
            return;
        };
        if self.into.as_ref() != Some(into) {
            self.into = Some(into.clone());
            self.plans = None;
            self.dirty = true;
        }
    }

    /// Changes the overwrite choice: planned again at the next poll.
    pub fn set_overwrite(&mut self, overwrite: OverwritePolicy) {
        if overwrite != self.overwrite {
            self.overwrite = overwrite;
            self.dirty = true;
        }
    }

    /// The plan the preview shows.
    pub fn shown(&self) -> Option<&Planned> {
        self.plans
            .as_ref()
            .map(|(copy, moved)| if self.show_move { moved } else { copy })
    }

    /// The answer, once (the window hands it to the view).
    pub fn take_answer(&mut self) -> Option<DropAnswer> {
        self.answer.take()
    }

    /// *Open as new comparison* was pressed: the paths, once (the window applies W4's rule to them).
    pub fn take_open_new(&mut self) -> Option<Vec<OsString>> {
        std::mem::take(&mut self.opened_new).then(|| self.sources.clone())
    }

    /// The view.
    pub fn view(&self) -> ViewId {
        self.view
    }

    fn ready(&self, moving: bool) -> bool {
        self.into.is_some()
            && !(moving && self.move_refused.is_some())
            && !self.dirty
            && self.job.is_none()
            && self
                .plans
                .as_ref()
                .is_some_and(|(copy, moved)| if moving { moved.is_ok() } else { copy.is_ok() })
    }
}

/// The two plans of a drop (a job: core reads the disk).
fn plan_both(
    svc: &Service,
    job: &hikasa_session::JobControl,
    sources: &[OsString],
    target: &OsString,
    overwrite: OverwritePolicy,
) -> (Planned, Planned) {
    let copy = VfsOpRequest::Copy {
        sources: sources.to_vec(),
        target: target.clone(),
        options: CopyOptions {
            recursive: true,
            overwrite,
            ..CopyOptions::default()
        },
    };
    let moved = VfsOpRequest::Move {
        sources: sources.to_vec(),
        target: target.clone(),
        options: MoveOptions {
            overwrite,
            ..MoveOptions::default()
        },
    };
    let planned = |request: &VfsOpRequest| -> Planned {
        svc.vfs_op_plan(request, job)
            .map(|planned| (planned.plan, vfs_op_lines(&planned.preview)))
            .map_err(|e: ApiError| e.to_string())
    };
    (planned(&copy), planned(&moved))
}

impl DialogModel for DropDialog {
    fn title(&self) -> String {
        "Copy or Move".to_owned()
    }

    fn help_topic(&self) -> &'static str {
        DROP_TOPIC
    }

    fn buttons(&self) -> Vec<DialogButton> {
        let button = |label: &str, role, enabled, default| DialogButton {
            label: label.to_owned(),
            role,
            enabled,
            default,
        };
        let mut buttons = vec![
            button(
                "Copy",
                ButtonRole::Other(1),
                self.ready(false),
                !self.show_move,
            ),
            button(
                "Move",
                ButtonRole::Other(2),
                self.ready(true),
                self.show_move,
            ),
        ];
        if self.open_new {
            buttons.push(button("Open as new comparison", OPEN_NEW, true, false));
        }
        buttons.push(button("Cancel", ButtonRole::Cancel, true, false));
        buttons
    }

    fn on_button(&mut self, role: ButtonRole, _svc: &Service) -> DialogStep {
        let moving = match role {
            ButtonRole::Other(1) => false,
            ButtonRole::Other(2) => true,
            OPEN_NEW if self.open_new => {
                self.opened_new = true;
                return DialogStep::Close;
            }
            ButtonRole::Cancel => return DialogStep::Close,
            _ => return DialogStep::Stay,
        };
        if !self.ready(moving) {
            return DialogStep::Stay;
        }
        let Some((copy, moved)) = self.plans.take() else {
            return DialogStep::Stay;
        };
        let chosen = if moving { moved } else { copy };
        let Ok((plan, _)) = chosen else {
            return DialogStep::Stay;
        };
        let Some(into) = self.into.clone() else {
            return DialogStep::Stay;
        };
        self.answer = Some(DropAnswer {
            into,
            moving,
            plan,
            sources: self.sources.clone(),
        });
        DialogStep::Close
    }

    fn poll(&mut self, svc: &Service) -> bool {
        if self.dirty {
            self.dirty = false;
            if let Some(job) = self.job.take() {
                job.cancel();
            }
            let Some(into) = &self.into else {
                return true;
            };
            let (sources, target, overwrite) =
                (self.sources.clone(), into.target.clone(), self.overwrite);
            self.job = Some(Job::spawn(
                svc,
                JobKind::VfsRead,
                "plan the drop".into(),
                &self.wake,
                move |svc, job| Ok(plan_both(svc, job, &sources, &target, overwrite)),
            ));
            return true;
        }
        let Some(result) = self.job.as_ref().and_then(Job::try_result) else {
            return false;
        };
        self.job = None;
        let (copy, moved) = match result {
            Ok(plans) => plans,
            Err(e) => (Err(e.to_string()), Err(e.to_string())),
        };
        // A source on a read-only side is never moved: its plan is the reason (F1).
        let moved = match &self.move_refused {
            Some(why) => Err(why.clone()),
            None => moved,
        };
        self.plans = Some((copy, moved));
        true
    }

    fn busy(&self) -> bool {
        self.job.is_some() || self.dirty
    }

    fn size(&self) -> Option<(f32, f32)> {
        Some((620.0, 320.0))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// The remembered overwrite word of the drop prompt, read back (`fail` when none or unknown).
pub fn overwrite_of(word: Option<&str>) -> OverwritePolicy {
    word.and_then(|w| hikasa_session::api::parse_overwrite(w).ok())
        .unwrap_or(OverwritePolicy::Fail)
}

/// The word of an overwrite choice as `state.dialogs` keeps it (core's vocabulary).
pub fn overwrite_word(policy: OverwritePolicy) -> &'static str {
    policy.as_str()
}
