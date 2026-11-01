//! The Options dialog (M2 W4c, §1.4.13; D-128…D-130): a searchable page tree, a page, Apply / OK / Cancel, over a
//! working copy of every settings file taken when it opens (a `SettingsSnapshot`). Each P0 key of `options.toml` is on
//! exactly one page (`pages.rs`); widgets follow `ValueKind`; values are checked live with the schemas' `validate`
//! and Apply is disabled while a problem remains. Apply hands the changed files to the window's settings queue
//! (`settings_save`, `IfUnchanged` per file); a `Conflict` asks *Overwrite* / *Reload this file (discard my edits)* /
//! *Cancel*. A file the program cannot read (a newer `schema_version`) is never overwritten: the page says why and
//! only *Reload* and *Cancel* are offered. Toolkit-free; `ui/options/` draws it.

use std::any::Any;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use crossbeam_channel::Receiver;
use hikasa_session::Service;
use hikasa_session::api::ApiError;
use hikasa_session::settings::theme::BUILTIN_SCHEMES;
use hikasa_session::settings::{
    ArchiveTypes, CommandOverrides, FileId, FileOutcome, FilterPresets, FormValue, OpenWith,
    Options, SchemeFile, SettingsChanges, SettingsFile, SettingsSaveDto, SettingsSchema,
    SettingsSnapshot, Shortcut, ThemeSelection, ValueKind, clamp_value, form_value, key_infos,
    set_form_value,
};

use crate::dialog::{
    ButtonRole, DialogButton, DialogModel, DialogRequest, DialogStep, Followup, Prompt,
};

pub mod commands_page;
pub mod lists;
pub mod pages;
pub mod theme_page;

use commands_page::CommandsPage;
use lists::OpenWithPage;
use pages::{Body, FIRST_PAGE, PAGES, Page, Widget, page_of};
use theme_page::ThemePage;

/// One option field: its key, kind and working value.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    /// The dotted key inside `options.toml`.
    pub path: &'static str,
    /// The label.
    pub label: &'static str,
    /// The OPT id.
    pub opt: &'static str,
    /// The kind (`key_infos`).
    pub kind: ValueKind,
    /// The widget.
    pub widget: Widget,
    /// The working value.
    pub value: FormValue,
}

/// A conflict answer (the prompt of D-128).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictAnswer {
    /// Save the conflicted files with `force`.
    Overwrite,
    /// Take the files from disk, dropping the edits of these files.
    Reload,
    /// Esc with unapplied edits was confirmed: the dialog closes without them.
    Discard,
}

/// What a settings job sends back to the dialog.
enum Outcome {
    /// `settings_save` answered.
    Saved(Box<SettingsSaveDto>),
    /// The files read back for *Reload*.
    Reloaded(Box<SettingsSnapshot>, Vec<FileId>),
    /// The call failed.
    Failed(String),
}

/// The Options dialog.
pub struct OptionsDialog {
    /// The files as last read or written (each file's change stamp: the base of `IfUnchanged`).
    snapshot: SettingsSnapshot,
    /// `options.toml`, working copy.
    pub options: Options,
    /// `commands.toml`, working copy.
    pub commands: CommandOverrides,
    /// `theme.toml`, working copy.
    pub theme: ThemeSelection,
    /// `themes/<name>.toml`, working copies (a deleted scheme is absent).
    pub schemes: BTreeMap<String, SchemeFile>,
    /// `filters.toml`, working copy.
    pub filters: FilterPresets,
    /// `openwith.toml`, working copy.
    pub openwith: OpenWith,
    /// `archives.toml`, working copy.
    pub archives: ArchiveTypes,
    /// The page shown.
    pub page: &'static str,
    /// The search text.
    pub search: String,
    /// Every option field of every page.
    pub fields: Vec<Field>,
    /// The saved workspaces (the Startup combos).
    pub workspaces: Vec<String>,
    /// The commands page.
    pub commands_page: CommandsPage,
    /// The Appearance pages.
    pub theme_page: ThemePage,
    /// The Open With page.
    pub open_with: OpenWithPage,
    /// A theme edit waits for the preview (D-130).
    pub theme_edited: bool,
    /// What the schemas refuse: `<file>: <key>: <message>`.
    problems: Vec<String>,
    /// Why the last Apply did not finish.
    pub message: Option<String>,
    /// A settings job runs.
    saving: Option<Receiver<Outcome>>,
    /// OK: close when every file succeeded.
    close_after: bool,
    /// The files the conflict prompt asks about.
    conflicted: Vec<FileId>,
    /// Requests for the window.
    requests: Vec<DialogRequest>,
    /// The notice key of a file changed under unapplied edits (the notice's *Reload*).
    pub notified: BTreeMap<String, FileId>,
    /// *Reload* asked while a settings job ran: runs when it ends.
    pending_reload: Vec<FileId>,
    /// The newest snapshot the window applied while a settings job ran.
    followed: Option<Box<SettingsSnapshot>>,
}

