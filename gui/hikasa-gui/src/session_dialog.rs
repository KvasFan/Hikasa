//! The session dialogs (M2 W4b, §1.4.9, §1.4.10; D-120, D-122): Save Session As, the name prompts of the Home tree
//! and the Session Settings dialog with its tab registry and scope selector. Toolkit-free models over the
//! framework of D-112: they validate what is pure and hand the rest to the app core as a [`SessionAction`], which
//! runs it on a job (D-105). `ui/session_dialog.rs` draws them.

use std::any::Any;

use hikasa_formats::TextEncoding;
use hikasa_session::Service;
use hikasa_session::ipc::ViewId;
use hikasa_session::sessions::{
    AutoId, Scope, Session, SessionName, SessionRef, SessionRules, SessionType, parse_folder,
};

use crate::dialog::{ButtonRole, DialogButton, DialogModel, DialogStep, Followup};

/// What a Save As saves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveSource {
    /// A view (origin becomes `Named`, its auto-saved slot is deleted).
    View(ViewId),
    /// A named session of the tree.
    Named(SessionRef),
    /// An auto-saved session of the tree.
    AutoSaved(AutoId),
    /// The management panel of a Home tab.
    Panel(ViewId),
}

/// A node of the Home tree a command acts on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeTarget {
    /// A named session.
    Named(SessionRef),
    /// A folder (its path).
    Folder(String),
    /// An auto-saved session.
    AutoSaved(AutoId),
}

/// What a session dialog asks the window to do.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionAction {
    /// The answer to a close question (D-84).
    CloseAnswer(ButtonRole),
    /// "Save changes to locked session?": Yes.
    SaveLocked(ViewId),
    /// Save As; `confirmed` once an overwrite was confirmed.
    SaveAs {
        /// What is saved.
        source: SaveSource,
        /// The new reference.
        reference: SessionRef,
        /// An existing file may be replaced.
        confirmed: bool,
    },
    /// `TreeNewSession`.
    Create {
        /// The reference.
        reference: SessionRef,
        /// The type.
        kind: SessionType,
    },
    /// `TreeNewFolder`.
    NewFolder(String),
    /// `TreeDelete`.
    Delete {
        /// The node.
        target: TreeTarget,
        /// Also a locked session.
        force: bool,
    },
    /// `TreeRename` or a drag.
    Rename {
        /// The node.
        target: TreeTarget,
        /// The new reference or folder path.
        to: String,
        /// Also a locked session.
        force: bool,
    },
    /// OK of the Session Settings dialog.
    Settings(Box<SettingsResult>),
    /// Save Workspace As; `confirmed` once an overwrite was confirmed.
    WorkspaceSave {
        /// The name.
        name: SessionName,
        /// An existing file may be replaced.
        confirmed: bool,
    },
    /// Load a workspace.
    WorkspaceLoad(SessionName),
    /// Rename a workspace.
    WorkspaceRename(SessionName, SessionName),
    /// Delete a workspace (after its prompt).
    WorkspaceDelete(SessionName),
    /// Ask before deleting a workspace.
    WorkspaceAskDelete(SessionName),
    /// Set a workspace's shortcut.
    WorkspaceShortcut(SessionName, String),
    /// "Folder … does not exist. Create it?": Yes (M2 W9a, D-239): create it, then re-root the view.
    CreateBase {
        /// The folder view.
        view: hikasa_session::ipc::ViewId,
        /// Both specs.
        specs: [String; 2],
        /// The side whose folder is created (0 left, 1 right).
        side: usize,
    },
    /// The same question: No; the message stays under the field.
    BaseError {
        /// The folder view.
        view: hikasa_session::ipc::ViewId,
        /// The side (0 left, 1 right).
        side: usize,
        /// The message.
        message: String,
    },
}

/// One button.
fn button(label: &str, role: ButtonRole, enabled: bool, default: bool) -> DialogButton {
    DialogButton {
        label: label.to_owned(),
        role,
        enabled,
        default,
    }
}

/// The folder of `reference` joined with a name.
fn join(folder: &str, name: &str) -> String {
    if folder.is_empty() {
        name.to_owned()
    } else {
        format!("{folder}/{name}")
    }
}

/// Save Session As (§1.4.9): name, folder (existing folders, a new folder); an existing name is confirmed by the
/// window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveSessionDialog {
    /// What is saved.
    pub source: SaveSource,
    /// The folders offered.
    pub folders: Vec<String>,
    /// The existing references (offered, and how an overwrite is noticed).
    pub names: Vec<String>,
    /// The name.
    pub name: String,
    /// The folder chosen (`""`: the top level).
    pub folder: String,
    /// A new folder inside it.
    pub new_folder: String,
    /// Why OK did not close.
    pub error: Option<String>,
}

