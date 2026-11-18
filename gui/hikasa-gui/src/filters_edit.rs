//! The Filters edit of the folder views' toolbar and *Add To Presets* (M2 W9c, FC-050, D-250). Toolkit-free models
//! (check 7): the field shows the view's name filter as core's canonical toolbar text (`NameFilter::to_filter_string`,
//! D-07's order); Enter parses the text with core (`NameFilter::parse`): an error shows under the field and nothing
//! runs, else the window changes the view's rules (the view runs again) and pushes the text to `mru.name_filters`.
//! The drop-down offers the presets of `filters.toml` and the recent filters. *Add To Presets* asks a unique name, then
//! writes `filters.toml` through `settings_save` on the window's settings queue; a `Conflict` offers W4c's *Overwrite*,
//! *Reload* (read the file again and add the preset to it) and *Cancel*.

use std::any::Any;
use std::path::PathBuf;

use hikasa_core::{NameFilter, NameFilterLists};
use hikasa_session::settings::filters::FilterPreset;
use hikasa_session::settings::{FileOutcome, SettingsChanges, SettingsSnapshot};
use hikasa_session::{ApiError, Service};

use crate::dialog::{
    ButtonRole, DialogButton, DialogModel, DialogRequest, DialogStep, Followup, Prompt,
};
use crate::folder_tabs::{TEXT_OPTIONS, toolbar_text};
use crate::options::ConflictAnswer;

/// The help topic of the field and the prompt.
pub const FILTERS_TOPIC: &str = "view-folder-compare";

/// The Filters edit of a folder view.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FiltersEdit {
    /// The text in the field.
    pub text: String,
    /// The view's filter, as the field showed it when the view was built (`""`: none).
    pub committed: String,
    /// Core's message of the last Enter (under the field).
    pub error: Option<String>,
}

impl FiltersEdit {
    /// The field of a view whose rules hold `lists`.
    pub fn of(lists: &NameFilterLists) -> FiltersEdit {
        let text = toolbar_text(lists).unwrap_or_default();
        FiltersEdit {
            text: text.clone(),
            committed: text,
            error: None,
        }
    }

    /// True while the text differs from the view's filter.
    pub fn edited(&self) -> bool {
        self.text != self.committed
    }

    /// The text parsed by core, the field left as it is: its canonical text and the four lists, or core's message.
    pub fn parsed(&self) -> Result<(String, NameFilterLists), String> {
        NameFilter::parse(&self.text, TEXT_OPTIONS)
            .map(|filter| (filter.to_filter_string(), filter.to_lists()))
            .map_err(|e| e.to_string())
    }

    /// Enter, the text left as it is (review F17: the window shows the canonical text only once the view takes it): the
    /// canonical text and the lists to apply (`None`: the view's own filter, nothing runs), or core's message, kept
    /// under the field (nothing runs).
    pub fn check(&mut self) -> Result<(String, Option<NameFilterLists>), String> {
        match self.parsed() {
            Ok((canonical, lists)) => {
                self.error = None;
                let lists = (canonical != self.committed).then_some(lists);
                Ok((canonical, lists))
            }
            Err(message) => {
                self.error = Some(message.clone());
                Err(message)
            }
        }
    }

    /// Esc: the view's filter again.
    pub fn revert(&mut self) {
        self.text.clone_from(&self.committed);
        self.error = None;
    }
}

/// What the preset save answered.
enum Saved {
    /// `settings_save`'s answer for `filters.toml`.
    Outcome(FileOutcome),
    /// The save failed (the message).
    Failed(String),
}

/// *Add To Presets*: the name prompt and the write of `filters.toml` (D-250).
pub struct PresetNameDialog {
    /// The name typed.
    pub name: String,
    /// The mask saved.
    pub mask: String,
    /// Why OK did not close.
    pub error: Option<String>,
    snapshot: SettingsSnapshot,
    saving: Option<crossbeam_channel::Receiver<Saved>>,
    requests: Vec<DialogRequest>,
}

impl PresetNameDialog {
    /// The prompt for `mask` over the settings as the window read them.
    pub fn new(mask: String, snapshot: SettingsSnapshot) -> PresetNameDialog {
        PresetNameDialog {
            name: String::new(),
            mask,
            error: None,
            snapshot,
            saving: None,
            requests: Vec::new(),
        }
    }

    /// The presets' names as read.
    pub fn names(&self) -> Vec<String> {
        self.snapshot
            .filters
            .value()
            .preset
            .iter()
            .map(|p| p.name.clone())
            .collect()
    }

