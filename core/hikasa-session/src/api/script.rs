//! Scripts through the Service (§1.8.11, §1.8.12, D-225, D-226): locate and load a script, check it with the vocabularies
//! the CLI flags use (the host's [`WordChecker`], T11), run it over the host of `script_host.rs`, and the document
//! `hikasa.script.v1`. The adapters call this module only; they name no `hikasa_script` item (check 7).

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::time::Duration;

use hikasa_core::{Classify, ErrorReport, ExitCode, Spec, Warning};
use hikasa_folder::ops::{ConfirmMode, Confirmation};
use hikasa_folder::select::SelectMask;
use hikasa_folder::{AttribFilter, Criteria, DateFilter, SizeFilter, UnixTypeFilter};
use hikasa_report::ReportKind;
use hikasa_vfs::{EntryKind, FollowLinks, SpecUse, Vfs};
use serde::Serialize;

pub use hikasa_script::parse::KEYWORDS as SCRIPT_KEYWORDS;
pub use hikasa_script::{
    CommandName, CommandStatus, ConfirmWord, Errors as ScriptErrors, HostUi, LineKind, LogLevel,
    Pos, References, SCRIPT_MAX_BYTES, SCRIPT_MAX_ERRORS, Script, ScriptError, ScriptErrorCode,
    ScriptEvent, StopAt, StopReason, elapsed_text,
};
use hikasa_script::{RefCheck, ReportFor, Slot, WordChecker};

use super::report::{ReportFlags, ReportTarget};
use super::script_host::Host;
use super::{ApiError, Service};
use crate::clipboard::ClipboardPayload;
use crate::jobs::{JobControl, run_guarded};

/// The JSON item lists of one command are capped here (§1.8.12); the rest is counted in `items_omitted`.
pub const SCRIPT_JSON_ITEMS_MAX: usize = 100_000;

/// Where a script comes from (§1.8.11).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptSource {
    /// A spec as located: a local file, or a member of an archive.
    Spec(Spec),
    /// Standard input (`-`).
    Stdin,
}

/// A script read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptText {
    /// Where it came from.
    pub source: ScriptSource,
    /// The script as messages name it (`-` for standard input), escaped.
    pub display: String,
    /// `%0`: the script as located, as `Spec::parse` takes it.
    pub path: OsString,
    /// Its bytes.
    pub bytes: Vec<u8>,
}

/// What `script_check` found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptCheck {
    /// The checked script (`None` with errors).
    pub script: Option<Script>,
    /// The errors (empty when it checked).
    pub errors: ScriptErrors,
    /// What it references (also with errors, for what can be known).
    pub references: References,
}

impl ScriptCheck {
    /// The number of commands checked (0 with errors), without making them (`ok: <n> commands`).
    pub fn commands(&self) -> usize {
        self.script.as_ref().map_or(0, Script::len)
    }

    /// 0, 105 or 106.
    pub fn exit_code(&self) -> ExitCode {
        if self.errors.is_empty() {
            ExitCode::Success
        } else if self.errors.is_load() {
            ExitCode::ScriptLoadError
        } else {
            ExitCode::ScriptSyntaxError
        }
    }

    /// The error of a check that failed: `ScriptSyntax` (106) or `ScriptLoad` (105, `too_large`).
    pub fn error(&self, text: &ScriptText) -> Option<ApiError> {
        if self.errors.is_empty() {
            return None;
        }
        if self.errors.is_load() {
            let message = self
                .errors
                .first()
                .map_or_else(String::new, |e| e.message.clone());
            return Some(ApiError::ScriptLoad {
                kind: hikasa_core::ErrorKind::TooLarge,
                message: format!("{}: {message}", text.display),
            });
        }
        Some(ApiError::ScriptSyntax {
            script: text.display.clone(),
            errors: self.errors.clone(),
        })
    }
}

