//! The host of a script over the Service (§1.8.5, §1.8.6, D-219): one invisible Folder Compare session, and each command
//! through the public calls `folder op`, `folder sync` and `report` make (T11): `folder_prepare_in`/`folder_sync_prepare`,
//! `folder_run`, `folder_selection`, `folder_op_plan`, `folder_confirmation_strict`, `folder_op_execute`, `folder_refresh`,
//! `folder_sync_plan`, `folder_sync_execute`, `folder_contents`, `report_request`, `report_plan`, `report_execute`,
//! `text_compare`, `vfs_run`. Nothing here plans, compares, renders or writes by itself.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::sync::Arc;
use std::time::Duration;

use hikasa_core::{CancelToken, ErrorReport, JobKind, RelPath, Side, Spec, Warning, WarningCode};
use hikasa_folder::ops::{
    ConfirmMode, FolderOp, OpPlan, ReadOnlySides, Recycle, RenameMask, TouchFrom,
};
use hikasa_folder::select::{Selection, SideSet};
use hikasa_folder::sync::{SyncOptions, sync_folder_rules};
use hikasa_folder::{Change, DateFilter, FolderModel, NodeId, NullObserver};
use hikasa_report::{ReportKind, ReportScope, ReportWords};
use hikasa_script::{
    Args, CommandName, CommandOutcome, ConfirmWord, CopyToArgs, ExpandArgs, FilterArgs, HostUi,
    LineKind, LoadArgs, LocalTime, LogArgs, LogLevel, RenameArgs, ReportArgs, Resolved, RunState,
    Script, ScriptEvent, ScriptHost, Slot, SyncArgs, SyncMode, TouchArgs, WordChecker,
};
use hikasa_vfs::ops::OverwritePolicy;
use serde::Serialize;

use super::dto::{
    FOLDER_COMPARE_SCHEMA, FOLDER_CONTENTS_SCHEMA, FOLDER_OP_SCHEMA, FOLDER_SYNC_SCHEMA,
    REPORT_SCHEMA,
};
use super::folder_ops::{ContentsKind, ContentsRequest, FolderOpOutcome, item_line, summary_text};
use super::report::{ReportFlags, ReportSource, ReportTarget};
use super::script::{
    SCRIPT_JSON_ITEMS_MAX, STDOUT_NONE, STDOUT_SHARED, ScriptRunRequest, ScriptUi, Vocabulary,
    names_version, script_sides,
};
use super::script_log::{LineSink, LogFile, stamped};
use super::sessions::SessionRunRequest;
use super::{
    ApiError, ApiOp, ExpandWords, FolderCompareRequest, FolderCompareResponse, FolderOpRequest,
    FolderSyncRequest, OpOverrides, OpRequest, PreparedFolderCompare, SelectRequest, Service,
    TextCompareRequest, VfsOpRequest, resolve_folder_display,
};
use crate::jobs::JobControl;
use crate::sessions::SessionRef;

/// What one command keeps for `hikasa.script.v1`.
#[derive(Debug, Clone, Default)]
pub(crate) struct Kept {
    pub(crate) schema: Option<&'static str>,
    pub(crate) result: Option<serde_json::Value>,
    pub(crate) items_omitted: u64,
    pub(crate) error: Option<ErrorReport>,
    pub(crate) warnings: Vec<Warning>,
}

/// The loaded comparison.
struct Loaded {
    request: FolderCompareRequest,
    prepared: PreparedFolderCompare,
    response: FolderCompareResponse,
    read_only: ReadOnlySides,
}

/// Which folders are open.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Expanded {
    All,
    Paths(BTreeSet<String>),
}

/// The log of a run: the level, the file, and the adapter's interface (§1.8.9).
struct LogUi<'a> {
    ui: &'a mut dyn ScriptUi,
    level: LogLevel,
    file: Option<Box<dyn LineSink + 'a>>,
    service: &'a Service,
    /// Warnings of the log itself (a file that failed).
    warnings: Vec<Warning>,
    /// The `run` line of the current command: a file opened by `log` starts with it (its own line).
    last_run: Option<(Option<u32>, String)>,
}

impl HostUi for LogUi<'_> {
    fn event(&mut self, event: ScriptEvent) {
        if let ScriptEvent::Log { kind, line, text } = &event {
            if *kind == LineKind::Run {
                self.last_run = Some((*line, text.clone()));
            }
            if !self.level.shows(*kind) {
                return;
            }
            if let Some(file) = &mut self.file {
                let now = local_time(self.service);
                if let Err(e) = file.write_line(&stamped(now, *kind, *line, text)) {
                    self.file = None;
                    self.warnings.push(Warning {
                        code: WarningCode::LogNotWritten,
                        message: format!("the script's log file could not be written: {e}"),
                        spec: None,
                    });
                }
            }
        }
        self.ui.event(event);
    }

    fn can_ask(&self) -> bool {
        self.ui.can_ask()
    }

    fn ask_continue(&mut self, line: u32, message: &str) -> bool {
        self.ui.ask_continue(line, message)
    }

    fn beep(&mut self) {
        self.ui.beep();
    }
}

/// The local time of the Service's clock in the user's zone (T1).
pub(crate) fn local_time(service: &Service) -> LocalTime {
    let now = service.clock().now();
    LocalTime {
        year: now.year(),
        month: u8::try_from(now.month()).unwrap_or(1),
        day: u8::try_from(now.day()).unwrap_or(1),
        hour: u8::try_from(now.hour()).unwrap_or(0),
        minute: u8::try_from(now.minute()).unwrap_or(0),
        second: u8::try_from(now.second()).unwrap_or(0),
    }
}

/// The host of one run.
pub(crate) struct Host<'a> {
    service: &'a Service,
    script: &'a Script,
    req: &'a ScriptRunRequest,
    job: &'a JobControl,
    log: LogUi<'a>,
    stdout: Option<&'a mut dyn Write>,
    /// The settings a load of folders uses (left and right empty).
    settings: FolderCompareRequest,
    /// The loaded session's sync.
    preset: Option<SyncOptions>,
    loaded: Option<Loaded>,
    expanded: Expanded,
    selection: Selection,
    started: jiff::Timestamp,
    kept: BTreeMap<usize, Kept>,
    warnings: Vec<Warning>,
    checker: Vocabulary<'a>,
}

/// A command's failure.
fn failed(message: impl Into<String>) -> CommandOutcome {
    CommandOutcome::Failed {
        message: message.into(),
    }
}

/// The text of `value` (a word of a slot that is UTF-8 after expansion, §1.8.4).
fn utf8(value: &OsStr) -> Result<&str, CommandOutcome> {
    value.to_str().ok_or_else(|| {
        failed(format!(
            "`{}` is not valid UTF-8",
            hikasa_script::display_os(value)
        ))
    })
}

/// The side a direction writes: `left->right` writes the right.
fn written_by(direction: &str) -> Side {
    if direction.to_ascii_lowercase().ends_with("->right") {
        Side::Right
    } else {
        Side::Left
    }
}

/// A spec as the log shows it: its canonical form, with the bytes of a local path that are not UTF-8 escaped (`\xFF`)
/// as the `run` lines show them (A8.20), never replaced.
fn spec_shown(spec: &Spec) -> String {
    match spec {
        Spec::Local(path) => hikasa_script::display_os(path.as_path().as_os_str()),
        Spec::ArchiveMember { archive, member } => format!("{}/{member}", spec_shown(archive)),
        other => other.canonical(),
    }
}

