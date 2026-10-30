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

use hikasa_session::settings::Shortcut;
use hikasa_session::{HIKASA_VERSION, Service};

use crate::session_dialog::SessionAction;
use crate::window::Invocation;

/// The role of a dialog button.
#[allow(dead_code)] // `Apply`, `Back`, `Next`, `Finish` are W4c's (contract seams)
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
    /// Polls the dialog's jobs; true when it changed.
    fn poll(&mut self, _svc: &Service) -> bool {
        false
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