/// One run (§1.8.12).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScriptRunRequest {
    /// `%1`…`%9`.
    pub args: Vec<OsString>,
    /// The captured environment (`host::capture_script_env` of the references).
    pub env: BTreeMap<String, OsString>,
    /// `--silent`.
    pub silent: bool,
    /// `--stop-on-error`.
    pub stop_on_error: bool,
    /// `--confirm`.
    pub confirm: Option<ConfirmMode>,
    /// `--dry-run` (T12).
    pub dry_run: bool,
    /// `output-to:-` is allowed (the headless CLI without `--json`).
    pub stdout_reports: bool,
    /// The JSON document is wanted: each command keeps its `result` (additive: without it nothing is kept).
    pub json: bool,
    /// The cap of each command's JSON item list (0: [`SCRIPT_JSON_ITEMS_MAX`]; a test lowers it).
    pub items_max: usize,
}

/// One command of a run, with its CLI result.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandRecord {
    /// Its first line.
    pub line: u32,
    /// Its last line.
    pub end_line: u32,
    /// The command.
    pub name: &'static str,
    /// The words as lexed.
    pub words: Vec<String>,
    /// The expanded line (run only).
    pub text: Option<String>,
    /// What became of it.
    pub status: CommandStatus,
    /// The schema of `result`.
    pub schema: Option<&'static str>,
    /// The `result` of the matching CLI command's document.
    pub result: Option<serde_json::Value>,
    /// Items past the cap.
    pub items_omitted: u64,
    /// The failure.
    pub error: Option<ErrorReport>,
    /// The warnings of the command.
    pub warnings: Vec<Warning>,
}

/// What a run did (§1.8.12).
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptOutcome {
    /// 0, 100 or 107.
    pub exit_code: ExitCode,
    /// One per command.
    pub commands: Vec<CommandRecord>,
    /// Where it ended early.
    pub stopped_at: Option<StopAt>,
    /// Cancelled.
    pub cancelled: bool,
    /// How long it took.
    pub elapsed: Option<Duration>,
    /// Every warning of the run.
    pub warnings: Vec<Warning>,
}

/// The user interface of a run (§1.8.12): the events, the stop-on-error question and beeps of [`HostUi`], W6's questions
/// and the clipboard.
pub trait ScriptUi: HostUi {
    /// Answers the questions of `machine` (the CLI on a terminal, the status window's dialogs); called only when
    /// [`HostUi::can_ask`] is true and the run is not silent. A cancel ends the questions as a `Cancel`.
    fn answer(&mut self, machine: &mut Confirmation, cancel: &hikasa_core::CancelToken);
    /// Places a report's clipboard payload on this adapter's clipboard.
    fn clipboard(&mut self, payload: &ClipboardPayload) -> Result<(), ApiError>;
    /// Where the run sends `ScriptEvent::Progress`: the progress of the current call, reported on the threads of the
    /// call (so not through [`HostUi::event`]); `None` (the default, additive in W8b): no progress events.
    fn progress_events(&self) -> Option<ProgressEvents> {
        None
    }
}

/// The receiver of `ScriptEvent::Progress` of [`ScriptUi::progress_events`] (any thread).
pub type ProgressEvents = std::sync::Arc<dyn Fn(ScriptEvent) + Send + Sync>;

/// `{line, column, code, message}` of `hikasa.script.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScriptErrorDto {
    /// The line.
    pub line: u32,
    /// The column.
    pub column: u32,
    /// The code.
    pub code: &'static str,
    /// The message.
    pub message: String,
}

/// `stopped_at` of `hikasa.script.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StopDto {
    /// The line.
    pub line: u32,
    /// `load_failed`, `stop_on_error`, `cancelled`.
    pub reason: StopReason,
}

/// One command of `hikasa.script.v1`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ScriptCommandDto {
    /// Its first line.
    pub line: u32,
    /// Its last line.
    pub end_line: u32,
    /// The command.
    pub name: &'static str,
    /// The words as lexed.
    pub words: Vec<String>,
    /// The expanded line (run only).
    pub text: Option<String>,
    /// `checked`, `ok`, `failed`, `refused`, `not_run`.
    pub status: CommandStatus,
    /// The schema of `result`.
    pub schema: Option<&'static str>,
    /// The CLI command's `result`.
    pub result: Option<serde_json::Value>,
    /// Items past the cap.
    pub items_omitted: u64,
    /// The failure.
    pub error: Option<ErrorReport>,
    /// The warnings.
    pub warnings: Vec<Warning>,
}

