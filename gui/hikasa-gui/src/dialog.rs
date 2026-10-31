//! The dialog framework (M2 W4a, D-112; SES-092 framework, UI-006): dialogs are toolkit-free models
//! ([`DialogModel`]) on a stack per window ([`DialogStack`]), drawn by `ui/dialog.rs` as egui windows inside
//! their native window, modal to it. Esc is Cancel (or Close), Enter the default button, F1 the dialog's help
//! topic. [`Prompt`] serves the questions of W4 (multiple-close warnings, conflicts, overwrites) and of later
//! waves; [`AboutDialog`] is Help ▸ About.
//!
//! *W4 note:* a dialog's buttons end with [`DialogStep::CloseWith`] a list of [`Followup`]s rather than plain
//! invocations: closing a window or quitting after a confirmed warning are not catalogue commands, and a
//! confirmed command must not ask again.

use std::any::Any;
use std::path::PathBuf;

use hikasa_session::settings::Shortcut;
use hikasa_session::{ApiError, HIKASA_VERSION, Service};

use crate::options::ConflictAnswer;
use crate::session_dialog::SessionAction;
use crate::window::Invocation;

/// The role of a dialog button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ButtonRole {
    /// OK.
    Ok,
    /// Apply.
    Apply,
    /// Cancel.
    Cancel,
    /// Yes.
    Yes,
    /// No.
    No,
    /// Back (wizards).
    Back,
    /// Next (wizards).
    Next,
    /// Finish (wizards).
    Finish,
    /// Close.
    Close,
    /// A dialog's own button.
    Other(u8),
}

/// One button.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialogButton {
    /// Label.
    pub label: String,
    /// Role.
    pub role: ButtonRole,
    /// Can be pressed.
    pub enabled: bool,
    /// Enter presses it.
    pub default: bool,
}

/// What the window does after a dialog was confirmed.
#[derive(Debug, Clone, PartialEq)]
pub enum Followup {
    /// Runs a command in the dialog's window without asking again.
    Run(Invocation),
    /// Closes the dialog's window without asking again.
    CloseWindow,
    /// Quits without asking again.
    Exit,
    /// "Do not ask again": sets the boolean `options.toml` key (a dotted path below `options`) to `false`.
    DisableOption(&'static str),
    /// A session or workspace action of the window (W4b).
    Session(SessionAction),
    /// The answer to the conflict prompt of the Options dialog (D-128).
    Options(ConflictAnswer),
    /// The overwrite of an existing package was confirmed (the Export wizard, D-131).
    WizardOverwrite,
    /// The Import wizard drops its choices and takes this package (D-127).
    SwitchPackage(PathBuf),
}

/// The body of a write on the window's settings queue: writes and returns the paths to acknowledge.
pub type SettingsBody = Box<dyn FnOnce(&Service) -> Result<Vec<PathBuf>, ApiError> + Send>;

/// What a dialog asks of its window between two calls (W4c): the dialogs that write settings use the window's
/// ordered settings queue instead of a job of their own.
pub enum DialogRequest {
    /// A write on the settings queue (acknowledged, then the settings reload: D-117, D-119).
    Settings {
        /// The job's title.
        title: String,
        /// The write.
        body: SettingsBody,
    },
    /// A prompt over the dialog.
    Ask(Box<Prompt>),
    /// The dialog closes.
    Close,
    /// The Home tree and the menus read the stores again.
    Refresh,
}

/// What a button or key did.
#[derive(Debug, Clone, PartialEq)]
pub enum DialogStep {
    /// The dialog stays open.
    Stay,
    /// The dialog closes.
    Close,
    /// The dialog closes and the window does these.
    CloseWith(Vec<Followup>),
}

/// A dialog: a toolkit-free model tested without a window.
pub trait DialogModel: Any {
    /// The title.
    fn title(&self) -> String;
    /// The help topic F1 shows.
    fn help_topic(&self) -> &'static str;
    /// The buttons, in order.
    fn buttons(&self) -> Vec<DialogButton>;
    /// A button was pressed.
    fn on_button(&mut self, role: ButtonRole, svc: &Service) -> DialogStep;
    /// A key: Esc → Cancel (or Close, or No), Enter → the default button.
    fn on_key(&mut self, key: &Shortcut) -> Option<ButtonRole> {
        default_key(&self.buttons(), key)
    }
    /// The OS close button with this dialog on top; true when the dialog stays because it asks first (the question
    /// is one of its requests), false for a plain Cancel.
    fn on_close(&mut self) -> bool {
        false
    }
    /// Polls the dialog's jobs; true when it changed.
    fn poll(&mut self, _svc: &Service) -> bool {
        false
    }
    /// True while a job of the dialog runs.
    fn busy(&self) -> bool {
        false
    }
    /// The size of the dialog's body (`None`: a small dialog that follows its content).
    fn size(&self) -> Option<(f32, f32)> {
        None
    }
    /// True while every key goes to [`DialogModel::on_key`] (a shortcut capture).
    fn wants_keys(&self) -> bool {
        false
    }
    /// What the dialog asks of its window since the last call.
    fn take_requests(&mut self) -> Vec<DialogRequest> {
        Vec::new()
    }
    /// For the drawing layer (downcast to the concrete dialog).
    fn as_any(&self) -> &dyn Any;
    /// For the drawing layer.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

/// The default key handling of a dialog (D-112): Esc is Cancel, else Close, else No; Enter is the enabled
/// default button. Other keys are the dialog's own.
pub fn default_key(buttons: &[DialogButton], key: &Shortcut) -> Option<ButtonRole> {
    if key.ctrl || key.alt || key.cmd || key.shift {
        return None;
    }
    match key.key.as_str() {
        "Esc" => [ButtonRole::Cancel, ButtonRole::Close, ButtonRole::No]
            .into_iter()
            .find(|r| buttons.iter().any(|b| b.role == *r && b.enabled)),
        "Enter" => buttons
            .iter()
            .find(|b| b.default && b.enabled)
            .map(|b| b.role),
        _ => None,
    }
}

/// The dialogs of one window, the top one active.
#[derive(Default)]
pub struct DialogStack {
    dialogs: Vec<Box<dyn DialogModel>>,
}

impl std::fmt::Debug for DialogStack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list()
            .entries(self.dialogs.iter().map(|d| d.title()))
            .finish()
    }
}

