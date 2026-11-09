//! Saving edited documents through the Service (M2 W10b; contract §1.10.13, §1.10.14, §1.10.23; D-273…D-277).
//!
//! ```text
//! text_save_plan([(side, &doc, &req)], job)       every side checked first (ensure_editable: EditBlocked, exit 100),
//!                                                 then hikasa_text::save::plan_save_with on the router (a job,
//!                                                 JobKind::TextSave; External save programs through the service's
//!                                                 runner) → SavePlan
//! save_confirmation(&plan, SaveAnswers)           W6's machine (SaveConfirmation) with [file_operations]'s policy and
//!                                                 the adapter's flags answering their classes → SaveDriver (an `Ask`)
//! text_save_execute(plan, &[&doc], job)           execute_save streamed from SaveSource → SaveReport
//! apply_save_report(&report, &mut [&mut doc])     EditableDocument::saved_to per done item (marks, stamp, location)
//! ```
//!
//! The backups of a save are `[backups] back_up_before_save` with `naming` and `folder`
//! ([`Service::backup_policy_for_save`]); `--nobackups` (a view's [`super::LiveOptions::no_backups`]) and
//! `--no-backup` turn them off, `--backup` on. Exit codes (§1.10.24): 0 when every item was done (or skipped), 100
//! otherwise ([`save_exit_code`]); a refused request (a blocked side, open questions without a person) is an error.

use std::collections::BTreeMap;

use hikasa_core::confirm::{Ask, ConfirmMode, Declined, Reply};
use hikasa_core::{ErrorReport, ExitCode, Side, Spec};
use hikasa_text::save::{
    SaveAsk, SaveConfirmation, SaveEnv, SaveOptions, SavePlan, SavePolicy, SaveQuestion,
    SaveReport, SaveRequest, SaveSource, UnmappablePolicy, execute_save, plan_save_with,
};
use hikasa_text::{EditableDocument, TextError};
use hikasa_vfs::ops::{BackupPolicy, ItemStamp, ItemStatus, OpReport, SkipReason};
use hikasa_vfs::{EntryKind, Vfs};
use serde::Serialize;

use super::text_live::LiveTextView;
use super::{ApiError, Service, ensure_editable};
use crate::jobs::{JobControl, run_guarded};
use crate::settings::options::Options;

/// The questions `[file_operations]` asks of a save (§1.10.14, W6's keys): `confirm_read_only` for
/// `ReadOnlyTarget`, `confirm_overwrite_newer` for `OverwriteBackup` (as W6 asks a backup that replaces a file).
pub fn save_policy(options: &Options) -> SavePolicy {
    let o = &options.file_operations;
    SavePolicy {
        read_only: o.confirm_read_only,
        overwrite_backup: o.confirm_overwrite_newer,
    }
}

/// What the adapter's flags answer (§1.10.14's CLI column): `--force` answers `ChangedOnDisk`, `--unmappable
/// replace` `Unmappable`, `--yes` (or `--confirm yes-to-all`) every other class; `--confirm no-to-all` declines
/// every item that asks. A class no flag answers is asked of a person, or stays open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SaveAnswers {
    /// `--yes`: `ReadOnlyTarget`, `LossyRewritten`, `ReplaceExisting`, `OverwriteBackup`.
    pub yes: bool,
    /// `--force`: `ChangedOnDisk` (the file changed on disk since it was loaded).
    pub force: bool,
    /// `--unmappable replace`: `Unmappable` (characters written as `?`).
    pub unmappable_replace: bool,
    /// `--confirm no-to-all`: every item that asks is declined.
    pub no_to_all: bool,
}

impl SaveAnswers {
    /// The answers of a W6 mode: `YesToAll` is `--yes`, `NoToAll` declines; `Prompt` answers nothing.
    pub fn of_mode(mode: ConfirmMode) -> SaveAnswers {
        SaveAnswers {
            yes: mode == ConfirmMode::YesToAll,
            no_to_all: mode == ConfirmMode::NoToAll,
            ..SaveAnswers::default()
        }
    }