impl OptionsDialog {
    /// The dialog over `snapshot`, on `page` (`None` or unknown: the first page).
    pub fn new(
        snapshot: SettingsSnapshot,
        page: Option<&str>,
        workspaces: Vec<String>,
    ) -> OptionsDialog {
        let options = snapshot.options.value().clone();
        let infos = key_infos(&FileId::Options);
        let mut fields = Vec::new();
        for def in PAGES.iter().flat_map(|p| p.fields) {
            let key = format!("options.{}", def.path);
            let Some(info) = infos.iter().find(|i| i.key == key) else {
                continue;
            };
            let Some(value) = form_value(&options, def.path) else {
                continue;
            };
            fields.push(Field {
                path: def.path,
                label: def.label,
                opt: info.opt.unwrap_or_default(),
                kind: info.kind,
                widget: def.widget,
                value,
            });
        }
        let mut dialog = OptionsDialog {
            options,
            commands: snapshot.commands.value().clone(),
            theme: snapshot.theme.value().clone(),
            schemes: snapshot
                .schemes
                .iter()
                .map(|(k, v)| (k.clone(), v.value().clone()))
                .collect(),
            filters: snapshot.filters.value().clone(),
            openwith: snapshot.openwith.value().clone(),
            archives: snapshot.archives.value().clone(),
            snapshot,
            page: page
                .and_then(page_of)
                .map_or(FIRST_PAGE, |p: &'static Page| p.id),
            search: String::new(),
            fields,
            workspaces,
            commands_page: CommandsPage::default(),
            theme_page: ThemePage::default(),
            open_with: OpenWithPage::default(),
            theme_edited: false,
            problems: Vec::new(),
            message: None,
            saving: None,
            close_after: false,
            conflicted: Vec::new(),
            requests: Vec::new(),
            notified: BTreeMap::new(),
            pending_reload: Vec::new(),
            followed: None,
        };
        dialog.refresh_commands();
        dialog.refresh_scheme();
        dialog.check();
        dialog
    }

    /// True while a settings job runs (the page is read-only then).
    pub fn saving(&self) -> bool {
        self.saving.is_some()
    }

    /// What the schemas refuse now.
    pub fn problems(&self) -> &[String] {
        &self.problems
    }

    /// What the page says about the files that cannot be read.
    pub fn unreadable_notes(&self) -> Vec<String> {
        self.snapshot
            .unreadable
            .iter()
            .map(|(id, why)| {
                format!(
                    "{} cannot be read, so it is left as it is and this dialog cannot overwrite it: {why}",
                    id.relative_path()
                )
            })
            .collect()
    }

    /// The value of a field, for the drawing layer.
    pub fn field(&self, path: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.path == path)
    }

    /// Sets an option field: the widget's clamp (D-128), then the typed working copy.
    pub fn set_field(&mut self, path: &str, value: FormValue) {
        let Some(field) = self.fields.iter_mut().find(|f| f.path == path) else {
            return;
        };
        let value = clamp_value(field.kind, value);
        if set_form_value(&mut self.options, path, &value).is_ok() {
            field.value = value;
            if path == "appearance.theme_mode" {
                self.theme_edited = true;
            }
            self.edited();
        }
    }

    /// Anything was edited: the live checks run again.
    pub fn edited(&mut self) {
        self.message = None;
        self.check();
    }

