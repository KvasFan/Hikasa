//! The verbs of a folder view (M2 W9b, D-244, D-245; contract §1.9.12, §1.9.13): how each command of §1.9.13's table
//! starts — a plan at once, a question of the side, a verb dialog over a core preview, the one-row Rename's inline
//! edit — and how a verb dialog's Start comes back to the view. A child module of `folder_view` (it reads the view's
//! fields); toolkit-free (check 7).

use std::sync::Arc;

use hikasa_core::{Side, Spec};
use hikasa_folder::NodeId;
use hikasa_folder::ops::{Confirmation, FolderOp, PathStructure, RenameMask, base_mask};
use hikasa_folder::select::{Selection, SideSet};
use hikasa_session::Service;
use hikasa_session::api::{OpRequest, parse_overwrite, parse_structure};
use hikasa_session::ipc::ViewId;
use hikasa_session::log::LogKind;
use hikasa_session::settings::State;
use hikasa_vfs::ops::OverwritePolicy;

use super::{FolderViewModel, Mark, Replan, Task};
use crate::commands::CommandId;
use crate::dialog::{ButtonRole, DialogModel, Followup, Prompt};
use crate::op_dialogs::{
    COPY_TO_DIALOG, NEW_FOLDER_DIALOG, OpContext, OpDialog, OpForm, PlanConfirmDialog,
    RENAME_DIALOG, SideChoice, TOUCH_DIALOG, rename_mode_of, side_choice_of, touch_from_other_of,
};
use crate::op_preview::PreviewList;
use crate::status::LogNote;
use crate::window::{CommandArg, Invocation};

/// The one-row Rename's inline edit (D-245): the row, the sides it renames and the name typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineRename {
    /// The row.
    pub node: NodeId,
    /// The sides renamed (the selected ones).
    pub sides: SideSet,
    /// The name as typed (starts as the row's name).
    pub text: String,
    /// The row's name.
    pub original: String,
}

/// What the window knows when a verb starts.
#[derive(Debug, Clone)]
pub struct VerbOptions<'a> {
    /// The pane a context menu was opened on (`CopySide`, `MoveSide`).
    pub arg: Option<Side>,
    /// The remembered choices (`state.dialogs`) and the recent folders.
    pub state: &'a State,
    /// `include_hidden_by_default`.
    pub include_hidden: bool,
    /// Names follow Windows' rules.
    pub windows: bool,
    /// Rows dragged onto a folder row of the other pane (M2 W9c, D-251): Copy/Move to Folder's target prefilled with
    /// that folder (its spec: the dialog keeps it as it is until the target is edited, review F5), structure
    /// *relative*, from the side the rows were dragged from.
    pub into: Option<(Spec, Side)>,
}

/// How a verb started.
pub enum VerbStart {
    /// Nothing (not available; the predicates say the same).
    Nothing,
    /// A plan job runs, or the inline edit opened.
    Started,
    /// A dialog for the window's stack.
    Dialog(Box<dyn DialogModel>),
}

/// The side a `…Side` command copies to: the side opposite the pane it was given (or the focus side).
fn opposite_of(arg: Option<Side>, focus: SideSet) -> Option<Side> {
    let from = match arg {
        Some(side @ (Side::Left | Side::Right)) => side,
        _ if focus == SideSet::LEFT => Side::Left,
        _ if focus == SideSet::RIGHT => Side::Right,
        _ => return None,
    };
    Some(from.opposite())
}

impl FolderViewModel {
    /// Sets *Include hidden items* of the plans made without a dialog (the window's `include_hidden_by_default`).
    pub fn set_include_hidden(&mut self, on: bool) {
        self.include_hidden = on;
    }

    /// The selection with an item somewhere, and what it holds per side, while an operation may start.
    fn verb_items(&self) -> Option<(Selection, [bool; 2])> {
        self.operable()
    }

