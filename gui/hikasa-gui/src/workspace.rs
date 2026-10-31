//! Workspaces in the window (M2 W4b, §1.4.12; D-125, D-126): the Save Workspace As and Manage Workspaces dialogs,
//! the capture of the open windows and the pending tabs of a restored workspace. Toolkit-free (§1.4.16); the app
//! core runs the jobs, `ui/workspace.rs` draws the dialogs.

use std::any::Any;

use hikasa_session::Service;
use hikasa_session::api::{WorkspaceItemDto, WorkspaceLoadDto};
use hikasa_session::ipc::{ViewId, ViewInfo};
use hikasa_session::sessions::OpaqueTable;
use hikasa_session::sessions::{AutoId, SessionName, label};
use hikasa_session::settings::Shortcut;
use hikasa_session::settings::state::WindowState;
use hikasa_session::workspaces::{RestoredTab, WorkspaceTab, WorkspaceWindow};

use crate::dialog::{ButtonRole, DialogButton, DialogModel, DialogStep, Followup};
use crate::session_dialog::SessionAction;
use crate::shared::Shared;
use crate::view_session::{ViewCapture, merged_with_file};
use crate::window::PendingTab;

fn button(label: &str, role: ButtonRole, enabled: bool, default: bool) -> DialogButton {
    DialogButton {
        label: label.to_owned(),
        role,
        enabled,
        default,
    }
}

/// Save Workspace As (§1.4.12): a name, the existing ones offered; an overwrite is confirmed by the window.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SaveWorkspaceDialog {
    /// The existing names.
    pub names: Vec<String>,
    /// The name.
    pub name: String,
    /// Why Save did not close.
    pub error: Option<String>,
}

impl DialogModel for SaveWorkspaceDialog {
    fn title(&self) -> String {
        "Save Workspace As".to_owned()
    }