/// `counts` of `hikasa.script.v1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct ScriptCountsDto {
    /// Commands.
    pub commands: u64,
    /// `ok`.
    pub ok: u64,
    /// `failed`.
    pub failed: u64,
    /// `refused`.
    pub refused: u64,
    /// `not_run`.
    pub not_run: u64,
}

/// `hikasa.script.v1` (`result`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ScriptDto {
    /// `run` or `check`.
    pub mode: &'static str,
    /// The script as located.
    pub script: String,
    /// The arguments, escaped.
    pub args: Vec<String>,
    /// `--dry-run`.
    pub dry_run: bool,
    /// `--silent`.
    pub silent: bool,
    /// Cancelled.
    pub cancelled: bool,
    /// Where it ended early.
    pub stopped_at: Option<StopDto>,
    /// The check's errors.
    pub errors: Vec<ScriptErrorDto>,
    /// The errors past the cap.
    pub errors_omitted: u64,
    /// The commands.
    pub commands: Vec<ScriptCommandDto>,
    /// Their counts.
    pub counts: ScriptCountsDto,
    /// Seconds (run only).
    pub elapsed_s: Option<f64>,
}

fn arg_texts(args: &[OsString]) -> Vec<String> {
    args.iter().map(|a| hikasa_script::display_os(a)).collect()
}

fn error_dtos(errors: &ScriptErrors) -> Vec<ScriptErrorDto> {
    errors
        .list
        .iter()
        .map(|e| ScriptErrorDto {
            line: e.pos.line,
            column: e.pos.column,
            code: e.code.as_str(),
            message: e.message.clone(),
        })
        .collect()
}

fn counts(statuses: impl Iterator<Item = CommandStatus>) -> ScriptCountsDto {
    let mut counts = ScriptCountsDto::default();
    for status in statuses {
        counts.commands += 1;
        match status {
            CommandStatus::Ok => counts.ok += 1,
            CommandStatus::Failed => counts.failed += 1,
            CommandStatus::Refused => counts.refused += 1,
            CommandStatus::NotRun => counts.not_run += 1,
            CommandStatus::Checked => {}
        }
    }
    counts
}

impl ScriptCheck {
    /// `hikasa.script.v1` of `script check`.
    pub fn dto(&self, text: &ScriptText, args: &[OsString]) -> ScriptDto {
        let commands: Vec<ScriptCommandDto> = self
            .script
            .iter()
            .flat_map(|script| script.commands())
            .map(|c| ScriptCommandDto {
                line: c.pos.line,
                end_line: c.end_line,
                name: c.name.as_str(),
                words: c.word_texts(),
                text: None,
                status: CommandStatus::Checked,
                schema: None,
                result: None,
                items_omitted: 0,
                error: None,
                warnings: Vec::new(),
            })
            .collect();
        ScriptDto {
            mode: "check",
            script: text.display.clone(),
            args: arg_texts(args),
            dry_run: false,
            silent: false,
            cancelled: false,
            stopped_at: None,
            errors: error_dtos(&self.errors),
            errors_omitted: self.errors.more,
            counts: ScriptCountsDto {
                commands: commands.len() as u64,
                ..ScriptCountsDto::default()
            },
            commands,
            elapsed_s: None,
        }
    }
}

impl ScriptOutcome {
    /// The counts of `ScriptFinished` and `hikasa.gui_script.v1` (M2 W8b, §1.8.13).
    pub fn summary(&self) -> crate::ipc::ScriptSummary {
        let count = |status: CommandStatus| {
            u32::try_from(self.commands.iter().filter(|c| c.status == status).count())
                .unwrap_or(u32::MAX)
        };
        crate::ipc::ScriptSummary {
            commands: u32::try_from(self.commands.len()).unwrap_or(u32::MAX),
            ok: count(CommandStatus::Ok),
            failed: count(CommandStatus::Failed),
            refused: count(CommandStatus::Refused),
            not_run: count(CommandStatus::NotRun),
            stopped_at: self.stopped_at.map(|stop| stop.line),
        }
    }