impl SaveSessionDialog {
    /// The dialog.
    pub fn new(source: SaveSource, folders: Vec<String>, names: Vec<String>) -> SaveSessionDialog {
        SaveSessionDialog {
            source,
            folders,
            names,
            name: String::new(),
            folder: String::new(),
            new_folder: String::new(),
            error: None,
        }
    }

    /// The reference of the fields.
    pub fn reference(&self) -> Result<SessionRef, String> {
        let folder = if self.new_folder.trim().is_empty() {
            self.folder.clone()
        } else {
            join(&self.folder, self.new_folder.trim())
        };
        SessionRef::parse(&join(&folder, self.name.trim())).map_err(|e| e.to_string())
    }
}

impl DialogModel for SaveSessionDialog {
    fn title(&self) -> String {
        "Save Session As".to_owned()
    }

    fn help_topic(&self) -> &'static str {
        "dialog-save-session"
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
        match self.reference() {
            Err(e) => {
                self.error = Some(e);
                DialogStep::Stay
            }
            Ok(reference) => {
                DialogStep::CloseWith(vec![Followup::Session(SessionAction::SaveAs {
                    source: self.source.clone(),
                    reference,
                    confirmed: false,
                })])
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

/// What a [`NameDialog`] names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamePurpose {
    /// A new session of a type.
    Session(SessionType),
    /// A new folder.
    Folder,
}

/// The name prompt of `TreeNewSession` and `TreeNewFolder`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameDialog {
    /// What it names.
    pub purpose: NamePurpose,
    /// The folder it goes into (`""`: the top level).
    pub folder: String,
    /// The name.
    pub name: String,
    /// Why OK did not close.
    pub error: Option<String>,
}

impl NameDialog {
    /// The dialog.
    pub fn new(purpose: NamePurpose, folder: String) -> NameDialog {
        NameDialog {
            purpose,
            folder,
            name: String::new(),
            error: None,
        }
    }
}

impl DialogModel for NameDialog {
    fn title(&self) -> String {
        match self.purpose {
            NamePurpose::Session(_) => "New Session".to_owned(),
            NamePurpose::Folder => "New Folder".to_owned(),
        }
    }

    fn help_topic(&self) -> &'static str {
        "view-home"
    }

    fn buttons(&self) -> Vec<DialogButton> {
        vec![
            button("OK", ButtonRole::Ok, !self.name.trim().is_empty(), true),
            button("Cancel", ButtonRole::Cancel, true, false),
        ]
    }

