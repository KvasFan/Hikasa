//! The File Formats manager (M2 W11c; contract §1.11.26, FF-001, FF-004; D-318): Tools ▸ *File Formats…*. The list of
//! `format_list` in detection order (enable check, name, kind, mask, origin; shadowed formats in the important text
//! colour with the formats covering them, disabled ones muted, *Everything Else* last and fixed) and its buttons, each
//! one Service call written at once (**open default**: no OK/Cancel transaction) and equal to the CLI command: the
//! enable check (`format enable|disable`), *Move Up/Down/Top/Bottom* (`format move`), *New* ▸ kind and *Clone* (a prompt
//! for the id and name, `format new`, then the editor), *Edit…* (the editor, `editor.rs`), *Delete* (user formats,
//! after a confirmation), *Import…* and *Export…* (the browse dialog), *Restore Built-in* (its plan first, D-75's
//! `--dry-run`). Writes go through the window's settings queue (acknowledged, then the registry reloads, D-117/D-119);
//! the list refreshes after each action and when the watch reloads `formats/`; an error is core's message under the
//! list; the views using a changed format get D-315's *Reopen* notice.
//!
//! The model is toolkit-free; `ui/formats.rs` draws it.

pub mod editor;
pub mod grammar_item;
pub mod preview;

use std::any::Any;
use std::path::PathBuf;
use std::sync::Arc;

use hikasa_core::JobKind;
use hikasa_formats::FormatKind;
use hikasa_session::api::{FormatRestoreDto, FormatWriteDto, MoveTo};
use hikasa_session::{ApiError, Service};
use parking_lot::Mutex;

use crate::dialog::{
    ButtonRole, DialogButton, DialogModel, DialogRequest, DialogStep, Followup, Prompt,
};
use crate::job::{Job, Wake};

/// The help topic of the manager.
pub const TOPIC: &str = "dialog-file-formats";

/// The kinds *New* ▸ offers, in menu order.
pub const NEW_KINDS: [(FormatKind, &str); 7] = [
    (FormatKind::Text, "Text"),
    (FormatKind::Table, "Table"),
    (FormatKind::Hex, "Hex"),
    (FormatKind::Picture, "Picture"),
    (FormatKind::Media, "Media"),
    (FormatKind::Version, "Version"),
    (FormatKind::Registry, "Registry"),
];

/// The result of a write on the settings queue, read by the dialog that asked for it.
pub struct Pending<T> {
    slot: Arc<Mutex<Option<Result<T, ApiError>>>>,
}

impl<T> Pending<T> {
    /// The result once the write ran.
    pub fn take(&self) -> Option<Result<T, ApiError>> {
        self.slot.lock().take()
    }
}

/// A write on the window's settings queue (acknowledged, then the registry reloads) whose result the dialog reads:
/// `body` returns its value and the files it wrote. A failure writes nothing and is the dialog's to show.
pub fn queued<T: Send + 'static>(
    title: String,
    body: impl FnOnce(&Service) -> Result<(T, Vec<PathBuf>), ApiError> + Send + 'static,
) -> (DialogRequest, Pending<T>) {
    let slot = Arc::new(Mutex::new(None));
    let out = Arc::clone(&slot);
    let request = DialogRequest::Settings {
        title,
        body: Box::new(move |svc| match body(svc) {
            Ok((value, written)) => {
                *out.lock() = Some(Ok(value));
                Ok(written)
            }
            Err(e) => {
                *out.lock() = Some(Err(e));
                Ok(Vec::new())
            }
        }),
    };
    (request, Pending { slot })
}

/// The paths of a format write.
fn paths(dto: &FormatWriteDto) -> Vec<PathBuf> {
    dto.written
        .iter()
        .chain(&dto.removed)
        .map(PathBuf::from)
        .collect()
}

/// Where a format comes from (FF-001).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowOrigin {
    /// Shipped with the program.
    BuiltIn,
    /// A user file.
    User,
    /// A user file with a built-in's id.
    Override,
}

impl RowOrigin {
    /// The origin column's caption.
    pub fn caption(self) -> &'static str {
        match self {
            RowOrigin::BuiltIn => "built-in",
            RowOrigin::User => "user",
            RowOrigin::Override => "override",
        }
    }
}

/// One row of the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatRow {
    /// The id (`""` for *Everything Else*).
    pub id: String,
    /// The name.
    pub name: String,
    /// The kind word.
    pub kind: String,
    /// The mask.
    pub mask: String,
    /// Enabled.
    pub enabled: bool,
    /// The origin.
    pub origin: RowOrigin,
    /// The names of the enabled formats before it that cover its whole mask (shadowed: the important text colour).
    pub shadowed_by: Vec<String>,
    /// *Everything Else*: fixed, last, not editable.
    pub fixed: bool,
}