    /// `hikasa.script.v1` of `script run`.
    pub fn dto(
        &self,
        text: &ScriptText,
        args: &[OsString],
        dry_run: bool,
        silent: bool,
    ) -> ScriptDto {
        ScriptDto {
            mode: "run",
            script: text.display.clone(),
            args: arg_texts(args),
            dry_run,
            silent,
            cancelled: self.cancelled,
            stopped_at: self.stopped_at.map(|s| StopDto {
                line: s.line,
                reason: s.reason,
            }),
            errors: Vec::new(),
            errors_omitted: 0,
            counts: counts(self.commands.iter().map(|c| c.status)),
            commands: self
                .commands
                .iter()
                .map(|c| ScriptCommandDto {
                    line: c.line,
                    end_line: c.end_line,
                    name: c.name,
                    words: c.words.clone(),
                    text: c.text.clone(),
                    status: c.status,
                    schema: c.schema,
                    result: c.result.clone(),
                    items_omitted: c.items_omitted,
                    error: c.error.clone(),
                    warnings: c.warnings.clone(),
                })
                .collect(),
            elapsed_s: self.elapsed.map(|e| e.as_secs_f64()),
        }
    }
}

/// The checker of a script's words (§1.8.2): every vocabulary word through the parser of the CLI flag it stands for.
pub(crate) struct Vocabulary<'a> {
    pub(crate) service: &'a Service,
    /// `output-to:-` is refused with this message (`None`: allowed).
    pub(crate) stdout_refusal: Option<&'static str>,
}

/// The sides of a script word: `all` is both; `left`, `right` as `--side` reads them.
pub(crate) fn script_sides(word: &str) -> Result<hikasa_folder::select::SideSet, ApiError> {
    if word.eq_ignore_ascii_case("all") {
        return Ok(hikasa_folder::select::SideSet::all());
    }
    super::parse_sides(word)
}

/// True when the criteria list names `version` (FC-028): refused when the command runs (D-221).
pub(crate) fn names_version(list: &str) -> bool {
    list.split(|c: char| c == ',' || c.is_ascii_whitespace())
        .any(|w| w.eq_ignore_ascii_case("version"))
}

fn report_kind(kind: ReportFor) -> Option<ReportKind> {
    match kind {
        ReportFor::Text => Some(ReportKind::Text),
        ReportFor::Folder => Some(ReportKind::Folder),
        ReportFor::Other => None,
    }
}

