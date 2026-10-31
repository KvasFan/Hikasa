//! The Export, Import and Restore wizards (M2 W4c, §1.4.14; D-131, SES-060…062): step dialogs over the W2 calls.
//! Every read runs on a job; what writes settings goes through the window's settings queue
//! ([`DialogRequest::Settings`]), so the written paths are acknowledged and the settings reload with no notice for
//! the GUI's own writes. Finish re-plans in core (T12); when the executed plan differs from the summary shown, the
//! result says so. Toolkit-free; `ui/wizards.rs` draws them.

use std::any::Any;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use hikasa_core::JobKind;
use hikasa_session::api::{ApiError, PlanDto, SettingsImportDto};
use hikasa_session::package::{
    Category, CategoryCounts, ExportDto, ExportRequest, ImportChoices, PACKAGE_EXTENSION,
    PlanAction, category_files, package_path,
};
use hikasa_session::{Service, api::SettingsRestoreDto};

use crate::dialog::{
    ButtonRole, DialogButton, DialogModel, DialogRequest, DialogStep, Followup, Prompt,
};
use crate::job::{Job, Wake};

fn button(label: &str, role: ButtonRole, enabled: bool, default: bool) -> DialogButton {
    DialogButton {
        label: label.to_owned(),
        role,
        enabled,
        default,
    }
}

/// The caption of a category.
pub fn category_caption(category: Category) -> &'static str {
    match category {
        Category::Options => "Program Options",
        Category::Theme => "Theme",
        Category::Commands => "Toolbars, Shortcuts, Menus",
        Category::State => "Program State",
        Category::Sessions => "Sessions",
        Category::Formats => "File Formats",
        Category::Workspaces => "Workspaces",
        Category::SystemInfo => "System Info",
    }
}

fn plural(category: Category) -> (&'static str, &'static str) {
    match category {
        Category::Sessions => ("session", "sessions"),
        Category::Formats => ("file format", "file formats"),
        Category::Workspaces => ("workspace", "workspaces"),
        Category::Theme => ("theme file", "theme files"),
        Category::Commands => ("command file", "command files"),
        Category::State => ("state file", "state files"),
        Category::Options | Category::SystemInfo => ("settings file", "settings files"),
    }
}

fn noun(category: Category, n: usize) -> String {
    let (one, many) = plural(category);
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The caption of a "delete existing" choice.
pub fn delete_caption(category: Category) -> String {
    let (_, many) = plural(category);
    format!("Delete existing {many}")
}

/// One category with its items, chosen by check boxes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategoryRow {
    /// The category.
    pub category: Category,
    /// The category is chosen.
    pub checked: bool,
    /// All its items (else the ticked ones).
    pub all: bool,
    /// The items and whether each is ticked.
    pub items: Vec<ItemRow>,
}

/// One item of a category.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemRow {
    /// The item name.
    pub name: String,
    /// Ticked.
    pub checked: bool,
    /// The plan's action (Import).
    pub action: Option<PlanAction>,
}

impl CategoryRow {
    fn names(&self) -> Vec<String> {
        self.items
            .iter()
            .filter(|i| i.checked)
            .map(|i| i.name.clone())
            .collect()
    }

    /// The category needs an item list (some items are not chosen).
    fn some(&self) -> bool {
        self.checked && !self.all
    }
}

fn pick_valid(rows: &[CategoryRow]) -> bool {
    rows.iter().any(|r| r.checked) && rows.iter().all(|r| !r.some() || !r.names().is_empty())
}

// ---- Export ----------------------------------------------------------------------------------------------------

/// What an export job found.
enum ExportDone {
    /// The target exists: confirm first.
    Exists,
    /// Written.
    Written(ExportDto),
}

/// The Export wizard.
pub struct ExportWizard {
    /// 0 categories and items, 1 target, 2 result.
    pub step: u8,
    /// The categories.
    pub rows: Vec<CategoryRow>,
    /// The target file.
    pub target: String,
    /// The written items.
    pub result: Option<ExportDto>,
    /// Why the wizard cannot go on.
    pub error: Option<String>,
    wake: Wake,
    lists: Option<Job<Vec<CategoryRow>>>,
    job: Option<Job<ExportDone>>,
    requests: Vec<DialogRequest>,
}