    /// The context a verb dialog plans on.
    #[allow(clippy::too_many_arguments)] // what a dialog plans on
    fn op_context(
        &self,
        service: &Service,
        view: ViewId,
        command: CommandId,
        caption: &str,
        items: Selection,
        on: [bool; 2],
        windows: bool,
    ) -> Option<(OpContext, Mark)> {
        let prepared = self.prepared()?;
        let mark = Mark::of(prepared);
        // The source side: the focus side, else the side that has the items, else the left (review F13).
        let source = match (self.selection.focus, on) {
            (f, _) if f == SideSet::RIGHT => Side::Right,
            (f, _) if f == SideSet::LEFT => Side::Left,
            (_, [false, true]) => Side::Right,
            _ => Side::Left,
        };
        let style = service.spec_context().style;
        let target_context = match prepared.specs().get(source) {
            Some(hikasa_core::Spec::Local(path)) => Some(hikasa_core::SpecContext {
                cwd: Some(path.as_path().to_path_buf()),
                ..service.spec_context()
            }),
            _ => None,
        };
        Some((
            OpContext {
                view,
                command,
                caption: caption.to_owned(),
                model: Arc::clone(&mark.model),
                generation: mark.generation,
                selection: items,
                display: self.display,
                read_only: self.write.read_only,
                overrides: self.write.overrides(),
                on,
                zone: service.clock().now().time_zone().clone(),
                windows,
                target_context,
                style,
            },
            mark,
        ))
    }