/// The settings of `folder-compare`'s defaults (`load <default>`, and a run's start).
fn default_settings(service: &Service) -> FolderCompareRequest {
    let mut request = FolderCompareRequest::default();
    if let Ok(session) = service.session_defaults(crate::sessions::SessionType::FolderCompare) {
        if let crate::sessions::SessionRules::Folder(rules) = &session.rules {
            request.base_rules = Some(rules.clone());
        }
        request.base_text_rules = session.text_rules.clone();
    }
    request
}

/// Items of a JSON list past `max` are dropped and counted.
fn cap_items(value: &mut serde_json::Value, max: usize) -> u64 {
    let mut omitted = 0u64;
    if let Some(object) = value.as_object_mut() {
        for key in ["items", "rows"] {
            if let Some(serde_json::Value::Array(list)) = object.get_mut(key)
                && list.len() > max
            {
                omitted += (list.len() - max) as u64;
                list.truncate(max);
            }
        }
    }
    omitted
}

impl<'a> Host<'a> {
    pub(crate) fn new(
        service: &'a Service,
        script: &'a Script,
        req: &'a ScriptRunRequest,
        ui: &'a mut dyn ScriptUi,
        stdout: Option<&'a mut dyn Write>,
        job: &'a JobControl,
    ) -> Host<'a> {
        let stdout_refusal = if req.json {
            Some(STDOUT_SHARED)
        } else if req.stdout_reports {
            None
        } else {
            Some(STDOUT_NONE)
        };
        Host {
            service,
            script,
            req,
            job,
            log: LogUi {
                ui,
                level: LogLevel::Normal,
                file: None,
                service,
                warnings: Vec::new(),
                last_run: None,
            },
            stdout,
            settings: default_settings(service),
            preset: None,
            loaded: None,
            expanded: Expanded::Paths(BTreeSet::new()),
            selection: Selection::default(),
            started: service.clock().now().timestamp(),
            kept: BTreeMap::new(),
            warnings: Vec::new(),
            checker: Vocabulary {
                service,
                stdout_refusal,
            },
        }
    }

    /// What the run kept per command, and its warnings.
    pub(crate) fn finish(self) -> (BTreeMap<usize, Kept>, Vec<Warning>) {
        let mut warnings = self.warnings;
        warnings.extend(self.log.warnings);
        (self.kept, warnings)
    }

    /// A job of `kind` inside the run: the run's cancel and progress.
    fn sub_job(&self, kind: JobKind) -> JobControl {
        JobControl {
            cancel: self.job.cancel.clone(),
            ..self.service.job(kind, self.job.progress.clone())
        }
    }

    fn log(&mut self, kind: LineKind, line: u32, text: String) {
        self.log.event(ScriptEvent::Log {
            kind,
            line: Some(line),
            text,
        });
    }

    fn warn(&mut self, line: u32, warnings: &[Warning]) {
        for warning in warnings {
            self.log(
                LineKind::Warning,
                line,
                format!("[{}] {}", warning.code, warning.message),
            );
        }
        self.warnings.extend(warnings.iter().cloned());
    }

    /// Keeps `result` (with `--json` only).
    fn keep<T: Serialize>(
        &mut self,
        index: usize,
        schema: &'static str,
        result: &T,
        warnings: &[Warning],
    ) {
        let entry = self.kept.entry(index).or_default();
        entry.warnings.extend(warnings.iter().cloned());
        if !self.req.json {
            return;
        }
        let max = if self.req.items_max == 0 {
            SCRIPT_JSON_ITEMS_MAX
        } else {
            self.req.items_max
        };
        if let Ok(mut value) = serde_json::to_value(result) {
            entry.items_omitted = cap_items(&mut value, max);
            entry.result = Some(value);
            entry.schema = Some(schema);
        }
    }

    /// Keeps the error of a failed call and returns the command's outcome.
    fn fail_with(&mut self, index: usize, error: &ApiError) -> CommandOutcome {
        if error.kind() == hikasa_core::ErrorKind::Cancelled || self.job.cancel.is_cancelled() {
            return CommandOutcome::Cancelled;
        }
        self.kept.entry(index).or_default().error = Some(error.report());
        failed(error.to_string())
    }

    // ---- the comparison -------------------------------------------------------------------------------------------

    /// The display a headless command sees: `all` with the folder structure (§1.8.5).
    fn display(&self, iu: bool) -> Result<hikasa_folder::DisplayOptions, ApiError> {
        resolve_folder_display(Some("all"), None, false, iu)
    }

    /// The open folders in words.
    fn expand_words(&self) -> ExpandWords {
        match &self.expanded {
            Expanded::All => ExpandWords::All,
            Expanded::Paths(paths) if paths.is_empty() => ExpandWords::None,
            Expanded::Paths(paths) => ExpandWords::Paths(paths.iter().cloned().collect()),
        }
    }

    /// The selection a command acts on.
    fn op_select(&self, iu: bool) -> Result<SelectRequest, ApiError> {
        Ok(SelectRequest {
            masks: Vec::new(),
            paths: Vec::new(),
            selection: Some(self.selection.clone()),
            include_hidden: None,
            display: Some(self.display(iu)?),
            expand: self.expand_words(),
            iu,
        })
    }

    /// The sync options of a `sync` (its words over the session's preset or the defaults), `Ok(None)` when it names no
    /// direction and no session holds one.
    fn sync_options_of(&self, args: &SyncArgs<String>) -> Result<Option<SyncOptions>, ApiError> {
        let preset = args.mode.as_ref().map(|(mode, word)| {
            let lower = word.to_ascii_lowercase();
            let way = if lower == "all" {
                "both"
            } else if lower.ends_with("->right") {
                "right"
            } else {
                "left"
            };
            match mode {
                SyncMode::Update => format!("update-{way}"),
                SyncMode::Mirror => format!("mirror-{way}"),
            }
        });
        if preset.is_none() && self.preset.is_none() {
            return Ok(None);
        }
        let words = crate::ipc::SyncWords {
            preset,
            create_empty: args.create_empty,
            ..crate::ipc::SyncWords::default()
        };
        let defaults = SyncOptions::default();
        let base = self.preset.as_ref().unwrap_or(&defaults);
        let windows = self.service.env().platform == crate::host::Platform::Windows;
        super::sync_options(&words, base, windows).map(Some)
    }

    /// The sync the comparison of the command at `index` will serve, when the next command that uses a comparison is a
    /// `sync` whose words are known: the load compares with the sync's rules, as `folder sync` does (one scan).
    fn sync_ahead(&self, index: usize) -> Option<SyncOptions> {
        for command in self.script.commands().skip(index + 1) {
            match command.name {
                CommandName::Option | CommandName::Log | CommandName::Beep => {}
                CommandName::Sync => {
                    let Args::Sync(args) = &command.args else {
                        return None;
                    };
                    if args.mode.as_ref().is_some_and(|(_, w)| w.expands) {
                        return None;
                    }
                    let args = SyncArgs {
                        visible: args.visible,
                        create_empty: args.create_empty,
                        mode: args.mode.as_ref().map(|(m, w)| {
                            (
                                *m,
                                hikasa_script::normalize_sides(
                                    Slot::SyncDirection,
                                    &w.text,
                                    w.quoted,
                                )
                                .into_owned(),
                            )
                        }),
                    };
                    return self.sync_options_of(&args).ok().flatten();
                }
                _ => return None,
            }
        }
        None
    }

    /// Compares `request` (with the sync's rules when `sync`): a base that cannot be loaded is 107.
    fn compare(
        &mut self,
        index: usize,
        request: FolderCompareRequest,
        sync: Option<&SyncOptions>,
        read_only: ReadOnlySides,
    ) -> Result<Vec<Warning>, CommandOutcome> {
        let job = self.sub_job(JobKind::FolderCompare);
        let prepared = match sync {
            Some(options) => self.service.folder_sync_prepare(&request, options, &job),
            None => self.service.folder_prepare_in(&request, &job),
        };
        let load_failed =
            |e: &ApiError| e.exit_code(ApiOp::Script) == hikasa_core::ExitCode::LoadFailed;
        let prepared = match prepared {
            Ok(prepared) => prepared,
            Err(e) if load_failed(&e) => {
                self.kept.entry(index).or_default().error = Some(e.report());
                return Err(CommandOutcome::LoadFailed {
                    message: e.to_string(),
                });
            }
            Err(e) => return Err(self.fail_with(index, &e)),
        };
        let response = match self.service.folder_run(&prepared, &job, &NullObserver) {
            Ok(response) => response,
            Err(e) if load_failed(&e) => {
                self.kept.entry(index).or_default().error = Some(e.report());
                return Err(CommandOutcome::LoadFailed {
                    message: e.to_string(),
                });
            }
            Err(e) => return Err(self.fail_with(index, &e)),
        };
        if job.cancel.is_cancelled() {
            return Err(CommandOutcome::Cancelled);
        }
        let warnings = response.warnings.clone();
        self.loaded = Some(Loaded {
            request,
            prepared,
            response,
            read_only,
        });
        Ok(warnings)
    }

    /// The model of the loaded comparison.
    fn model(&self) -> Option<Arc<FolderModel>> {
        self.loaded.as_ref().map(|l| l.prepared.model().clone())
    }

    /// The paths of the selection and of the open folders, to keep them over a new comparison.
    fn remembered(&self) -> (Vec<(String, SideSet)>, Option<BTreeSet<String>>) {
        let Some(model) = self.model() else {
            return (Vec::new(), None);
        };
        let read = model.read();
        let selected = self
            .selection
            .iter()
            .map(|(node, sides)| (read.path(node).as_str().to_owned(), sides))
            .collect();
        let open = match &self.expanded {
            Expanded::All => None,
            Expanded::Paths(paths) => Some(paths.clone()),
        };
        (selected, open)
    }

    /// Restores what [`Host::remembered`] kept, where the paths still exist.
    fn restore(&mut self, selected: Vec<(String, SideSet)>, open: Option<BTreeSet<String>>) {
        let Some(model) = self.model() else { return };
        let read = model.read();
        let mut selection = Selection::default();
        for (path, sides) in selected {
            if let Some(node) = RelPath::parse(&path).ok().and_then(|p| read.node_at(&p)) {
                selection.insert(node, sides);
            }
        }
        self.selection = selection;
        self.expanded = match open {
            None => Expanded::All,
            Some(paths) => Expanded::Paths(
                paths
                    .into_iter()
                    .filter(|p| {
                        RelPath::parse(p)
                            .ok()
                            .and_then(|r| read.node_at(&r))
                            .is_some()
                    })
                    .collect(),
            ),
        };
    }

    /// Re-runs the loaded comparison with the current settings (D-219), keeping open folders and selected rows by path.
    fn rerun(
        &mut self,
        index: usize,
        sync: Option<&SyncOptions>,
    ) -> Result<Vec<Warning>, CommandOutcome> {
        let Some(loaded) = &self.loaded else {
            return Ok(Vec::new());
        };
        let request = FolderCompareRequest {
            left: loaded.request.left.clone(),
            right: loaded.request.right.clone(),
            ..self.settings.clone()
        };
        let read_only = loaded.read_only;
        let (selected, open) = self.remembered();
        let warnings = self.compare(index, request, sync, read_only)?;
        self.restore(selected, open);
        Ok(warnings)
    }

    /// The open folders a load starts with: none, unless the rules expand on load (W6).
    fn expanded_on_load(&mut self) {
        let Some(loaded) = &self.loaded else { return };
        let read = loaded.prepared.model().read();
        let ids = hikasa_folder::expanded_on_load(&read, &loaded.prepared.rules().handling);
        self.expanded = Expanded::Paths(
            ids.into_iter()
                .map(|id| read.path(id).as_str().to_owned())
                .collect(),
        );
    }

    /// After a mutation: the subtrees it changed are refreshed; a flat comparison (or a failed refresh) runs again.
    fn refresh(&mut self, index: usize, nodes: &[NodeId]) {
        let Some(model) = self.model() else { return };
        let job = self.sub_job(JobKind::FolderRefresh);
        let refreshed = self
            .service
            .folder_refresh(&model, nodes, &job, &NullObserver);
        match refreshed {
            Ok(dto) if dto.report.is_some() => {}
            _ => {
                if let Err(CommandOutcome::Failed { message }) = self.rerun(index, None) {
                    self.log(
                        LineKind::Warning,
                        self.script.line(index).unwrap_or(0),
                        message,
                    );
                }
            }
        }
    }

    // ---- the commands ---------------------------------------------------------------------------------------------

    /// `load` (§1.8.5): the comparison, the selection, the open folders and the sync preset go first, so nothing can
    /// run on an earlier comparison; any failure of a load ends the script (107), whatever its cause.
    fn load(&mut self, index: usize, line: u32, args: &LoadArgs<OsString>) -> CommandOutcome {
        self.loaded = None;
        self.selection = Selection::default();
        self.expanded = Expanded::Paths(BTreeSet::new());
        self.preset = None;
        match self.load_inner(index, line, args) {
            CommandOutcome::Failed { message } | CommandOutcome::Refused { message } => {
                CommandOutcome::LoadFailed { message }
            }
            outcome => outcome,
        }
    }

    fn load_inner(&mut self, index: usize, line: u32, args: &LoadArgs<OsString>) -> CommandOutcome {
        match args {
            LoadArgs::Default => {
                self.settings = default_settings(self.service);
                CommandOutcome::Ok
            }
            LoadArgs::Session(name) => {
                let name = match utf8(name) {
                    Ok(name) => name,
                    Err(outcome) => return outcome,
                };
                let not_found = || CommandOutcome::LoadFailed {
                    message: format!(
                        "{name}: no session of that name; loading folders needs two specs"
                    ),
                };
                let Ok(reference) = SessionRef::parse(name) else {
                    return not_found();
                };
                let run = match self.service.session_run_request(&reference) {
                    Ok(run) => run,
                    Err(ApiError::NotFound(_)) => return not_found(),
                    Err(e) => {
                        return CommandOutcome::LoadFailed {
                            message: format!("{name}: {e}"),
                        };
                    }
                };
                let (request, preset) = match run {
                    SessionRunRequest::Folder(request) => (request, None),
                    SessionRunRequest::Sync { request, options } => (request, Some(options)),
                    SessionRunRequest::Text(_) => {
                        return CommandOutcome::LoadFailed {
                            message: format!(
                                "{name}: scripts load Folder Compare and Folder Sync sessions"
                            ),
                        };
                    }
                };
                let read_only = match self.service.session_read_only(&reference) {
                    Ok(sides) => sides,
                    Err(e) => return self.fail_with(index, &e),
                };
                // SES-010: the session's settings, never its `[display]` (a script's display is `all`).
                self.settings = FolderCompareRequest {
                    left: OsString::new(),
                    right: OsString::new(),
                    ..request.clone()
                };
                self.preset = preset.clone();
                let sync = preset.or_else(|| self.sync_ahead(index));
                self.finish_load(index, line, request, sync.as_ref(), read_only)
            }
            LoadArgs::Folders {
                create,
                left,
                right,
            } => {
                for base in [left, right] {
                    if let Some(refusal) = self.base_refusal(base) {
                        return refusal;
                    }
                }
                if let Some(word) = create {
                    let sides = match utf8(word).map(script_sides) {
                        Ok(Ok(sides)) => sides,
                        Ok(Err(e)) => return self.fail_with(index, &e),
                        Err(outcome) => return outcome,
                    };
                    if let Err(outcome) = self.create_bases(index, line, sides, left, right) {
                        return outcome;
                    }
                }
                let request = FolderCompareRequest {
                    left: left.clone(),
                    right: right.clone(),
                    ..self.settings.clone()
                };
                let sync = self.sync_ahead(index);
                self.finish_load(
                    index,
                    line,
                    request,
                    sync.as_ref(),
                    ReadOnlySides::default(),
                )
            }
        }
    }

    /// Compares a load, logs and keeps its result.
    fn finish_load(
        &mut self,
        index: usize,
        line: u32,
        request: FolderCompareRequest,
        sync: Option<&SyncOptions>,
        read_only: ReadOnlySides,
    ) -> CommandOutcome {
        self.selection = Selection::default();
        let warnings = match self.compare(index, request, sync, read_only) {
            Ok(warnings) => warnings,
            Err(outcome) => return outcome,
        };
        self.expanded_on_load();
        self.warn(line, &warnings);
        if let Some(loaded) = &self.loaded {
            let stats = &loaded.response;
            let text = format!(
                "compared {} and {}: {}",
                spec_shown(loaded.prepared.specs().left),
                spec_shown(loaded.prepared.specs().right),
                stats.verdict.as_str().replace('_', " ")
            );
            // The comparison's document only with `--json`: building it costs a pass over every row.
            let display = self
                .req
                .json
                .then(|| self.display(loaded.response.ignore_unimportant).ok())
                .flatten();
            let json = display.map(|d| {
                let mut json = serde_json::to_value(loaded.response.json(&d)).unwrap_or_default();
                if let Some(object) = json.as_object_mut() {
                    object.remove("items");
                }
                json
            });
            self.log(LineKind::Info, line, text);
            if let Some(json) = json {
                self.keep(index, FOLDER_COMPARE_SCHEMA, &json, &warnings);
            }
        }
        CommandOutcome::Ok
    }

    /// A base this version cannot load (D-221): a remote scheme, a `.hksnap` file; `-` and `clipboard://` are refused.
    fn base_refusal(&self, base: &OsStr) -> Option<CommandOutcome> {
        let shown = hikasa_script::display_os(base);
        if let Some(message) = hikasa_script::spec_word_refusal(base, "base folders") {
            return Some(failed(message));
        }
        if shown
            .to_ascii_lowercase()
            .trim_end_matches(['/', '\\'])
            .ends_with(".hksnap")
        {
            return Some(CommandOutcome::LoadFailed {
                message: format!("{shown}: snapshots are not available in this version"),
            });
        }
        match self.service.parse_spec(base) {
            Ok(Spec::Remote(_)) => Some(CommandOutcome::LoadFailed {
                message: format!("{shown}: remote folders are not available in this version"),
            }),
            Ok(_) => None,
            Err(_) if shown.contains("://") => Some(CommandOutcome::LoadFailed {
                message: format!("{shown}: remote folders are not available in this version"),
            }),
            Err(_) => None,
        }
    }

    /// `load create:SIDES`: `mkdir -p` of each missing base on those sides through `vfs_run` (W3, T12; no question).
    fn create_bases(
        &mut self,
        index: usize,
        line: u32,
        sides: SideSet,
        left: &OsString,
        right: &OsString,
    ) -> Result<(), CommandOutcome> {
        let router = self.service.router();
        let mut missing = Vec::new();
        for (side, base) in [(Side::Left, left), (Side::Right, right)] {
            if !sides.contains(SideSet::of(side)) {
                continue;
            }
            let Ok(spec) = self.service.parse_spec(base) else {
                continue;
            };
            if hikasa_vfs::Vfs::stat(&*router, &spec, hikasa_vfs::FollowLinks::Yes).is_err() {
                missing.push((base.clone(), spec));
            }
        }
        if missing.is_empty() {
            return Ok(());
        }
        if self.req.dry_run {
            let (_, spec) = &missing[0];
            return Err(CommandOutcome::LoadFailed {
                message: format!(
                    "{} does not exist yet; a dry run cannot compare it",
                    spec_shown(spec)
                ),
            });
        }
        let request = VfsOpRequest::Mkdir {
            specs: missing.iter().map(|(raw, _)| raw.clone()).collect(),
            parents: true,
        };
        let job = self.sub_job(JobKind::VfsWrite);
        match self.service.vfs_run(&request, &job) {
            Ok(dto) if dto.ok() => {
                for (_, spec) in &missing {
                    self.log(
                        LineKind::Info,
                        line,
                        format!("created {}", spec_shown(spec)),
                    );
                }
                Ok(())
            }
            Ok(dto) => Err(CommandOutcome::LoadFailed {
                message: dto
                    .items
                    .iter()
                    .find_map(|i| i.error.as_ref().map(|e| e.message.clone()))
                    .unwrap_or_else(|| "a base could not be created".to_owned()),
            }),
            Err(e) => {
                self.kept.entry(index).or_default().error = Some(e.report());
                Err(CommandOutcome::LoadFailed {
                    message: e.to_string(),
                })
            }
        }
    }

    fn criteria(&mut self, index: usize, line: u32, words: &[OsString]) -> CommandOutcome {
        let mut list = Vec::new();
        for word in words {
            match utf8(word) {
                Ok(text) => list.push(text.to_owned()),
                Err(outcome) => return outcome,
            }
        }
        let joined = list.join(" ");
        if names_version(&joined) {
            return CommandOutcome::Refused {
                message: "criterion version (FC-028) is not available in this version".to_owned(),
            };
        }
        self.settings.criteria = Some(joined);
        self.after_settings(index, line)
    }

    /// `criteria` and `filter` after a load re-run it (D-219).
    fn after_settings(&mut self, index: usize, line: u32) -> CommandOutcome {
        if self.loaded.is_none() {
            return CommandOutcome::Ok;
        }
        match self.rerun(index, None) {
            Ok(warnings) => {
                self.warn(line, &warnings);
                CommandOutcome::Ok
            }
            Err(outcome) => outcome,
        }
    }

    fn filter(&mut self, index: usize, line: u32, args: &FilterArgs<OsString>) -> CommandOutcome {
        let text = |word: &OsString| utf8(word).map(str::to_owned);
        let folder = |e: hikasa_folder::FolderError| ApiError::InvalidArgument(e.to_string());
        let service = self.service;
        let patch = &mut self.settings.patch;
        let result: Result<(), ApiError> = (|| {
            match args {
                FilterArgs::Masks(masks) => {
                    patch.name_filter = Some(text(masks).map_err(|_| utf8_error(masks))?)
                }
                FilterArgs::Cutoff(expr) => {
                    patch.date_filter = Some(match expr {
                        None => Change::Clear,
                        Some(expr) => {
                            let zone = service.clock().now().time_zone().clone();
                            let expr = text(expr).map_err(|_| utf8_error(expr))?;
                            Change::Set(DateFilter::parse(&expr, &zone).map_err(folder)?)
                        }
                    });
                }
                FilterArgs::Size(expr) => {
                    patch.size_filter = Some(match expr {
                        None => Change::Clear,
                        Some(expr) => Change::Set(
                            hikasa_folder::SizeFilter::parse(
                                &text(expr).map_err(|_| utf8_error(expr))?,
                            )
                            .map_err(folder)?,
                        ),
                    });
                }
                FilterArgs::Attrib(expr) => {
                    patch.attrib_filter = Some(match expr {
                        None => Change::Clear,
                        Some(expr) => Change::Set(
                            hikasa_folder::AttribFilter::parse(
                                &text(expr).map_err(|_| utf8_error(expr))?,
                            )
                            .map_err(folder)?,
                        ),
                    });
                }
                FilterArgs::UnixType(expr) => {
                    patch.unix_type_filter = Some(match expr {
                        None => Change::Clear,
                        Some(expr) => Change::Set(
                            hikasa_folder::UnixTypeFilter::parse(
                                &text(expr).map_err(|_| utf8_error(expr))?,
                            )
                            .map_err(folder)?,
                        ),
                    });
                }
                FilterArgs::ExcludeProtected => patch.exclude_protected = Some(true),
                FilterArgs::IncludeProtected => patch.exclude_protected = Some(false),
            }
            Ok(())
        })();
        if let Err(e) = result {
            return self.fail_with(index, &e);
        }
        self.after_settings(index, line)
    }

    fn expand(&mut self, index: usize, args: &ExpandArgs<OsString>, open: bool) -> CommandOutcome {
        let Some(model) = self.model() else {
            return failed("no comparison is loaded");
        };
        let paths = match args {
            ExpandArgs::All => {
                self.expanded = if open {
                    Expanded::All
                } else {
                    Expanded::Paths(BTreeSet::new())
                };
                return CommandOutcome::Ok;
            }
            ExpandArgs::Paths(paths) => paths,
        };
        let windows = self.service.env().platform == crate::host::Platform::Windows;
        let read = model.read();
        if !open && self.expanded == Expanded::All {
            let all = read
                .preorder()
                .filter(|&(id, _)| id != NodeId::ROOT && read.children(id).len() > 0)
                .map(|(id, _)| read.path(id).as_str().to_owned())
                .collect();
            self.expanded = Expanded::Paths(all);
        }
        let mut unknown = Vec::new();
        for raw in paths {
            let Ok(text) = utf8(raw) else {
                unknown.push(hikasa_script::display_os(raw));
                continue;
            };
            let text = if windows {
                text.replace('\\', "/")
            } else {
                text.to_owned()
            };
            let text = text.trim_matches('/').to_owned();
            let found = RelPath::parse(&text).ok().and_then(|p| read.node_at(&p));
            let Some(node) = found else {
                unknown.push(text);
                continue;
            };
            let Expanded::Paths(set) = &mut self.expanded else {
                continue;
            };
            if open {
                for ancestor in read.ancestors(node) {
                    if ancestor != NodeId::ROOT {
                        set.insert(read.path(ancestor).as_str().to_owned());
                    }
                }
                set.insert(read.path(node).as_str().to_owned());
            } else {
                set.remove(read.path(node).as_str());
            }
        }
        drop(read);
        if unknown.is_empty() {
            CommandOutcome::Ok
        } else {
            let error =
                ApiError::NotFound(format!("no row {} in this comparison", unknown.join(", ")));
            self.fail_with(index, &error)
        }
    }

    fn select(&mut self, index: usize, line: u32, masks: &[OsString]) -> CommandOutcome {
        let Some(model) = self.model() else {
            return failed("no comparison is loaded");
        };
        let mut words = Vec::new();
        for mask in masks {
            match utf8(mask) {
                Ok(text) => words.push(text.to_owned()),
                Err(outcome) => return outcome,
            }
        }
        let iu = self
            .loaded
            .as_ref()
            .is_some_and(|l| l.request.ignore_unimportant);
        let request = match self.display(iu) {
            Ok(display) => SelectRequest {
                masks: words,
                paths: Vec::new(),
                selection: None,
                include_hidden: None,
                display: Some(display),
                expand: self.expand_words(),
                iu,
            },
            Err(e) => return self.fail_with(index, &e),
        };
        match self.service.folder_selection(&model, &request) {
            Ok(selection) => {
                let count = selection.len();
                self.selection = selection;
                self.log(LineKind::Info, line, format!("{count} items selected"));
                if count == 0 {
                    let warning = Warning {
                        code: WarningCode::NothingSelected,
                        message: "the masks selected nothing".to_owned(),
                        spec: None,
                    };
                    self.kept
                        .entry(index)
                        .or_default()
                        .warnings
                        .push(warning.clone());
                    self.warnings.push(warning);
                }
                CommandOutcome::Ok
            }
            Err(e) => self.fail_with(index, &e),
        }
    }

    fn compare_contents(
        &mut self,
        index: usize,
        line: u32,
        method: Option<&OsString>,
    ) -> CommandOutcome {
        let Some(model) = self.model() else {
            return failed("no comparison is loaded");
        };
        let word = match method.map(|m| utf8(m)) {
            Some(Ok(word)) => word.to_owned(),
            Some(Err(outcome)) => return outcome,
            None => "rules-based".to_owned(),
        };
        let method = match super::parse_content_method(&word) {
            Ok(method) => method,
            Err(e) => return self.fail_with(index, &e),
        };
        let iu = self
            .loaded
            .as_ref()
            .is_some_and(|l| l.request.ignore_unimportant);
        let select = match self.op_select(iu) {
            Ok(select) => select,
            Err(e) => return self.fail_with(index, &e),
        };
        let request = ContentsRequest {
            select,
            kind: ContentsKind::Compare(method),
            text_rules: self
                .loaded
                .as_ref()
                .and_then(|l| l.prepared.text_rules().cloned()),
        };
        let job = self.sub_job(JobKind::FolderOp);
        match self.service.folder_contents(&model, &request, &job) {
            Ok(dto) => {
                let ok = dto.ok();
                self.log(
                    LineKind::Info,
                    line,
                    format!("compared the contents of {} items", dto.items.len()),
                );
                self.keep(index, FOLDER_CONTENTS_SCHEMA, &dto, &[]);
                if job.cancel.is_cancelled() {
                    return CommandOutcome::Cancelled;
                }
                if ok {
                    CommandOutcome::Ok
                } else {
                    failed("the contents of some items could not be compared")
                }
            }
            Err(e) => self.fail_with(index, &e),
        }
    }

    /// The answers of a plan in the script's mode (§1.8.8): no question in `yes-to-all`/`no-to-all`; `prompt` asks the
    /// adapter, declines in silent mode or without a terminal.
    fn answered(
        &mut self,
        plan: OpPlan,
        mode: ConfirmWord,
        silent: bool,
    ) -> Result<OpPlan, String> {
        let mode = match mode {
            ConfirmWord::Prompt => ConfirmMode::Prompt,
            ConfirmWord::YesToAll => ConfirmMode::YesToAll,
            ConfirmWord::NoToAll => ConfirmMode::NoToAll,
        };
        let mut machine = self.service.folder_confirmation_strict(&plan, Some(mode));
        if let Some(text) = machine.next().map(|q| q.text.clone()) {
            if silent {
                return Err(format!(
                    "Attempted to open a dialog when running silently: {text}"
                ));
            }
            if !self.log.ui.can_ask() {
                let total: u64 = machine.open_questions().values().sum();
                return Err(format!(
                    "{total} questions need an answer and there is no terminal: use option confirm:yes-to-all or \
                     confirm:no-to-all, or --confirm"
                ));
            }
            self.log.ui.event(ScriptEvent::Question { text });
            let cancel: CancelToken = self.job.cancel.clone();
            self.log.ui.answer(&mut machine, &cancel);
        }
        machine
            .finish(plan)
            .map_err(|_| "declined: nothing was written".to_owned())
    }

    /// Logs an operation's outcome: its summary, its failed items, a detail line per item when verbose, its warnings.
    fn log_outcome(&mut self, line: u32, outcome: &FolderOpOutcome) {
        let text = summary_text(outcome.verb, outcome.dry_run, &outcome.report.summary, None);
        self.log(LineKind::Info, line, text);
        let verbose = self.log.level >= LogLevel::Verbose;
        let mut details = 0usize;
        for index in 0..outcome.item_count() {
            let Some(item) = outcome.item_dto(index) else {
                continue;
            };
            match item.status {
                hikasa_vfs::ops::ItemStatus::Failed => {
                    let message = item
                        .error
                        .as_ref()
                        .map_or("", |e| e.message.as_str())
                        .to_owned();
                    let text = format!(
                        "failed {} {}: {message}",
                        super::folder_ops::item_word(&item),
                        item.target
                    );
                    self.log(LineKind::Error, line, text);
                }
                _ if outcome.dry_run => {
                    self.log(LineKind::Info, line, item_line(&item, true));
                }
                _ if verbose => {
                    details += 1;
                    if details <= crate::log::LOG_DETAILS_MAX {
                        self.log(LineKind::Detail, line, item_line(&item, false));
                    }
                }
                _ => {}
            }
        }
        if details > crate::log::LOG_DETAILS_MAX {
            let more = details - crate::log::LOG_DETAILS_MAX;
            self.log(LineKind::Detail, line, format!("\u{2026} and {more} more"));
        }
    }

    /// Plans an operation, answers it, executes it (or shows it in a dry run), refreshes and logs.
    fn operation(
        &mut self,
        index: usize,
        line: u32,
        op: OpRequest,
        state: &RunState,
    ) -> CommandOutcome {
        let Some(model) = self.model() else {
            return failed("no comparison is loaded");
        };
        let (iu, read_only) = match &self.loaded {
            Some(l) => (l.request.ignore_unimportant, l.read_only),
            None => (false, ReadOnlySides::default()),
        };
        let select = match self.op_select(iu) {
            Ok(select) => select,
            Err(e) => return self.fail_with(index, &e),
        };
        let request = FolderOpRequest {
            op,
            select,
            read_only,
            overrides: OpOverrides::default(),
        };
        let job = self.sub_job(JobKind::FolderOp);
        let plan = match self.service.folder_op_plan(&model, &request, &job) {
            Ok(plan) => plan,
            Err(e) => return self.fail_with(index, &e),
        };
        let outcome = if state.dry_run {
            self.service.folder_op_dry_run(plan)
        } else {
            let plan = match self.answered(plan, state.confirm, state.silent) {
                Ok(plan) => plan,
                Err(message) => {
                    if self.job.cancel.is_cancelled() {
                        return CommandOutcome::Cancelled;
                    }
                    return failed(message);
                }
            };
            match self.service.folder_op_execute(plan, &job) {
                Ok(outcome) => outcome,
                Err(e) => return self.fail_with(index, &e),
            }
        };
        self.log_outcome(line, &outcome);
        self.warn(line, &outcome.warnings);
        if self.req.json {
            let dto = outcome.dto();
            self.keep(index, FOLDER_OP_SCHEMA, &dto, &outcome.warnings);
        } else {
            self.keep(index, FOLDER_OP_SCHEMA, &(), &outcome.warnings);
        }
        // A `copyto` changes nothing of the bases: no refresh (its target lies outside the comparison).
        if !outcome.dry_run && outcome.verb != hikasa_folder::ops::OpVerb::CopyTo {
            self.refresh(index, &outcome.refresh_nodes);
        }
        if outcome.report.cancelled || self.job.cancel.is_cancelled() {
            return CommandOutcome::Cancelled;
        }
        if outcome.ok() {
            CommandOutcome::Ok
        } else {
            failed(summary_text(
                outcome.verb,
                outcome.dry_run,
                &outcome.report.summary,
                None,
            ))
        }
    }

    fn sync(
        &mut self,
        index: usize,
        line: u32,
        args: &SyncArgs<OsString>,
        state: &RunState,
    ) -> CommandOutcome {
        let args = match args
            .mode
            .as_ref()
            .map(|(m, w)| utf8(w).map(|w| (*m, w.to_owned())))
        {
            Some(Err(outcome)) => return outcome,
            Some(Ok(mode)) => SyncArgs {
                visible: args.visible,
                create_empty: args.create_empty,
                mode: Some(mode),
            },
            None => SyncArgs {
                visible: args.visible,
                create_empty: args.create_empty,
                mode: None,
            },
        };
        let options = match self.sync_options_of(&args) {
            Ok(Some(options)) => options,
            Ok(None) => return failed("no sync direction and the session holds none"),
            Err(e) => return self.fail_with(index, &e),
        };
        // The comparison of a sync has the sync's rules (§1.6.14): one prepared otherwise is compared again.
        let rules_differ = self.loaded.as_ref().is_some_and(|l| {
            sync_folder_rules(l.prepared.rules(), &options)
                .is_ok_and(|rules| &rules != l.prepared.rules())
        });
        if rules_differ && let Err(outcome) = self.rerun(index, Some(&options)) {
            return outcome;
        }
        let Some(model) = self.model() else {
            return failed("no comparison is loaded");
        };
        let (iu, read_only) = match &self.loaded {
            Some(l) => (l.request.ignore_unimportant, l.read_only),
            None => (false, ReadOnlySides::default()),
        };
        let visible = if args.visible {
            match self.display(iu) {
                Ok(display) => Some(display),
                Err(e) => return self.fail_with(index, &e),
            }
        } else {
            None
        };
        let request = FolderSyncRequest {
            options,
            select: None,
            visible,
            include_hidden: None,
            iu,
            read_only,
            allow_empty_source: false,
            overrides: OpOverrides::default(),
        };
        let job = self.sub_job(JobKind::FolderSync);
        let mut plan = match self.service.folder_sync_plan(&model, &request, &job) {
            Ok(plan) => plan,
            Err(e) => return self.fail_with(index, &e),
        };
        let outcome = if state.dry_run {
            self.service.folder_sync_dry_run(plan)
        } else {
            let stand_in = OpPlan {
                verb: plan.op.verb,
                bases: plan.op.bases.clone(),
                plan: hikasa_vfs::ops::Plan {
                    op: plan.op.plan.op,
                    options: plan.op.plan.options,
                    items: Vec::new(),
                    warnings: Vec::new(),
                },
                meta: Vec::new(),
                exec: hikasa_vfs::ops::ExecOptions::default(),
                warnings: Vec::new(),
                refresh: Vec::new(),
            };
            let op = std::mem::replace(&mut plan.op, stand_in);
            plan.op = match self.answered(op, state.confirm, state.silent) {
                Ok(op) => op,
                Err(message) => {
                    if self.job.cancel.is_cancelled() {
                        return CommandOutcome::Cancelled;
                    }
                    return failed(message);
                }
            };
            match self.service.folder_sync_execute(plan, &job) {
                Ok(outcome) => outcome,
                Err(e) => return self.fail_with(index, &e),
            }
        };
        self.log_outcome(line, &outcome.op);
        self.log(
            LineKind::Info,
            line,
            super::folder_ops::sync_counts_text(&outcome.counts),
        );
        self.warn(line, &outcome.op.warnings);
        if self.req.json {
            let dto = outcome.dto(&model.read(), false);
            self.keep(index, FOLDER_SYNC_SCHEMA, &dto, &outcome.op.warnings);
        } else {
            self.keep(index, FOLDER_SYNC_SCHEMA, &(), &outcome.op.warnings);
        }
        if !outcome.op.dry_run {
            self.refresh(index, &outcome.op.refresh_nodes);
        }
        if outcome.op.report.cancelled || self.job.cancel.is_cancelled() {
            return CommandOutcome::Cancelled;
        }
        if outcome.ok() {
            CommandOutcome::Ok
        } else {
            failed(summary_text(
                outcome.op.verb,
                outcome.op.dry_run,
                &outcome.op.report.summary,
                None,
            ))
        }
    }

    /// The flags of a report command's words.
    fn report_flags(args: &ReportArgs<OsString>) -> Result<ReportFlags, CommandOutcome> {
        let list = |words: &[OsString]| -> Result<Vec<String>, CommandOutcome> {
            words.iter().map(|w| utf8(w).map(str::to_owned)).collect()
        };
        let one = |word: &Option<OsString>| -> Result<Option<String>, CommandOutcome> {
            word.as_ref()
                .map(|w| utf8(w).map(str::to_owned))
                .transpose()
        };
        Ok(ReportFlags {
            words: ReportWords {
                layout: one(&args.layout)?,
                options: list(&args.options)?,
                output_options: list(&args.output_options)?,
                title: one(&args.title)?,
            },
            ..ReportFlags::default()
        })
    }

    fn report(
        &mut self,
        index: usize,
        line: u32,
        name: CommandName,
        args: &ReportArgs<OsString>,
        state: &RunState,
    ) -> CommandOutcome {
        let kind = if name == CommandName::FolderReport {
            ReportKind::Folder
        } else {
            ReportKind::Text
        };
        let flags = match Self::report_flags(args) {
            Ok(flags) => flags,
            Err(outcome) => return outcome,
        };
        let Some(raw) = &args.output_to else {
            return failed(format!("{} needs output-to:", name.as_str()));
        };
        let target = ReportTarget::parse(raw);
        if target == ReportTarget::Stdout && self.stdout.is_none() {
            return failed(STDOUT_NONE);
        }
        let mut req = match self
            .service
            .report_request(kind, "side-by-side", &flags, &target)
        {
            Ok(req) => req,
            Err(e) => return self.fail_with(index, &e),
        };
        let job = self.sub_job(JobKind::Report);
        let outcome = match kind {
            ReportKind::Text => {
                let request = match self.text_request(index, args) {
                    Ok(request) => request,
                    Err(outcome) => return outcome,
                };
                let mut request = request;
                request.ignore_unimportant |= req.ignore_unimportant();
                let compare_job = self.sub_job(JobKind::TextCompare);
                let resp = match self.service.text_compare(&request, &compare_job, None) {
                    Ok(resp) => resp,
                    Err(e) => return self.fail_with(index, &e),
                };
                if compare_job.cancel.is_cancelled() {
                    return CommandOutcome::Cancelled;
                }
                let source = ReportSource::Text(&resp);
                self.run_report(source, &mut req, &target, &job, state)
            }
            ReportKind::Folder => {
                if args.just_selection {
                    req.scope = ReportScope::FolderRows(self.selection.clone());
                }
                let Some(loaded) = self.loaded.take() else {
                    return failed("no comparison is loaded");
                };
                let source = ReportSource::Folder {
                    request: &loaded.request,
                    prepared: &loaded.prepared,
                    response: &loaded.response,
                };
                let outcome = self.run_report(source, &mut req, &target, &job, state);
                self.loaded = Some(loaded);
                outcome
            }
        };
        match outcome {
            Ok(outcome) => {
                let lines = if outcome.dry_run {
                    outcome.dry_run_lines()
                } else {
                    vec![format!(
                        "wrote the report to {}",
                        hikasa_script::display_os(raw)
                    )]
                };
                for text in lines {
                    self.log(LineKind::Info, line, text);
                }
                self.warn(line, &outcome.warnings);
                self.keep(index, REPORT_SCHEMA, &outcome.dto(), &outcome.warnings);
                if let Some(payload) = &outcome.clipboard
                    && let Err(e) = self.log.ui.clipboard(payload)
                {
                    return self.fail_with(index, &e);
                }
                CommandOutcome::Ok
            }
            Err(e) => self.fail_with(index, &e),
        }
    }

    /// Plans and runs (or dry-runs) a report.
    fn run_report(
        &mut self,
        source: ReportSource<'_>,
        req: &mut hikasa_report::ReportRequest,
        target: &ReportTarget,
        job: &JobControl,
        state: &RunState,
    ) -> Result<super::report_write::ReportOutcome, ApiError> {
        let plan = self.service.report_plan(source, req, target, None, job)?;
        if state.dry_run {
            return Ok(self.service.report_dry_run(plan));
        }
        let stdout: Option<&mut dyn Write> = match (target, &mut self.stdout) {
            (ReportTarget::Stdout, Some(out)) => Some(&mut **out),
            _ => None,
        };
        self.service.report_execute(source, req, plan, stdout, job)
    }

    /// The comparison of `text-report`: two specs, or one Text Compare session.
    fn text_request(
        &mut self,
        index: usize,
        args: &ReportArgs<OsString>,
    ) -> Result<TextCompareRequest, CommandOutcome> {
        match args.specs.as_slice() {
            [left, right] => Ok(TextCompareRequest {
                left: left.clone(),
                right: right.clone(),
                ..TextCompareRequest::default()
            }),
            [session] => {
                let name = utf8(session)?;
                let reference = SessionRef::parse(name).map_err(|e| self.fail_with(index, &e))?;
                match self.service.session_run_request(&reference) {
                    Ok(SessionRunRequest::Text(request)) => Ok(request),
                    Ok(_) => Err(failed(format!(
                        "session `{name}` is not a Text Compare session"
                    ))),
                    Err(e) => Err(self.fail_with(index, &e)),
                }
            }
            _ => Err(failed(
                "text-report needs a text-compare session or two files",
            )),
        }
    }

    fn log_command(
        &mut self,
        line: u32,
        args: &LogArgs<OsString>,
        state: &RunState,
    ) -> CommandOutcome {
        if let Some(level) = args
            .level
            .as_ref()
            .and_then(|w| w.to_str())
            .and_then(LogLevel::parse)
        {
            self.log.level = level;
        }
        let Some(file) = &args.file else {
            return CommandOutcome::Ok;
        };
        if state.dry_run {
            let warning = Warning {
                code: WarningCode::LogNotWritten,
                message: "log file not written in a dry run".to_owned(),
                spec: None,
            };
            self.warn(line, std::slice::from_ref(&warning));
            return CommandOutcome::Ok;
        }
        let spec = match self.service.parse_spec(file) {
            Ok(spec) => spec,
            Err(e) => return failed(e.to_string()),
        };
        let Spec::Local(local) = &spec else {
            return CommandOutcome::Failed {
                message: "a script log must be a local file".to_owned(),
            };
        };
        // A second `log <file>` closes the first.
        self.log.file = None;
        match LogFile::open(local.as_path(), args.append) {
            Ok(mut file) => {
                // The file starts with the `log` line itself, when the level shows it.
                if let Some((at, text)) = self.log.last_run.clone()
                    && self.log.level.shows(LineKind::Run)
                {
                    let now = local_time(self.service);
                    if let Err(e) = file.write_line(&stamped(now, LineKind::Run, at, &text)) {
                        let warning = Warning {
                            code: WarningCode::LogNotWritten,
                            message: format!("the script's log file could not be written: {e}"),
                            spec: Some(spec.canonical()),
                        };
                        self.warn(line, std::slice::from_ref(&warning));
                        return CommandOutcome::Ok;
                    }
                }
                self.log.file = Some(Box::new(file));
                CommandOutcome::Ok
            }
            Err(e) => failed(format!("{}: {e}", spec_shown(&spec))),
        }
    }

    /// Replaces the log file by a test's sink (A8.9).
    #[allow(dead_code)]
    pub(crate) fn set_sink(&mut self, sink: Box<dyn LineSink + 'a>) {
        self.log.file = Some(sink);
    }
}