impl DialogStack {
    /// Opens a dialog on top.
    pub fn push(&mut self, dialog: Box<dyn DialogModel>) {
        self.dialogs.push(dialog);
    }

    /// Closes the top dialog.
    pub fn pop(&mut self) -> Option<Box<dyn DialogModel>> {
        self.dialogs.pop()
    }

    /// The dialog `T` of the stack, the top-most one first.
    pub fn find_mut<T: DialogModel>(&mut self) -> Option<&mut T> {
        self.dialogs
            .iter_mut()
            .rev()
            .find_map(|d| d.as_any_mut().downcast_mut::<T>())
    }

    /// True while a dialog's job runs.
    pub fn busy(&self) -> bool {
        self.dialogs.iter().any(|d| d.busy())
    }

    /// True when the top dialog takes every key.
    pub fn wants_keys(&self) -> bool {
        self.dialogs.last().is_some_and(|d| d.wants_keys())
    }

    /// True when the stack holds a dialog `T`.
    pub fn holds<T: DialogModel>(&self) -> bool {
        self.dialogs.iter().any(|d| d.as_any().is::<T>())
    }

    /// Closes dialog `index`.
    pub fn remove(&mut self, index: usize) -> Box<dyn DialogModel> {
        self.dialogs.remove(index)
    }

    /// The requests of every dialog, with its index, bottom first.
    pub fn take_requests(&mut self) -> Vec<(usize, DialogRequest)> {
        self.dialogs
            .iter_mut()
            .enumerate()
            .flat_map(|(i, d)| d.take_requests().into_iter().map(move |r| (i, r)))
            .collect()
    }

    /// The top dialog.
    #[cfg(test)]
    pub fn top(&self) -> Option<&dyn DialogModel> {
        self.dialogs.last().map(|d| &**d)
    }

    /// The top dialog, mutable.
    pub fn top_mut(&mut self) -> Option<&mut Box<dyn DialogModel>> {
        self.dialogs.last_mut()
    }

    /// Number of open dialogs.
    pub fn len(&self) -> usize {
        self.dialogs.len()
    }

    /// True when no dialog is open.
    pub fn is_empty(&self) -> bool {
        self.dialogs.is_empty()
    }

    /// Polls every dialog's jobs; true when one changed.
    pub fn poll(&mut self, svc: &Service) -> bool {
        let mut changed = false;
        for d in &mut self.dialogs {
            changed |= d.poll(svc);
        }
        changed
    }
}