    fn on_button(&mut self, role: ButtonRole, _svc: &Service) -> DialogStep {
        if role != ButtonRole::Ok {
            return DialogStep::Close;
        }
        let full = join(&self.folder, self.name.trim());
        let action = match self.purpose {
            NamePurpose::Session(kind) => {
                SessionRef::parse(&full).map(|reference| SessionAction::Create { reference, kind })
            }
            NamePurpose::Folder => parse_folder(&full).map(|_| SessionAction::NewFolder(full)),
        };
        match action {
            Ok(action) => DialogStep::CloseWith(vec![Followup::Session(action)]),
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

/// What the Session Settings dialog edits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditTarget {
    /// A view (`SessionEdit`).
    View(ViewId),
    /// A named session (`TreeEdit`).
    Named(SessionRef),
    /// The defaults of a type (`TreeEdit`): specs disabled (D-82).
    Defaults(SessionType),
}

/// A tab of the Session Settings dialog (D-122): W4 provides Specs; W9, W11 and W13 register their rule tabs in
/// [`tabs`]. Draw-independent: the drawing layer downcasts.
pub trait SessionTab: Any + Send {
    /// The caption.
    fn title(&self) -> String;
    /// The help topic.
    fn help_topic(&self) -> &'static str {
        "dialog-session-settings"
    }
    /// True when this tab changed the rules.
    fn changes_rules(&self) -> bool;
    /// The rules as this tab leaves them, given the session's (`None`: unchanged).
    fn rules(&self, current: &SessionRules) -> Option<SessionRules>;
    /// Fills the tab from the session's rules (M2 W9c, D-249). Default: nothing.
    fn load(&mut self, _rules: &SessionRules) {}
    /// What the window knows that the rules do not hold: the formats, the presets (a job before the dialog opens).
    fn prepare(&mut self, _extras: &TabExtras) {}
    /// Checks the tab's edits over `current` with core (`check_folder_rules`, `SyncMatrix::validate`): its message
    /// becomes [`SessionTab::error`].
    fn check(&mut self, _svc: &Service, _current: &SessionRules) {}
    /// Why OK is off: core's message, shown on this tab.
    fn error(&self) -> Option<&str> {
        None
    }
    /// For the drawing layer.
    fn as_any(&self) -> &dyn Any;
    /// For the drawing layer.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

/// What the rule tabs read besides the session (M2 W9c): the registry's formats and the name-filter presets.
#[derive(Debug, Clone, Default)]
pub struct TabExtras {
    /// `format list`: id, name, globally enabled.
    pub formats: Vec<(String, String, bool)>,
    /// `filters.toml`: name, mask.
    pub presets: Vec<(String, String)>,
}

/// The Specs tab (§1.4.10).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SpecsTab {
    /// A Browse button was pressed (0 left, 1 right; the window opens the browse dialog, D-240).
    pub browse: Option<usize>,
    /// *Disable editing* per side (M2 W9c, FC-003): `read_only_left`, `read_only_right`; offered for folder sessions.
    pub read_only: [bool; 2],
    /// Left.
    pub left: String,
    /// Right.
    pub right: String,
    /// The description.
    pub description: String,
    /// The format id (`""`: detect).
    pub format: String,
    /// The left encoding (`""`: detect).
    pub left_encoding: String,
    /// The right encoding (`""`: detect).
    pub right_encoding: String,
    /// The specs are disabled (a defaults entry, D-82).
    pub specs_disabled: bool,
    /// Format and encodings apply (file types).
    pub file_type: bool,
    /// The MRU list of the left side.
    pub mru_left: Vec<String>,
    /// The MRU list of the right side.
    pub mru_right: Vec<String>,
    /// The formats offered: id, name.
    pub formats: Vec<(String, String)>,
}

impl SpecsTab {
    /// The first encoding label that does not exist.
    pub fn bad_encoding(&self) -> Option<&str> {
        [&self.left_encoding, &self.right_encoding]
            .into_iter()
            .map(String::as_str)
            .find(|label| !label.is_empty() && TextEncoding::for_label(label).is_none())
    }
}

impl SessionTab for SpecsTab {
    fn title(&self) -> String {
        "Specs".to_owned()
    }

    fn changes_rules(&self) -> bool {
        false
    }

