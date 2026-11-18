//! `hikasa text find` (TC-064; D-284, D-285; contract §1.10.19, §1.10.23, §1.10.24): Find in Files through the
//! Service, and replace in files as one save plan (T12).
//!
//! ```text
//! text_find(req, sink, job)                  the search: the roots parsed (a trailing separator wants a folder), the
//!                                            masks (FC-050, the host's case rule), the encoding, the format and the
//!                                            limits checked first; hikasa_text::find::files::find_in_files_with on
//!                                            the router with the service's pool and External runner; every event
//!                                            goes to `sink` as it comes and into hikasa.text_find.v1 (its hits with
//!                                            `keep_hits`) → TextFindOutcome. With `replace`: the steps below with the
//!                                            request's answers and no person
//! text_find_replace_plan(req, sink, job)     the search, then each file with a hit loaded again as the search read it
//!                                            (its format and encoding, no characters-per-line limit), Replace All
//!                                            applied as one edit, and one save plan of the files that change (in
//!                                            place, backups by back_up_before_save); nothing written
//! text_find_replace_confirmation(&plan)      ReplaceDriver: the operation question "Replace N occurrences in M
//!                                            files?" (W6's shape), then the save questions (SaveDriver)
//! text_find_replace_dry_run(plan)            nothing written
//! text_find_replace_execute(plan, job)       execute_save: a file changed since the plan fails its item
//! ```
//!
//! Exit codes (§1.10.19): 0 whether or not anything matched; 100 for a request that cannot run (an invalid pattern
//! or replacement, a missing root, an unknown encoding or format, a bad mask or limit, open questions nobody answers),
//! a cancel, and a replace with a failed or unreached file.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::sync::Arc;

use hikasa_core::confirm::{Ask, Reply};
use hikasa_core::{
    CaseSensitivity, ErrorKind, ErrorReport, ExitCode, MaskOptions, NameFilter, Spec, Warning,
};
use hikasa_formats::{CompiledFormat, EnabledOverrides, FormatError, TextEncoding};
use hikasa_text::find::files::{
    FilesEvent, FilesLimits, FilesQuery, FilesSink, FindArchives, SkipWhy, find_in_files_with,
};
use hikasa_text::find::{FindQuery, Finder, Replacement, replace_all};
use hikasa_text::save::{SavePlan, SaveRequest, UnmappablePolicy};
use hikasa_text::{
    EditBlock, Editability, EditableDocument, GroupHint, LoadOptions, TextError, load_document,
};
use hikasa_vfs::ops::{ItemStatus, SkipReason};
use hikasa_vfs::{EntryKind, Vfs};
use serde::Serialize;

use super::text_edit::{SaveAnswers, SaveDriver, SavePlanItemDto, save_plan_items};
use super::text_live::{converts_without_save, edit_options};
use super::{ApiError, Service};
use crate::jobs::{JobControl, run_guarded};

/// `--replace` of `hikasa text find` (§1.10.19).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TextFindReplace {
    /// The replacement: literal for plain text; `$1`, `${1}`, `${name}`, `$$` for a regex (§1.10.18).
    pub text: String,
    /// `--dry-run`: the plan and the counts per file; nothing is written.
    pub dry_run: bool,
    /// `--yes` / `--confirm`: `yes` (or `no_to_all`) answers the operation question; the save questions as
    /// [`SaveAnswers`] says.
    pub answers: SaveAnswers,
    /// `--backup` (`Some(true)`) / `--no-backup` (`Some(false)`); `None`: `back_up_before_save`.
    pub backup: Option<bool>,
    /// What a character the file's encoding cannot write does (`Ask`: a question of the plan).
    pub unmappable: UnmappablePolicy,
}

/// `hikasa text find` (§1.10.19). Words (`--archives`, `--max-file-size`) are parsed by [`super::vocab`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TextFindRequest {
    /// What to find.
    pub pattern: String,
    /// The files and folders searched, in this order (`-` is standard input).
    pub roots: Vec<OsString>,
    /// `--regex`.
    pub regex: bool,
    /// `--case`: match case (the default ignores it, Unicode simple folding).
    pub match_case: bool,
    /// `--word`: whole words (the format's word characters, D-58).
    pub whole_word: bool,
    /// `--recursive`: the folders below a folder root too.
    pub recursive: bool,
    /// `--mask <masks>` (FC-050: `*.c;*.h;-obj\`), names only, the host's case rule.
    pub mask: Option<String>,
    /// `--include-hidden`.
    pub include_hidden: bool,
    /// `--archives files|opened`.
    pub archives: FindArchives,
    /// `--encoding <label>`: every file is read in it (else each file's detection, D-42).
    pub encoding: Option<String>,
    /// `--format <id|name>` (additive): one format for every file (else detection by name).
    pub format: Option<String>,
    /// `--convert`: files whose format converts them (External, Sorted) are searched in their converted text.
    pub convert: bool,
    /// `--max-matches <n>` (default 100,000).
    pub max_matches: Option<u64>,
    /// `--max-matches-per-file <n>` (additive; default 10,000).
    pub max_matches_per_file: Option<u32>,
    /// `--max-file-size <bytes>` (default 128 MiB).
    pub max_file_size: Option<u64>,
    /// The JSON document keeps every hit delivered (`--json`); else only the files and their counts (the stream had
    /// the hits), so memory never grows with them.
    pub keep_hits: bool,
    /// `--replace <TEXT>`.
    pub replace: Option<TextFindReplace>,
}