fn utf8_error(value: &OsString) -> ApiError {
    ApiError::InvalidArgument(format!(
        "`{}` is not valid UTF-8",
        hikasa_script::display_os(value)
    ))
}

impl ScriptHost for Host<'_> {
    fn check_resolved(&self, slot: Slot, word: &OsStr) -> Result<(), String> {
        match word.to_str() {
            Some(text) => self.checker.check(slot, text),
            None => Ok(()),
        }
    }

    fn execute(&mut self, cmd: &Resolved<'_>, state: &RunState) -> CommandOutcome {
        let index = cmd.index;
        let line = cmd.command.pos.line;
        let name = cmd.command.name;
        let run = std::panic::AssertUnwindSafe(|| -> CommandOutcome {
            match &cmd.args {
                Args::Load(args) => self.load(index, line, args),
                Args::Criteria { words } => self.criteria(index, line, words),
                Args::Filter(args) => self.filter(index, line, args),
                Args::Expand(args) => self.expand(index, args, true),
                Args::Collapse(args) => self.expand(index, args, false),
                Args::Select { masks } => self.select(index, line, masks),
                Args::Compare { method } => self.compare_contents(index, line, method.as_ref()),
                Args::Copy { direction } | Args::Move { direction } => {
                    let to = written_by(&direction.to_string_lossy());
                    let op = if name == CommandName::Copy {
                        FolderOp::Copy {
                            to,
                            overwrite: OverwritePolicy::Always,
                        }
                    } else {
                        FolderOp::Move {
                            to,
                            overwrite: OverwritePolicy::Always,
                        }
                    };
                    self.operation(index, line, OpRequest::Op(op), state)
                }
                Args::CopyTo(args) | Args::MoveTo(args) => {
                    self.copy_to(index, line, name, args, state)
                }
                Args::Delete { recycle, sides } => {
                    let sides = match utf8(sides).map(script_sides) {
                        Ok(Ok(sides)) => sides,
                        Ok(Err(e)) => return self.fail_with(index, &e),
                        Err(outcome) => return outcome,
                    };
                    let use_bin = recycle
                        .as_ref()
                        .is_some_and(|w| w.to_str().is_some_and(|w| w.eq_ignore_ascii_case("yes")));
                    let op = FolderOp::Delete {
                        sides,
                        recycle: Recycle {
                            use_bin,
                            fallback: false,
                        },
                    };
                    self.operation(index, line, OpRequest::Op(op), state)
                }
                Args::Rename(args) => {
                    let mask = match args {
                        RenameArgs::Mask(mask) => match utf8(mask) {
                            Ok(mask) => RenameMask::Dos(mask.to_owned()),
                            Err(outcome) => return outcome,
                        },
                        RenameArgs::Regex { old, new } => match (utf8(old), utf8(new)) {
                            (Ok(old), Ok(new)) => RenameMask::Regex {
                                old: old.to_owned(),
                                new: new.to_owned(),
                            },
                            (Err(outcome), _) | (_, Err(outcome)) => return outcome,
                        },
                    };
                    let op = FolderOp::Rename {
                        sides: SideSet::all(),
                        mask,
                    };
                    self.operation(index, line, OpRequest::Op(op), state)
                }
                Args::Touch(args) => {
                    let op = match args {
                        TouchArgs::Direction(direction) => {
                            let text = direction.to_string_lossy();
                            if !text.contains("->") {
                                return failed(format!(
                                    "touch needs left->right, right->left, or <sides>:now or <sides>:<time>, not `{text}`"
                                ));
                            }
                            let to = written_by(&text);
                            OpRequest::Op(FolderOp::Touch {
                                sides: SideSet::of(to),
                                from: TouchFrom::OtherSide { to },
                            })
                        }
                        TouchArgs::At { sides, time } => {
                            let sides = match utf8(sides).map(script_sides) {
                                Ok(Ok(sides)) => sides,
                                Ok(Err(e)) => return self.fail_with(index, &e),
                                Err(outcome) => return outcome,
                            };
                            let time = match utf8(time) {
                                Ok(time) => time,
                                Err(outcome) => return outcome,
                            };
                            if time.eq_ignore_ascii_case("now") {
                                OpRequest::TouchNow { sides }
                            } else {
                                match super::parse_time(time) {
                                    Ok(at) => OpRequest::Op(FolderOp::Touch {
                                        sides,
                                        from: TouchFrom::At(at),
                                    }),
                                    Err(e) => return self.fail_with(index, &e),
                                }
                            }
                        }
                    };
                    self.operation(index, line, op, state)
                }
                Args::Attrib { changes } => {
                    let mut words = Vec::new();
                    for change in changes {
                        match utf8(change) {
                            Ok(word) => words.push(word.to_owned()),
                            Err(outcome) => return outcome,
                        }
                    }
                    let change = match super::parse_attrib_change(&words) {
                        Ok(change) => change,
                        Err(e) => return self.fail_with(index, &e),
                    };
                    let op = FolderOp::Attrib {
                        sides: SideSet::all(),
                        change,
                    };
                    self.operation(index, line, OpRequest::Op(op), state)
                }
                Args::Sync(args) => self.sync(index, line, args, state),
                Args::Report(args) => self.report(index, line, name, args, state),
                Args::Log(args) => self.log_command(line, args, state),
                // The interpreter's own (D-221 refusals, `option`, `beep`): nothing for the host.
                Args::Snapshot { .. } | Args::Option(_) | Args::Beep => CommandOutcome::Ok,
            }
        });
        match std::panic::catch_unwind(run) {
            Ok(outcome) => outcome,
            Err(_) => failed("internal error: the command panicked (a bug)"),
        }
    }

    fn now(&self) -> LocalTime {
        local_time(self.service)
    }

    fn run_time(&self) -> Option<Duration> {
        let now = self.service.clock().now().timestamp();
        let span = now.duration_since(self.started);
        Duration::try_from(span).ok()
    }

    fn ui(&mut self) -> &mut dyn HostUi {
        &mut self.log
    }
}