    fn rules(&self, _current: &SessionRules) -> Option<SessionRules> {
        None
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// The tab registry of a session type (D-122): Specs, then the rule tabs (W9c: the folder tabs, D-249; W11 and W13 add
/// theirs here).
pub fn tabs(kind: SessionType) -> Vec<Box<dyn SessionTab>> {
    let mut tabs: Vec<Box<dyn SessionTab>> = vec![Box::new(SpecsTab::default())];
    tabs.extend(crate::folder_tabs::rule_tabs(kind));
    tabs
}

/// What OK of the Session Settings dialog changes.
#[derive(Debug, Clone, PartialEq)]
pub struct SettingsResult {
    /// What was edited.
    pub target: EditTarget,
    /// The session with the dialog's specs and description (specs as typed).
    pub session: Session,
    /// The specs changed.
    pub specs_changed: bool,
    /// The rules changed, with the scope chosen.
    pub rules: Option<(Scope, SessionRules)>,
    /// The parent link's text.
    pub parent: Option<String>,
}

/// The scopes of the footer (SES-021).
pub const SCOPES: [(Scope, &str); 4] = [
    (Scope::View, "This view only"),
    (Scope::Defaults, "Update session defaults"),
    (Scope::Pair, "These files within parent"),
    (Scope::Type, "All files within parent"),
];

impl SessionSettingsDialog {
    /// The Specs tab's Browse button: the side, the field's text and whether a folder is picked (D-240).
    pub fn take_browse(&mut self) -> Option<(usize, String, bool)> {
        let folder = matches!(
            self.session.kind,
            SessionType::FolderCompare | SessionType::FolderSync
        );
        let specs = self.specs_mut();
        let side = specs.browse.take()?;
        let start = if side == 0 {
            specs.left.clone()
        } else {
            specs.right.clone()
        };
        Some((side, start, folder))
    }
}

/// The Session Settings dialog (D-122).
pub struct SessionSettingsDialog {
    /// What it edits.
    pub target: EditTarget,
    /// The session as it was when the dialog opened.
    pub session: Session,
    /// The tabs.
    pub tabs: Vec<Box<dyn SessionTab>>,
    /// The tab shown.
    pub active: usize,
    /// The scope chosen.
    pub scope: Scope,
    /// The parent is a named or auto-saved folder session.
    pub folder_parent: bool,
    /// The parent link's text.
    pub parent: Option<String>,
    /// Why OK did not close.
    pub error: Option<String>,
    /// The rules the tabs were last checked with (`poll` checks again when they change).
    checked: Option<Option<SessionRules>>,
}

impl SessionSettingsDialog {
    /// The dialog over `session`.
    pub fn new(
        target: EditTarget,
        session: Session,
        folder_parent: bool,
        parent: Option<String>,
        mru: [Vec<String>; 2],
        formats: Vec<(String, String)>,
    ) -> SessionSettingsDialog {
        let mut tabs = tabs(session.kind);
        let [mru_left, mru_right] = mru;
        if let Some(specs) = tabs[0].as_any_mut().downcast_mut::<SpecsTab>() {
            *specs = SpecsTab {
                browse: None,
                read_only: [session.specs.read_only_left, session.specs.read_only_right],
                left: session.specs.left.clone(),
                right: session.specs.right.clone(),
                description: session.description.clone(),
                format: session.specs.format.clone(),
                left_encoding: session.specs.left_encoding.clone(),
                right_encoding: session.specs.right_encoding.clone(),
                specs_disabled: matches!(target, EditTarget::Defaults(_)),
                file_type: !session.kind.is_folder(),
                mru_left,
                mru_right,
                formats,
            };
        }
        for tab in &mut tabs {
            tab.load(&session.rules);
        }
        SessionSettingsDialog {
            target,
            session,
            tabs,
            active: 0,
            scope: Scope::View,
            folder_parent,
            parent,
            error: None,
            checked: None,
        }
    }

    /// The Specs tab.
    pub fn specs(&self) -> &SpecsTab {
        self.tabs
            .iter()
            .find_map(|t| t.as_any().downcast_ref())
            .expect("the registry always holds the Specs tab")
    }

    /// The Specs tab, mutable.
    pub fn specs_mut(&mut self) -> &mut SpecsTab {
        self.tabs
            .iter_mut()
            .find_map(|t| t.as_any_mut().downcast_mut())
            .expect("the registry always holds the Specs tab")
    }

    /// The help topic (tests).
    #[cfg(test)]
    pub fn help_topic_for_test(&self) -> &'static str {
        self.help_topic()
    }

    fn rules_changed(&self) -> bool {
        self.tabs.iter().any(|t| t.changes_rules())
    }

    /// Gives the rule tabs what the window knows (the formats of `format list`, the presets): a job, before the dialog
    /// opens (M2 W9c).
    pub fn prepare_rule_tabs(&mut self, svc: &Service, presets: Vec<(String, String)>) {
        let formats = svc
            .format_list()
            .formats
            .into_iter()
            .map(|f| (f.id, f.name, f.enabled))
            .collect();
        let extras = TabExtras { formats, presets };
        for tab in &mut self.tabs {
            tab.prepare(&extras);
        }
    }

    /// Shows the tab of that title (`Rules`: Comparison, `NameFilter`: Name Filters); unknown: the first.
    pub fn show_tab(&mut self, title: &str) {
        self.active = self
            .tabs
            .iter()
            .position(|t| t.title() == title)
            .unwrap_or(0);
    }

    /// The tab of type `T`, mutable (the drawing layer and the tests).
    pub fn tab_mut<T: SessionTab>(&mut self) -> Option<&mut T> {
        self.tabs
            .iter_mut()
            .find_map(|t| t.as_any_mut().downcast_mut())
    }

    /// The tab of type `T`.
    pub fn tab<T: SessionTab>(&self) -> Option<&T> {
        self.tabs.iter().find_map(|t| t.as_any().downcast_ref())
    }

    /// Core checks every rule tab's edits over the session's rules (D-249): each refusal stays on its tab.
    pub fn check_tabs(&mut self, svc: &Service) {
        let current = self.session.rules.clone();
        for tab in &mut self.tabs {
            tab.check(svc, &current);
        }
        self.checked = Some(self.signature());
    }

    /// The first tab with an error.
    pub fn tab_error(&self) -> Option<(usize, &str)> {
        self.tabs
            .iter()
            .enumerate()
            .find_map(|(k, t)| t.error().map(|e| (k, e)))
    }

    /// What the tabs would write (`None` while a field does not parse); a change of it is checked again at the next
    /// poll.
    fn signature(&self) -> Option<SessionRules> {
        let mut rules = self.session.rules.clone();
        for tab in self.tabs.iter().filter(|t| t.changes_rules()) {
            match tab.rules(&rules) {
                Some(next) => rules = next,
                None => return None,
            }
        }
        Some(rules)
    }

    /// True when `scope` can be chosen now (§1.4.10): *this view only* always; the others only while a rule tab
    /// changed something, and `Pair`/`Type` only with a named or auto-saved folder parent.
    pub fn scope_available(&self, scope: Scope) -> bool {
        match scope {
            Scope::View => true,
            Scope::Defaults => self.rules_changed(),
            Scope::Pair | Scope::Type => self.rules_changed() && self.folder_parent,
        }
    }

    fn result(&self) -> Result<SettingsResult, String> {
        let specs = self.specs();
        if let Some(label) = specs.bad_encoding() {
            return Err(format!("`{label}` is not a text encoding."));
        }
        let mut session = self.session.clone();
        let before = session.specs.clone();
        if !specs.specs_disabled {
            session.specs.left.clone_from(&specs.left);
            session.specs.right.clone_from(&specs.right);
            [session.specs.read_only_left, session.specs.read_only_right] = specs.read_only;
            if specs.file_type {
                session.specs.format.clone_from(&specs.format);
                session.specs.left_encoding.clone_from(&specs.left_encoding);
                session
                    .specs
                    .right_encoding
                    .clone_from(&specs.right_encoding);
            }
        }
        session.description.clone_from(&specs.description);
        // Each rule tab writes its own keys over what the tabs before it left (D-249).
        let mut rules = None;
        for tab in self.tabs.iter().filter(|t| t.changes_rules()) {
            let base = rules.as_ref().unwrap_or(&session.rules);
            if let Some(next) = tab.rules(base) {
                rules = Some(next);
            }
        }
        let rules = rules.map(|rules| (self.scope, rules));
        Ok(SettingsResult {
            target: self.target.clone(),
            specs_changed: session.specs != before,
            session,
            rules,
            parent: self.parent.clone(),
        })
    }
}

impl DialogModel for SessionSettingsDialog {
    fn poll(&mut self, svc: &Service) -> bool {
        // An edit of a rule tab is checked at the next frame (core's message under the tab, OK off).
        if self.tabs.len() < 2 {
            return false;
        }
        let signature = self.signature();
        // A field that does not parse is checked every time (its message names the text).
        if signature.is_some() && self.checked.as_ref() == Some(&signature) {
            return false;
        }
        let before: Vec<Option<String>> = self
            .tabs
            .iter()
            .map(|t| t.error().map(str::to_owned))
            .collect();
        self.check_tabs(svc);
        let after: Vec<Option<String>> = self
            .tabs
            .iter()
            .map(|t| t.error().map(str::to_owned))
            .collect();
        signature.is_some() || before != after
    }