/// `{pattern, regex, match_case, whole_word}` of `hikasa.text_find.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FindQueryDto {
    /// The pattern.
    pub pattern: String,
    /// A regex.
    pub regex: bool,
    /// Case matched.
    pub match_case: bool,
    /// Whole words.
    pub whole_word: bool,
}

/// One hit of `hikasa.text_find.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FindHitDto {
    /// 1-based physical line.
    pub line: u32,
    /// 1-based column in characters.
    pub column: u32,
    /// Length in characters.
    pub len: u32,
    /// The line, cut to 400 characters around the hit.
    pub preview: String,
    /// The hit in the preview, in characters (`[a, b)`).
    pub preview_range: [u32; 2],
    /// The preview is cut.
    pub truncated: bool,
}

/// One file with at least one hit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FindFileDto {
    /// The file's canonical spec.
    pub spec: String,
    /// The encoding it was read in.
    pub encoding: &'static str,
    /// Its hits (empty without `keep_hits`).
    pub hits: Vec<FindHitDto>,
    /// Hits delivered.
    pub hit_count: u64,
    /// The file stopped at `--max-matches-per-file` (or the search at `--max-matches`) before its end.
    pub stopped: bool,
}

/// A file or folder not searched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FindSkippedDto {
    /// Its canonical spec.
    pub spec: String,
    /// `binary`, `too_large`, `unreadable`, `converted`, `filtered`.
    pub reason: &'static str,
    /// Why it could not be read (`unreadable`).
    pub error: Option<ErrorReport>,
}

/// The counts of `hikasa.text_find.v1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct FindCountsDto {
    /// Files searched.
    pub files_searched: u64,
    /// Files with at least one hit.
    pub files_matched: u64,
    /// Hits delivered.
    pub hits: u64,
    /// Files and folders skipped.
    pub skipped: u64,
}

/// One file of a replace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FindReplaceResultDto {
    /// The file's canonical spec.
    pub spec: String,
    /// Occurrences replaced (planned, in a dry run).
    pub replaced: u64,
    /// `planned`, `done`, `skipped`, `failed`, `not_run`.
    pub status: ItemStatus,
    /// The skip reason (`no_change`, `declined`; additive).
    pub skip: Option<SkipReason>,
    /// The failure.
    pub error: Option<ErrorReport>,
}

/// The counts of a replace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct FindReplaceSummaryDto {
    /// Occurrences replaced in the files done (planned, in a dry run; additive).
    pub replaced: u64,
    /// Files written.
    pub done: u64,
    /// Files skipped.
    pub skipped: u64,
    /// Files failed.
    pub failed: u64,
    /// Files not reached (a cancel).
    pub not_run: u64,
}

/// `replace` of `hikasa.text_find.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FindReplaceDto {
    /// The replacement as given.
    pub text: String,
    /// Nothing was written.
    pub dry_run: bool,
    /// The save plan, one item per file with a hit.
    pub plan: Vec<SavePlanItemDto>,
    /// One per file with a hit, in the search's order.
    pub results: Vec<FindReplaceResultDto>,
    /// The counts.
    pub summary: FindReplaceSummaryDto,
}

/// `hikasa.text_find.v1` (`result`, §1.10.24).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TextFindDto {
    /// The query.
    pub query: FindQueryDto,
    /// The roots' canonical specs.
    pub roots: Vec<String>,
    /// `--mask` as given.
    pub mask: Option<String>,
    /// `--recursive`.
    pub recursive: bool,
    /// `--include-hidden`.
    pub include_hidden: bool,
    /// `files`, `opened`.
    pub archives: &'static str,
    /// The files with at least one hit, in the search's order.
    pub files: Vec<FindFileDto>,
    /// The files and folders not searched, in order.
    pub skipped: Vec<FindSkippedDto>,
    /// The counts.
    pub counts: FindCountsDto,
    /// `--max-matches` was reached: the rest was not searched.
    pub limit_reached: bool,
    /// The replace (`null` without `--replace`).
    pub replace: Option<FindReplaceDto>,
}

/// What `text find` found (and replaced).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextFindOutcome {
    /// `hikasa.text_find.v1`.
    pub dto: TextFindDto,
    /// Every warning (the search's, the plan's, the execution's), for the envelope.
    pub warnings: Vec<Warning>,
    /// 0 for a search, matched or not; for a replace 0 when every file was done or skipped, else 100.
    pub exit_code: ExitCode,
    /// The replace plan's human lines (`write <target> (…)`).
    pub plan_lines: Vec<String>,
}

impl TextFindOutcome {
    /// One line per file of the replace (§1.10.19): `replaced <n> occurrences: <spec>` (`would replace …` in a dry
    /// run), `no change: <spec>`, `skipped (<reason>): <spec>`, `failed: <spec>: <message>`, `not run: <spec>`.
    /// Empty without `--replace`.
    pub fn replace_lines(&self) -> Vec<String> {
        let Some(replace) = &self.dto.replace else {
            return Vec::new();
        };
        replace
            .results
            .iter()
            .map(|r| match r.status {
                ItemStatus::Done | ItemStatus::Planned => format!(
                    "{} {} occurrences: {}",
                    if replace.dry_run {
                        "would replace"
                    } else {
                        "replaced"
                    },
                    r.replaced,
                    r.spec
                ),
                ItemStatus::Skipped if r.skip == Some(SkipReason::NoChange) => {
                    format!("no change: {}", r.spec)
                }
                ItemStatus::Skipped => format!(
                    "skipped ({}): {}",
                    r.skip.map_or("skipped", SkipReason::as_str),
                    r.spec
                ),
                ItemStatus::Failed => format!(
                    "failed: {}: {}",
                    r.spec,
                    r.error
                        .as_ref()
                        .map_or("failed", |error| error.message.as_str())
                ),
                ItemStatus::NotRun => format!("not run: {}", r.spec),
            })
            .collect()
    }
}