/// The default target (D-131): `~/hikasa-settings-<YYYY-MM-DD>.hikasa-pkg`, by the injected clock.
pub fn default_target(service: &Service) -> String {
    let date = service.clock().now().strftime("%Y-%m-%d").to_string();
    let name = format!("hikasa-settings-{date}.{PACKAGE_EXTENSION}");
    match &service.env().home {
        Some(home) => home.join(name).display().to_string(),
        None => name,
    }
}

impl ExportWizard {
    /// The wizard; its item lists are read on a job.
    pub fn new(service: &Service, wake: Wake) -> ExportWizard {
        let lists = Job::spawn(
            service,
            JobKind::VfsRead,
            "list the settings".into(),
            &wake,
            |svc, _| {
                let store = svc.settings_store();
                let dir = store.dir();
                Ok(Category::ALL
                    .into_iter()
                    .map(|category| CategoryRow {
                        category,
                        checked: category != Category::SystemInfo,
                        all: true,
                        items: if category.per_item() {
                            category_files(dir, category)
                                .into_iter()
                                .map(|(name, _)| ItemRow {
                                    name,
                                    checked: true,
                                    action: None,
                                })
                                .collect()
                        } else {
                            Vec::new()
                        },
                    })
                    .collect())
            },
        );
        ExportWizard {
            step: 0,
            rows: Vec::new(),
            target: default_target(service),
            result: None,
            error: None,
            wake,
            lists: Some(lists),
            job: None,
            requests: Vec::new(),
        }
    }

    /// The request of the choices.
    pub fn request(&self) -> ExportRequest {
        let mut request = ExportRequest::default();
        for row in self.rows.iter().filter(|r| r.checked) {
            if row.some() {
                request.items.insert(row.category, row.names());
            } else {
                request.categories.push(row.category);
            }
        }
        request
    }

    fn start(&mut self, service: &Service, overwrite: bool) {
        let request = self.request();
        let target = match package_path(&self.target) {
            Ok(target) => target,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };
        self.error = None;
        self.job = Some(Job::spawn(
            service,
            JobKind::VfsWrite,
            "export the settings".into(),
            &self.wake,
            move |svc, _| {
                if !overwrite && target.exists() {
                    return Ok(ExportDone::Exists);
                }
                Ok(ExportDone::Written(svc.settings_export(&request, &target)?))
            },
        ));
    }

    /// The overwrite was confirmed.
    pub fn overwrite(&mut self, service: &Service) {
        self.start(service, true);
    }

    /// True while a job runs.
    pub fn busy(&self) -> bool {
        self.lists.is_some() || self.job.is_some()
    }
}

impl DialogModel for ExportWizard {
    fn title(&self) -> String {
        "Export Settings".to_owned()
    }