    /// The live check covers only the files `changes` would write: a file left alone blocks nothing (it may hold
    /// an element a lenient load kept as it is), and neither it nor its problem belongs to this Apply.
    fn check(&mut self) {
        fn add(
            out: &mut Vec<String>,
            file: &str,
            problems: Vec<hikasa_session::settings::Problem>,
        ) {
            out.extend(
                problems
                    .into_iter()
                    .map(|p| format!("{file}: {}: {}", p.key, p.message)),
            );
        }
        fn check_one<T: SettingsSchema>(out: &mut Vec<String>, file: &str, value: Option<&T>) {
            if let Some(value) = value {
                add(out, file, value.validate());
            }
        }
        let c = self.changes();
        let mut out = Vec::new();
        check_one(&mut out, "options", c.options.as_ref());
        check_one(&mut out, "commands", c.commands.as_ref());
        check_one(&mut out, "theme", c.theme.as_ref());
        for (name, file) in &c.schemes {
            check_one(&mut out, &format!("themes.{name}"), file.as_ref());
        }
        check_one(&mut out, "filters", c.filters.as_ref());
        check_one(&mut out, "openwith", c.openwith.as_ref());
        check_one(&mut out, "archives", c.archives.as_ref());
        self.problems = out;
    }

    /// The pages the search leaves in the tree, in tree order, ancestors kept (no match: empty).
    pub fn visible_pages(&self) -> Vec<&'static Page> {
        let needle = self.search.trim().to_lowercase();
        if needle.is_empty() {
            return PAGES.iter().collect();
        }
        let mut wanted: BTreeSet<&'static str> = BTreeSet::new();
        for page in PAGES {
            if self.page_matches(page, &needle) {
                wanted.insert(page.id);
                if let Some(parent) = page.parent {
                    wanted.insert(parent);
                }
            }
        }
        PAGES.iter().filter(|p| wanted.contains(p.id)).collect()
    }

    fn page_matches(&self, page: &Page, needle: &str) -> bool {
        let has = |text: &str| text.to_lowercase().contains(needle);
        if has(page.title) || page.opts.iter().any(|o| has(o)) {
            return true;
        }
        if let Body::Scheme(group) = page.body
            && self
                .theme_page
                .resolved
                .colours(group)
                .iter()
                .any(|(name, _)| has(name))
        {
            return true;
        }
        page.fields.iter().any(|def| {
            has(def.label) || has(def.path) || self.field(def.path).is_some_and(|f| has(f.opt))
        })
    }

    /// The search text changed: a page it does not match gives way to the first one that it does, so the page
    /// shown is one the tree lists for this search (no match: the page stays, the drawing layer hides it).
    pub fn search_changed(&mut self) {
        let needle = self.search.trim().to_lowercase();
        if needle.is_empty() {
            return;
        }
        if page_of(self.page).is_some_and(|p| self.page_matches(p, &needle)) {
            return;
        }
        if let Some(first) = PAGES.iter().find(|p| self.page_matches(p, &needle)) {
            self.show_page(first.id);
        }
    }

    /// Shows a page.
    pub fn show_page(&mut self, id: &'static str) {
        self.page = id;
        self.end_capture();
    }

    /// The working copy's files that differ from the snapshot (D-128: a file equal to its snapshot is not written).
    pub fn changes(&self) -> SettingsChanges {
        let s = &self.snapshot;
        let mut changes = SettingsChanges {
            options: (self.options != *s.options.value()).then(|| self.options.clone()),
            commands: (self.commands != *s.commands.value()).then(|| self.commands.clone()),
            theme: (self.theme != *s.theme.value()).then(|| self.theme.clone()),
            filters: (self.filters != *s.filters.value()).then(|| self.filters.clone()),
            openwith: (self.openwith != *s.openwith.value()).then(|| self.openwith.clone()),
            archives: (self.archives != *s.archives.value()).then(|| self.archives.clone()),
            ..SettingsChanges::default()
        };
        for (name, file) in &self.schemes {
            let stored = s.schemes.get(name);
            // *Reset all* also takes the file's unknown keys, so it changes a file whose value is empty already.
            let unknown = stored.is_some_and(|f| !f.unknown().is_empty());
            let reset_all = self.theme_page.reset_all.contains(name);
            if stored.map(SettingsFile::value) == Some(file) && !(reset_all && unknown) {
                continue;
            }
            // A built-in with nothing left has no override file, unless the file holds keys this build does not
            // know: only *Reset all* deletes those (D-130, T13).
            let empty = BUILTIN_SCHEMES.contains(&name.as_str())
                && *file == SchemeFile::default()
                && (reset_all || !unknown);
            changes
                .schemes
                .insert(name.clone(), (!empty).then(|| file.clone()));
        }
        for name in s.schemes.keys() {
            if !self.schemes.contains_key(name) {
                changes.schemes.insert(name.clone(), None);
            }
        }
        changes
    }

    /// True when `id` has unapplied edits.
    pub fn has_edits(&self, id: &FileId) -> bool {
        let c = self.changes();
        match id {
            FileId::Options => c.options.is_some(),
            FileId::Commands => c.commands.is_some(),
            FileId::Theme => c.theme.is_some(),
            FileId::Themes(name) => c.schemes.contains_key(name),
            FileId::Filters => c.filters.is_some(),
            // An open item panel is an edit: its row must stay the row it was opened on.
            FileId::OpenWith => c.openwith.is_some() || self.open_with.editing.is_some(),
            FileId::Archives => c.archives.is_some(),
            _ => false,
        }
    }

    fn only(changes: SettingsChanges, files: &[FileId]) -> SettingsChanges {
        let has = |id: &FileId| files.contains(id);
        SettingsChanges {
            options: changes.options.filter(|_| has(&FileId::Options)),
            commands: changes.commands.filter(|_| has(&FileId::Commands)),
            theme: changes.theme.filter(|_| has(&FileId::Theme)),
            schemes: changes
                .schemes
                .into_iter()
                .filter(|(name, _)| has(&FileId::Themes(name.clone())))
                .collect(),
            filters: changes.filters.filter(|_| has(&FileId::Filters)),
            openwith: changes.openwith.filter(|_| has(&FileId::OpenWith)),
            archives: changes.archives.filter(|_| has(&FileId::Archives)),
        }
    }

    fn is_empty(c: &SettingsChanges) -> bool {
        c.options.is_none()
            && c.commands.is_none()
            && c.theme.is_none()
            && c.schemes.is_empty()
            && c.filters.is_none()
            && c.openwith.is_none()
            && c.archives.is_none()
    }

    /// True when closing now loses something: unapplied edits or an open item panel.
    pub fn dirty(&self) -> bool {
        !Self::is_empty(&self.changes()) || self.open_with.editing.is_some()
    }

    fn ask_discard(&mut self) {
        self.requests.push(DialogRequest::Ask(Box::new(Prompt::ask(
            "Unapplied Changes",
            "Discard the changes that were not applied?",
            ButtonRole::Cancel,
            vec![
                (
                    ButtonRole::Other(1),
                    "Discard",
                    vec![Followup::Options(ConflictAnswer::Discard)],
                ),
                (ButtonRole::Cancel, "Keep editing", Vec::new()),
            ],
            "dialog-options",
        ))));
    }

    /// Apply or OK: the changed files go to the window's settings queue; nothing changed: nothing is written.
    fn start_save(&mut self, close: bool, changes: SettingsChanges, force: bool) {
        if Self::is_empty(&changes) {
            if close {
                self.requests.push(DialogRequest::Close);
            }
            return;
        }
        self.close_after = close;
        let (tx, rx) = crossbeam_channel::unbounded();
        self.saving = Some(rx);
        let snapshot = self.snapshot.clone();
        self.requests.push(DialogRequest::Settings {
            title: "save the settings".to_owned(),
            body: Box::new(move |svc: &Service| -> Result<Vec<PathBuf>, ApiError> {
                match svc.settings_save(&snapshot, &changes, force) {
                    Ok(dto) => {
                        let written = dto
                            .files
                            .iter()
                            .filter(|(_, _, o)| {
                                matches!(o, FileOutcome::Written | FileOutcome::Deleted)
                            })
                            .map(|(_, p, _)| p.clone())
                            .collect();
                        let _ = tx.send(Outcome::Saved(Box::new(dto)));
                        Ok(written)
                    }
                    Err(e) => {
                        let _ = tx.send(Outcome::Failed(e.to_string()));
                        Ok(Vec::new())
                    }
                }
            }),
        });
    }

    /// Takes `id` from `fresh`: its stamp and, with `value`, its working copy too.
    fn adopt_file(&mut self, id: &FileId, fresh: &SettingsSnapshot) {
        match id {
            FileId::Options => {
                self.snapshot.options = fresh.options.clone();
                self.options = fresh.options.value().clone();
                let options = self.options.clone();
                for field in &mut self.fields {
                    if let Some(value) = form_value(&options, field.path) {
                        field.value = value;
                    }
                }
            }
            FileId::Commands => {
                self.snapshot.commands = fresh.commands.clone();
                self.commands = fresh.commands.value().clone();
            }
            FileId::Theme => {
                self.snapshot.theme = fresh.theme.clone();
                self.theme = fresh.theme.value().clone();
            }
            FileId::Themes(name) => match fresh.schemes.get(name) {
                Some(file) => {
                    self.theme_page.reset_all.remove(name);
                    self.snapshot.schemes.insert(name.clone(), file.clone());
                    self.schemes.insert(name.clone(), file.value().clone());
                }
                None => {
                    self.snapshot.schemes.remove(name);
                    self.schemes.remove(name);
                }
            },
            FileId::Filters => {
                self.snapshot.filters = fresh.filters.clone();
                self.filters = fresh.filters.value().clone();
            }
            FileId::OpenWith => {
                self.snapshot.openwith = fresh.openwith.clone();
                self.openwith = fresh.openwith.value().clone();
                self.open_with.editing = None;
                self.open_with.capturing = false;
                let rows = self.openwith.app.len();
                self.open_with.selected = self.open_with.selected.filter(|i| *i < rows);
            }
            FileId::Archives => {
                self.snapshot.archives = fresh.archives.clone();
                self.archives = fresh.archives.value().clone();
            }
            _ => {}
        }
    }

    /// Every file of a snapshot.
    fn files_of(snapshot: &SettingsSnapshot) -> Vec<FileId> {
        let mut ids = vec![
            FileId::Options,
            FileId::Commands,
            FileId::Theme,
            FileId::Filters,
            FileId::OpenWith,
            FileId::Archives,
        ];
        ids.extend(snapshot.schemes.keys().cloned().map(FileId::Themes));
        ids
    }

    /// The window applied a newer snapshot: a file without unapplied edits follows it; a file with edits keeps its
    /// old stamp, so the next Apply meets `Conflict` (§1.4.7).
    pub fn follow(&mut self, fresh: &SettingsSnapshot) {
        if self.saving() {
            self.followed = Some(Box::new(fresh.clone()));
        } else {
            self.follow_now(fresh);
        }
    }

    /// Every file without unapplied edits takes `fresh` (its stamp too, so a later edit of it meets no false
    /// `Conflict`).
    fn follow_now(&mut self, fresh: &SettingsSnapshot) {
        self.snapshot.unreadable = fresh.unreadable.clone();
        let mut ids = Self::files_of(fresh);
        ids.extend(
            self.snapshot
                .schemes
                .keys()
                .filter(|k| !fresh.schemes.contains_key(*k))
                .cloned()
                .map(FileId::Themes),
        );
        let mut any = false;
        for id in &ids {
            if !self.has_edits(id) {
                self.adopt_file(id, fresh);
                any = true;
            }
        }
        if any {
            self.after_adopt();
        }
    }

    fn after_adopt(&mut self) {
        self.refresh_commands();
        self.refresh_scheme();
        self.theme_edited = true;
        self.check();
    }

    fn on_saved(&mut self, dto: &SettingsSaveDto) {
        let mut conflicts = Vec::new();
        let mut unreadable = BTreeMap::new();
        let mut errors = Vec::new();
        for (id, _, outcome) in &dto.files {
            match outcome {
                FileOutcome::Written | FileOutcome::Unchanged | FileOutcome::Deleted => {
                    self.adopt_file(id, &dto.snapshot);
                }
                FileOutcome::Conflict => conflicts.push(id.clone()),
                FileOutcome::Unreadable(why) => {
                    unreadable.insert(id.clone(), why.clone());
                }
                FileOutcome::Invalid(problems) => {
                    for p in problems {
                        errors.push(format!("{id}: {}: {}", p.key, p.message));
                    }
                }
                FileOutcome::Failed(e) => errors.push(format!("{id}: {e}")),
            }
        }
        self.follow_now(&dto.snapshot);
        self.after_adopt();
        let refused: Vec<FileId> = unreadable.keys().cloned().collect();
        self.snapshot.unreadable.extend(unreadable);
        // A file the page already names as unreadable fails every read back: nothing new to say, and OK may close.
        if let Some(e) = dto
            .reread_error
            .as_ref()
            .filter(|_| self.snapshot.unreadable.is_empty())
        {
            errors.push(format!(
                "The files above are saved, but reading the settings back failed: {e}"
            ));
        }
        if !errors.is_empty() {
            // OK closes only when every file of the run succeeded: a later Overwrite must not lose this edit.
            self.close_after = false;
            self.message = Some(errors.join("\n"));
        }
        if !refused.is_empty() {
            // First: *Overwrite* must not be on offer for these. A plain conflict beside them meets the prompt at the
            // next Apply, its edits intact.
            self.close_after = false;
            self.ask_unreadable(refused);
        } else if !conflicts.is_empty() {
            let names: Vec<String> = conflicts.iter().map(FileId::relative_path).collect();
            let text = format!(
                "{} changed on disk since this dialog read it.",
                names.join(", ")
            );
            self.conflicted = conflicts;
            self.requests.push(DialogRequest::Ask(Box::new(Prompt::ask(
                "Settings Changed on Disk",
                text,
                ButtonRole::Cancel,
                vec![
                    (
                        ButtonRole::Other(1),
                        "Overwrite",
                        vec![Followup::Options(ConflictAnswer::Overwrite)],
                    ),
                    (
                        ButtonRole::Other(2),
                        "Reload (discard my edits)",
                        vec![Followup::Options(ConflictAnswer::Reload)],
                    ),
                    (ButtonRole::Cancel, "Cancel", Vec::new()),
                ],
                "dialog-options",
            ))));
        } else if errors.is_empty() && self.close_after {
            self.requests.push(DialogRequest::Close);
        }
    }

    /// Files that cannot be read met an Apply: they are not overwritten, whatever is answered.
    fn ask_unreadable(&mut self, files: Vec<FileId>) {
        let names: Vec<String> = files.iter().map(FileId::relative_path).collect();
        let text = format!(
            "{} cannot be read by this version of Hikasa, so it cannot be overwritten from this dialog. The page says \
             why.\n\nReload discards the edits of it; Cancel keeps them, unsaved.",
            names.join(", ")
        );
        self.conflicted = files;
        self.requests.push(DialogRequest::Ask(Box::new(Prompt::ask(
            "Settings Cannot Be Read",
            text,
            ButtonRole::Cancel,
            vec![
                (
                    ButtonRole::Other(2),
                    "Reload (discard my edits)",
                    vec![Followup::Options(ConflictAnswer::Reload)],
                ),
                (ButtonRole::Cancel, "Cancel", Vec::new()),
            ],
            "dialog-options",
        ))));
    }

    /// The answer to the conflict prompt.
    pub fn answer(&mut self, answer: ConflictAnswer) {
        let files = std::mem::take(&mut self.conflicted);
        match answer {
            ConflictAnswer::Overwrite => {
                let changes = Self::only(self.changes(), &files);
                self.start_save(self.close_after, changes, true);
            }
            ConflictAnswer::Reload => self.reload_files(files),
            ConflictAnswer::Discard => self.requests.push(DialogRequest::Close),
        }
    }

    /// Reads `files` again and drops their edits (*Reload*, off the UI thread).
    pub fn reload_files(&mut self, files: Vec<FileId>) {
        if self.saving() {
            self.pending_reload.extend(files);
            return;
        }
        let (tx, rx) = crossbeam_channel::unbounded();
        self.saving = Some(rx);
        self.close_after = false;
        self.requests.push(DialogRequest::Settings {
            title: "read the settings".to_owned(),
            body: Box::new(move |svc: &Service| -> Result<Vec<PathBuf>, ApiError> {
                let outcome = match svc.settings_snapshot_partial() {
                    Ok(snapshot) => Outcome::Reloaded(Box::new(snapshot), files),
                    Err(e) => Outcome::Failed(e.to_string()),
                };
                let _ = tx.send(outcome);
                Ok(Vec::new())
            }),
        });
    }

    /// The key a capture waits for goes to its page.
    pub fn capturing(&self) -> bool {
        self.commands_page.capturing || self.open_with.capturing
    }
}