/// A sink that drops every event (the caller only wants the outcome, as `--json` does).
#[derive(Debug, Clone, Copy, Default)]
pub struct NoEvents;

impl FilesSink for NoEvents {
    fn event(&mut self, _: FilesEvent<'_>) {}
}

/// Collects the document while forwarding every event to the caller's sink.
struct Collect<'a> {
    inner: &'a mut dyn FilesSink,
    keep_hits: bool,
    files: Vec<FindFileDto>,
    skipped: Vec<FindSkippedDto>,
    current: Option<(Spec, FindFileDto)>,
    /// The files with a hit, in order, with their hit counts.
    matched: Vec<Spec>,
}

impl FilesSink for Collect<'_> {
    fn event(&mut self, e: FilesEvent<'_>) {
        match &e {
            FilesEvent::File { spec, encoding, .. } => {
                self.current = Some((
                    (*spec).clone(),
                    FindFileDto {
                        spec: spec.canonical(),
                        encoding: encoding.name(),
                        hits: Vec::new(),
                        hit_count: 0,
                        stopped: false,
                    },
                ));
            }
            FilesEvent::Hit {
                line,
                column,
                len_chars,
                preview,
                preview_range,
                truncated,
                ..
            } => {
                if let Some((_, file)) = &mut self.current {
                    file.hit_count += 1;
                    if self.keep_hits {
                        file.hits.push(FindHitDto {
                            line: *line,
                            column: *column,
                            len: *len_chars,
                            preview: (*preview).to_owned(),
                            preview_range: [preview_range.start, preview_range.end],
                            truncated: *truncated,
                        });
                    }
                }
            }
            FilesEvent::Done { hits, stopped, .. } => {
                if let Some((spec, mut file)) = self.current.take()
                    && *hits > 0
                {
                    file.stopped = *stopped;
                    self.matched.push(spec);
                    self.files.push(file);
                }
            }
            FilesEvent::Skipped { spec, reason } => {
                self.skipped.push(FindSkippedDto {
                    spec: spec.canonical(),
                    reason: reason.as_str(),
                    error: match reason {
                        SkipWhy::Unreadable(report) => Some(report.clone()),
                        _ => None,
                    },
                });
            }
        }
        self.inner.event(e);
    }
}

/// A finished search: its document (without `replace`), the files with hits and the query it ran.
struct Searched {
    dto: TextFindDto,
    matched: Vec<Spec>,
    warnings: Vec<Warning>,
    query: FilesQuery,
}

impl Searched {
    fn outcome(self) -> TextFindOutcome {
        TextFindOutcome {
            dto: self.dto,
            warnings: self.warnings,
            exit_code: ExitCode::Success,
            plan_lines: Vec::new(),
        }
    }
}

/// A file with a hit before the save plan.
#[derive(Debug, Clone)]
enum Pre {
    /// Refused or not loaded again.
    Failed(ErrorReport),
    /// No occurrence when loaded again (it changed since the search): not rewritten.
    NoChange,
    /// In the save plan at this item.
    Planned(usize),
}

/// One file of a replace plan.
#[derive(Debug, Clone)]
struct ReplaceFile {
    spec: String,
    encoding: Option<&'static str>,
    bom: bool,
    replaced: u64,
    pre: Pre,
}

/// A planned replace in files (§1.10.19): the search's document, each file with a hit, the edited documents and the
/// save plan of those that change.
#[derive(Debug, Clone)]
pub struct TextFindReplacePlan {
    search: TextFindDto,
    text: String,
    dry_run: bool,
    answers: SaveAnswers,
    files: Vec<ReplaceFile>,
    /// The documents of the save plan's items, in its order, with Replace All applied.
    docs: Vec<EditableDocument>,
    save: Option<SavePlan>,
    warnings: Vec<Warning>,
}

impl TextFindReplacePlan {
    /// The save plan (`None` when no file changes).
    pub fn save_plan(&self) -> Option<&SavePlan> {
        self.save.as_ref()
    }

    /// The documents of the save plan's items, in its order, with Replace All applied.
    pub fn documents(&self) -> &[EditableDocument] {
        &self.docs
    }

    /// Occurrences the plan replaces (the files planned to be written).
    pub fn occurrences(&self) -> u64 {
        self.planned().map(|f| f.replaced).sum()
    }

    /// Files the plan writes.
    pub fn files_to_write(&self) -> u64 {
        self.planned().count() as u64
    }

    fn planned(&self) -> impl Iterator<Item = &ReplaceFile> {
        self.files
            .iter()
            .filter(|f| matches!(f.pre, Pre::Planned(_)))
    }

