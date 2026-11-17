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
use crate::commands::CommandId;
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

/// What a re-root of a view with an operation under way answers (§1.9.2 rule 4).
pub const WAIT: &str = "Wait for the current operation to end.";

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
        // `-` and `clipboard://` read standard input or the clipboard: never for a field of the window.
        if svc.reads_input(std::ffi::OsStr::new(spec)) {
            return Ok(BaseCheck::Bad(
                k,
                hikasa_session::api::paths::INPUT_SPEC_REFUSED.to_owned(),
            ));
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
                // The question names the folder it creates: absolute (a relative text is the service's current
                // folder's), and the very spec the creation gets.
                let absolute = hikasa_core::spec::Spec::parse_os(
                    std::ffi::OsStr::new(spec),
                    &svc.spec_context(),
                )
                .map_or_else(|_| spec.clone(), |s| s.canonical());
                return Ok(BaseCheck::Missing(k, absolute));
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
        if self.refuse_busy(view) {
            return;
        }
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
        // A newer Accept supersedes a check still running (§1.9.2 rule 3).
        let generation = {
            let g = self.rebase_generation.entry(view).or_insert(0);
            *g += 1;
            *g
        };
        #[cfg(test)]
        let gate = self.rebase_gate.clone();
        let job = Job::spawn(
            &self.service,
            JobKind::VfsRead,
            "check the base folders".into(),
            &self.wake(),
            move |svc, job| {
                #[cfg(test)]
                if let Some(gate) = &gate {
                    let (open, turn) = &**gate;
                    let mut open = open.lock().expect("gate");
                    while !*open {
                        open = turn.wait(open).expect("gate");
                    }
                }
                check(svc, &job_specs, &changed, create, job)
            },
        );
        self.jobs
            .push(CoreJob::Rebase(view, generation, specs, job));
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
                // An operation that started while the check ran keeps the view (§1.9.2 rule 4).
                if self.refuse_busy(view) {
                    return;
                }
                let old = match self.tab_mut(view).and_then(|t| t.model_mut()) {
                    Some(ViewModel::Folder(m)) => [
                        m.path_edit(0).committed.clone(),
                        m.path_edit(1).committed.clone(),
                    ],
                    _ => return,
                };
                // Pushed once the view runs on the new base (a base that loads, D-239), not before.
                let waiting: Vec<(MruList, String)> = [(0, MruList::Left), (1, MruList::Right)]
                    .into_iter()
                    .filter(|&(k, _)| specs[k] != old[k])
                    .map(|(k, list)| (list, specs[k].clone()))
                    .collect();
                self.mru_after_load.insert(view, (false, waiting));
                self.reopen_changed(view, "change the base folders", Some(specs), |_, _| Ok(()));
            }
            Ok(BaseCheck::Missing(k, path)) => {
                let Some(window) = self.window_of(view) else {
                    return;
                };
                // Created as named in the question.
                let mut specs = specs;
                specs[k].clone_from(&path);
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
            Err(e) => {
                // The check itself failed: under the first field that changed.
                let k = match self.tab_mut(view).and_then(|t| t.model_mut()) {
                    Some(ViewModel::Folder(m)) => (0..2)
                        .find(|&k| m.path_edit(k).committed != specs[k])
                        .unwrap_or(0),
                    _ => 0,
                };
                self.base_error(view, k, e.to_string());
            }
        }
    }

    /// True (with the notice "Wait for the current operation to end.") when folder view `view` has an operation, a
    /// refresh or a plan under way or a question of one open: nothing re-roots or reopens it then, so a running
    /// operation is never cancelled by a new base (§1.9.2 rule 4).
    pub(super) fn refuse_busy(&mut self, view: ViewId) -> bool {
        let busy = matches!(
            self.tab(view).and_then(|t| t.model()),
            Some(ViewModel::Folder(m)) if !m.idle()
        );
        if busy {
            let window = self.window_of(view).unwrap_or_else(|| self.focused());
            self.push_notice(
                window,
                crate::notify::Notice::new(
                    format!("busy:{}", view.0),
                    crate::notify::NoticeKind::Warning,
                    WAIT.to_owned(),
                ),
            );
        }
        busy
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
        specs: Option<[String; 2]>,
        change: impl FnOnce(&Service, &mut Session) -> Result<(), ApiError> + Send + 'static,
    ) {
        let Some(tab) = self.tab(view) else {
            return;
        };
        let Some(mut capture) = AppCore::capture(tab) else {
            return;
        };
        // New bases go into the captured request (a cleared view has none to read the session from).
        if let Some([left, right]) = specs
            && let hikasa_session::api::ViewRequest::Folder { request, .. } = &mut capture.request
        {
            request.left = OsString::from(left);
            request.right = OsString::from(right);
        }
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
                rebase: true,
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

    /// A Home tab's Browse button (side 0, 1): folder mode for a folder session (or no session), else file mode.
    pub fn home_browse(&mut self, window: WindowId, side: usize) {
        let Some(tab) = self.active_tab_mut(window) else {
            return;
        };
        let view = tab.id;
        let Some(ViewModel::Home(home)) = tab.model_mut() else {
            return;
        };
        let folder = home.panel.session.as_ref().is_none_or(|s| {
            matches!(
                s.kind,
                hikasa_session::sessions::SessionType::FolderCompare
                    | hikasa_session::sessions::SessionType::FolderSync
            )
        });
        let start = if side == 0 {
            home.panel.left.clone()
        } else {
            home.panel.right.clone()
        };
        let mode = if folder {
            BrowseMode::Folder
        } else {
            BrowseMode::File
        };
        self.open_browse(window, mode, start, BrowseTarget::Home { view, side });
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
    pub(crate) fn browsed(
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
        if let BrowseTarget::TextSaveAs { view, side, local } = target {
            self.text_save_as_answered(window, view, side, local, answer.map(|(spec, _)| spec));
            return;
        }
        if self.text_more_browsed(
            window,
            &target,
            answer.as_ref().map(|(spec, _)| spec.clone()),
        ) {
            return;
        }
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
            BrowseTarget::Home { view, side } => {
                if let Some(ViewModel::Home(home)) = self.tab_mut(view).and_then(|t| t.model_mut())
                {
                    if side == 0 {
                        home.panel.left = spec;
                    } else {
                        home.panel.right = spec;
                    }
                    home.error = None;
                }
            }
            BrowseTarget::Specs(side) => {
                if let Some(specs) = self
                    .window_mut(window)
                    .and_then(|w| {
                        w.dialogs
                            .find_mut::<crate::session_dialog::SessionSettingsDialog>()
                    })
                    .map(|d| d.specs_mut())
                {
                    if side == 0 {
                        specs.left = spec;
                    } else {
                        specs.right = spec;
                    }
                }
            }
            BrowseTarget::OptionKey(key) => {
                if let Some(d) = self
                    .window_mut(window)
                    .and_then(|w| w.dialogs.find_mut::<crate::options::OptionsDialog>())
                {
                    d.set_field(key, hikasa_session::settings::form::FormValue::Text(spec));
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
            BrowseTarget::OpTarget(view) => {
                if let Some(d) = self.window_mut(window).and_then(|w| {
                    w.dialogs
                        .find_mut_where::<crate::op_dialogs::OpDialog>(|d| d.view() == view)
                }) {
                    d.set_target(&spec);
                }
            }
            BrowseTarget::CompareTo(view) => self.compare_to_browsed(window, view, spec),
            // Answered before the recent lists (above).
            BrowseTarget::TextSaveAs { .. }
            | BrowseTarget::TextOpen { .. }
            | BrowseTarget::EditTextFile
            | BrowseTarget::FindFilesRoot => {}
        }
    }
}

impl AppCore {
    /// The base commands of the active view (FC-004, SES-008; D-243): Up One Level, Set as Base Folders, Set as Base
    /// on Other Side and Clear Session re-root the view through its reopen path; Swap Sides exchanges the sides of its
    /// session; Compare Parent Folders opens a new Folder Compare view of the folders holding the row's (the files')
    /// items.
    pub(super) fn base_command(&mut self, window: WindowId, id: CommandId) {
        let Some(tab) = self.active_tab_mut(window) else {
            return;
        };
        let view = tab.id;
        match tab.model_mut() {
            Some(ViewModel::Folder(m)) => {
                let mut specs = [
                    m.path_edit(0).committed.clone(),
                    m.path_edit(1).committed.clone(),
                ];
                match id {
                    CommandId::UpOneLevelLeft
                    | CommandId::UpOneLevelRight
                    | CommandId::UpOneLevelBoth => {
                        let parents = m.base_parents();
                        let sides: &[usize] = match id {
                            CommandId::UpOneLevelLeft => &[0],
                            CommandId::UpOneLevelRight => &[1],
                            _ => &[0, 1],
                        };
                        for &k in sides {
                            let Some(parent) = parents[k].clone() else {
                                return;
                            };
                            specs[k] = parent;
                        }
                        self.rebase(view, specs, None);
                    }
                    CommandId::SetAsBaseFolders => {
                        if let Some(bases) = m.selected_bases() {
                            self.rebase(view, bases, None);
                        }
                    }
                    CommandId::SetAsOtherBaseFolder => {
                        if let Some((k, spec)) = m.selected_other_base() {
                            specs[k] = spec;
                            self.rebase(view, specs, None);
                        }
                    }
                    CommandId::CompareParents => {
                        if let Some(parents) = m.row_parents() {
                            self.open_folders(window, parents);
                        }
                    }
                    CommandId::Swap => self.swap_sides(view),
                    CommandId::SessionClear => self.clear_session(view),
                    _ => {}
                }
            }
            Some(ViewModel::Table(m)) => match id {
                CommandId::Swap => self.swap_sides(view),
                CommandId::CompareParents => {
                    let (left, right) = (m.request().left.clone(), m.request().right.clone());
                    self.open_parents(window, [left, right]);
                }
                _ => {}
            },
            Some(ViewModel::Text(m)) => match id {
                CommandId::Swap => self.swap_sides(view),
                CommandId::CompareParents => {
                    // The specs are parsed in the job (an archive member's resolution reads the archive), never
                    // standard input or the clipboard.
                    let (left, right) = (m.request().left.clone(), m.request().right.clone());
                    self.open_parents(window, [left, right]);
                }
                _ => {}
            },
            _ => {}
        }
    }

    /// Clear Session (D-243): the view becomes an empty session of its type (`session_defaults`): empty path edits,
    /// no comparison, the specs' recent lists offered; Accept runs it. The request is the defaults' with the bases
    /// cleared (a session without specs has no view of its own).
    fn clear_session(&mut self, view: ViewId) {
        let Some(tab) = self.tab(view) else {
            return;
        };
        // An empty view already: nothing to clear (no error).
        if let Some(ViewModel::Folder(m)) = tab.model()
            && m.prepared().is_none()
            && m.committed().iter().all(|s| s.is_empty())
        {
            return;
        }
        let Some(capture) = AppCore::capture(tab) else {
            return;
        };
        if let Some(vs) = self.tab_mut(view).and_then(|t| t.session.as_mut()) {
            vs.settings_pending = true;
        }
        self.store("clear the session", Some(view), move |svc, _| {
            let (current, _) = capture.session(svc)?;
            let mut session = svc.session_defaults(current.kind)?;
            let mut probe = session.clone();
            probe.specs.left.clone_from(&current.specs.left);
            probe.specs.right.clone_from(&current.specs.right);
            let mut opened = svc.session_view_of(&probe, String::new())?;
            if let hikasa_session::api::ViewRequest::Folder { request, .. } = &mut opened.request {
                request.left.clear();
                request.right.clear();
            }
            opened.specs = vec![String::new(), String::new()];
            opened.title = String::new();
            session.specs.left.clear();
            session.specs.right.clear();
            Ok(StoreDone::Reopen {
                view,
                opened: Box::new(opened),
                session: Box::new(session),
                dirty: true,
                rebase: true,
            })
        });
    }

    /// Swap Sides (SES-008): `Session::swap_sides` (specs, encodings, read-only sides, parent paths, the `_left` /
    /// `_right` view keys), then the view runs again (a swapped model is not derived).
    fn swap_sides(&mut self, view: ViewId) {
        self.reopen_changed(view, "swap the sides", None, |_, session| {
            session.swap_sides();
            Ok(())
        });
    }

    /// A new Folder Compare view of the folders holding the files `specs` (a text view's `CompareParents`): the specs
    /// are parsed and resolved in the job; `-` and `clipboard://` have no folder.
    fn open_parents(&mut self, window: WindowId, specs: [OsString; 2]) {
        let job = Job::spawn(
            &self.service,
            JobKind::VfsRead,
            "open the parent folders".into(),
            &self.wake(),
            move |svc, _| {
                let parent = |spec: &OsString| {
                    if svc.reads_input(spec) {
                        return None;
                    }
                    svc.parse_spec(spec)
                        .ok()
                        .and_then(|s| s.parent())
                        .map(|p| p.canonical())
                };
                let (Some(left), Some(right)) = (parent(&specs[0]), parent(&specs[1])) else {
                    return Err(ApiError::InvalidArgument(
                        "These files have no parent folders to compare.".to_owned(),
                    ));
                };
                let request = hikasa_session::api::GuiOpenRequest {
                    specs: vec![OsString::from(left), OsString::from(right)],
                    ..hikasa_session::api::GuiOpenRequest::default()
                };
                crate::home::resolve(svc, &request)
            },
        );
        self.jobs.push(CoreJob::Drop(window, job));
    }

    /// A new Folder Compare view of `folders` in `window` (`CompareParents`), resolved as two dropped paths are.
    fn open_folders(&mut self, window: WindowId, folders: [String; 2]) {
        let request = hikasa_session::api::GuiOpenRequest {
            specs: folders.into_iter().map(OsString::from).collect(),
            ..hikasa_session::api::GuiOpenRequest::default()
        };
        let job = Job::spawn(
            &self.service,
            JobKind::VfsRead,
            "open the parent folders".into(),
            &self.wake(),
            move |svc, _| crate::home::resolve(svc, &request),
        );
        self.jobs.push(CoreJob::Drop(window, job));
    }
}