impl FormatRow {
    /// The tooltip of a shadowed row.
    pub fn tooltip(&self) -> Option<String> {
        (!self.shadowed_by.is_empty())
            .then(|| format!("Never detected: covered by {}", self.shadowed_by.join(", ")))
    }
}

/// The rows of `format list` with the fixed *Everything Else* last.
pub fn rows(svc: &Service, builtin: &[String]) -> Vec<FormatRow> {
    let list = svc.format_list().formats;
    let name_of = |id: &str| {
        list.iter()
            .find(|f| f.id == id)
            .map_or_else(|| id.to_owned(), |f| f.name.clone())
    };
    let mut out: Vec<FormatRow> = list
        .iter()
        .map(|f| FormatRow {
            id: f.id.clone(),
            name: f.name.clone(),
            kind: f.kind.to_owned(),
            mask: f.mask.clone(),
            enabled: f.enabled,
            origin: match (f.origin, builtin.contains(&f.id)) {
                ("built_in", _) => RowOrigin::BuiltIn,
                (_, true) => RowOrigin::Override,
                _ => RowOrigin::User,
            },
            shadowed_by: if f.status.status == "shadowed" {
                f.status.by.iter().map(|id| name_of(id)).collect()
            } else {
                Vec::new()
            },
            fixed: false,
        })
        .collect();
    out.push(FormatRow {
        id: String::new(),
        name: "Everything Else".to_owned(),
        kind: "text".to_owned(),
        mask: String::new(),
        enabled: true,
        origin: RowOrigin::BuiltIn,
        shadowed_by: Vec::new(),
        fixed: true,
    });
    out
}

/// The answers of the manager's and the editors' prompts (routed to the dialog by the window).
#[derive(Debug, Clone, PartialEq)]
pub enum FormatsFollowup {
    /// *Delete* confirmed.
    Delete(String),
    /// *Import…* of an id that exists: *Replace*.
    ImportReplace(PathBuf),
    /// *Restore Built-in*'s plan confirmed.
    Restore {
        /// The ids (empty with `all`).
        ids: Vec<String>,
        /// *All…*.
        all: bool,
    },
    /// The format editor's conflict answer (W4c): *Overwrite*, *Reload* or *Cancel*.
    Conflict {
        /// The format's id.
        id: String,
        /// The answer.
        answer: editor::ConflictAnswer,
    },
    /// The grammar item dialog's OK: the item at `index` (`None`: a new one) and its element's case flag.
    Item {
        /// The format's id.
        id: String,
        /// The edited item, its place and its element's flag.
        done: Box<grammar_item::ItemDone>,
    },
}

/// What a manager action waits for.
enum Waiting {
    /// A write; the names of the formats it changed.
    Write(Pending<FormatWriteDto>, Vec<String>, Option<String>),
    /// A restore's plan (dry run).
    Plan(Job<FormatRestoreDto>, Vec<String>, bool),
    /// A restore.
    Restore(Pending<FormatRestoreDto>, Vec<String>),
    /// An import (the path, whether it replaces).
    Import(Pending<FormatWriteDto>, PathBuf),
    /// An export.
    Export(Pending<()>),
}

/// The prompt of *New* ▸ kind and *Clone*: the id and the name of the new format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPrompt {
    /// The kind (*New*), `None` for *Clone*.
    pub kind: Option<FormatKind>,
    /// The format cloned.
    pub from: Option<String>,
    /// The id typed.
    pub id: String,
    /// The name typed.
    pub name: String,
}

/// The File Formats manager.
pub struct FileFormatManager {
    /// The list.
    pub rows: Vec<FormatRow>,
    /// The built-in ids.
    pub builtin: Vec<String>,
    /// The selected row.
    pub selected: Option<usize>,
    /// The *New*/*Clone* prompt shown under the list.
    pub prompt: Option<NewPrompt>,
    /// Core's message of the last action.
    pub error: Option<String>,
    /// *Restore Built-in* (`false`) or *All…* (`true`) was pressed: its plan starts at the next poll.
    restore_asked: Option<bool>,
    waiting: Vec<Waiting>,
    requests: Vec<DialogRequest>,
    wake: Wake,
    /// The registry the list was read from (refresh when the watch reloaded it).
    seen: Option<Arc<hikasa_formats::FormatRegistry>>,
}