    /// Leaves the files of `specs` (canonical specs) out of the plan (M2 W11b, D-308: the window's files open in a view
    /// with unsaved edits): their save items are skipped as declined and the files are reported failed with `reason`.
    /// The specs left out, in the plan's order. Additive.
    pub fn leave_out(&mut self, specs: &[String], reason: &str) -> Vec<String> {
        let mut out = Vec::new();
        for file in &mut self.files {
            if !specs.contains(&file.spec) {
                continue;
            }
            if let Pre::Planned(at) = file.pre
                && let Some(planned) = self.save.as_mut().and_then(|s| s.plan.items.get_mut(at))
            {
                planned.action = hikasa_vfs::ops::Action::Skip;
                planned.skip = Some(SkipReason::Declined);
                planned.replaces = false;
                planned.backup = None;
            }
            if matches!(file.pre, Pre::Planned(_) | Pre::NoChange) {
                file.pre = Pre::Failed(ErrorReport {
                    kind: hikasa_core::ErrorKind::PermissionDenied,
                    message: reason.to_owned(),
                    spec: Some(file.spec.clone()),
                    causes: Vec::new(),
                });
                out.push(file.spec.clone());
            }
        }
        out
    }

    /// *Replace N occurrences in M files?*
    pub fn question(&self) -> String {
        format!(
            "Replace {} occurrences in {} files?",
            self.occurrences(),
            self.files_to_write()
        )
    }

    /// The plan with the answers of `driver` ([`ReplaceDriver`], its save questions by [`SaveDriver::finish`]): a no to
    /// the operation, a cancel or an open operation question is `Cancelled` (nothing runs); declined files are
    /// skipped.
    pub fn answered(mut self, driver: ReplaceDriver) -> Result<TextFindReplacePlan, ApiError> {
        if driver.declined || driver.operation.is_some() {
            return Err(ApiError::Cancelled);
        }
        if let (Some(save), Some(machine)) = (self.save.take(), driver.save) {
            self.save = Some(machine.finish(save).map_err(|_| ApiError::Cancelled)?);
        }
        Ok(self)
    }
}

/// The question machine of a replace in files as an adapter drives it: the operation question *Replace N occurrences in
/// M files?* first (W6's operation question: `--yes` or `--confirm yes-to-all|no-to-all` answer it), then the save
/// questions of [`SaveDriver`] (§1.10.14). An [`Ask`].
#[derive(Debug, Clone)]
pub struct ReplaceDriver {
    /// The operation question while it is open.
    operation: Option<String>,
    /// A no to the operation, or a cancel.
    declined: bool,
    save: Option<SaveDriver>,
}

impl ReplaceDriver {
    /// The question a person must answer; `None`: nothing left.
    pub fn next_question(&self) -> Option<&str> {
        if self.declined {
            return None;
        }
        match &self.operation {
            Some(text) => Some(text.as_str()),
            None => self
                .save
                .as_ref()
                .and_then(|s| s.next())
                .map(|q| q.text.as_str()),
        }
    }

    /// The classes of the open questions with their counts (`replace` for the operation question).
    pub fn open_questions(&self) -> BTreeMap<&'static str, u64> {
        let mut open = BTreeMap::new();
        if self.declined {
            return open;
        }
        if self.operation.is_some() {
            open.insert("replace", 1);
        }
        if let Some(save) = &self.save {
            for (kind, n) in save.open_questions() {
                *open.entry(kind.as_str()).or_insert(0) += n;
            }
        }
        open
    }

    /// Answers the current question: to the operation, `Yes`/`YesToAll` go on and anything else declines (nothing
    /// runs); the save questions as [`SaveDriver::reply`].
    pub fn reply(&mut self, reply: Reply) {
        if self.declined {
            return;
        }
        if self.operation.is_some() {
            self.operation = None;
            if !matches!(reply, Reply::Yes | Reply::YesToAll) {
                self.declined = true;
            }
            return;
        }
        if reply == Reply::Cancel {
            self.declined = true;
            return;
        }
        if let Some(save) = &mut self.save {
            save.reply(reply);
        }
    }

    /// The refusal of a plan whose questions no person can answer (W6 §1.6.11, no terminal): `InvalidArgument` with
    /// the counts and the flags that answer them; `None` when nothing is open.
    pub fn refusal(&self) -> Option<ApiError> {
        let open = self.open_questions();
        if open.is_empty() {
            return None;
        }
        let total: u64 = open.values().sum();
        let mut flags: Vec<&str> = Vec::new();
        if self.operation.is_some() {
            flags.push("--yes");
        }
        if let Some(save) = &self.save {
            flags.extend(
                save.open_questions()
                    .keys()
                    .map(|k| SaveAnswers::flag_of(*k)),
            );
        }
        flags.sort_unstable();
        flags.dedup();
        let words: Vec<String> = open.iter().map(|(k, n)| format!("{k} {n}")).collect();
        Some(ApiError::InvalidArgument(format!(
            "{total} questions need an answer: pass {} (or --confirm yes-to-all|no-to-all), or --dry-run to see the \
             plan ({})",
            flags.join(" or "),
            words.join(", ")
        )))
    }
}

impl Ask for ReplaceDriver {
    fn question(&self) -> Option<&str> {
        self.next_question()
    }

    fn answer(&mut self, reply: Reply) {
        self.reply(reply);
    }

    fn remaining(&self) -> u64 {
        if self.declined {
            return 0;
        }
        u64::from(self.operation.is_some())
            + self
                .save
                .as_ref()
                .and_then(|s| s.next())
                .map_or(0, |q| q.remaining)
    }
}

/// The report of an item error.
fn item_error(kind: ErrorKind, spec: &str, message: String) -> ErrorReport {
    ErrorReport {
        kind,
        message,
        spec: Some(spec.to_owned()),
        causes: Vec::new(),
    }
}