impl WordChecker for Vocabulary<'_> {
    fn check(&self, slot: Slot, word: &str) -> Result<(), String> {
        let text = |e: ApiError| e.to_string();
        let folder = |e: hikasa_folder::FolderError| e.to_string();
        match slot {
            Slot::Side => super::parse_side(word).map(drop).map_err(text),
            Slot::Sides => script_sides(word).map(drop).map_err(text),
            Slot::Structure => super::parse_structure(word).map(drop).map_err(text),
            Slot::SelectMask => SelectMask::parse(word).map(drop).map_err(folder),
            Slot::Criteria => {
                let rest: Vec<&str> = word
                    .split(|c: char| c == ',' || c.is_ascii_whitespace())
                    .filter(|w| !w.is_empty() && !w.eq_ignore_ascii_case("version"))
                    .collect();
                Criteria::parse_list(&rest.join(","))
                    .map(drop)
                    .map_err(folder)
            }
            Slot::NameFilter => hikasa_core::NameFilter::parse(
                word,
                hikasa_core::MaskOptions {
                    case: hikasa_core::CaseSensitivity::Sensitive,
                },
            )
            .map(drop)
            .map_err(|e| e.to_string()),
            Slot::Cutoff => {
                let zone = self.service.clock().now().time_zone().clone();
                DateFilter::parse(word, &zone).map(drop).map_err(folder)
            }
            Slot::SizeFilter => SizeFilter::parse(word).map(drop).map_err(folder),
            Slot::AttribFilter => AttribFilter::parse(word).map(drop).map_err(folder),
            Slot::UnixTypeFilter => UnixTypeFilter::parse(word).map(drop).map_err(folder),
            Slot::ConfirmMode => super::parse_confirm_mode(word).map(drop).map_err(text),
            Slot::Time => {
                if word.eq_ignore_ascii_case("now") {
                    Ok(())
                } else {
                    super::parse_time(word).map(drop).map_err(text)
                }
            }
            // `none` parses (as `--content none` does) but tests nothing: `compare` refuses it when it runs.
            Slot::ContentMethod => match super::parse_content_method(word) {
                Ok(hikasa_core::ContentMethod::None) => {
                    Err("a content test needs a method".to_owned())
                }
                Ok(_) => Ok(()),
                Err(e) => Err(text(e)),
            },
            Slot::Regex => hikasa_folder::ops::RegexRename::new(word, "")
                .map(drop)
                .map_err(folder),
            Slot::AttribChange => super::parse_attrib_change(&[word.to_owned()])
                .map(drop)
                .map_err(text),
            Slot::ReportLayout(kind)
            | Slot::ReportOptions(kind)
            | Slot::ReportOutputOptions(kind) => {
                let Some(kind) = report_kind(kind) else {
                    return Ok(());
                };
                let mut flags = ReportFlags::default();
                match slot {
                    Slot::ReportLayout(_) => flags.words.layout = Some(word.to_owned()),
                    Slot::ReportOptions(_) => flags.words.options.push(word.to_owned()),
                    _ => flags.words.output_options.push(word.to_owned()),
                }
                let target = ReportTarget::File(OsString::from("report"));
                self.service
                    .report_request(kind, "side-by-side", &flags, &target)
                    .map(drop)
                    .map_err(text)
            }
            Slot::ReportTarget => match (word, self.stdout_refusal) {
                ("-", Some(why)) => Err(why.to_owned()),
                _ => Ok(()),
            },
            Slot::Direction
            | Slot::SyncDirection
            | Slot::Recycle
            | Slot::LogLevel
            | Slot::RenameMask
            | Slot::Spec
            | Slot::Session
            | Slot::Path
            | Slot::Text => Ok(()),
        }
    }

    fn check_command(
        &self,
        name: CommandName,
        words: &[(Slot, &str)],
    ) -> Result<(), (usize, String)> {
        match name {
            CommandName::Attrib => first_refused(words, &|words| {
                let changes: Vec<String> = words.iter().map(|(_, w)| (*w).to_owned()).collect();
                super::parse_attrib_change(&changes)
                    .map(drop)
                    .map_err(|e| e.to_string())
            }),
            CommandName::Rename => match words {
                [(Slot::Regex, old), (Slot::Text, new)] => {
                    hikasa_folder::ops::RegexRename::new(old, new)
                        .map(drop)
                        .map_err(|e| (1, e.to_string()))
                }
                _ => Ok(()),
            },
            _ if name.report().is_some() => first_refused(words, &|words| self.report_words(words)),
            _ => Ok(()),
        }
    }
}

impl Vocabulary<'_> {
    /// The report words of a command (layout, options, output options) through one `report_request`, as the command
    /// builds them when it runs.
    fn report_words(&self, words: &[(Slot, &str)]) -> Result<(), String> {
        let mut flags = ReportFlags::default();
        let mut kind = None;
        for (slot, word) in words {
            match slot {
                Slot::ReportLayout(k) => {
                    kind = report_kind(*k);
                    flags.words.layout = Some((*word).to_owned());
                }
                Slot::ReportOptions(k) => {
                    kind = report_kind(*k);
                    flags.words.options.push((*word).to_owned());
                }
                Slot::ReportOutputOptions(k) => {
                    kind = report_kind(*k);
                    flags.words.output_options.push((*word).to_owned());
                }
                _ => {}
            }
        }
        let Some(kind) = kind else { return Ok(()) };
        let target = ReportTarget::File(OsString::from("report"));
        self.service
            .report_request(kind, "side-by-side", &flags, &target)
            .map(drop)
            .map_err(|e| e.to_string())
    }
}

/// A judge of a command's words together.
type WordsJudge<'a> = dyn Fn(&[(Slot, &str)]) -> Result<(), String> + 'a;

/// The first `i` such that `words[..=i]` is refused by `refuses`, when `words` as a whole is: the word that makes a
/// command's words conflict.
fn first_refused(words: &[(Slot, &str)], refuses: &WordsJudge<'_>) -> Result<(), (usize, String)> {
    let Err(message) = refuses(words) else {
        return Ok(());
    };
    for end in 1..words.len() {
        if let Err(message) = refuses(&words[..end]) {
            return Err((end - 1, message));
        }
    }
    Err((words.len().saturating_sub(1), message))
}