impl DialogModel for OptionsDialog {
    fn title(&self) -> String {
        "Options".to_owned()
    }

    fn help_topic(&self) -> &'static str {
        "dialog-options"
    }

    fn busy(&self) -> bool {
        self.saving()
    }

    fn size(&self) -> Option<(f32, f32)> {
        Some((860.0, 560.0))
    }

    fn buttons(&self) -> Vec<DialogButton> {
        let free = !self.saving();
        let valid = self.problems.is_empty();
        let button = |label: &str, role, enabled, default| DialogButton {
            label: label.to_owned(),
            role,
            enabled,
            default,
        };
        vec![
            button("Apply", ButtonRole::Apply, free && valid, false),
            button("OK", ButtonRole::Ok, free && valid, false),
            button("Cancel", ButtonRole::Cancel, free, false),
        ]
    }

    fn on_close(&mut self) -> bool {
        let ask = self.dirty();
        if ask {
            self.ask_discard();
        }
        ask
    }

    fn on_button(&mut self, role: ButtonRole, _svc: &Service) -> DialogStep {
        match role {
            ButtonRole::Apply | ButtonRole::Ok => {
                if !self.saving() && self.problems.is_empty() {
                    self.message = None;
                    // The open item panel is part of the page: taken in, or nothing is applied.
                    if self.open_with.editing.is_some() {
                        self.open_with_commit();
                        if self.open_with.editing.is_some() {
                            self.show_page("open_with");
                            self.message = Some(
                                "The Open With item is not valid: correct it or cancel it."
                                    .to_owned(),
                            );
                            return DialogStep::Stay;
                        }
                    }
                }
                if !self.saving() && self.problems.is_empty() {
                    let close = role == ButtonRole::Ok;
                    self.start_save(close, self.changes(), false);
                }
                DialogStep::Stay
            }
            ButtonRole::Cancel if !self.saving() => DialogStep::Close,
            _ => DialogStep::Stay,
        }
    }

    fn on_key(&mut self, key: &Shortcut) -> Option<ButtonRole> {
        if self.capturing() {
            self.capture(key);
            return None;
        }
        let esc = key.key == "Esc" && !key.ctrl && !key.alt && !key.shift && !key.cmd;
        if esc && !self.saving() && self.dirty() {
            self.ask_discard();
            return None;
        }
        // Enter belongs to the page's fields: no button is the default (a typed value is not an Apply).
        crate::dialog::default_key(&self.buttons(), key)
    }

    fn wants_keys(&self) -> bool {
        self.capturing()
    }

    fn poll(&mut self, _svc: &Service) -> bool {
        let Some(rx) = &self.saving else {
            return false;
        };
        let Ok(outcome) = rx.try_recv() else {
            return false;
        };
        self.saving = None;
        match outcome {
            Outcome::Saved(dto) => self.on_saved(&dto),
            Outcome::Reloaded(fresh, files) => {
                for id in &files {
                    self.adopt_file(id, &fresh);
                }
                self.follow_now(&fresh);
                self.after_adopt();
            }
            Outcome::Failed(e) => {
                self.close_after = false;
                self.message = Some(e);
            }
        }
        if let Some(fresh) = self.followed.take() {
            self.follow_now(&fresh);
        }
        if !self.pending_reload.is_empty() {
            let files = std::mem::take(&mut self.pending_reload);
            self.reload_files(files);
        }
        true
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