/// An engine error as the Service's: a cancel is `Cancelled`, a root that cannot be read its VFS error.
fn find_error(error: TextError) -> ApiError {
    match error {
        e if e.is_cancelled() => ApiError::Cancelled,
        TextError::Load { source, .. } => ApiError::from_vfs(source),
        e => ApiError::Text(e),
    }
}

/// A limit of 0 is refused (it would search nothing).
fn positive<T: PartialEq + Default>(value: Option<T>, flag: &str) -> Result<Option<T>, ApiError> {
    match value {
        Some(v) if v == T::default() => Err(ApiError::InvalidArgument(format!(
            "{flag} must be at least 1"
        ))),
        other => Ok(other),
    }
}

impl Service {
    /// `text find` (§1.10.19, §1.10.23): the search, streamed to `sink` and collected into `hikasa.text_find.v1`
    /// (see the module documentation). With `req.replace` the replace runs in the same call with the request's answers
    /// (open questions nobody answers refuse the request before anything is written, W6 §1.6.11): plan, then the dry
    /// run or the execution. A file that cannot be read is skipped (`unreadable`), never an error; the call fails for
    /// a request that cannot run and a cancel.
    pub fn text_find(
        &self,
        req: &TextFindRequest,
        sink: &mut dyn FilesSink,
        job: &JobControl,
    ) -> Result<TextFindOutcome, ApiError> {
        self.text_find_in(None, req, sink, job)
    }

    /// [`Service::text_find`] on the back-end `vfs` (a test's `MemoryVfs`, or a wrapper of [`Service::vfs`]): the
    /// search, the replace plan and its execution all read and write there. Additive.
    pub fn text_find_on(
        &self,
        vfs: &dyn Vfs,
        req: &TextFindRequest,
        sink: &mut dyn FilesSink,
        job: &JobControl,
    ) -> Result<TextFindOutcome, ApiError> {
        self.text_find_in(Some(vfs), req, sink, job)
    }

    fn text_find_in(
        &self,
        vfs: Option<&dyn Vfs>,
        req: &TextFindRequest,
        sink: &mut dyn FilesSink,
        job: &JobControl,
    ) -> Result<TextFindOutcome, ApiError> {
        let Some(replace) = &req.replace else {
            return run_guarded(|| self.find_search(vfs, req, sink, job).map(Searched::outcome));
        };
        let plan = run_guarded(|| self.replace_plan_in(vfs, req, sink, job))?;
        if replace.dry_run {
            return Ok(self.text_find_replace_dry_run(plan));
        }
        let plan = match self.text_find_replace_confirmation(&plan) {
            Some(driver) => {
                if let Some(refusal) = driver.refusal() {
                    return Err(refusal);
                }
                plan.answered(driver)?
            }
            None => plan,
        };
        run_guarded(|| Ok(self.replace_execute_in(vfs, plan, job)))
    }

    /// The engine's query of a request, every word and limit checked (nothing read yet but the roots' specs).
    fn find_query(&self, req: &TextFindRequest, job: &JobControl) -> Result<FilesQuery, ApiError> {
        if req.roots.is_empty() {
            return Err(ApiError::InvalidArgument(
                "no file or folder to search".to_owned(),
            ));
        }
        let max_matches = positive(req.max_matches, "--max-matches")?;
        let per_file = positive(req.max_matches_per_file, "--max-matches-per-file")?;
        let max_file_size = positive(req.max_file_size, "--max-file-size")?;
        let find = FindQuery {
            text: req.pattern.clone(),
            regex: req.regex,
            match_case: req.match_case,
            whole_word: req.whole_word,
        };
        let registry = self.registry();
        // The pattern (and the replacement against its groups) is refused before any file is read.
        let finder = Finder::new(&find, "", registry.limits())
            .map_err(|e| ApiError::Text(TextError::Find(e)))?;
        if let Some(replace) = &req.replace {
            Replacement::new(&replace.text, &finder)
                .map_err(|e| ApiError::Text(TextError::Find(e)))?;
        }
        let encoding = req
            .encoding
            .as_deref()
            .map(|label| {
                TextEncoding::for_label(label).ok_or_else(|| {
                    ApiError::Format(FormatError::UnknownEncoding {
                        label: label.to_owned(),
                    })
                })
            })
            .transpose()?;
        let format = req
            .format
            .as_deref()
            .map(|name| registry.force(name).map(|d| d.format))
            .transpose()?;
        let mask_options = MaskOptions {
            case: CaseSensitivity::HOST_DEFAULT,
        };
        let masks = match req.mask.as_deref() {
            Some(mask) => NameFilter::parse(mask, mask_options)
                .map_err(|e| ApiError::InvalidArgument(format!("--mask {mask:?}: {e}")))?,
            None => NameFilter::match_all(mask_options),
        };
        let mut roots = Vec::with_capacity(req.roots.len());
        for raw in &req.roots {
            roots.push(self.parse_spec_in(raw, self.use_of(raw), &job.cancel)?);
        }
        let defaults = FilesLimits::default();
        Ok(FilesQuery {
            find,
            roots,
            masks,
            recursive: req.recursive,
            include_hidden: req.include_hidden,
            archives: req.archives,
            limits: FilesLimits {
                max_matches: max_matches.unwrap_or(defaults.max_matches),
                max_matches_per_file: per_file.unwrap_or(defaults.max_matches_per_file),
                max_file_bytes: max_file_size.unwrap_or(defaults.max_file_bytes),
                preview_chars: defaults.preview_chars,
            },
            encoding,
            convert: req.convert,
            format,
            overrides: EnabledOverrides::default(),
            locale: self.locale_encoding(),
        })
    }