/// The message of `output-to:-` refused with `--json` (§1.8.12).
pub const STDOUT_SHARED: &str = "the report and the JSON document cannot share stdout";
/// The message of `output-to:-` where no standard output is a target (the GUI).
pub const STDOUT_NONE: &str = "standard output is a report target only of `hikasa script run`";

/// Whether `spec` exists, any kind (§1.8.11 step 2): `Ok(true)` for a folder (or a link to one), `Ok(false)` for
/// anything else that exists, a dangling link included (it exists, and reading it fails with 105: only an absent name
/// falls through to the shared scripts folder); `Err(NotFound)` when nothing has that name.
fn present(router: &dyn Vfs, spec: &Spec) -> Result<bool, hikasa_vfs::VfsError> {
    let entry = router.stat(spec, FollowLinks::No)?;
    if entry.kind == EntryKind::Dir {
        return Ok(true);
    }
    match router.stat(spec, FollowLinks::Yes) {
        Ok(target) => Ok(target.kind == EntryKind::Dir),
        Err(_) => Ok(false),
    }
}

fn load_error(kind: hikasa_core::ErrorKind, message: String) -> ApiError {
    ApiError::ScriptLoad { kind, message }
}

impl Service {
    /// Locates a script (§1.8.11, D-225): `-` is standard input; a spec that exists as given (a folder is 105); a relative
    /// local path absent here is looked up below `[tweaks.scripts] shared_scripts_folder`; only *absent* falls through.
    pub fn script_locate(&self, arg: &OsStr) -> Result<ScriptSource, ApiError> {
        run_guarded(|| {
            if arg == "-" {
                return Ok(ScriptSource::Stdin);
            }
            let shown = hikasa_script::display_os(arg);
            let spec = self.parse_spec_as(arg, SpecUse::Any).map_err(|e| {
                load_error(
                    hikasa_core::ErrorKind::InvalidInput,
                    format!("script {shown}: {e}"),
                )
            })?;
            let router = self.router();
            match present(&*router, &spec) {
                Ok(true) => {
                    return Err(load_error(
                        hikasa_core::ErrorKind::InvalidInput,
                        format!("{} is a folder", spec.canonical()),
                    ));
                }
                Ok(false) => return Ok(ScriptSource::Spec(spec)),
                Err(hikasa_vfs::VfsError::NotFound { .. }) => {}
                Err(e) => {
                    return Err(load_error(
                        e.kind(),
                        format!("script {}: {e}", spec.canonical()),
                    ));
                }
            }
            let cwd = self
                .env()
                .cwd
                .as_ref()
                .map_or_else(|| ".".to_owned(), |p| p.display().to_string());
            let relative =
                !std::path::Path::new(arg).is_absolute() && matches!(spec, Spec::Local(_));
            let shared = self
                .fresh_options()
                .tweaks
                .scripts
                .shared_scripts_folder
                .clone();
            if relative && !shared.is_empty() {
                let mut folder = std::path::PathBuf::from(&shared);
                if folder.is_relative() {
                    folder = self.config().dir.join(folder);
                }
                let candidate = folder.join(arg);
                let spec2 = self
                    .parse_spec_as(candidate.as_os_str(), SpecUse::Any)
                    .map_err(|e| load_error(hikasa_core::ErrorKind::InvalidInput, e.to_string()))?;
                match present(&*router, &spec2) {
                    Ok(true) => {
                        return Err(load_error(
                            hikasa_core::ErrorKind::InvalidInput,
                            format!("{} is a folder", spec2.canonical()),
                        ));
                    }
                    Ok(false) => return Ok(ScriptSource::Spec(spec2)),
                    Err(hikasa_vfs::VfsError::NotFound { .. }) => {
                        return Err(load_error(
                            hikasa_core::ErrorKind::NotFound,
                            format!(
                                "script {shown} not found (looked in {cwd} and {})",
                                folder.display()
                            ),
                        ));
                    }
                    Err(e) => {
                        return Err(load_error(
                            e.kind(),
                            format!("script {}: {e}", spec2.canonical()),
                        ));
                    }
                }
            }
            Err(load_error(
                hikasa_core::ErrorKind::NotFound,
                format!("script {shown} not found (looked in {cwd})"),
            ))
        })
    }