    /// Starts a verb of §1.9.13 that W6d did not have (`CopyRight`, `CopyLeft`, `Delete` and `PermanentDelete` are
    /// [`FolderViewModel::start_operation`]'s).
    pub fn start_verb(
        &mut self,
        service: &Service,
        view: ViewId,
        command: CommandId,
        caption: &str,
        options: VerbOptions<'_>,
    ) -> VerbStart {
        if command == CommandId::NewFolder {
            return self.start_new_folder(service, view, caption, options);
        }
        let Some((items, on)) = self.verb_items() else {
            return VerbStart::Nothing;
        };
        let Some(mark) = self.prepared().map(Mark::of) else {
            return VerbStart::Nothing;
        };
        let writable = |side: Side| self.write.writable(side);
        let k = |side: Side| usize::from(side == Side::Right);
        match command {
            CommandId::MoveRight
            | CommandId::MoveLeft
            | CommandId::CopySide
            | CommandId::MoveSide => {
                let moving = matches!(
                    command,
                    CommandId::MoveRight | CommandId::MoveLeft | CommandId::MoveSide
                );
                let to = match command {
                    CommandId::MoveRight => Side::Right,
                    CommandId::MoveLeft => Side::Left,
                    _ => match opposite_of(options.arg, self.selection.focus) {
                        Some(to) => to,
                        None => return VerbStart::Nothing,
                    },
                };
                let from = to.opposite();
                if !writable(to) || !on[k(from)] || (moving && !writable(from)) {
                    return VerbStart::Nothing;
                }
                self.cancel_crc();
                let op = if moving {
                    FolderOp::Move {
                        to,
                        overwrite: OverwritePolicy::Always,
                    }
                } else {
                    FolderOp::Copy {
                        to,
                        overwrite: OverwritePolicy::Always,
                    }
                };
                let mut sent = items.clone();
                sent.restrict(from);
                let subject = self.subject(&sent, SideSet::of(from), Some(to));
                self.plan(service, view, caption, op, items, sent, mark, subject);
                VerbStart::Started
            }
            CommandId::CopyPrompt | CommandId::MovePrompt => {
                // "…to Side…": the side asked, then the matching `…Side` command from the other pane.
                let moving = command == CommandId::MovePrompt;
                let then = if moving {
                    CommandId::MoveSide
                } else {
                    CommandId::CopySide
                };
                let mut buttons = Vec::new();
                for (to, role, label) in [
                    (Side::Left, ButtonRole::Other(1), "Left"),
                    (Side::Right, ButtonRole::Other(2), "Right"),
                ] {
                    let from = to.opposite();
                    if writable(to) && on[k(from)] && (!moving || writable(from)) {
                        buttons.push((
                            role,
                            label,
                            vec![Followup::Run(Invocation {
                                id: then,
                                arg: Some(CommandArg::Side(from)),
                            })],
                        ));
                    }
                }
                if buttons.is_empty() {
                    return VerbStart::Nothing;
                }
                let default = buttons[0].0;
                buttons.push((ButtonRole::Cancel, "Cancel", Vec::new()));
                let what = if moving { "Move" } else { "Copy" };
                let prompt = Prompt::ask(
                    caption.trim_end_matches("..."),
                    format!("{what} the {} selected items to which side?", items.len()),
                    default,
                    buttons,
                    "dialog-confirm",
                );
                VerbStart::Dialog(Box::new(prompt))
            }
            CommandId::MoveBoth => {
                if !(writable(Side::Left) && writable(Side::Right)) {
                    return VerbStart::Nothing;
                }
                self.cancel_crc();
                let both = SideSet::LEFT | SideSet::RIGHT;
                let subject = self.subject(&items, both, None);
                self.plan(
                    service,
                    view,
                    caption,
                    FolderOp::Exchange,
                    items.clone(),
                    items,
                    mark,
                    subject,
                );
                VerbStart::Started
            }
            CommandId::CopyTo | CommandId::MoveTo => {
                let moving = command == CommandId::MoveTo;
                let state = options.state;
                let structure = state
                    .dialog_text(COPY_TO_DIALOG, crate::op_dialogs::structure_key())
                    .and_then(|w| parse_structure(w).ok())
                    .unwrap_or(PathStructure::Relative);
                let overwrite = state
                    .dialog_text(COPY_TO_DIALOG, crate::op_dialogs::overwrite_key())
                    .and_then(|w| parse_overwrite(w).ok())
                    .unwrap_or(OverwritePolicy::Fail);
                let recent = state
                    .mru(hikasa_session::settings::state::MruList::Folders)
                    .to_vec();
                let prefilled = options.into.as_ref().map(|(folder, _)| folder.clone());
                let (target, structure, sides) = match &options.into {
                    Some((folder, from)) => (
                        folder.canonical(),
                        PathStructure::Relative,
                        if *from == Side::Right {
                            SideChoice::Right
                        } else {
                            SideChoice::Left
                        },
                    ),
                    None => (
                        recent.first().cloned().unwrap_or_default(),
                        structure,
                        self.side_preset(on, moving),
                    ),
                };
                let started = self.open_dialog(
                    service,
                    view,
                    command,
                    caption,
                    items,
                    on,
                    options,
                    OpForm::CopyTo {
                        moving,
                        target,
                        structure,
                        sides,
                        overwrite,
                    },
                );
                match started {
                    VerbStart::Dialog(mut dialog) => {
                        if let Some(d) = dialog.as_any_mut().downcast_mut::<OpDialog>() {
                            d.recent = recent;
                            if let Some(spec) = prefilled {
                                d.prefill_target(spec);
                            }
                        }
                        VerbStart::Dialog(dialog)
                    }
                    other => other,
                }
            }
            CommandId::Rename => {
                if items.len() == 1 {
                    let Some((node, sides)) = items.iter().next() else {
                        return VerbStart::Nothing;
                    };
                    let sides = sides
                        .sides()
                        .filter(|s| writable(*s))
                        .fold(SideSet::default(), |all, s| all | SideSet::of(s));
                    if sides.is_empty() {
                        return VerbStart::Nothing;
                    }
                    let name = self.name_of(node).unwrap_or_default();
                    self.inline_rename = Some(InlineRename {
                        node,
                        sides,
                        text: name.clone(),
                        original: name,
                    });
                    return VerbStart::Started;
                }
                let state = options.state;
                let names = self.names_of(&items);
                let refs: Vec<&str> = names.iter().map(String::as_str).collect();
                let mode = rename_mode_of(state.dialog_text(RENAME_DIALOG, "mode"));
                let text = |key| {
                    state
                        .dialog_text(RENAME_DIALOG, key)
                        .unwrap_or("")
                        .to_owned()
                };
                let sides = self.side_preset(on, true);
                self.open_dialog(
                    service,
                    view,
                    command,
                    caption,
                    items,
                    on,
                    options,
                    OpForm::Rename {
                        mode,
                        dos: base_mask(&refs),
                        old: text("old"),
                        new: text("new"),
                        sides,
                    },
                )
            }
            CommandId::Touch => {
                let state = options.state;
                let from_other = touch_from_other_of(state.dialog_text(TOUCH_DIALOG, "mode"));
                let text = state
                    .dialog_text(TOUCH_DIALOG, "last")
                    .map(str::to_owned)
                    .unwrap_or_else(|| {
                        service
                            .clock()
                            .now()
                            .strftime("%Y-%m-%d %H:%M:%S")
                            .to_string()
                    });
                let sides = self.side_preset(on, true);
                let to = if (on[1] && !on[0]) || writable(Side::Right) {
                    Side::Right
                } else {
                    Side::Left
                };
                self.open_dialog(
                    service,
                    view,
                    command,
                    caption,
                    items,
                    on,
                    options,
                    OpForm::Touch {
                        from_other,
                        to,
                        text,
                        sides,
                    },
                )
            }
            _ => VerbStart::Nothing,
        }
    }