    /// The search of a request (no replace).
    fn find_search(
        &self,
        vfs: Option<&dyn Vfs>,
        req: &TextFindRequest,
        sink: &mut dyn FilesSink,
        job: &JobControl,
    ) -> Result<Searched, ApiError> {
        let query = self.find_query(req, job)?;
        // After the specs were parsed: reading standard input puts it into the router.
        let router = self.router();
        let vfs: &dyn Vfs = vfs.unwrap_or(&*router);
        let registry = self.registry();
        let mut collect = Collect {
            inner: sink,
            keep_hits: req.keep_hits,
            files: Vec::new(),
            skipped: Vec::new(),
            current: None,
            matched: Vec::new(),
        };
        let summary = find_in_files_with(
            vfs,
            &registry,
            &query,
            Some(self.external()),
            &mut collect,
            Some(self.pool()),
            &job.ctl(),
        )
        .map_err(find_error)?;
        let Collect {
            files,
            skipped,
            matched,
            ..
        } = collect;
        let dto = TextFindDto {
            query: FindQueryDto {
                pattern: req.pattern.clone(),
                regex: req.regex,
                match_case: req.match_case,
                whole_word: req.whole_word,
            },
            roots: query.roots.iter().map(Spec::canonical).collect(),
            mask: req.mask.clone(),
            recursive: req.recursive,
            include_hidden: req.include_hidden,
            archives: req.archives.as_str(),
            files,
            skipped,
            counts: FindCountsDto {
                files_searched: summary.files_searched,
                files_matched: summary.files_matched,
                hits: summary.hits,
                skipped: summary.skipped,
            },
            limit_reached: summary.limit_reached,
            replace: None,
        };
        Ok(Searched {
            dto,
            matched,
            warnings: summary.warnings,
            query,
        })
    }

    /// Plans `text find --replace` (§1.10.19): the search (streamed to `sink`), then every file with a hit loaded again
    /// as the search read it (the same format, encoding and size limit; no characters-per-line limit), Replace All
    /// applied as one edit (every occurrence of the file, also past `--max-matches-per-file`; files the search did not
    /// reach after `--max-matches` are not touched), and one save plan of the files that change: in place, backups by
    /// `back_up_before_save` unless `backup` says otherwise, the format's save program for an External format.
    /// Refused per file (its item fails): binary content, a format that converts it without a save program, a
    /// document cut at the size limit, a member of an archive that cannot be written. A file without an occurrence
    /// when loaded again is skipped (`no_change`). Nothing is written.
    pub fn text_find_replace_plan(
        &self,
        req: &TextFindRequest,
        sink: &mut dyn FilesSink,
        job: &JobControl,
    ) -> Result<TextFindReplacePlan, ApiError> {
        run_guarded(|| self.replace_plan_in(None, req, sink, job))
    }

    fn replace_plan_in(
        &self,
        vfs: Option<&dyn Vfs>,
        req: &TextFindRequest,
        sink: &mut dyn FilesSink,
        job: &JobControl,
    ) -> Result<TextFindReplacePlan, ApiError> {
        {
            let replace = req.replace.clone().ok_or_else(|| {
                ApiError::InvalidArgument("text find: no replacement given".to_owned())
            })?;
            let backup = self.backup_policy_for_save_with(replace.backup)?;
            let searched = self.find_search(vfs, req, sink, job)?;
            let router = self.router();
            let vfs: &dyn Vfs = vfs.unwrap_or(&*router);
            let options = self.fresh_options();
            let mut files = Vec::with_capacity(searched.matched.len());
            let mut docs: Vec<(EditableDocument, SaveRequest)> = Vec::new();
            for spec in &searched.matched {
                job.cancel.check().map_err(|_| ApiError::Cancelled)?;
                let mut file = ReplaceFile {
                    spec: spec.canonical(),
                    encoding: None,
                    bom: false,
                    replaced: 0,
                    pre: Pre::NoChange,
                };
                file.pre =
                    match self.replace_in(vfs, spec, &searched.query, &replace, &options, job)? {
                        Err(error) => Pre::Failed(error),
                        Ok((doc, format, count)) => {
                            file.encoding = Some(doc.encoding().0.name());
                            file.bom = doc.encoding().1;
                            file.replaced = count;
                            if count == 0 {
                                Pre::NoChange
                            } else {
                                let save = SaveRequest {
                                    target: None,
                                    backup: backup.clone(),
                                    unmappable: replace.unmappable,
                                    conversion: SaveRequest::conversion_of(&format),
                                    transform: None,
                                };
                                docs.push((doc, save));
                                Pre::Planned(docs.len() - 1)
                            }
                        }
                    };
                files.push(file);
            }
            let save = if docs.is_empty() {
                None
            } else {
                let triples: Vec<_> = docs.iter().map(|(doc, save)| (None, doc, save)).collect();
                Some(self.plan_save_in(vfs, &triples, job)?)
            };
            Ok(TextFindReplacePlan {
                search: searched.dto,
                text: replace.text,
                dry_run: replace.dry_run,
                answers: replace.answers,
                files,
                docs: docs.into_iter().map(|(doc, _)| doc).collect(),
                save,
                warnings: searched.warnings,
            })
        }
    }