    /// Reads a located script through the VFS, at most [`SCRIPT_MAX_BYTES`] (counted while reading; 105).
    pub fn script_load(
        &self,
        source: ScriptSource,
        stdin: Option<&mut dyn Read>,
    ) -> Result<ScriptText, ApiError> {
        run_guarded(|| {
            let cap = SCRIPT_MAX_BYTES + 1;
            let (display, path, bytes) = match &source {
                ScriptSource::Stdin => {
                    let Some(stdin) = stdin else {
                        return Err(load_error(
                            hikasa_core::ErrorKind::InvalidInput,
                            "standard input is not readable here".to_owned(),
                        ));
                    };
                    let mut bytes = Vec::new();
                    stdin.take(cap).read_to_end(&mut bytes).map_err(|e| {
                        load_error(hikasa_core::io_error_kind(&e), format!("-: {e}"))
                    })?;
                    ("-".to_owned(), OsString::from("-"), bytes)
                }
                ScriptSource::Spec(spec) => {
                    let router = self.router();
                    let canonical = spec.canonical();
                    let read = || -> Result<Vec<u8>, hikasa_vfs::VfsError> {
                        let mut reader = router.open_read(spec)?;
                        let mut bytes = Vec::new();
                        (&mut reader)
                            .take(cap)
                            .read_to_end(&mut bytes)
                            .map_err(|e| hikasa_vfs::VfsError::Io {
                                spec: canonical.clone(),
                                op: hikasa_vfs::VfsOp::Read,
                                source: e,
                            })?;
                        Ok(bytes)
                    };
                    let bytes = read()
                        .map_err(|e| load_error(e.kind(), format!("script {canonical}: {e}")))?;
                    let path = match spec {
                        Spec::Local(local) => local.as_path().as_os_str().to_owned(),
                        other => OsString::from(other.canonical()),
                    };
                    (
                        hikasa_report::escape_controls(&canonical).into_owned(),
                        path,
                        bytes,
                    )
                }
            };
            if bytes.len() as u64 > SCRIPT_MAX_BYTES {
                return Err(load_error(
                    hikasa_core::ErrorKind::TooLarge,
                    format!("{display}: the script is larger than the 16 MiB a script may hold"),
                ));
            }
            Ok(ScriptText {
                source,
                display,
                path,
                bytes,
            })
        })
    }

    /// Checks a script (§1.8.7): lexical rules, grammar, each word without `%` through the CLI's parsers, the flow check,
    /// and the references (`%n` against `args` when given, `%NAME%` against `env` always). Runs nothing.
    pub fn script_check(
        &self,
        text: &ScriptText,
        args: Option<&[OsString]>,
        env: &BTreeMap<String, OsString>,
    ) -> ScriptCheck {
        self.script_check_for(text, args, env, Some(STDOUT_NONE))
    }

    /// [`Service::script_check`] with the refusal of `output-to:-` the adapter needs (`None`: allowed; the headless
    /// CLI without `--json`).
    pub fn script_check_for(
        &self,
        text: &ScriptText,
        args: Option<&[OsString]>,
        env: &BTreeMap<String, OsString>,
        stdout_refusal: Option<&'static str>,
    ) -> ScriptCheck {
        let vocabulary = Vocabulary {
            service: self,
            stdout_refusal,
        };
        let refs = RefCheck {
            args,
            env,
            env_case_insensitive: self.env().platform == crate::host::Platform::Windows,
        };
        match hikasa_script::parse_checked(&text.bytes, &vocabulary, Some(&refs)) {
            Ok(script) => ScriptCheck {
                references: script.references.clone(),
                script: Some(script),
                errors: ScriptErrors::default(),
            },
            // Only a script that does not check is lexed a second time, for what it references.
            Err(errors) => ScriptCheck {
                script: None,
                errors,
                references: self.script_references(text),
            },
        }
    }

