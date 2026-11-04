//! A folder view's base folders change (M2 W9a, FC-002, FC-004; D-239, D-243): the path edits' Accept, the browse
//! dialog's answer and the base commands. The new specs are checked in a job first — a folder that does not exist asks
//! to be created (Yes: `vfs_op_plan` + `vfs_op_execute` of `mkdir`, then the run; No: the message under its field), a
//! spec that cannot be read or a file where a folder is needed shows its message under the field — then the view's
//! session with the new specs reopens in its tab through W4b's path (origin kept, dirty) and the specs that changed are
//! pushed to `mru.left` / `.right`.

use std::ffi::OsString;

use hikasa_core::{ErrorKind, JobKind};
use hikasa_session::api::VfsOpRequest;
use hikasa_session::ipc::ViewId;
use hikasa_session::sessions::{Session, is_dirty};
use hikasa_session::settings::state::MruList;
use hikasa_session::{ApiError, Service};
use hikasa_vfs::{EntryKind, FollowLinks};

use super::flow::{StoreDone, stored_specs};
use super::{AppCore, CoreJob};
use crate::browse::{BROWSE_DIALOG, BrowseDialog, BrowseMode, BrowseTarget};
use crate::dialog::{ButtonRole, Followup, Prompt};
use crate::job::Job;
use crate::session_dialog::SessionAction;
use crate::window::{ViewModel, WindowId};

/// What the check of new base folders found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BaseCheck {
    /// Every changed spec is a folder (or an archive): the view may run on them.
    Ready,
    /// The folder of side `0`/`1` does not exist: its path.
    Missing(usize, String),
    /// The spec of a side cannot be used: the message under its field.
    Bad(usize, String),
}

/// The help topic of the questions.
const TOPIC: &str = "view-folder-compare";