    /// Loads one file with a hit again and applies Replace All: `Ok(Err(_))` a failure of the file, `Err` a cancel.
    #[allow(clippy::type_complexity)]
    fn replace_in(
        &self,
        vfs: &dyn Vfs,
        spec: &Spec,
        q: &FilesQuery,
        replace: &TextFindReplace,
        options: &crate::settings::options::Options,
        job: &JobControl,
    ) -> Result<Result<(EditableDocument, Arc<CompiledFormat>, u64), ErrorReport>, ApiError> {
        let ctl = job.ctl();
        let canonical = spec.canonical();
        let fail = |e: ApiError| -> Result<Result<_, ErrorReport>, ApiError> {
            match e {
                ApiError::Cancelled => Err(ApiError::Cancelled),
                e => {
                    let mut report = e.report();
                    report.spec.get_or_insert_with(|| canonical.clone());
                    Ok(Err(report))
                }
            }
        };
        let registry = self.registry();
        // The format and the reading of the search (Find in Files' rules, §1.10.19).
        let format = match &q.format {
            Some(format) => Arc::clone(format),
            None => {
                let name = spec.file_name().unwrap_or_default();
                match registry.detect(&name, &q.overrides) {
                    Ok(detection) => detection.format,
                    Err(e) => return fail(ApiError::Format(e)),
                }
            }
        };
        let has_location = !matches!(spec, Spec::Stdin | Spec::Clipboard);
        // What is not a regular file is never rewritten, and is refused before it is read again (a FIFO without a
        // writer would block the read, W10b note 65).
        if has_location
            && let Ok(entry) = vfs.stat(spec, hikasa_vfs::FollowLinks::Yes)
            && entry.kind != EntryKind::File
        {
            return Ok(Err(ApiError::from_vfs(
                hikasa_vfs::VfsError::InvalidTarget {
                    spec: canonical.clone(),
                    reason: "is not a regular file",
                },
            )
            .report()));
        }
        let mut load = LoadOptions::default();
        load.decode.locale_fallback = q.locale;
        load.decode.override_encoding = q.encoding;
        load.chars_per_line_limit = Some(0);
        load.max_bytes = q.limits.max_file_bytes;
        let external = q.convert.then(|| self.external());
        let doc = match load_document(vfs, spec, &format, &load, external, &ctl) {
            Ok((doc, _raw)) => doc,
            Err(e) if e.is_cancelled() => return Err(ApiError::Cancelled),
            Err(TextError::Load { source, .. }) => return fail(ApiError::from_vfs(source)),
            Err(e) => return fail(ApiError::Text(e)),
        };
        // The refusals of a save (§1.10.7's blocks), as `text convert` refuses a file.
        let block = if doc.binary().is_binary {
            Some(EditBlock::Binary)
        } else if converts_without_save(&format) {
            Some(EditBlock::Converted)
        } else if doc.truncated() {
            Some(EditBlock::Truncated)
        } else if matches!(spec, Spec::ArchiveMember { .. })
            && vfs.writable(spec).ok().flatten() == Some(false)
        {
            Some(EditBlock::ArchiveReadOnly)
        } else {
            None
        };
        if let Some(block) = block {
            let mut report = ApiError::EditBlocked { side: None, block }.report();
            report.spec = Some(canonical.clone());
            return Ok(Err(report));
        }
        // Lines are the file's: no characters-per-line wrapping (hits are on physical lines).
        let mut edit = edit_options(&format, options, self.env().platform);
        edit.chars_per_line_limit = None;
        let mut doc = EditableDocument::new(
            Arc::new(doc),
            edit,
            Editability::with_blocks([], has_location),
        );
        // The finder of the file's word characters (whole word, D-58), as the search used.
        let finder = Finder::new(&q.find, &format.text().word_chars, registry.limits())
            .or_else(|_| Finder::new(&q.find, "", registry.limits()))
            .map_err(|e| ApiError::Text(TextError::Find(e)))?;
        let with = match Replacement::new(&replace.text, &finder) {
            Ok(with) => with,
            Err(e) => return fail(ApiError::Text(TextError::Find(e))),
        };
        let (edit_request, count) = match replace_all(&doc, &finder, &with, None, &ctl) {
            Ok(found) => found,
            Err(hikasa_text::find::FindError::Cancelled) => return Err(ApiError::Cancelled),
            Err(e) => return fail(ApiError::Text(TextError::Find(e))),
        };
        if let Some(request) = edit_request
            && let Err(e) = doc.apply(&request, GroupHint::None)
        {
            return Ok(Err(item_error(
                ErrorKind::InvalidInput,
                &canonical,
                e.to_string(),
            )));
        }
        Ok(Ok((doc, format, count)))
    }

    /// The question machine of a planned replace (`--yes`, `--confirm`): the operation question, unless the answers
    /// say yes to it (`yes`, `no_to_all`), then the save questions. `None` when nothing is planned to be written.
    pub fn text_find_replace_confirmation(
        &self,
        plan: &TextFindReplacePlan,
    ) -> Option<ReplaceDriver> {
        let save = plan.save.as_ref()?;
        if plan.files_to_write() == 0 {
            return None;
        }
        let operation = (!(plan.answers.yes || plan.answers.no_to_all)).then(|| plan.question());
        Some(ReplaceDriver {
            operation,
            declined: false,
            save: Some(self.save_confirmation(save, plan.answers)),
        })
    }