    /// The side choice a dialog opens with: the sides the selection has items on (writable ones where the verb writes
    /// them).
    fn side_preset(&self, on: [bool; 2], writes: bool) -> SideChoice {
        let ok = |k: usize, side: Side| on[k] && (!writes || self.write.writable(side));
        match (ok(0, Side::Left), ok(1, Side::Right)) {
            (true, true) => SideChoice::Both,
            (false, true) => SideChoice::Right,
            _ => SideChoice::Left,
        }
    }

    #[allow(clippy::too_many_arguments)] // one call per verb
    fn open_dialog(
        &mut self,
        service: &Service,
        view: ViewId,
        command: CommandId,
        caption: &str,
        items: Selection,
        on: [bool; 2],
        options: VerbOptions<'_>,
        form: OpForm,
    ) -> VerbStart {
        let Some((ctx, mark)) = self.op_context(
            service,
            view,
            command,
            caption,
            items.clone(),
            on,
            options.windows,
        ) else {
            return VerbStart::Nothing;
        };
        self.cancel_crc();
        let dialog = OpDialog::new(ctx, form, options.include_hidden, Arc::clone(&self.wake));
        self.task = Some(Task::Dialog {
            caption: caption.to_owned(),
            mark,
            items,
        });
        VerbStart::Dialog(Box::new(dialog))
    }

    /// New Folder (FC-076): in the current row's folder (a folder row itself, a file's parent, else the base), on the
    /// focus side when one side is focused, else the remembered sides, else both writable sides.
    fn start_new_folder(
        &mut self,
        service: &Service,
        view: ViewId,
        caption: &str,
        options: VerbOptions<'_>,
    ) -> VerbStart {
        if self.busy() || self.task.is_some() {
            return VerbStart::Nothing;
        }
        let Some(prepared) = self.prepared() else {
            return VerbStart::Nothing;
        };
        let parent = {
            let read = prepared.model().read();
            match self
                .selection
                .current
                .and_then(|id| read.node(id).map(|n| (id, n)))
            {
                Some((id, node)) if node.is_folder() => read.path(id).as_str().to_owned(),
                Some((_, node)) => node
                    .parent
                    .filter(|p| *p != NodeId::ROOT)
                    .map(|p| read.path(p).as_str().to_owned())
                    .unwrap_or_default(),
                None => String::new(),
            }
        };
        let writable = [
            self.write.writable(Side::Left),
            self.write.writable(Side::Right),
        ];
        if !writable[0] && !writable[1] {
            return VerbStart::Nothing;
        }
        let focus = self.selection.focus;
        let sides = if focus == SideSet::LEFT && writable[0] {
            SideChoice::Left
        } else if focus == SideSet::RIGHT && writable[1] {
            SideChoice::Right
        } else {
            side_choice_of(options.state.dialog_text(NEW_FOLDER_DIALOG, "sides"))
                .filter(|c| match c {
                    SideChoice::Left => writable[0],
                    SideChoice::Right => writable[1],
                    SideChoice::Both => writable[0] && writable[1],
                })
                .unwrap_or(match writable {
                    [true, true] => SideChoice::Both,
                    [true, false] => SideChoice::Left,
                    _ => SideChoice::Right,
                })
        };
        let items = Selection::default();
        self.open_dialog(
            service,
            view,
            CommandId::NewFolder,
            caption,
            items,
            [true, true],
            options,
            OpForm::NewFolder {
                name: String::new(),
                sides,
                parent,
            },
        )
    }