impl FileFormatManager {
    /// The manager over the registry as it is now.
    pub fn new(svc: &Service, wake: Wake) -> FileFormatManager {
        let mut m = FileFormatManager {
            rows: Vec::new(),
            builtin: svc.format_builtin_ids(),
            selected: None,
            prompt: None,
            error: None,
            restore_asked: None,
            waiting: Vec::new(),
            requests: Vec::new(),
            wake,
            seen: None,
        };
        m.refresh(svc);
        m
    }

    /// Reads the list again (the selection follows its id).
    pub fn refresh(&mut self, svc: &Service) {
        let id = self.selected_row().map(|r| r.id.clone());
        self.rows = rows(svc, &self.builtin);
        self.selected = id.and_then(|id| self.rows.iter().position(|r| r.id == id && !r.fixed));
        self.seen = Some(svc.registry());
    }

    /// The selected row.
    pub fn selected_row(&self) -> Option<&FormatRow> {
        self.selected.and_then(|k| self.rows.get(k))
    }

    /// Selects a row (the fixed *Everything Else* too: its buttons are off).
    pub fn select(&mut self, k: usize) {
        if k < self.rows.len() {
            self.selected = Some(k);
        }
    }

    /// True while an action runs (its buttons wait).
    pub fn waiting(&self) -> bool {
        !self.waiting.is_empty()
    }

    fn selected_id(&self) -> Option<String> {
        self.selected_row()
            .filter(|r| !r.fixed)
            .map(|r| r.id.clone())
    }

    fn write(
        &mut self,
        title: String,
        changed: Vec<String>,
        then_edit: Option<String>,
        body: impl FnOnce(&Service) -> Result<FormatWriteDto, ApiError> + Send + 'static,
    ) {
        self.error = None;
        let (request, pending) = queued(title, move |svc| {
            let dto = body(svc)?;
            let written = paths(&dto);
            Ok((dto, written))
        });
        self.requests.push(request);
        self.waiting
            .push(Waiting::Write(pending, changed, then_edit));
    }

    /// The enable check of row `k` (`format enable|disable`).
    pub fn set_enabled(&mut self, k: usize, enabled: bool) {
        let Some(row) = self.rows.get(k).filter(|r| !r.fixed) else {
            return;
        };
        let (id, name) = (row.id.clone(), row.name.clone());
        let verb = if enabled { "enable" } else { "disable" };
        self.write(
            format!("{verb} the format {id}"),
            vec![name],
            None,
            move |svc| svc.format_enable(&id, enabled),
        );
    }

    /// *Move Up/Down/Top/Bottom* of the selected format (`format move`).
    pub fn move_selected(&mut self, to: MoveTo) {
        let Some(id) = self.selected_id() else {
            return;
        };
        self.write(
            format!("move the format {id}"),
            Vec::new(),
            None,
            move |svc| svc.format_move(&id, &to),
        );
    }

    /// *New* ▸ kind: the prompt for the id and the name.
    pub fn start_new(&mut self, kind: FormatKind) {
        self.prompt = Some(NewPrompt {
            kind: Some(kind),
            from: None,
            id: String::new(),
            name: String::new(),
        });
    }

    /// *Clone* of the selected format: the prompt.
    pub fn start_clone(&mut self) {
        let Some(row) = self.selected_row().filter(|r| !r.fixed) else {
            return;
        };
        self.prompt = Some(NewPrompt {
            kind: None,
            from: Some(row.id.clone()),
            id: String::new(),
            name: format!("{} (copy)", row.name),
        });
    }

    /// The prompt's *Create*: `format new` (from a clone or of a kind), then the editor.
    pub fn create(&mut self) {
        let Some(p) = self.prompt.take() else {
            return;
        };
        let id = p.id.trim().to_owned();
        if id.is_empty() {
            self.error = Some("Type an id for the new format.".to_owned());
            self.prompt = Some(p);
            return;
        }
        let name = p.name.trim().to_owned();
        let name = (!name.is_empty()).then_some(name);
        let edit = id.clone();
        self.write(
            format!("new format {id}"),
            Vec::new(),
            Some(edit),
            move |svc| svc.format_new(&id, p.from.as_deref(), name.as_deref(), None, p.kind),
        );
    }

    /// *Edit…*: the editor of the selected format.
    pub fn edit(&mut self) {
        if let Some(id) = self.selected_id() {
            self.requests.push(DialogRequest::EditFormat(id));
        }
    }