    fn help_topic(&self) -> &'static str {
        "wizard-export"
    }

    fn busy(&self) -> bool {
        Self::busy(self)
    }

    fn size(&self) -> Option<(f32, f32)> {
        Some((560.0, 420.0))
    }

    fn buttons(&self) -> Vec<DialogButton> {
        let free = !self.busy();
        match self.step {
            0 => vec![
                button(
                    "Next",
                    ButtonRole::Next,
                    free && pick_valid(&self.rows),
                    true,
                ),
                button("Cancel", ButtonRole::Cancel, true, false),
            ],
            1 => vec![
                button("Back", ButtonRole::Back, free, false),
                button(
                    "Finish",
                    ButtonRole::Finish,
                    free && !self.target.trim().is_empty(),
                    true,
                ),
                button("Cancel", ButtonRole::Cancel, free, false),
            ],
            _ => vec![button("Close", ButtonRole::Close, true, true)],
        }
    }

    fn on_button(&mut self, role: ButtonRole, svc: &Service) -> DialogStep {
        match role {
            ButtonRole::Next if pick_valid(&self.rows) => self.step = 1,
            ButtonRole::Back => self.step = 0,
            ButtonRole::Finish => self.start(svc, false),
            ButtonRole::Cancel if self.job.is_none() => return DialogStep::Close,
            ButtonRole::Close => return DialogStep::Close,
            _ => {}
        }
        DialogStep::Stay
    }

    fn poll(&mut self, _svc: &Service) -> bool {
        if let Some(job) = &self.lists
            && let Some(result) = job.try_result()
        {
            self.lists = None;
            match result {
                Ok(rows) => self.rows = rows,
                Err(e) => self.error = Some(e.to_string()),
            }
            return true;
        }
        if let Some(job) = &self.job
            && let Some(result) = job.try_result()
        {
            self.job = None;
            match result {
                Ok(ExportDone::Exists) => {
                    let text = format!("{} exists. Replace it?", self.target.trim());
                    self.requests.push(DialogRequest::Ask(Box::new(Prompt::ask(
                        "Export Settings",
                        text,
                        ButtonRole::Cancel,
                        vec![
                            (ButtonRole::Yes, "Replace", vec![Followup::WizardOverwrite]),
                            (ButtonRole::Cancel, "Cancel", Vec::new()),
                        ],
                        "wizard-export",
                    ))));
                }
                Ok(ExportDone::Written(dto)) => {
                    self.result = Some(dto);
                    self.step = 2;
                }
                Err(e) => self.error = Some(e.to_string()),
            }
            return true;
        }
        false
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

// ---- Import ----------------------------------------------------------------------------------------------------

/// The summary lines of a plan (step 3): one per category, then the deletes and the locked sessions kept.
pub fn summary_lines(plan: &PlanDto) -> Vec<String> {
    let mut lines = Vec::new();
    for (category, counts) in &plan.summary {
        let CategoryCounts {
            added,
            replaced,
            deleted,
        } = *counts;
        let imported = added + replaced;
        if imported > 0 {
            let mut line = format!("{} will be imported", noun(*category, imported));
            if replaced > 0 {
                line.push_str(&format!(", replacing {replaced} existing"));
            }
            line.push('.');
            lines.push(line);
        }
        if deleted > 0 {
            lines.push(format!(
                "{} will be deleted first.",
                noun(*category, deleted)
            ));
        }
    }
    // Only sessions lock (D-91), both those the package would replace and those "delete existing" would delete.
    if plan.locked_kept > 0 {
        lines.push(format!(
            "{} locked: kept as is.",
            noun(Category::Sessions, plan.locked_kept)
        ));
    }
    if lines.is_empty() {
        lines.push("Nothing will change.".to_owned());
    }
    lines
}

/// What a read job of the Import wizard found.
enum ImportRead {
    /// Step 1: the package's plan with every category.
    Package(SettingsImportDto),
    /// Step 2: the dry run of the exact choices.
    Summary(SettingsImportDto),
}

/// What the settings job sends back.
enum Finished {
    Import(Result<Box<SettingsImportDto>, String>),
    Restore(Result<Box<SettingsRestoreDto>, String>),
}

/// What an open Import wizard does with another package (IPC, the command line; D-127).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PackageOffer {
    /// Taken (step 1), or the package it shows already.
    Taken,
    /// The choices made so far would go: ask.
    Ask,
    /// Done with its package: a fresh wizard takes this one.
    Fresh,
    /// A job runs: it cannot change now.
    Refused,
}

/// The Import wizard.
pub struct ImportWizard {
    /// 0 package, 1 categories and items, 2 summary, 3 result.
    pub step: u8,
    /// The package path.
    pub path: String,
    /// The categories the package holds.
    pub rows: Vec<CategoryRow>,
    /// "Delete all existing …" per category.
    pub delete_existing: BTreeSet<Category>,
    /// "Include locked sessions".
    pub force: bool,
    /// The summary shown (the dry run of the exact choices).
    pub summary: Option<PlanDto>,
    /// The execution.
    pub result: Option<SettingsImportDto>,
    /// Why the wizard cannot go on.
    pub error: Option<String>,
    wake: Wake,
    job: Option<Job<ImportRead>>,
    running: Option<crossbeam_channel::Receiver<Finished>>,
    requests: Vec<DialogRequest>,
}

impl ImportWizard {
    /// The wizard, prefilled with `path` (the command line, IPC).
    pub fn new(path: Option<&Path>, wake: Wake) -> ImportWizard {
        ImportWizard {
            step: 0,
            path: path.map(|p| p.display().to_string()).unwrap_or_default(),
            rows: Vec::new(),
            delete_existing: BTreeSet::new(),
            force: false,
            summary: None,
            result: None,
            error: None,
            wake,
            job: None,
            running: None,
            requests: Vec::new(),
        }
    }