impl Host<'_> {
    fn copy_to(
        &mut self,
        index: usize,
        line: u32,
        name: CommandName,
        args: &CopyToArgs<OsString>,
        state: &RunState,
    ) -> CommandOutcome {
        let sides = match args.sides.as_ref().map(|w| utf8(w).map(script_sides)) {
            None => SideSet::all(),
            Some(Ok(Ok(sides))) => sides,
            Some(Ok(Err(e))) => return self.fail_with(index, &e),
            Some(Err(outcome)) => return outcome,
        };
        let structure = match args
            .structure
            .as_ref()
            .map(|w| utf8(w).map(super::parse_structure))
        {
            None => hikasa_folder::ops::PathStructure::default(),
            Some(Ok(Ok(structure))) => structure,
            Some(Ok(Err(e))) => return self.fail_with(index, &e),
            Some(Err(outcome)) => return outcome,
        };
        let target = args.target.clone();
        if let Some(message) =
            hikasa_script::spec_word_refusal(&target, &format!("targets of {}", name.as_str()))
        {
            return self.fail_with(index, &ApiError::InvalidArgument(message));
        }
        let op = if name == CommandName::CopyTo {
            OpRequest::CopyTo {
                sides,
                target,
                structure,
                overwrite: OverwritePolicy::Always,
            }
        } else {
            OpRequest::MoveTo {
                sides,
                target,
                structure,
                overwrite: OverwritePolicy::Always,
            }
        };
        self.operation(index, line, op, state)
    }
}