/// A question with buttons (D-112): the multiple-close warnings, conflicts, overwrites, deletes, D-84.
#[derive(Debug, Clone, PartialEq)]
pub struct Prompt {
    /// The title.
    pub title: String,
    /// The question.
    pub message: String,
    /// The buttons (role, label), in order.
    pub buttons: Vec<(ButtonRole, String)>,
    /// The button Enter presses.
    pub default: ButtonRole,
    /// "Do not ask again", bound to a boolean `options.toml` key set to `false` when ticked.
    pub do_not_ask: Option<&'static str>,
    /// The check box state.
    pub ticked: bool,
    /// What each button does besides closing.
    pub actions: Vec<(ButtonRole, Vec<Followup>)>,
    /// The help topic.
    pub topic: &'static str,
}

impl Prompt {
    /// A Yes/Cancel question: Yes runs `on_yes`.
    pub fn confirm(
        title: impl Into<String>,
        message: impl Into<String>,
        do_not_ask: Option<&'static str>,
        on_yes: Vec<Followup>,
    ) -> Prompt {
        Prompt {
            title: title.into(),
            message: message.into(),
            buttons: vec![
                (ButtonRole::Yes, "Yes".to_owned()),
                (ButtonRole::Cancel, "Cancel".to_owned()),
            ],
            default: ButtonRole::Yes,
            do_not_ask,
            ticked: false,
            actions: vec![(ButtonRole::Yes, on_yes)],
            topic: "main-window",
        }
    }
}

impl DialogModel for Prompt {
    fn title(&self) -> String {
        self.title.clone()
    }

    fn help_topic(&self) -> &'static str {
        self.topic
    }

    fn buttons(&self) -> Vec<DialogButton> {
        self.buttons
            .iter()
            .map(|(role, label)| DialogButton {
                label: label.clone(),
                role: *role,
                enabled: true,
                default: *role == self.default,
            })
            .collect()
    }

    fn on_button(&mut self, role: ButtonRole, _svc: &Service) -> DialogStep {
        if !self.buttons.iter().any(|(r, _)| *r == role) {
            return DialogStep::Stay;
        }
        let mut followups: Vec<Followup> = self
            .actions
            .iter()
            .filter(|(r, _)| *r == role)
            .flat_map(|(_, f)| f.iter().cloned())
            .collect();
        if self.ticked
            && role != ButtonRole::Cancel
            && let Some(key) = self.do_not_ask
        {
            // First: the option is written before the window closes or the program quits.
            followups.insert(0, Followup::DisableOption(key));
        }
        if followups.is_empty() {
            DialogStep::Close
        } else {
            DialogStep::CloseWith(followups)
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl Prompt {
    /// A question with the buttons `buttons`, each with what it does.
    pub fn ask(
        title: impl Into<String>,
        message: impl Into<String>,
        default: ButtonRole,
        buttons: Vec<(ButtonRole, &str, Vec<Followup>)>,
        topic: &'static str,
    ) -> Prompt {
        Prompt {
            title: title.into(),
            message: message.into(),
            default,
            do_not_ask: None,
            ticked: false,
            actions: buttons
                .iter()
                .map(|(role, _, then)| (*role, then.clone()))
                .collect(),
            buttons: buttons
                .into_iter()
                .map(|(role, label, _)| (role, label.to_owned()))
                .collect(),
            topic,
        }
    }
}

/// Help ▸ About (D-115: a dialog since M2 W4a).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AboutDialog;

impl AboutDialog {
    /// The text of the dialog.
    pub fn text(&self) -> String {
        format!(
            "Hikasa {HIKASA_VERSION}: compare folders and files.\n\nEvery function is also available from the \
             hikasa command line."
        )
    }
}

impl DialogModel for AboutDialog {
    fn title(&self) -> String {
        "About Hikasa".to_owned()
    }

    fn help_topic(&self) -> &'static str {
        "main-window"
    }

    fn buttons(&self) -> Vec<DialogButton> {
        vec![DialogButton {
            label: "Close".to_owned(),
            role: ButtonRole::Close,
            enabled: true,
            default: true,
        }]
    }

    fn on_button(&mut self, _role: ButtonRole, _svc: &Service) -> DialogStep {
        DialogStep::Close
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