    /// The choices of the check boxes.
    pub fn choices(&self) -> ImportChoices {
        let mut choices = ImportChoices {
            delete_existing: self.delete_existing.clone(),
            force: self.force,
            ..ImportChoices::default()
        };
        for row in self.rows.iter().filter(|r| r.checked) {
            if row.all || row.items.iter().all(|i| i.checked) {
                choices.categories.push(row.category);
            } else {
                choices.items.insert(row.category, row.names());
            }
        }
        choices
    }

    /// True while a job runs.
    pub fn busy(&self) -> bool {
        self.job.is_some() || self.running.is_some()
    }

    /// True when the summary differs from what ran: the result page says so (D-131).
    pub fn plan_changed(&self) -> bool {
        matches!(
            (&self.summary, &self.result),
            (Some(shown), Some(done)) if *shown != done.plan
        )
    }

    /// Another package arrived (IPC while the wizard is open): never ignored in silence, never mixed into the
    /// choices made for the package shown.
    pub fn offer(&mut self, path: &Path) -> PackageOffer {
        let text = path.display().to_string();
        if self.path.trim() == text {
            return PackageOffer::Taken;
        }
        if self.busy() {
            return PackageOffer::Refused;
        }
        match self.step {
            0 => {
                self.path = text;
                self.error = None;
                PackageOffer::Taken
            }
            3 => PackageOffer::Fresh,
            _ => PackageOffer::Ask,
        }
    }

    fn read(&mut self, service: &Service, summary: bool) {
        let path = match package_path(&self.path) {
            Ok(path) => path,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };
        let choices = if summary {
            self.choices()
        } else {
            ImportChoices::default()
        };
        self.error = None;
        self.job = Some(Job::spawn(
            service,
            JobKind::VfsRead,
            "read the package".into(),
            &self.wake,
            move |svc, _| {
                let dto = svc.settings_import(&path, &choices, true)?;
                Ok(if summary {
                    ImportRead::Summary(dto)
                } else {
                    ImportRead::Package(dto)
                })
            },
        ));
    }

    fn rows_of(plan: &PlanDto) -> Vec<CategoryRow> {
        let mut by: BTreeMap<Category, Vec<ItemRow>> = BTreeMap::new();
        for item in &plan.items {
            by.entry(item.category).or_default().push(ItemRow {
                name: item.name.clone(),
                checked: true,
                action: Some(item.action),
            });
        }
        by.into_iter()
            .map(|(category, items)| CategoryRow {
                category,
                checked: true,
                all: true,
                items,
            })
            .collect()
    }

    fn finish(&mut self) {
        let path = match package_path(&self.path) {
            Ok(path) => path,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };
        let choices = self.choices();
        let (tx, rx) = crossbeam_channel::unbounded();
        self.running = Some(rx);
        self.requests.push(DialogRequest::Settings {
            title: "import the settings".to_owned(),
            body: Box::new(move |svc: &Service| -> Result<Vec<PathBuf>, ApiError> {
                // The paths come from the plan: acknowledged even when one item fails.
                let planned = svc.settings_import(&path, &choices, true);
                let mut paths: Vec<PathBuf> = Vec::new();
                if let Ok(dto) = &planned {
                    paths.extend(dto.plan.items.iter().map(|i| PathBuf::from(&i.path)));
                    paths.extend(dto.plan.deletes.iter().map(PathBuf::from));
                }
                let done = svc
                    .settings_import(&path, &choices, false)
                    .map(Box::new)
                    .map_err(|e| e.to_string());
                let _ = tx.send(Finished::Import(done));
                Ok(paths)
            }),
        });
    }
}

impl DialogModel for ImportWizard {
    fn title(&self) -> String {
        "Import Settings".to_owned()
    }