    fn take_requests(&mut self) -> Vec<crate::dialog::DialogRequest> {
        let mask = self
            .tab_mut::<crate::folder_tabs::NameFiltersTab>()
            .and_then(|t| t.add_to_presets.take());
        match mask {
            Some(mask) => vec![crate::dialog::DialogRequest::AddToPresets(mask)],
            None => Vec::new(),
        }
    }

    fn title(&self) -> String {
        match &self.target {
            EditTarget::View(_) => "Session Settings".to_owned(),
            EditTarget::Named(r) => format!("Session Settings: {r}"),
            EditTarget::Defaults(kind) => format!("Session Settings: defaults of {kind}"),
        }
    }

    fn help_topic(&self) -> &'static str {
        self.tabs
            .get(self.active)
            .map_or("dialog-session-settings", |t| t.help_topic())
    }

    fn buttons(&self) -> Vec<DialogButton> {
        vec![
            button("OK", ButtonRole::Ok, self.tab_error().is_none(), true),
            button("Cancel", ButtonRole::Cancel, true, false),
        ]
    }

    fn on_button(&mut self, role: ButtonRole, svc: &Service) -> DialogStep {
        if role != ButtonRole::Ok {
            return DialogStep::Close;
        }
        // The edits are checked as the comparison would take them (D-249): a refusal shows on its tab.
        self.check_tabs(svc);
        if let Some((k, _)) = self.tab_error() {
            self.active = k;
            return DialogStep::Stay;
        }
        match self.result() {
            Err(e) => {
                self.error = Some(e);
                DialogStep::Stay
            }
            Ok(result) => DialogStep::CloseWith(vec![Followup::Session(SessionAction::Settings(
                Box::new(result),
            ))]),
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