    /// A verb dialog closed: Start hands its plan and questions (D-244: the previewed plan, unchanged); nothing else
    /// runs anything.
    pub fn op_dialog_closed(
        &mut self,
        service: &Service,
        view: ViewId,
        started: Option<(PreviewList, Confirmation)>,
    ) {
        let Some(Task::Dialog {
            caption,
            mark,
            items,
        }) = self.task.take()
        else {
            return;
        };
        let Some((list, confirmation)) = started else {
            return;
        };
        if !mark.holds(self.prepared()) {
            self.refuse_changed(&caption);
            return;
        }
        if !items.is_empty() && self.operable_items() != Some(&items) {
            self.refuse_moved(&caption);
            return;
        }
        if confirmation.next().is_none() {
            let Some(plan) = list.into_op() else {
                return;
            };
            match confirmation.finish(plan) {
                Ok(plan) => self.execute_plan(service, &caption, plan, mark),
                Err(_) => self.notes.push(LogNote {
                    kind: LogKind::Summary,
                    text: format!("{caption}: cancelled, nothing was written."),
                }),
            }
            return;
        }
        let subject = self.subject(&items, SideSet::all(), None);
        let dialog = PlanConfirmDialog::new(view, &caption, subject, list, confirmation, None);
        self.task = Some(Task::Asking {
            caption,
            mark,
            again: None,
        });
        self.outbox
            .push(super::FolderEvent::Dialog(Box::new(dialog)));
    }

    /// The confirmation's *Include hidden items* was toggled: the same request is planned again with it.
    pub fn replan(&mut self, service: &Service, view: ViewId, include_hidden: bool) {
        let Some(Task::Asking {
            caption,
            mark,
            again: Some((again, items, subject)),
        }) = self.task.take()
        else {
            return;
        };
        if !mark.holds(self.prepared()) {
            self.refuse_changed(&caption);
            return;
        }
        let again = Replan {
            include_hidden,
            ..*again
        };
        self.plan_request(service, view, &caption, again, items, mark, subject);
    }

    // ---- the inline Rename ----------------------------------------------------------------------------------------

    /// The one-row Rename's edit, while it is open.
    pub fn inline_rename(&self) -> Option<&InlineRename> {
        self.inline_rename.as_ref()
    }

    /// The name typed in the inline edit.
    pub fn set_inline_text(&mut self, text: &str) {
        if let Some(edit) = &mut self.inline_rename {
            edit.text = text.to_owned();
        }
    }

    /// Esc: the edit closes, nothing is renamed.
    pub fn cancel_inline_rename(&mut self) {
        self.inline_rename = None;
    }

    /// Enter: the row is renamed to the name typed (`RenameMask::Name`), through the plan and its questions; an
    /// unchanged or empty name closes the edit.
    pub fn commit_inline_rename(&mut self, service: &Service, view: ViewId, caption: &str) {
        let Some(edit) = self.inline_rename.take() else {
            return;
        };
        if edit.text.is_empty() || edit.text == edit.original || self.task.is_some() {
            return;
        }
        let Some(mark) = self.prepared().map(Mark::of) else {
            return;
        };
        let mut items = Selection::default();
        items.insert(edit.node, edit.sides);
        let subject = self.subject(&items, edit.sides, None);
        let again = Replan {
            op: OpRequest::Op(FolderOp::Rename {
                sides: edit.sides,
                mask: RenameMask::Name(edit.text),
            }),
            sent: items.clone(),
            include_hidden: self.include_hidden,
        };
        self.cancel_crc();
        self.plan_request(service, view, caption, again, items, mark, subject);
    }

    /// The name of a row (its last path component).
    fn name_of(&self, node: NodeId) -> Option<String> {
        let path = self.path_of(node)?;
        Some(path.rsplit('/').next().unwrap_or(&path).to_owned())
    }