    fn help_topic(&self) -> &'static str {
        "wizard-import"
    }

    fn busy(&self) -> bool {
        Self::busy(self)
    }

    fn size(&self) -> Option<(f32, f32)> {
        Some((620.0, 460.0))
    }

    fn buttons(&self) -> Vec<DialogButton> {
        let free = !self.busy();
        match self.step {
            0 => vec![
                button(
                    "Next",
                    ButtonRole::Next,
                    free && !self.path.trim().is_empty(),
                    true,
                ),
                button("Cancel", ButtonRole::Cancel, free, false),
            ],
            1 => vec![
                button("Back", ButtonRole::Back, free, false),
                button(
                    "Next",
                    ButtonRole::Next,
                    free && pick_valid(&self.rows),
                    true,
                ),
                button("Cancel", ButtonRole::Cancel, free, false),
            ],
            2 => vec![
                button("Back", ButtonRole::Back, free, false),
                button("Finish", ButtonRole::Finish, free, true),
                button("Cancel", ButtonRole::Cancel, free, false),
            ],
            _ => vec![button("Close", ButtonRole::Close, true, true)],
        }
    }

    fn on_button(&mut self, role: ButtonRole, svc: &Service) -> DialogStep {
        match (role, self.step) {
            (ButtonRole::Next, 0) if !self.busy() => self.read(svc, false),
            (ButtonRole::Next, 1) if !self.busy() && pick_valid(&self.rows) => self.read(svc, true),
            (ButtonRole::Back, 1 | 2) if !self.busy() => self.step -= 1,
            (ButtonRole::Finish, 2) if !self.busy() => self.finish(),
            (ButtonRole::Cancel, _) if !self.busy() => return DialogStep::Close,
            (ButtonRole::Close, _) => return DialogStep::Close,
            _ => {}
        }
        DialogStep::Stay
    }

    fn poll(&mut self, _svc: &Service) -> bool {
        if let Some(job) = &self.job
            && let Some(result) = job.try_result()
        {
            self.job = None;
            match result {
                Ok(ImportRead::Package(dto)) => {
                    self.rows = Self::rows_of(&dto.plan);
                    self.delete_existing.clear();
                    self.force = false;
                    if self.rows.is_empty() {
                        self.error = Some("The package holds nothing to import.".to_owned());
                    } else {
                        self.step = 1;
                    }
                }
                Ok(ImportRead::Summary(dto)) => {
                    self.summary = Some(dto.plan);
                    self.step = 2;
                }
                Err(e) => self.error = Some(e.to_string()),
            }
            return true;
        }
        if let Some(rx) = &self.running
            && let Ok(Finished::Import(done)) = rx.try_recv()
        {
            self.running = None;
            match done {
                Ok(dto) => {
                    self.result = Some(*dto);
                    self.step = 3;
                }
                Err(e) => self.error = Some(e),
            }
            self.requests.push(DialogRequest::Refresh);
            return true;
        }
        false
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

// ---- Restore ---------------------------------------------------------------------------------------------------

/// The categories Restore offers.
pub const RESTORE_CATEGORIES: [Category; 5] = [
    Category::Options,
    Category::Theme,
    Category::Commands,
    Category::State,
    Category::Formats,
];

/// The Restore wizard.
pub struct RestoreWizard {
    /// 0 categories, 1 the files to delete, 2 result.
    pub step: u8,
    /// The categories and whether each is chosen.
    pub categories: Vec<(Category, bool)>,
    /// `_defaults` of the sessions are restored with the category `sessions`.
    pub session_defaults: bool,
    /// "Delete all named sessions".
    pub delete_sessions: bool,
    /// "Delete all workspaces".
    pub delete_workspaces: bool,
    /// "Include locked sessions".
    pub force: bool,
    /// The dry run shown.
    pub plan: Option<SettingsRestoreDto>,
    /// The execution.
    pub result: Option<SettingsRestoreDto>,
    /// Why the wizard cannot go on.
    pub error: Option<String>,
    wake: Wake,
    job: Option<Job<SettingsRestoreDto>>,
    running: Option<crossbeam_channel::Receiver<Finished>>,
    requests: Vec<DialogRequest>,
}

impl RestoreWizard {
    /// The wizard.
    pub fn new(wake: Wake) -> RestoreWizard {
        RestoreWizard {
            step: 0,
            categories: RESTORE_CATEGORIES.iter().map(|c| (*c, true)).collect(),
            session_defaults: true,
            delete_sessions: false,
            delete_workspaces: false,
            force: false,
            plan: None,
            result: None,
            error: None,
            wake,
            job: None,
            running: None,
            requests: Vec::new(),
        }
    }

    /// The chosen categories.
    pub fn chosen(&self) -> Vec<Category> {
        let mut out: Vec<Category> = self
            .categories
            .iter()
            .filter(|(_, on)| *on)
            .map(|(c, _)| *c)
            .collect();
        if self.session_defaults {
            out.push(Category::Sessions);
        }
        out
    }

    /// True while a job runs.
    pub fn busy(&self) -> bool {
        self.job.is_some() || self.running.is_some()
    }

    fn valid(&self) -> bool {
        !self.chosen().is_empty() || self.delete_sessions || self.delete_workspaces
    }

    /// True when the files deleted differ from the plan shown (D-131).
    pub fn plan_changed(&self) -> bool {
        let paths = |d: &SettingsRestoreDto| -> Vec<String> {
            d.files.iter().map(|f| f.path.clone()).collect()
        };
        matches!(
            (&self.plan, &self.result),
            (Some(shown), Some(done)) if paths(shown) != paths(done)
        )
    }
}

impl DialogModel for RestoreWizard {
    fn title(&self) -> String {
        "Restore Factory Defaults".to_owned()
    }

    fn help_topic(&self) -> &'static str {
        "wizard-restore"
    }

    fn busy(&self) -> bool {
        Self::busy(self)
    }

    fn size(&self) -> Option<(f32, f32)> {
        Some((560.0, 420.0))
    }

    fn buttons(&self) -> Vec<DialogButton> {
        let free = !self.busy();
        match self.step {
            0 => vec![
                button("Next", ButtonRole::Next, free && self.valid(), true),
                button("Cancel", ButtonRole::Cancel, free, false),
            ],
            1 => vec![
                button("Back", ButtonRole::Back, free, false),
                button("Finish", ButtonRole::Finish, free, false),
                button("Cancel", ButtonRole::Cancel, free, true),
            ],
            _ => vec![button("Close", ButtonRole::Close, true, true)],
        }
    }

    fn on_button(&mut self, role: ButtonRole, svc: &Service) -> DialogStep {
        match (role, self.step) {
            (ButtonRole::Next, 0) if !self.busy() && self.valid() => {
                let (categories, sessions, workspaces, force) = (
                    self.chosen(),
                    self.delete_sessions,
                    self.delete_workspaces,
                    self.force,
                );
                self.error = None;
                self.job = Some(Job::spawn(
                    svc,
                    JobKind::VfsRead,
                    "plan the restore".into(),
                    &self.wake,
                    move |svc, _| {
                        svc.settings_restore_categories(
                            &categories,
                            sessions,
                            workspaces,
                            force,
                            true,
                        )
                    },
                ));
            }
            (ButtonRole::Back, 1) if !self.busy() => self.step = 0,
            (ButtonRole::Finish, 1) if !self.busy() => {
                let (categories, sessions, workspaces, force) = (
                    self.chosen(),
                    self.delete_sessions,
                    self.delete_workspaces,
                    self.force,
                );
                let (tx, rx) = crossbeam_channel::unbounded();
                self.running = Some(rx);
                self.requests.push(DialogRequest::Settings {
                    title: "restore the defaults".to_owned(),
                    body: Box::new(move |svc: &Service| -> Result<Vec<PathBuf>, ApiError> {
                        let run = |dry| {
                            svc.settings_restore_categories(
                                &categories,
                                sessions,
                                workspaces,
                                force,
                                dry,
                            )
                        };
                        let paths = run(true).map_or_else(
                            |_| Vec::new(),
                            |d| d.files.iter().map(|f| PathBuf::from(&f.path)).collect(),
                        );
                        let done = run(false).map(Box::new).map_err(|e| e.to_string());
                        let _ = tx.send(Finished::Restore(done));
                        Ok(paths)
                    }),
                });
            }
            (ButtonRole::Cancel, _) if !self.busy() => return DialogStep::Close,
            (ButtonRole::Close, _) => return DialogStep::Close,
            _ => {}
        }
        DialogStep::Stay
    }

    fn poll(&mut self, _svc: &Service) -> bool {
        if let Some(job) = &self.job
            && let Some(result) = job.try_result()
        {
            self.job = None;
            match result {
                Ok(plan) => {
                    self.plan = Some(plan);
                    self.step = 1;
                }
                Err(e) => self.error = Some(e.to_string()),
            }
            return true;
        }
        if let Some(rx) = &self.running
            && let Ok(Finished::Restore(done)) = rx.try_recv()
        {
            self.running = None;
            match done {
                Ok(dto) => {
                    self.result = Some(*dto);
                    self.step = 2;
                }
                Err(e) => self.error = Some(e),
            }
            self.requests.push(DialogRequest::Refresh);
            return true;
        }
        false
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