/// Checks the specs of the sides in `changed` (`create`: that side's folder is created first).
fn check(
    svc: &Service,
    specs: &[String; 2],
    changed: &[usize],
    create: Option<usize>,
    job: &hikasa_session::JobControl,
) -> Result<BaseCheck, ApiError> {
    if let Some(k) = create {
        let request = VfsOpRequest::Mkdir {
            specs: vec![OsString::from(&specs[k])],
            parents: true,
        };
        let made = svc
            .vfs_op_plan(&request, job)
            .and_then(|plan| svc.vfs_op_execute(plan.plan, job));
        if let Err(e) = made {
            return Ok(BaseCheck::Bad(k, e.to_string()));
        }
    }
    for &k in changed {
        let spec = &specs[k];
        if spec.is_empty() {
            return Ok(BaseCheck::Bad(k, "Type a folder.".to_owned()));
        }
        match svc.vfs_stat_in(std::ffi::OsStr::new(spec), FollowLinks::Yes, job) {
            Ok(stat) => {
                let folder = stat.entry.kind == EntryKind::Dir.as_str();
                let archive = svc.archive_format_of(&stat.entry.name).is_some();
                if !folder && !archive {
                    return Ok(BaseCheck::Bad(k, format!("{spec} is not a folder.")));
                }
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {
                return Ok(BaseCheck::Missing(k, spec.clone()));
            }
            Err(e) => return Ok(BaseCheck::Bad(k, e.to_string())),
        }
    }
    Ok(BaseCheck::Ready)
}

impl AppCore {
    /// Re-roots folder view `view` on `specs` once they are checked (`create`: the folder of that side is created
    /// first, after the user said Yes).
    pub(super) fn rebase(&mut self, view: ViewId, specs: [String; 2], create: Option<usize>) {
        let Some(ViewModel::Folder(m)) = self.tab_mut(view).and_then(|t| t.model_mut()) else {
            return;
        };
        let old = [
            m.path_edit(0).committed.clone(),
            m.path_edit(1).committed.clone(),
        ];
        let changed: Vec<usize> = (0..2)
            .filter(|&k| specs[k] != old[k] || create == Some(k))
            .collect();
        if changed.is_empty() {
            return;
        }
        let job_specs = specs.clone();
        let job = Job::spawn(
            &self.service,
            JobKind::VfsRead,
            "check the base folders".into(),
            &self.wake(),
            move |svc, job| check(svc, &job_specs, &changed, create, job),
        );
        self.jobs.push(CoreJob::Rebase(view, specs, job));
    }

    /// The check of [`AppCore::rebase`] ended.
    pub(super) fn rebase_checked(
        &mut self,
        view: ViewId,
        specs: [String; 2],
        result: Result<BaseCheck, ApiError>,
    ) {
        match result {
            Ok(BaseCheck::Ready) => {
                let old = match self.tab_mut(view).and_then(|t| t.model_mut()) {
                    Some(ViewModel::Folder(m)) => [
                        m.path_edit(0).committed.clone(),
                        m.path_edit(1).committed.clone(),
                    ],
                    _ => return,
                };
                for (k, list) in [(0, MruList::Left), (1, MruList::Right)] {
                    if specs[k] != old[k] {
                        self.push_mru(list, specs[k].clone());
                    }
                }
                let [left, right] = specs;
                self.reopen_changed(view, "change the base folders", move |_, session| {
                    session.specs.left = left;
                    session.specs.right = right;
                    Ok(())
                });
            }
            Ok(BaseCheck::Missing(k, path)) => {
                let Some(window) = self.window_of(view) else {
                    return;
                };
                let prompt = Prompt::ask(
                    "Create Folder",
                    format!("Folder {path} does not exist. Create it?"),
                    ButtonRole::Yes,
                    vec![
                        (
                            ButtonRole::Yes,
                            "Yes",
                            vec![Followup::Session(SessionAction::CreateBase {
                                view,
                                specs,
                                side: k,
                            })],
                        ),
                        (
                            ButtonRole::No,
                            "No",
                            vec![Followup::Session(SessionAction::BaseError {
                                view,
                                side: k,
                                message: format!("Folder {path} does not exist."),
                            })],
                        ),
                    ],
                    TOPIC,
                );
                if let Some(w) = self.window_mut(window) {
                    w.dialogs.push(Box::new(prompt));
                }
            }
            Ok(BaseCheck::Bad(k, message)) => self.base_error(view, k, message),
            Err(e) => self.base_error(view, 0, e.to_string()),
        }
    }

    /// The message under path edit `side` of `view`.
    pub(super) fn base_error(&mut self, view: ViewId, side: usize, message: String) {
        if let Some(ViewModel::Folder(m)) = self.tab_mut(view).and_then(|t| t.model_mut()) {
            m.path_edit(side).error = Some(message);
        }
    }

    /// The view's current session, changed by `change` in a store job, reopens in its tab (W4b's path: origin kept;
    /// dirty when it differs from the loaded file).
    pub(super) fn reopen_changed(
        &mut self,
        view: ViewId,
        title: &str,
        change: impl FnOnce(&Service, &mut Session) -> Result<(), ApiError> + Send + 'static,
    ) {
        let Some(tab) = self.tab(view) else {
            return;
        };
        let Some(capture) = AppCore::capture(tab) else {
            return;
        };
        let vs = tab.session.as_ref();
        let name = vs
            .and_then(|s| s.reference().map(ToString::to_string))
            .unwrap_or_default();
        let loaded = vs.and_then(|s| s.loaded.clone());
        if let Some(vs) = self.tab_mut(view).and_then(|t| t.session.as_mut()) {
            vs.settings_pending = true;
        }
        self.store(title, Some(view), move |svc, _| {
            let (mut session, _) = capture.session(svc)?;
            change(svc, &mut session)?;
            let session = stored_specs(svc, session)?;
            let opened = svc.session_view_of(&session, name)?;
            let dirty = loaded.as_ref().is_some_and(|l| is_dirty(l, &session));
            Ok(StoreDone::Reopen {
                view,
                opened: Box::new(opened),
                session: Box::new(session),
                dirty,
            })
        });
    }
}

impl AppCore {
    /// Opens the browse dialog in `window` (D-240) in `mode` at `start` for `target`, with the active folder view's
    /// bases among the places and *Show hidden items* as remembered (`state.dialogs.browse.show_hidden`).
    pub fn open_browse(
        &mut self,
        window: WindowId,
        mode: BrowseMode,
        start: String,
        target: BrowseTarget,
    ) {
        let bases = match self.active_tab_mut(window).and_then(|t| t.model_mut()) {
            Some(ViewModel::Folder(m)) => vec![
                m.path_edit(0).committed.clone(),
                m.path_edit(1).committed.clone(),
            ],
            _ => Vec::new(),
        };
        let show_hidden = self.state.dialog_text(BROWSE_DIALOG, "show_hidden") == Some("true");
        let dialog = BrowseDialog::new(mode, &start, target, bases, show_hidden, self.wake());
        if let Some(w) = self.window_mut(window) {
            w.dialogs.push(Box::new(dialog));
        }
    }

    /// `BrowseLeft` / `BrowseRight` (side 0, 1): the browse dialog in folder mode at the field's folder.
    pub(super) fn browse_base(&mut self, window: WindowId, side: usize) {
        let Some(tab) = self.active_tab_mut(window) else {
            return;
        };
        let view = tab.id;
        let Some(ViewModel::Folder(m)) = tab.model_mut() else {
            return;
        };
        let start = m.path_edit(side).text.clone();
        self.open_browse(
            window,
            BrowseMode::Folder,
            start,
            BrowseTarget::PathEdit { view, side },
        );
    }

    /// The browse dialog closed: its answer goes to its target (a path edit commits it as if typed and accepted);
    /// a confirmed folder enters `mru.folders`, a file `mru.files`; *Show hidden items* is remembered.
    pub(super) fn browsed(
        &mut self,
        window: WindowId,
        target: BrowseTarget,
        answer: Option<(String, bool)>,
        show_hidden: bool,
    ) {
        self.state.set_dialog_text(
            BROWSE_DIALOG,
            "show_hidden",
            if show_hidden { "true" } else { "false" },
        );
        let Some((spec, folder)) = answer else {
            return;
        };
        self.push_mru(
            if folder {
                MruList::Folders
            } else {
                MruList::Files
            },
            spec.clone(),
        );
        match target {
            BrowseTarget::PathEdit { view, side } => {
                if let Some(ViewModel::Folder(m)) = self.tab_mut(view).and_then(|t| t.model_mut()) {
                    m.path_edit(side).set_text(&spec);
                    m.accept_paths();
                }
                // The view's outbox is read with its next poll.
                let _ = window;
            }
            BrowseTarget::Export => {
                if let Some(d) = self
                    .window_mut(window)
                    .and_then(|w| w.dialogs.find_mut::<crate::wizards::ExportWizard>())
                {
                    d.target = spec;
                }
            }
            BrowseTarget::RunScript => {
                if let Some(d) = self.window_mut(window).and_then(|w| {
                    w.dialogs
                        .find_mut::<crate::script_dialog::RunScriptDialog>()
                }) {
                    d.path = spec;
                }
            }
        }
    }
}