    /// True when a flag answers `kind` yes.
    pub fn answers(&self, kind: SaveQuestion) -> bool {
        match kind {
            SaveQuestion::ChangedOnDisk => self.force,
            SaveQuestion::Unmappable => self.unmappable_replace,
            SaveQuestion::ReadOnlyTarget
            | SaveQuestion::LossyRewritten
            | SaveQuestion::ReplaceExisting
            | SaveQuestion::OverwriteBackup => self.yes,
        }
    }

    /// The flag that answers `kind` (a refusal's message names it).
    pub fn flag_of(kind: SaveQuestion) -> &'static str {
        match kind {
            SaveQuestion::ChangedOnDisk => "--force",
            SaveQuestion::Unmappable => "--unmappable replace",
            SaveQuestion::ReadOnlyTarget
            | SaveQuestion::LossyRewritten
            | SaveQuestion::ReplaceExisting
            | SaveQuestion::OverwriteBackup => "--yes",
        }
    }
}

/// The question machine of a save as an adapter drives it (§1.10.14): [`SaveConfirmation`] (W6's shape) whose
/// classes the [`SaveAnswers`] answer by themselves, so only the others reach a person (the CLI's `confirm.rs`, W11's
/// prompts through [`Ask`]).
#[derive(Debug, Clone)]
pub struct SaveDriver {
    machine: SaveConfirmation,
    answers: SaveAnswers,
}

impl SaveDriver {
    /// The driver of `plan` under `policy` with the adapter's `answers`.
    pub fn new(plan: &SavePlan, policy: &SavePolicy, answers: SaveAnswers) -> SaveDriver {
        let mode = if answers.no_to_all {
            ConfirmMode::NoToAll
        } else {
            ConfirmMode::Prompt
        };
        let mut driver = SaveDriver {
            machine: SaveConfirmation::new(plan, policy, mode),
            answers,
        };
        driver.settle();
        driver
    }

    /// Answers every question at the head whose class a flag answers.
    fn settle(&mut self) {
        while let Some(kind) = self.machine.next().map(|q| q.kind) {
            if !self.answers.answers(kind) {
                return;
            }
            self.machine.reply(Reply::Yes);
        }
    }

    /// The question a person must answer; `None`: nothing left.
    pub fn next(&self) -> Option<&SaveAsk> {
        self.machine.next()
    }

    /// The classes of the questions a person still has to answer, with their counts.
    pub fn open_questions(&self) -> BTreeMap<SaveQuestion, u64> {
        let mut open = self.machine.open_questions();
        open.retain(|kind, _| !self.answers.answers(*kind));
        open
    }

    /// Answers the current question.
    pub fn reply(&mut self, reply: Reply) {
        self.machine.reply(reply);
        self.settle();
    }

    /// The plan with its answers ([`SaveConfirmation::finish`]): items still asking are declined; `Err` after a cancel.
    pub fn finish(self, plan: SavePlan) -> Result<SavePlan, Declined> {
        self.machine.finish(plan)
    }

    /// The refusal of a plan whose questions no person can answer (W6 §1.6.11, no terminal): `InvalidArgument` with the
    /// counts and the flags that answer them; `None` when nothing is open.
    pub fn refusal(&self) -> Option<ApiError> {
        let open = self.open_questions();
        if open.is_empty() {
            return None;
        }
        let total: u64 = open.values().sum();
        let mut flags: Vec<&str> = open.keys().map(|k| SaveAnswers::flag_of(*k)).collect();
        flags.sort_unstable();
        flags.dedup();
        Some(ApiError::InvalidArgument(format!(
            "{total} questions need an answer: pass {}, or --dry-run to see the plan ({})",
            flags.join(" or "),
            save_question_words(&open)
        )))
    }
}

impl Ask for SaveDriver {
    fn question(&self) -> Option<&str> {
        self.machine.next().map(|q| q.text.as_str())
    }