    /// The names of the selected rows (Rename's `base_mask`).
    fn names_of(&self, items: &Selection) -> Vec<String> {
        items
            .iter()
            .filter_map(|(node, _)| self.name_of(node))
            .collect()
    }
}

// ---- Ignored, Exclude, Compare Contents, Quick Compare, Compare To, Open With (M2 W9b, D-245) ---------------------

impl FolderViewModel {
    /// The comparison and the request of the selection a dialog of the window plans on (`None`: nothing selected, or
    /// an operation runs).
    pub fn selection_request(
        &self,
        include_hidden: bool,
    ) -> Option<(
        Arc<hikasa_folder::FolderModel>,
        hikasa_session::api::SelectRequest,
    )> {
        let (items, _) = self.operable()?;
        let model = Arc::clone(self.prepared()?.model());
        let mut select = super::select_request(items, self.display);
        select.include_hidden = Some(include_hidden);
        Some((model, select))
    }

    /// The session's content method (Compare Contents' preset).
    pub fn content_method(&self) -> hikasa_core::ContentMethod {
        self.prepared()
            .map_or(hikasa_core::ContentMethod::Binary, |p| {
                p.rules().criteria.content.method
            })
    }

    /// True when the current row is Ignored (FC-033: the `Ignored` check state).
    pub fn ignored_checked(&self) -> bool {
        let (Some(id), Some(prepared)) = (self.selection.current, self.prepared()) else {
            return false;
        };
        let read = prepared.model().read();
        read.node(id)
            .is_some_and(|n| n.flags.contains(hikasa_folder::NodeFlags::IGNORED))
    }

    /// `Ignored` (FC-033): the selected rows are marked (or unmarked when the current row is); the list core returns
    /// replaces the view's `rules.ignored` (its session becomes dirty); nothing runs again. False when nothing changed.
    pub fn toggle_ignored(&mut self, service: &Service) -> bool {
        let Some((items, _)) = self.operable() else {
            return false;
        };
        let Some(model) = self.prepared().map(|p| Arc::clone(p.model())) else {
            return false;
        };
        let on = !self.ignored_checked();
        let nodes: Vec<NodeId> = items.iter().map(|(node, _)| node).collect();
        let list = service.folder_set_ignored(&model, &nodes, on);
        self.request.patch.ignored = Some(list);
        self.rebuild();
        true
    }

    /// The rules' Ignored list the view's session keeps (tests).
    #[cfg(test)]
    pub fn ignored_list(&self) -> Option<&[String]> {
        self.request.patch.ignored.as_deref()
    }

    /// Compare Contents (FC-029 b) on the selection: `folder_contents` with `ContentsKind::Compare(method)` in a job
    /// (the comparison's text rules); the results enter the model, and with `show_results` their dialog follows.
    pub fn start_contents(
        &mut self,
        service: &Service,
        view: ViewId,
        caption: &str,
        method: hikasa_core::ContentMethod,
        show_results: bool,
    ) -> bool {
        let Some((items, _)) = self.operable() else {
            return false;
        };
        let Some(prepared) = self.prepared() else {
            return false;
        };
        let model = Arc::clone(prepared.model());
        let request = hikasa_session::api::ContentsRequest {
            select: super::select_request(items, self.display),
            kind: hikasa_session::api::ContentsKind::Compare(method),
            text_rules: prepared.text_rules().cloned(),
        };
        let crc = self.crc_yield();
        let job_model = Arc::clone(&model);
        #[cfg(test)]
        let gate = self.gate.clone();
        let job = crate::job::Job::spawn(
            service,
            hikasa_core::JobKind::FolderOp,
            caption.to_owned(),
            &self.wake,
            move |svc, job| {
                #[cfg(test)]
                super::wait_at(gate.as_ref());
                crc.wait();
                svc.folder_contents(&job_model, &request, job)
            },
        );
        let _ = view;
        self.task = Some(Task::Contents {
            caption: caption.to_owned(),
            show_results,
            began: std::time::Instant::now(),
            job,
        });
        true
    }