    /// The name, or why it cannot be one: empty, or used by a preset already.
    fn checked_name(&self) -> Result<String, String> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err("Type a name for the preset.".to_owned());
        }
        if self.names().iter().any(|n| n == name) {
            return Err(format!("A preset named `{name}` exists already."));
        }
        Ok(name.to_owned())
    }

    /// The write on the window's settings queue: the presets read (`fresh`: read again first) with this one added.
    fn save(&mut self, name: String, force: bool, fresh: bool) {
        let (tx, rx) = crossbeam_channel::unbounded();
        self.saving = Some(rx);
        let snapshot = self.snapshot.clone();
        let mask = self.mask.clone();
        self.requests.push(DialogRequest::Settings {
            title: "save the name filter presets".to_owned(),
            body: Box::new(move |svc: &Service| -> Result<Vec<PathBuf>, ApiError> {
                let snapshot = if fresh {
                    match svc.settings_snapshot() {
                        Ok(snapshot) => snapshot,
                        Err(e) => {
                            let _ = tx.send(Saved::Failed(e.to_string()));
                            return Ok(Vec::new());
                        }
                    }
                } else {
                    snapshot
                };
                let mut presets = snapshot.filters.value().clone();
                if presets.preset.iter().any(|p| p.name == name) {
                    let _ = tx.send(Saved::Failed(format!(
                        "A preset named `{name}` exists already."
                    )));
                    return Ok(Vec::new());
                }
                presets.preset.push(FilterPreset::new(&name, &mask));
                let changes = SettingsChanges {
                    filters: Some(presets),
                    ..SettingsChanges::default()
                };
                match svc.settings_save(&snapshot, &changes, force) {
                    Ok(dto) => {
                        let mut written = Vec::new();
                        let mut outcome = FileOutcome::Unchanged;
                        for (_, path, o) in dto.files {
                            if matches!(o, FileOutcome::Written) {
                                written.push(path);
                            }
                            outcome = o;
                        }
                        let _ = tx.send(Saved::Outcome(outcome));
                        Ok(written)
                    }
                    Err(e) => {
                        let _ = tx.send(Saved::Failed(e.to_string()));
                        Ok(Vec::new())
                    }
                }
            }),
        });
    }

    /// The answer to the conflict prompt (W4c's choices).
    pub fn answer(&mut self, answer: ConflictAnswer) {
        let Ok(name) = self.checked_name() else {
            return;
        };
        match answer {
            ConflictAnswer::Overwrite => self.save(name, true, false),
            ConflictAnswer::Reload => self.save(name, false, true),
            ConflictAnswer::Discard => self.requests.push(DialogRequest::Close),
        }
    }
}

impl DialogModel for PresetNameDialog {
    fn title(&self) -> String {
        "Add To Presets".to_owned()
    }

    fn help_topic(&self) -> &'static str {
        FILTERS_TOPIC
    }

    fn buttons(&self) -> Vec<DialogButton> {
        vec![
            DialogButton {
                label: "OK".to_owned(),
                role: ButtonRole::Ok,
                enabled: self.saving.is_none(),
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
        if role != ButtonRole::Ok {
            return DialogStep::Close;
        }
        if self.saving.is_some() {
            return DialogStep::Stay;
        }
        match self.checked_name() {
            Ok(name) => {
                self.error = None;
                self.save(name, false, false);
            }
            Err(e) => self.error = Some(e),
        }
        DialogStep::Stay
    }

    fn poll(&mut self, _svc: &Service) -> bool {
        let Some(answer) = self.saving.as_ref().and_then(|rx| rx.try_recv().ok()) else {
            return false;
        };
        self.saving = None;
        match answer {
            Saved::Outcome(FileOutcome::Written | FileOutcome::Unchanged) => {
                self.requests.push(DialogRequest::Close);
            }
            Saved::Outcome(FileOutcome::Conflict) => {
                self.requests.push(DialogRequest::Ask(Box::new(Prompt::ask(
                    "Settings Changed on Disk",
                    "filters.toml changed on disk since this dialog read it.".to_owned(),
                    ButtonRole::Cancel,
                    vec![
                        (
                            ButtonRole::Other(1),
                            "Overwrite",
                            vec![Followup::Presets(ConflictAnswer::Overwrite)],
                        ),
                        (
                            ButtonRole::Other(2),
                            "Reload",
                            vec![Followup::Presets(ConflictAnswer::Reload)],
                        ),
                        (ButtonRole::Cancel, "Cancel", Vec::new()),
                    ],
                    FILTERS_TOPIC,
                ))));
            }
            Saved::Outcome(FileOutcome::Invalid(problems)) => {
                self.error = Some(
                    problems
                        .iter()
                        .map(|p| format!("{}: {}", p.key, p.message))
                        .collect::<Vec<_>>()
                        .join("\n"),
                );
            }
            Saved::Outcome(FileOutcome::Unreadable(why)) => {
                self.error = Some(format!("filters.toml cannot be read: {why}"));
            }
            Saved::Outcome(FileOutcome::Failed(e)) => self.error = Some(e),
            Saved::Outcome(FileOutcome::Deleted) => self.requests.push(DialogRequest::Close),
            Saved::Failed(e) => self.error = Some(e),
        }
        true
    }

    fn busy(&self) -> bool {
        self.saving.is_some()
    }

    fn take_requests(&mut self) -> Vec<DialogRequest> {
        std::mem::take(&mut self.requests)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