    /// *Delete* (user formats only): the confirmation.
    pub fn ask_delete(&mut self) {
        let Some(row) = self.selected_row().filter(|r| !r.fixed) else {
            return;
        };
        if row.origin != RowOrigin::User {
            self.error = Some(format!(
                "`{}` is a built-in format: disable it, or restore it.",
                row.id
            ));
            return;
        }
        let id = row.id.clone();
        self.requests.push(DialogRequest::Ask(Box::new(Prompt {
            topic: TOPIC,
            ..Prompt::confirm(
                "Delete Format",
                format!(
                    "Delete the format {} ({id})? Its file is removed.",
                    row.name
                ),
                None,
                vec![Followup::Formats(FormatsFollowup::Delete(id))],
            )
        })));
    }

    /// *Import…* of `path` (the browse dialog's answer); an existing id asks *Replace* first.
    pub fn import(&mut self, path: PathBuf, replace: bool) {
        self.error = None;
        let file = path.clone();
        let (request, pending) = queued(format!("import {}", path.display()), move |svc| {
            let dto = svc.format_import(&file, replace)?;
            let written = paths(&dto);
            Ok((dto, written))
        });
        self.requests.push(request);
        self.waiting.push(Waiting::Import(pending, path));
    }

    /// *Export…* of the selected format to `path` (the browse dialog's answer).
    pub fn export(&mut self, path: PathBuf) {
        let Some(id) = self.selected_id() else {
            return;
        };
        self.error = None;
        let (request, pending) = queued(format!("export the format {id}"), move |svc| {
            svc.format_export(&id, Some(&path))?;
            Ok(((), Vec::new()))
        });
        self.requests.push(request);
        self.waiting.push(Waiting::Export(pending));
    }

    /// True when *Restore Built-in* applies to the selected row (a built-in with an override file).
    pub fn can_restore(&self) -> bool {
        self.selected_row()
            .is_some_and(|r| r.origin == RowOrigin::Override)
    }

    /// *Restore Built-in* (`all`: *All…*) pressed: the plan runs at the next poll.
    pub fn ask_restore(&mut self, all: bool) {
        self.restore_asked = Some(all);
    }

    /// *Restore Built-in* (`all`: *All…*): its plan first (a dry run), then the question.
    pub fn plan_restore(&mut self, svc: &Service, all: bool) {
        let ids: Vec<String> = if all {
            Vec::new()
        } else {
            match self
                .selected_row()
                .filter(|r| r.origin == RowOrigin::Override)
            {
                Some(r) => vec![r.id.clone()],
                None => return,
            }
        };
        self.error = None;
        let plan_ids = ids.clone();
        let job = Job::spawn(
            svc,
            JobKind::VfsRead,
            "plan the restore".into(),
            &self.wake,
            move |svc, _| svc.format_restore(&plan_ids, all, true),
        );
        self.waiting.push(Waiting::Plan(job, ids, all));
    }

    /// An answer of the manager's prompts.
    pub fn answer(&mut self, answer: FormatsFollowup) {
        match answer {
            FormatsFollowup::Delete(id) => {
                let name = self
                    .rows
                    .iter()
                    .find(|r| r.id == id)
                    .map(|r| r.name.clone());
                self.write(
                    format!("delete the format {id}"),
                    name.into_iter().collect(),
                    None,
                    move |svc| svc.format_delete(&id),
                );
            }
            FormatsFollowup::ImportReplace(path) => self.import(path, true),
            FormatsFollowup::Restore { ids, all } => {
                self.error = None;
                let names: Vec<String> = if all {
                    self.rows.iter().map(|r| r.name.clone()).collect()
                } else {
                    self.rows
                        .iter()
                        .filter(|r| ids.contains(&r.id))
                        .map(|r| r.name.clone())
                        .collect()
                };
                let (request, pending) =
                    queued("restore the built-in formats".into(), move |svc| {
                        let dto = svc.format_restore(&ids, all, false)?;
                        let written = dto.removed.iter().map(PathBuf::from).collect();
                        Ok((dto, written))
                    });
                self.requests.push(request);
                self.waiting.push(Waiting::Restore(pending, names));
            }
            _ => {}
        }
    }