    /// The selected items with their bases, in tree order, left side first per row (Open With, the associated
    /// application).
    pub fn selected_specs(&self) -> Vec<(hikasa_core::Spec, hikasa_core::Spec)> {
        let Some(prepared) = self.prepared() else {
            return Vec::new();
        };
        let read = prepared.model().read();
        let mut items = self.selection.items.clone();
        items.remove(NodeId::ROOT, SideSet::all());
        let mut out = Vec::new();
        for (node, side) in hikasa_folder::select::items_in_order(&read, &items) {
            if let (Some(spec), Some(base)) = (read.spec(node, side), read.spec(NodeId::ROOT, side))
            {
                out.push((spec, base));
            }
        }
        out
    }

    /// True when the selection holds two folders (Open With ▸ *Folder Compare*).
    pub fn two_folders(&self) -> bool {
        let Some(prepared) = self.prepared() else {
            return false;
        };
        let read = prepared.model().read();
        let items: Vec<(NodeId, SideSet)> = self.selection.items.iter().collect();
        let folders: usize = items
            .iter()
            .filter(|(node, _)| read.node(*node).is_some_and(|n| n.is_folder()))
            .map(|(_, sides)| sides.sides().count())
            .sum();
        folders == 2 && items.iter().map(|(_, s)| s.sides().count()).sum::<usize>() == 2
    }

    /// Compare To (FC-062): the specs of two selected items, or of one and the Space mark (`select::open_pair`); `None`
    /// with one item and no mark (the window asks for the second through the browse dialog).
    pub fn compare_to_pair(&self) -> Option<[hikasa_core::Spec; 2]> {
        let prepared = self.prepared()?;
        let read = prepared.model().read();
        let mut items = self.selection.items.clone();
        items.remove(NodeId::ROOT, SideSet::all());
        // Specs, never their canonical text: a name that is not valid Unicode stays exact (review F9).
        let pair = hikasa_folder::select::open_pair(&read, &items, self.selection.mark)?;
        Some([pair.left, pair.right])
    }

    /// The one selected item of Compare To when no second one is given: its spec and whether it is a folder.
    pub fn compare_to_first(&self) -> Option<(hikasa_core::Spec, bool)> {
        let prepared = self.prepared()?;
        let read = prepared.model().read();
        let mut found = None;
        for (node, sides) in self.selection.items.iter() {
            for side in sides.sides() {
                if found.is_some() {
                    return None;
                }
                let folder = read.node(node)?.is_folder();
                found = Some((read.spec(node, side)?, folder));
            }
        }
        found
    }

    /// The folders of the selected rows by path (Open With's `refresh_when_finished`; `""` is the base): the ids may
    /// move before the application ends (review F10).
    pub fn selected_parent_paths(&self) -> Vec<String> {
        let Some(prepared) = self.prepared() else {
            return Vec::new();
        };
        let read = prepared.model().read();
        let mut out: Vec<String> = Vec::new();
        for (node, _) in self.selection.items.iter() {
            let parent = read
                .node(node)
                .and_then(|n| n.parent)
                .unwrap_or(NodeId::ROOT);
            let path = read.path(parent).as_str().to_owned();
            if !out.contains(&path) {
                out.push(path);
            }
        }
        out
    }

    /// Refreshes the folders at `paths` as the model has them now (an application of Open With ended with
    /// `refresh_when_finished`); a folder gone is skipped; nothing while a task runs.
    pub fn refresh_after_launch(&mut self, service: &Service, paths: &[String]) {
        if self.task.is_some() || self.busy() {
            return;
        }
        let Some(prepared) = self.prepared() else {
            return;
        };
        let nodes: Vec<NodeId> = {
            let read = prepared.model().read();
            paths
                .iter()
                .filter_map(|path| {
                    if path.is_empty() {
                        Some(NodeId::ROOT)
                    } else {
                        super::live_at(&read, path, true)
                    }
                })
                .collect()
        };
        if !nodes.is_empty() {
            self.refresh_rows(service, nodes, super::AfterRefresh::Operation);
        }
    }

    /// Selects `node` on every side it has (Quick Compare's *Open*).
    pub fn select_node(&mut self, node: NodeId) {
        let sides = self.present(node);
        self.selection.select_one(node, sides);
        self.selected_index = self.selection.current_index(&self.rows);
    }
}