    fn answer(&mut self, reply: Reply) {
        self.reply(reply);
    }

    fn remaining(&self) -> u64 {
        self.machine.next().map_or(0, |q| q.remaining)
    }
}

/// `changed_on_disk 1, read_only_target 2`: the open questions in words.
pub fn save_question_words(open: &BTreeMap<SaveQuestion, u64>) -> String {
    open.iter()
        .map(|(kind, n)| format!("{} {n}", kind.as_str()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Applies a save's results to its documents (§1.10.13): `docs` are the plan's documents in the order of the plan's
/// items; each done item makes its file the next splice source of its document
/// ([`EditableDocument::saved_to`]: marks become `SavedEdit`, the stamp, the location after a Save As, the backup of
/// the one-generation rule). A failed or skipped item leaves its document modified. Returns the documents saved.
/// Pure.
pub fn apply_save_report(report: &SaveReport, docs: &mut [&mut EditableDocument]) -> usize {
    let mut saved = 0;
    for (doc, file) in docs.iter_mut().zip(&report.saved) {
        if let Some(file) = file {
            doc.saved_to(file);
            saved += 1;
        }
    }
    saved
}

/// The exit code of an executed save (§1.10.24): 0 when every item was done or skipped (declined, `no_change`,
/// a duplicate target) and nothing was cancelled, else 100.
pub fn save_exit_code(report: &SaveReport) -> ExitCode {
    let ok = !report.op.cancelled
        && report
            .op
            .results
            .iter()
            .all(|r| matches!(r.status, ItemStatus::Done | ItemStatus::Skipped));
    if ok {
        ExitCode::Success
    } else {
        ExitCode::UnknownError
    }
}

/// `Cancelled` for a cancelled save job, else `Text`.
pub(crate) fn save_error(error: TextError) -> ApiError {
    if error.is_cancelled() {
        ApiError::Cancelled
    } else {
        ApiError::Text(error)
    }
}

impl Service {
    /// The backup policy of a save (OPT-008, D-277; §1.10.23): `[backups] back_up_before_save` with `naming` and
    /// `folder`, or none with `no_backups` (`--nobackups`, `--no-backup`). A relative or empty `[backups] folder` with
    /// `naming = "folder"` is `InvalidArgument`, as W6's copies refuse it (the contract's `Option` became a `Result`).
    pub fn backup_policy_for_save(
        &self,
        no_backups: bool,
    ) -> Result<Option<BackupPolicy>, ApiError> {
        self.backup_policy_for_save_with(no_backups.then_some(false))
    }

    /// [`Service::backup_policy_for_save`] with a three-way switch: `Some(true)` (`--backup`) on, `Some(false)`
    /// (`--no-backup`, `--nobackups`) off, `None` as `back_up_before_save` says. Additive.
    pub fn backup_policy_for_save_with(
        &self,
        wanted: Option<bool>,
    ) -> Result<Option<BackupPolicy>, ApiError> {
        let options = self.fresh_options();
        let on = wanted.unwrap_or(options.backups.back_up_before_save);
        self.backup_policy_on(&options, on)
    }

    /// The question machine of a save plan with `[file_operations]`'s policy ([`save_policy`]) and the adapter's
    /// `answers` (§1.10.14). Pure but for the settings read.
    pub fn save_confirmation(&self, plan: &SavePlan, answers: SaveAnswers) -> SaveDriver {
        SaveDriver::new(plan, &save_policy(&self.fresh_options()), answers)
    }

    /// The request of a save of a live view's `side` (§1.10.14): in place (or to the view's `--savetarget`, which
    /// the right pane's document carries), backups unless the view was opened with `--nobackups`, the unmappable
    /// characters asked, and the format's External save program (FF-030) when it has one.
    pub fn live_save_request(
        &self,
        view: &LiveTextView,
        side: Side,
    ) -> Result<SaveRequest, ApiError> {
        let format = view.live.current().format_of(side);
        Ok(SaveRequest {
            target: None,
            backup: self.backup_policy_for_save(view.options.no_backups)?,
            unmappable: UnmappablePolicy::Ask,
            conversion: SaveRequest::conversion_of(format),
        })
    }

    /// Plans the save of `docs` as one plan (§1.10.13, §1.10.23; run as a job, `JobKind::TextSave`): every side is
    /// checked first and a blocked one refuses the whole call with [`ApiError::EditBlocked`] (kind
    /// `permission_denied`, exit 100) before anything is planned (§1.10.7); then `plan_save_with` on the router, with
    /// the service's temporary folder and runner for an External save program. Nothing is written. Item failures are
    /// in the plan; the call fails when it is cancelled.
    pub fn text_save_plan(
        &self,
        docs: &[(Option<Side>, &EditableDocument, &SaveRequest)],
        job: &JobControl,
    ) -> Result<SavePlan, ApiError> {
        run_guarded(|| {
            let router = self.router();
            self.text_save_plan_in(&*router, docs, job)
        })
    }

    /// [`Service::text_save_plan`] on the back-end `vfs` (a test's `MemoryVfs`). Additive.
    pub fn text_save_plan_on(
        &self,
        vfs: &dyn Vfs,
        docs: &[(Option<Side>, &EditableDocument, &SaveRequest)],
        job: &JobControl,
    ) -> Result<SavePlan, ApiError> {
        run_guarded(|| self.text_save_plan_in(vfs, docs, job))
    }

    fn text_save_plan_in(
        &self,
        vfs: &dyn Vfs,
        docs: &[(Option<Side>, &EditableDocument, &SaveRequest)],
        job: &JobControl,
    ) -> Result<SavePlan, ApiError> {
        for (side, doc, _) in docs {
            ensure_editable(*side, &doc.editability())?;
        }
        self.plan_save_in(vfs, docs, job)
    }

    /// `plan_save_with` with the service's External runner, no editability check (`text convert` decides its own
    /// refusals per file).
    pub(crate) fn plan_save_in(
        &self,
        vfs: &dyn Vfs,
        docs: &[(Option<Side>, &EditableDocument, &SaveRequest)],
        job: &JobControl,
    ) -> Result<SavePlan, ApiError> {
        let env = SaveEnv {
            external: Some(self.external()),
        };
        plan_save_with(vfs, docs, &env, &job.ctl()).map_err(save_error)
    }

    /// Executes a confirmed save plan (§1.10.13): every item's content streamed from its splice source and edited
    /// lines ([`SaveSource`]) and written by `hikasa_vfs::ops` (`Action::Write`: backups, archive rebuilds, the
    /// target's stamp checked). `docs` are the plan's documents in its items' order (a document changed since the
    /// plan refuses its item). The caller then applies the report ([`apply_save_report`]). Items that fail are in the
    /// report ([`save_exit_code`]: 100); the call itself fails only on a panic.
    pub fn text_save_execute(
        &self,
        plan: SavePlan,
        docs: &[&EditableDocument],
        job: &JobControl,
    ) -> Result<SaveReport, ApiError> {
        run_guarded(|| {
            let router = self.router();
            Ok(Self::save_execute_in(&*router, plan, docs, job))
        })
    }

    /// [`Service::text_save_execute`] on the back-end `vfs`. Additive.
    pub fn text_save_execute_on(
        &self,
        vfs: &dyn Vfs,
        plan: SavePlan,
        docs: &[&EditableDocument],
        job: &JobControl,
    ) -> Result<SaveReport, ApiError> {
        run_guarded(|| Ok(Self::save_execute_in(vfs, plan, docs, job)))
    }

    pub(crate) fn save_execute_in(
        vfs: &dyn Vfs,
        plan: SavePlan,
        docs: &[&EditableDocument],
        job: &JobControl,
    ) -> SaveReport {
        let source = SaveSource::new(vfs, &plan, docs);
        execute_save(vfs, plan, source, &job.ctl())
    }
}

impl LiveTextView {
    /// The format's *when saving* options of `side` (docs/10 §1): the adapter applies them
    /// ([`hikasa_text::save::apply_save_options`]) before it plans a save.
    pub fn save_options(&self, side: Side) -> SaveOptions {
        SaveOptions::of(self.live.current().format_of(side))
    }
}

/// One item of a save plan in JSON (`PlanItemJson` of §1.10.24: `hikasa.text_convert.v1`'s and
/// `hikasa.text_find.v1`'s `plan`): W3's item (`action: "write"`, `kind`, `source`, `target`, `size`, `replaces`,
/// `status`, `reason`, `error`) with the backup and the target's stamp the plan saw (`expect`), plus what the save
/// writes: `via_link`, `encoding`, `bom`, `bytes`, `spliced_bytes`, `encoded_lines`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SavePlanItemDto {
    /// `write`, or `skip` (a declined item, a duplicate target, `no_change`).
    pub action: &'static str,
    /// The item's kind (`file`).
    pub kind: EntryKind,
    /// The file read (the target itself for a write: its splice source is streamed).
    pub source: Option<String>,
    /// The file written (the file a link leads to).
    pub target: String,
    /// Bytes written.
    pub size: Option<u64>,
    /// The target exists and is replaced.
    pub replaces: bool,
    /// Where the replaced file goes (a member's: its archive's backup).
    pub backup: Option<String>,
    /// The target as the plan saw it: a change before the write fails the item (`null`: absent).
    pub expect: Option<ItemStamp>,
    /// `planned`, `done`, `skipped`, `failed`, `not_run`.
    pub status: ItemStatus,
    /// The skip reason.
    pub reason: Option<SkipReason>,
    /// The failure.
    pub error: Option<ErrorReport>,
    /// The link the save was asked through (it stays a link).
    pub via_link: Option<String>,
    /// The encoding written (`null` for a file refused before its content was made).
    pub encoding: Option<&'static str>,
    /// A BOM is written.
    pub bom: bool,
    /// Bytes written (as `size`).
    pub bytes: u64,
    /// Bytes spliced from the source unchanged.
    pub spliced_bytes: u64,
    /// Physical lines encoded.
    pub encoded_lines: u32,
}

/// The JSON items of `plan` (§1.10.24): statuses from `report` (an executed plan) or, without one, as a dry run shows
/// them (`planned`, or the planning failure and skip).
pub fn save_plan_items(plan: &SavePlan, report: Option<&OpReport>) -> Vec<SavePlanItemDto> {
    let dry;
    let report = match report {
        Some(report) => report,
        None => {
            dry = hikasa_vfs::ops::dry_run(plan.plan.clone());
            &dry
        }
    };
    plan.items
        .iter()
        .zip(&report.plan.items)
        .zip(&report.results)
        .map(|((item, planned), result)| {
            let backup = planned.backup.clone().or_else(|| {
                planned
                    .target
                    .outermost_file()
                    .and_then(|outer| plan.exec.archive_backups.get(&outer.canonical()))
                    .cloned()
            });
            SavePlanItemDto {
                action: planned.action.as_str(),
                kind: planned.kind,
                source: planned.source.as_ref().map(Spec::canonical),
                target: planned.target.canonical(),
                size: planned.size,
                replaces: planned.replaces,
                backup: backup.as_ref().map(Spec::canonical),
                expect: planned.expect.and_then(|e| e.target),
                status: result.status,
                reason: result.skip.or(planned.skip),
                error: result.error.clone(),
                via_link: item.via_link.as_ref().map(Spec::canonical),
                encoding: Some(item.content.encoding.name()),
                bom: item.content.bom,
                bytes: item.content.bytes,
                spliced_bytes: item.content.spliced_bytes,
                encoded_lines: item.content.encoded_lines,
            }
        })
        .collect()
}