    /// The references of a script's text, before it is checked: what `capture_script_env` reads (an unreadable text
    /// references nothing).
    pub fn script_references(&self, text: &ScriptText) -> References {
        hikasa_script::references(&text.bytes).unwrap_or_default()
    }

    /// Runs a checked script (§1.8.5–§1.8.9) over the host: every command through the calls of `folder op`, `folder sync`,
    /// `report` (T11). `stdout` is the writer of `output-to:-`. A panic in a command is that command's failure.
    pub fn script_run<'a>(
        &'a self,
        script: &'a Script,
        text: &ScriptText,
        req: &'a ScriptRunRequest,
        ui: &'a mut dyn ScriptUi,
        stdout: Option<&'a mut dyn Write>,
        job: &'a JobControl,
    ) -> ScriptOutcome {
        let mut host = Host::new(self, script, req, ui, stdout, job);
        let opts = hikasa_script::RunOptions {
            script: &text.path,
            display: &text.display,
            args: &req.args,
            env: &req.env,
            env_case_insensitive: self.env().platform == crate::host::Platform::Windows,
            state: hikasa_script::RunState {
                confirm: match req.confirm {
                    Some(ConfirmMode::YesToAll) => ConfirmWord::YesToAll,
                    Some(ConfirmMode::NoToAll) => ConfirmWord::NoToAll,
                    Some(ConfirmMode::Prompt) | None => ConfirmWord::Prompt,
                },
                stop_on_error: req.stop_on_error,
                log_level: LogLevel::Normal,
                silent: req.silent,
                dry_run: req.dry_run,
            },
        };
        let outcome = hikasa_script::run(script, &mut host, &opts, &job.cancel);
        if beeps_when_finished(&self.fresh_options(), req) {
            hikasa_script::ScriptHost::ui(&mut host).beep();
        }
        let (mut results, warnings) = host.finish();
        // The command a cancel stopped fails `cancelled`, not `invalid_input`.
        let cancelled_at = outcome
            .stopped_at
            .filter(|stop| stop.reason == StopReason::Cancelled)
            .map(|stop| stop.line);
        let commands = script
            .commands()
            .zip(&outcome.records)
            .map(|(command, record)| {
                let kept = results.remove(&record.index).unwrap_or_default();
                let error = kept.error.or_else(|| {
                    record.message.as_ref().map(|message| ErrorReport {
                        kind: match record.status {
                            CommandStatus::Refused => hikasa_core::ErrorKind::Unsupported,
                            _ if cancelled_at == Some(command.pos.line) => {
                                hikasa_core::ErrorKind::Cancelled
                            }
                            _ => hikasa_core::ErrorKind::InvalidInput,
                        },
                        message: message.clone(),
                        spec: None,
                        causes: Vec::new(),
                    })
                });
                CommandRecord {
                    line: command.pos.line,
                    end_line: command.end_line,
                    name: command.name.as_str(),
                    words: command.word_texts(),
                    text: record.text.clone(),
                    status: record.status,
                    schema: kept.schema,
                    result: kept.result,
                    items_omitted: kept.items_omitted,
                    error,
                    warnings: kept.warnings,
                }
            })
            .collect();
        ScriptOutcome {
            exit_code: outcome.exit_code,
            commands,
            stopped_at: outcome.stopped_at,
            cancelled: outcome.cancelled,
            elapsed: outcome.elapsed,
            warnings,
        }
    }

    /// The Run Script dialog's arguments, split by the lexer's word rules (§1.8.3); no expansion.
    pub fn script_args(&self, text: &str) -> Result<Vec<String>, ScriptError> {
        hikasa_script::split_words(text)
    }
}

/// `[tweaks.scripts] beep_when_finished` (§1.8.9, OPT-021): the adapter's beep once the run ended, whatever its exit
/// code; never in silent mode (§1.8.14: no window, no notice).
pub fn beeps_when_finished(options: &crate::settings::Options, req: &ScriptRunRequest) -> bool {
    options.tweaks.scripts.beep_when_finished && !req.silent
}

/// The names a set of references needs from the environment.
pub fn referenced_names(references: &References) -> &BTreeSet<String> {
    &references.env
}