    /// The plan as `--dry-run` shows it: nothing is written.
    pub fn text_find_replace_dry_run(&self, plan: TextFindReplacePlan) -> TextFindOutcome {
        let items = plan
            .save
            .as_ref()
            .map(|save| save_plan_items(save, None))
            .unwrap_or_default();
        let mut warnings = plan.warnings.clone();
        if let Some(save) = &plan.save {
            warnings.extend(save.warnings.iter().cloned());
        }
        let plan_lines = plan
            .save
            .as_ref()
            .map(SavePlan::human_lines)
            .unwrap_or_default();
        build(plan, items, warnings, plan_lines, false)
    }

    /// Executes a planned (and answered) replace: `execute_save` on the router; each item's source is checked against
    /// the file as the plan read it, so a file changed since fails its item (`changed since the plan was made`) and
    /// stays as it is. A cancel leaves the files not reached `not_run` (exit 100).
    pub fn text_find_replace_execute(
        &self,
        plan: TextFindReplacePlan,
        job: &JobControl,
    ) -> Result<TextFindOutcome, ApiError> {
        run_guarded(|| Ok(self.replace_execute_in(None, plan, job)))
    }

    fn replace_execute_in(
        &self,
        vfs: Option<&dyn Vfs>,
        plan: TextFindReplacePlan,
        job: &JobControl,
    ) -> TextFindOutcome {
        if plan.dry_run {
            return self.text_find_replace_dry_run(plan);
        }
        let mut warnings = plan.warnings.clone();
        let Some(save) = plan.save.clone() else {
            return build(plan, Vec::new(), warnings, Vec::new(), true);
        };
        warnings.extend(save.warnings.iter().cloned());
        let plan_lines = save.human_lines();
        let router = self.router();
        let vfs: &dyn Vfs = vfs.unwrap_or(&*router);
        let docs: Vec<&EditableDocument> = plan.docs.iter().collect();
        let report = Self::save_execute_in(vfs, save.clone(), &docs, job);
        warnings.extend(report.op.warnings.iter().cloned());
        let items = save_plan_items(&save, Some(&report.op));
        build(plan, items, warnings, plan_lines, true)
    }
}

/// The outcome of a replace: the search's document with `replace`, one result and one plan item per file with a hit.
fn build(
    plan: TextFindReplacePlan,
    items: Vec<SavePlanItemDto>,
    warnings: Vec<Warning>,
    plan_lines: Vec<String>,
    executed: bool,
) -> TextFindOutcome {
    let dry_run = !executed;
    let mut results = Vec::with_capacity(plan.files.len());
    let mut plan_items = Vec::with_capacity(plan.files.len());
    let mut summary = FindReplaceSummaryDto::default();
    for file in &plan.files {
        let (status, skip, error) = match &file.pre {
            Pre::Failed(error) => {
                plan_items.push(synthetic_item(
                    file,
                    ItemStatus::Failed,
                    None,
                    Some(error.clone()),
                ));
                (ItemStatus::Failed, None, Some(error.clone()))
            }
            Pre::NoChange => {
                plan_items.push(synthetic_item(
                    file,
                    if dry_run {
                        ItemStatus::Planned
                    } else {
                        ItemStatus::Skipped
                    },
                    Some(SkipReason::NoChange),
                    None,
                ));
                (ItemStatus::Skipped, Some(SkipReason::NoChange), None)
            }
            Pre::Planned(i) => match items.get(*i) {
                Some(item) => {
                    plan_items.push(item.clone());
                    // A dry run's planned item keeps its skip (declined, duplicate) as skipped.
                    let status = match (item.status, item.reason) {
                        (ItemStatus::Planned, Some(_)) => ItemStatus::Skipped,
                        (status, _) => status,
                    };
                    (status, item.reason, item.error.clone())
                }
                None => (ItemStatus::NotRun, None, None),
            },
        };
        match status {
            ItemStatus::Done | ItemStatus::Planned => {
                summary.done += u64::from(executed);
                summary.replaced += file.replaced;
            }
            ItemStatus::Skipped => summary.skipped += 1,
            ItemStatus::Failed => summary.failed += 1,
            ItemStatus::NotRun => summary.not_run += 1,
        }
        results.push(FindReplaceResultDto {
            spec: file.spec.clone(),
            replaced: file.replaced,
            status,
            skip,
            error,
        });
    }
    let ok = summary.failed == 0 && summary.not_run == 0;
    let mut dto = plan.search;
    dto.replace = Some(FindReplaceDto {
        text: plan.text,
        dry_run,
        plan: plan_items,
        results,
        summary,
    });
    TextFindOutcome {
        dto,
        warnings,
        exit_code: if ok {
            ExitCode::Success
        } else {
            ExitCode::UnknownError
        },
        plan_lines,
    }
}

/// The plan item of a file that is not in the save plan (refused before planning, or `no_change`).
fn synthetic_item(
    file: &ReplaceFile,
    status: ItemStatus,
    reason: Option<SkipReason>,
    error: Option<ErrorReport>,
) -> SavePlanItemDto {
    SavePlanItemDto {
        action: if reason.is_some() { "skip" } else { "write" },
        kind: EntryKind::File,
        source: Some(file.spec.clone()),
        target: file.spec.clone(),
        size: None,
        replaces: false,
        backup: None,
        expect: None,
        status,
        reason,
        error,
        via_link: None,
        encoding: file.encoding,
        bom: file.bom,
        bytes: 0,
        spliced_bytes: 0,
        encoded_lines: 0,
    }
}