    /// Polls the actions; true when something changed.
    fn poll_actions(&mut self, svc: &Service) -> bool {
        let mut changed = false;
        let mut keep = Vec::new();
        for w in std::mem::take(&mut self.waiting) {
            match w {
                Waiting::Write(pending, names, then_edit) => match pending.take() {
                    None => keep.push(Waiting::Write(pending, names, then_edit)),
                    Some(result) => {
                        changed = true;
                        match result {
                            Ok(dto) => {
                                self.refresh(svc);
                                if let Some(id) = then_edit {
                                    self.selected = self.rows.iter().position(|r| r.id == id);
                                    self.requests.push(DialogRequest::EditFormat(id));
                                }
                                let mut names = names;
                                if let Some(r) = self.rows.iter().find(|r| r.id == dto.id) {
                                    names.push(r.name.clone());
                                }
                                if !names.is_empty() {
                                    self.requests.push(DialogRequest::FormatsChanged(names));
                                }
                            }
                            Err(e) => self.error = Some(e.to_string()),
                        }
                    }
                },
                Waiting::Plan(job, ids, all) => match job.try_result() {
                    None => keep.push(Waiting::Plan(job, ids, all)),
                    Some(result) => {
                        changed = true;
                        match result {
                            Ok(plan) if plan.removed.is_empty() => {
                                self.error = Some("Nothing to restore.".to_owned());
                            }
                            Ok(plan) => {
                                let message = format!(
                                    "Restore the built-in formats? These files are removed:\n{}",
                                    plan.removed.join("\n")
                                );
                                self.requests.push(DialogRequest::Ask(Box::new(Prompt {
                                    topic: TOPIC,
                                    ..Prompt::confirm(
                                        "Restore Built-in Formats",
                                        message,
                                        None,
                                        vec![Followup::Formats(FormatsFollowup::Restore {
                                            ids,
                                            all,
                                        })],
                                    )
                                })));
                            }
                            Err(e) => self.error = Some(e.to_string()),
                        }
                    }
                },
                Waiting::Restore(pending, names) => match pending.take() {
                    None => keep.push(Waiting::Restore(pending, names)),
                    Some(result) => {
                        changed = true;
                        match result {
                            Ok(_) => {
                                self.refresh(svc);
                                self.requests.push(DialogRequest::FormatsChanged(names));
                            }
                            Err(e) => self.error = Some(e.to_string()),
                        }
                    }
                },
                Waiting::Import(pending, path) => match pending.take() {
                    None => keep.push(Waiting::Import(pending, path)),
                    Some(result) => {
                        changed = true;
                        match result {
                            Ok(dto) => {
                                self.refresh(svc);
                                if let Some(r) = self.rows.iter().find(|r| r.id == dto.id) {
                                    self.requests
                                        .push(DialogRequest::FormatsChanged(vec![r.name.clone()]));
                                }
                            }
                            Err(ApiError::AlreadyExists(message)) => {
                                self.requests.push(DialogRequest::Ask(Box::new(Prompt {
                                    topic: TOPIC,
                                    ..Prompt::confirm(
                                        "Import Format",
                                        format!("{message}. Replace it?"),
                                        None,
                                        vec![Followup::Formats(FormatsFollowup::ImportReplace(
                                            path,
                                        ))],
                                    )
                                })));
                            }
                            Err(e) => self.error = Some(e.to_string()),
                        }
                    }
                },
                Waiting::Export(pending) => match pending.take() {
                    None => keep.push(Waiting::Export(pending)),
                    Some(result) => {
                        changed = true;
                        if let Err(e) = result {
                            self.error = Some(e.to_string());
                        }
                    }
                },
            }
        }
        self.waiting = keep;
        changed
    }
}

impl DialogModel for FileFormatManager {
    fn title(&self) -> String {
        "File Formats".to_owned()
    }

    fn help_topic(&self) -> &'static str {
        TOPIC
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

    fn poll(&mut self, svc: &Service) -> bool {
        let mut changed = false;
        if let Some(all) = self.restore_asked.take() {
            self.plan_restore(svc, all);
            changed = true;
        }
        changed |= self.poll_actions(svc);
        // The watch reloaded `formats/` (FF-004): the list follows.
        let now = svc.registry();
        if self
            .seen
            .as_ref()
            .is_none_or(|seen| !Arc::ptr_eq(seen, &now))
        {
            self.refresh(svc);
            changed = true;
        }
        changed
    }

    fn busy(&self) -> bool {
        self.waiting()
    }

    fn size(&self) -> Option<(f32, f32)> {
        Some((760.0, 460.0))
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

/// The regex helper (FF-028, D-320): item `k` of `hikasa_formats::regex_helper::ITEMS` replaces the byte range
/// `selection` of `field`; the new caret (bytes). An index out of range changes nothing.
pub fn helper_insert(field: &mut String, selection: std::ops::Range<usize>, k: usize) -> usize {
    match hikasa_formats::regex_helper::ITEMS.get(k) {
        Some(item) => {
            let (text, caret) = hikasa_formats::regex_helper::apply(field, selection, item);
            *field = text;
            caret
        }
        None => selection.end.min(field.len()),
    }
}