    fn help_topic(&self) -> &'static str {
        "dialog-workspaces"
    }

    fn buttons(&self) -> Vec<DialogButton> {
        vec![
            button("Save", ButtonRole::Ok, !self.name.trim().is_empty(), true),
            button("Cancel", ButtonRole::Cancel, true, false),
        ]
    }

    fn on_button(&mut self, role: ButtonRole, _svc: &Service) -> DialogStep {
        if role != ButtonRole::Ok {
            return DialogStep::Close;
        }
        match SessionName::parse(self.name.trim()) {
            Ok(name) => {
                DialogStep::CloseWith(vec![Followup::Session(SessionAction::WorkspaceSave {
                    name,
                    confirmed: false,
                })])
            }
            Err(e) => {
                self.error = Some(e.to_string());
                DialogStep::Stay
            }
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// The buttons of Manage Workspaces other than Close.
pub const MANAGE_LOAD: ButtonRole = ButtonRole::Other(1);
/// Rename.
pub const MANAGE_RENAME: ButtonRole = ButtonRole::Other(2);
/// Delete.
pub const MANAGE_DELETE: ButtonRole = ButtonRole::Other(3);
/// Set the shortcut.
pub const MANAGE_SHORTCUT: ButtonRole = ButtonRole::Other(4);

/// Manage Workspaces (§1.4.12).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ManageWorkspacesDialog {
    /// The workspaces.
    pub rows: Vec<WorkspaceItemDto>,
    /// The selected row.
    pub selected: Option<usize>,
    /// The new name.
    pub rename: String,
    /// The shortcut as typed.
    pub shortcut: String,
    /// The effective global shortcuts: shortcut, command caption.
    pub globals: Vec<(Shortcut, String)>,
    /// Why a button did nothing.
    pub error: Option<String>,
}

impl ManageWorkspacesDialog {
    /// Selects a row: its name and shortcut fill the fields.
    pub fn select(&mut self, row: usize) {
        if let Some(item) = self.rows.get(row) {
            self.selected = Some(row);
            self.rename.clone_from(&item.name);
            self.shortcut.clone_from(&item.shortcut);
            self.error = None;
        }
    }

    fn selected_name(&self) -> Option<SessionName> {
        let item = self.rows.get(self.selected?)?;
        SessionName::parse(&item.name).ok()
    }

    /// What the typed shortcut conflicts with: global commands and other workspaces (shown, never blocking).
    pub fn conflicts(&self) -> Vec<String> {
        if self.shortcut.trim().is_empty() {
            return Vec::new();
        }
        let Ok(wanted) = Shortcut::parse(self.shortcut.trim()) else {
            return vec![format!("`{}` is not a shortcut.", self.shortcut.trim())];
        };
        let mut out: Vec<String> = self
            .globals
            .iter()
            .filter(|(s, _)| *s == wanted)
            .map(|(_, caption)| format!("{wanted} is also the command {caption}."))
            .collect();
        let own = self.selected;
        for (i, item) in self.rows.iter().enumerate() {
            if Some(i) != own && Shortcut::parse(&item.shortcut).is_ok_and(|s| s == wanted) {
                out.push(format!("{wanted} is also the workspace {}.", item.name));
            }
        }
        out
    }
}

impl DialogModel for ManageWorkspacesDialog {
    fn title(&self) -> String {
        "Manage Workspaces".to_owned()
    }

    fn help_topic(&self) -> &'static str {
        "dialog-workspaces"
    }

    fn buttons(&self) -> Vec<DialogButton> {
        let one = self.selected.is_some();
        vec![
            button("Load", MANAGE_LOAD, one, false),
            button("Rename", MANAGE_RENAME, one, false),
            button("Delete", MANAGE_DELETE, one, false),
            button("Set Shortcut", MANAGE_SHORTCUT, one, false),
            button("Close", ButtonRole::Close, true, true),
        ]
    }

    fn on_button(&mut self, role: ButtonRole, _svc: &Service) -> DialogStep {
        if role == ButtonRole::Close || role == ButtonRole::Cancel {
            return DialogStep::Close;
        }
        let Some(name) = self.selected_name() else {
            return DialogStep::Stay;
        };
        let action = match role {
            MANAGE_LOAD => SessionAction::WorkspaceLoad(name),
            MANAGE_RENAME => match SessionName::parse(self.rename.trim()) {
                Ok(to) => SessionAction::WorkspaceRename(name, to),
                Err(e) => {
                    self.error = Some(e.to_string());
                    return DialogStep::Stay;
                }
            },
            MANAGE_DELETE => SessionAction::WorkspaceAskDelete(name),
            MANAGE_SHORTCUT => {
                let text = self.shortcut.trim().to_owned();
                if !text.is_empty() && Shortcut::parse(&text).is_err() {
                    self.error = Some(format!("`{text}` is not a shortcut."));
                    return DialogStep::Stay;
                }
                SessionAction::WorkspaceShortcut(name, text)
            }
            _ => return DialogStep::Stay,
        };
        DialogStep::CloseWith(vec![Followup::Session(action)])
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// One tab as the UI thread hands it to the save job (D-125).
#[derive(Debug, Clone)]
pub enum CapturedTab {
    /// A named session.
    Named(String, OpaqueTable),
    /// An unsaved or auto-saved view.
    Inline {
        /// What the job needs.
        capture: Box<ViewCapture>,
        /// The table it was restored from.
        previous: Option<OpaqueTable>,
        /// Its auto-saved file: its identity (overrides, favorite) stays.
        slot: Option<AutoId>,
    },
    /// A tab not restored yet.
    Pending(Box<PendingTab>),
}

/// One window as the UI thread hands it to the save job.
#[derive(Debug, Clone)]
pub struct CapturedWindow {
    /// Its geometry.
    pub geometry: WindowState,
    /// The active tab among the saved tabs.
    pub active: u32,
    /// The saved tabs.
    pub tabs: Vec<CapturedTab>,
}

impl CapturedWindow {
    /// The workspace window (on the job).
    pub fn window(&self, service: &Service) -> Result<WorkspaceWindow, hikasa_session::ApiError> {
        let mut tabs = Vec::with_capacity(self.tabs.len());
        for tab in &self.tabs {
            tabs.push(match tab {
                CapturedTab::Named(session, view) => WorkspaceTab {
                    session: session.clone(),
                    view: view.clone(),
                    ..WorkspaceTab::default()
                },
                CapturedTab::Inline {
                    capture,
                    previous,
                    slot,
                } => {
                    let mut session = capture.session(service)?.0;
                    let file = slot
                        .as_ref()
                        .and_then(|id| service.session_load_autosaved(id).ok());
                    if let Some(file) = file {
                        session = merged_with_file(session, &file.value);
                        session.locked = false;
                    }
                    WorkspaceTab {
                        inline: service.inline_table(&session, previous.as_ref())?,
                        view: capture.view.clone(),
                        ..WorkspaceTab::default()
                    }
                }
                CapturedTab::Pending(p) => match &p.reference {
                    Some(r) => WorkspaceTab {
                        session: r.to_string(),
                        view: p.view.clone(),
                        ..WorkspaceTab::default()
                    },
                    None => WorkspaceTab {
                        inline: service.inline_table(&p.session, Some(&p.inline))?,
                        view: p.view.clone(),
                        ..WorkspaceTab::default()
                    },
                },
            });
        }
        Ok(WorkspaceWindow {
            width: i64::from(self.geometry.width),
            height: i64::from(self.geometry.height),
            x: i64::from(self.geometry.x),
            y: i64::from(self.geometry.y),
            maximized: self.geometry.maximized,
            active_tab: self.active,
            tabs,
        })
    }
}

/// The pending tab of a restored tab: `Ok(None)` for a Home tab, `Err` for a tab this GUI cannot show.
pub fn pending_tab(tab: &RestoredTab, stored: &WorkspaceTab) -> Result<Option<PendingTab>, String> {
    let (reference, session) = match tab {
        RestoredTab::Home => return Ok(None),
        RestoredTab::Named { reference, session } => (Some(reference.clone()), session),
        RestoredTab::Inline(session) => (None, session),
    };
    let name = reference
        .as_ref()
        .map_or_else(|| label(session), ToString::to_string);
    if !session.kind.view_kind().is_served_in_m1() {
        return Err(format!(
            "the tab `{name}` cannot be opened: {} sessions are not available in this version",
            session.kind
        ));
    }
    if session.specs.left.is_empty() || session.specs.right.is_empty() {
        return Err(format!("the tab `{name}` has no specs to compare"));
    }
    Ok(Some(PendingTab {
        kind: session.kind,
        reference,
        session: (**session).clone(),
        caption: name,
        view: stored.view.clone(),
        inline: stored.inline.clone(),
        starting: false,
    }))
}

/// Registers the tabs of a restored workspace with the IPC view list (D-126); per window and tab the view id, or
/// `None` for a Home tab or one this GUI cannot show.
pub fn register_tabs(shared: &Shared, load: &WorkspaceLoadDto) -> Vec<Vec<Option<ViewId>>> {
    load.windows
        .iter()
        .enumerate()
        .map(|(w, window)| {
            window
                .tabs
                .iter()
                .enumerate()
                .map(|(t, tab)| {
                    let stored = &load.workspace.windows[w].tabs[t];
                    let pending = pending_tab(tab, stored).ok().flatten()?;
                    let view = shared.allocate();
                    shared.register(
                        ViewInfo {
                            view,
                            kind: pending.kind.view_kind(),
                            title: pending.caption.clone(),
                            specs: vec![
                                pending.session.specs.left.clone(),
                                pending.session.specs.right.clone(),
                            ],
                        },
                        None,
                    );
                    Some(view)
                })
                .collect()
        })
        .collect()
}
