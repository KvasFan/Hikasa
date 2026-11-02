//! Folder operations through the Service (§1.6.16): selection words become a [`Selection`], Disable editing is enforced
//! here, before the builder (D-184), `hikasa_folder::ops` plans, and `hikasa_vfs::ops` executes on the calling thread. The
//! CLI, W8's interpreter and W9's commands all reach these calls: nothing else plans (T11). The DTOs of
//! `hikasa.folder_op.v1` and `hikasa.folder_contents.v1`, the contents on demand and Open With are here too.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

use hikasa_core::{
    ContentKind, ContentMethod, ErrorReport, ExitCode, RelPath, Side, Spec, Warning,
};
use hikasa_folder::ops::{
    ConfirmMode, ConfirmPolicy, Confirmation, FolderOp, ItemMeta, OpPlan, OpScope, OpSettings,
    OpVerb, PathStructure, QuestionKind, ReadOnlySides, TouchFrom, plan_op,
};
use hikasa_folder::select::{
    ExcludeWhat, Include, SelectMask, SelectScope, Selection, SideSet, exclude_masks, resolve,
    select,
};
use hikasa_folder::{
    DisplayOptions, FolderDisplayFilter, FolderEnv, FolderError, FolderMode, FolderModel, NodeId,
    RefreshReport, ScanObserver, TreeRead, compare_contents, compute_crcs, refresh_into,
};
use hikasa_formats::EnabledOverrides;
use hikasa_report::escape_controls;
use hikasa_text::{TextRules, TextRulesPatch};
use hikasa_vfs::ops::{
    self, BackupPolicy, BackupStyle, ItemStatus, LinkPolicy, OpReport, OverwritePolicy, Preserve,
    SkipReason, Summary,
};
use hikasa_vfs::{EntryKind, FollowLinks, SpecUse, Vfs};
use serde::Serialize;

use super::{ApiError, Service};
use crate::jobs::{JobControl, run_guarded};
use crate::launcher::{OpenCommand, OpenCommandDto};
use crate::settings::{
    ExpandContext, ExpandItem, InstanceMode, LoadMode, OpenWithApp, Options, SettingsStore,
    WorkingFolder, expand_command,
};

/// `FolderOp` before resolution: the clock's now, a target as typed, a parent by its path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpRequest {
    /// A resolved operation.
    Op(FolderOp),
    /// `touch --now`.
    TouchNow {
        /// The sides.
        sides: SideSet,
    },
    /// `mkdir --in <REL>`.
    MkdirIn {
        /// The sides.
        sides: SideSet,
        /// The row it goes into, by its path (`""`: the base).
        parent: String,
        /// The name.
        name: String,
        /// A trailing separator was given.
        folder: bool,
    },
    /// `copyto`.
    CopyTo {
        /// The sides.
        sides: SideSet,
        /// The target as typed.
        target: OsString,
        /// The layout.
        structure: PathStructure,
        /// What an existing target means.
        overwrite: OverwritePolicy,
    },
    /// `moveto`.
    MoveTo {
        /// The sides.
        sides: SideSet,
        /// The target as typed.
        target: OsString,
        /// The layout.
        structure: PathStructure,
        /// What an existing target means.
        overwrite: OverwritePolicy,
    },
}

/// Which folders count as open for `select`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ExpandWords {
    /// Every folder (the default of a headless command).
    #[default]
    All,
    /// None: a collapsed folder is one row.
    None,
    /// These, by path.
    Paths(Vec<String>),
}

/// A selection in words.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SelectRequest {
    /// `--select`, parsed by `SelectMask::parse`.
    pub masks: Vec<String>,
    /// `--path`: rows by path; empty sides: the verb's.
    pub paths: Vec<(String, SideSet)>,
    /// A selection a view holds.
    pub selection: Option<Selection>,
    /// `--include-hidden`; `None`: `[file_operations] include_hidden_by_default`.
    pub include_hidden: Option<bool>,
    /// What is shown; `None`: everything.
    pub display: Option<DisplayOptions>,
    /// What is open.
    pub expand: ExpandWords,
    /// *Ignore unimportant differences*.
    pub iu: bool,
}

/// What a command changes of the settings of one plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OpOverrides {
    /// `--backup`, `--no-backup`; `None`: `[backups]`.
    pub backup: Option<bool>,
    /// `--no-preserve`, `--preserve-created`; `None`: `[file_operations]`.
    pub preserve: Option<Preserve>,
    /// `--symlinks`.
    pub links: Option<LinkPolicy>,
}

/// One folder operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderOpRequest {
    /// The operation.
    pub op: OpRequest,
    /// The selection.
    pub select: SelectRequest,
    /// The read-only sides.
    pub read_only: ReadOnlySides,
    /// The settings.
    pub overrides: OpOverrides,
}

/// What ran, or would run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderOpOutcome {
    /// The report.
    pub report: OpReport,
    /// One per item of the report.
    pub meta: Vec<ItemMeta>,
    /// The verb.
    pub verb: OpVerb,
    /// Nothing was written.
    pub dry_run: bool,
    /// The bases.
    pub bases: (Spec, Spec),
    /// Each outermost file with its backup.
    pub archive_backups: Vec<(Spec, Spec)>,
    /// The folders a refresh takes in.
    pub refresh_nodes: Vec<NodeId>,
    /// What planning and execution found.
    pub warnings: Vec<Warning>,
}

impl FolderOpOutcome {
    /// Every item done or skipped, not cancelled, and no row left out because its comparison failed (ARCH-101).
    pub fn ok(&self) -> bool {
        let compare_error = self
            .report
            .plan
            .items
            .iter()
            .any(|item| item.skip == Some(SkipReason::CompareError));
        self.report.ok() && !compare_error
    }

    /// 0, or 100 (§1.6.17).
    pub fn exit_code(&self) -> ExitCode {
        if self.ok() {
            ExitCode::Success
        } else {
            ExitCode::UnknownError
        }
    }

    /// `hikasa.folder_op.v1` (`result`).
    pub fn dto(&self) -> FolderOpDto {
        let items = self
            .report
            .plan
            .items
            .iter()
            .zip(&self.report.results)
            .zip(&self.meta)
            .map(|((item, result), meta)| FolderOpItemDto {
                action: item.action.as_str(),
                kind: item.kind,
                side: meta.side.map(Side::as_str),
                path: meta.path.clone(),
                source: item.source.as_ref().map(Spec::canonical),
                target: item.target.canonical(),
                size: item.size,
                replaces: item.replaces,
                backup: item.backup.as_ref().map(Spec::canonical),
                conflicts: meta.conflicts.names(),
                answer: meta.answer,
                status: result.status,
                reason: result.skip.or(item.skip),
                error: result.error.clone(),
            })
            .collect();
        FolderOpDto {
            op: self.verb.as_str(),
            dry_run: self.dry_run,
            cancelled: self.report.cancelled,
            left: self.bases.0.canonical(),
            right: self.bases.1.canonical(),
            items,
            backups: self
                .archive_backups
                .iter()
                .map(|(file, backup)| BackupDto {
                    file: file.canonical(),
                    backup: backup.canonical(),
                })
                .collect(),
            summary: self.report.summary,
            commands: None,
            warnings: self.warnings.clone(),
        }
    }
}

/// What a sync ran. W6c adds its rows and counts; the log model of W6b reads the operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderSyncOutcome {
    /// The operation.
    pub op: FolderOpOutcome,
}

/// One item of `hikasa.folder_op.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FolderOpItemDto {
    /// The action.
    pub action: &'static str,
    /// The item's kind.
    pub kind: EntryKind,
    /// `left`, `right`; `null` for a target outside the bases.
    pub side: Option<&'static str>,
    /// Its path below the bases.
    pub path: String,
    /// The source.
    pub source: Option<String>,
    /// The item written, removed or touched.
    pub target: String,
    /// Bytes.
    pub size: Option<u64>,
    /// It replaces a target.
    pub replaces: bool,
    /// Where the replaced target goes.
    pub backup: Option<String>,
    /// What it needs asked.
    pub conflicts: Vec<&'static str>,
    /// What was answered.
    pub answer: Option<bool>,
    /// `planned`, `done`, `skipped`, `failed`, `not_run`.
    pub status: ItemStatus,
    /// The skip reason.
    pub reason: Option<SkipReason>,
    /// The failure.
    pub error: Option<ErrorReport>,
}

/// A backup of an archive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BackupDto {
    /// The file.
    pub file: String,
    /// Its backup.
    pub backup: String,
}

/// `hikasa.folder_op.v1` (`result`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FolderOpDto {
    /// The verb.
    pub op: &'static str,
    /// Nothing was written.
    pub dry_run: bool,
    /// The operation was cancelled.
    pub cancelled: bool,
    /// The left base.
    pub left: String,
    /// The right base.
    pub right: String,
    /// The items in plan order.
    pub items: Vec<FolderOpItemDto>,
    /// The backups of archives.
    pub backups: Vec<BackupDto>,
    /// The counts.
    pub summary: Summary,
    /// What `open-with` runs.
    pub commands: Option<Vec<OpenCommandDto>>,
    /// The envelope's warnings; not in `result`.
    #[serde(skip)]
    pub warnings: Vec<Warning>,
}

/// What `folder op compare` and `crc` do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentsKind {
    /// A content test by this method.
    Compare(ContentMethod),
    /// CRC-32 of these sides.
    Crc(SideSet),
}

/// Contents on demand.
#[derive(Debug, Clone, PartialEq)]
pub struct ContentsRequest {
    /// The rows.
    pub select: SelectRequest,
    /// What runs.
    pub kind: ContentsKind,
    /// The text rules of the rules method; `None`: the defaults.
    pub text_rules: Option<TextRules>,
}

/// One row of `hikasa.folder_contents.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContentsItemDto {
    /// Its path.
    pub path: String,
    /// The side of a CRC.
    pub side: Option<&'static str>,
    /// The result of a content test.
    pub result: Option<ContentKind>,
    /// A CRC-32, in hexadecimal.
    pub crc32: Option<String>,
    /// A failure.
    pub error: Option<ErrorReport>,
}

/// `counts` of `hikasa.folder_contents.v1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
pub struct ContentsCountsDto {
    /// `binary_same` results.
    pub binary_same: u32,
    /// `rules_same` results.
    pub rules_same: u32,
    /// `unimportant_only` results.
    pub unimportant_only: u32,
    /// `important_differences` results.
    pub important_differences: u32,
    /// `binary_different` results.
    pub binary_different: u32,
    /// `error` results.
    pub error: u32,
}

/// `hikasa.folder_contents.v1` (`result`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContentsDto {
    /// `compare`, `crc`.
    pub op: &'static str,
    /// The method of a content test.
    pub method: Option<&'static str>,
    /// Cancelled: the rows not reached are not here.
    pub cancelled: bool,
    /// The rows.
    pub items: Vec<ContentsItemDto>,
    /// The results.
    pub counts: ContentsCountsDto,
}

impl ContentsDto {
    /// No row failed and nothing was cancelled.
    pub fn ok(&self) -> bool {
        !self.cancelled
            && self.counts.error == 0
            && self.items.iter().all(|item| item.error.is_none())
    }
}

/// What a refresh did. `None`: the comparison is flat, and the caller re-runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshDto {
    /// The report.
    pub report: Option<RefreshReport>,
    /// What it found.
    pub warnings: Vec<Warning>,
}

/// What `open-with` starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenTarget {
    /// An application by its place in `openwith.toml`.
    App(usize),
    /// An application by its description.
    Named(String),
    /// The associated application.
    Associated,
}

/// `open-with`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenWithRequest {
    /// What is started.
    pub target: OpenTarget,
    /// Each item with its base.
    pub items: Vec<(Spec, Spec)>,
    /// `%l`.
    pub line: Option<u64>,
}

/// `[file_operations]` as data.
pub fn confirm_policy(options: &Options) -> ConfirmPolicy {
    let o = &options.file_operations;
    ConfirmPolicy {
        copy: o.confirm_copy,
        move_: o.confirm_move,
        delete: o.confirm_delete,
        read_only: o.confirm_read_only,
        system: o.confirm_system,
        overwrite_newer: o.confirm_overwrite_newer,
        replace_during_move: o.confirm_replace_during_move,
    }
}

/// What `op` writes (D-184's table): each side refused when it is read-only.
fn written_sides(op: &OpRequest) -> SideSet {
    match op {
        OpRequest::Op(FolderOp::Copy { to, .. }) => SideSet::of(*to),
        OpRequest::Op(FolderOp::Move { .. } | FolderOp::Exchange) => SideSet::all(),
        OpRequest::Op(
            FolderOp::Delete { sides, .. }
            | FolderOp::Rename { sides, .. }
            | FolderOp::Mkdir { sides, .. }
            | FolderOp::MoveTo { sides, .. },
        )
        | OpRequest::TouchNow { sides }
        | OpRequest::MkdirIn { sides, .. }
        | OpRequest::MoveTo { sides, .. } => *sides,
        OpRequest::Op(FolderOp::Touch { sides, from }) => match from {
            TouchFrom::At(_) => *sides,
            TouchFrom::OtherSide { to } => SideSet::of(*to),
        },
        OpRequest::Op(FolderOp::CopyTo { .. }) | OpRequest::CopyTo { .. } => SideSet::empty(),
    }
}

/// The side `--path` means without a prefix: what the verb reads.
fn verb_sides(op: &OpRequest) -> SideSet {
    match op {
        OpRequest::Op(FolderOp::Copy { to, .. } | FolderOp::Move { to, .. }) => {
            SideSet::of(to.opposite())
        }
        OpRequest::Op(FolderOp::Exchange) => SideSet::all(),
        OpRequest::Op(
            FolderOp::Delete { sides, .. }
            | FolderOp::Rename { sides, .. }
            | FolderOp::Touch { sides, .. }
            | FolderOp::Mkdir { sides, .. }
            | FolderOp::CopyTo { sides, .. }
            | FolderOp::MoveTo { sides, .. },
        )
        | OpRequest::TouchNow { sides }
        | OpRequest::MkdirIn { sides, .. }
        | OpRequest::CopyTo { sides, .. }
        | OpRequest::MoveTo { sides, .. } => *sides,
    }
}

/// True for a verb that plans from a selection: `mkdir` names its parent.
fn needs_selection(op: &OpRequest) -> bool {
    !matches!(
        op,
        OpRequest::MkdirIn { .. } | OpRequest::Op(FolderOp::Mkdir { .. })
    )
}

/// The display a headless command sees: everything, folders as they compare.
fn show_everything(iu: bool) -> DisplayOptions {
    DisplayOptions {
        filter: FolderDisplayFilter::All,
        folders: FolderMode::CompareStructure,
        ignore_unimportant: iu,
        suppress_filters: false,
    }
}

/// A row by its path; `/` and, on Windows, `\`.
fn row_of(read: &TreeRead<'_>, path: &str, windows: bool) -> Result<NodeId, ApiError> {
    let text = if windows {
        path.replace('\\', "/")
    } else {
        path.to_owned()
    };
    let unknown = || ApiError::NotFound(format!("no row `{path}` in this comparison"));
    let rel = RelPath::parse(text.trim_end_matches('/')).map_err(|_| unknown())?;
    read.node_at(&rel).ok_or_else(unknown)
}

/// A file and a folder of one name are two rows (D-20): `row`, or its sibling of that name which has `sides`.
fn row_on(read: &TreeRead<'_>, row: NodeId, sides: SideSet) -> NodeId {
    let has = |id: NodeId| {
        read.node(id)
            .is_some_and(|view| sides.sides().any(|side| view.side(side).is_some()))
    };
    if has(row) {
        return row;
    }
    let path = read.path(row);
    let parent = read
        .node(row)
        .and_then(|view| view.parent)
        .unwrap_or(NodeId::ROOT);
    read.children(parent)
        .find(|&child| has(child) && read.path(child) == path)
        .unwrap_or(row)
}

impl Service {
    /// What a selection in words holds on `read`, with what it takes in.
    fn select_of(
        &self,
        read: &TreeRead<'_>,
        req: &SelectRequest,
        verb: SideSet,
    ) -> Result<(Selection, Include), ApiError> {
        let windows = self.env().platform == crate::host::Platform::Windows;
        let display = req.display.unwrap_or_else(|| show_everything(req.iu));
        let masks = req
            .masks
            .iter()
            .map(|word| SelectMask::parse(word))
            .collect::<Result<Vec<_>, FolderError>>()
            .map_err(ApiError::from_folder)?;
        let open: BTreeSet<NodeId> = match &req.expand {
            ExpandWords::Paths(paths) => paths
                .iter()
                .map(|path| row_of(read, path, windows))
                .collect::<Result<_, _>>()?,
            _ => BTreeSet::new(),
        };
        let closed = |id: NodeId| open.contains(&id);
        let expanded: Option<&dyn Fn(NodeId) -> bool> = match req.expand {
            ExpandWords::All => None,
            ExpandWords::None | ExpandWords::Paths(_) => Some(&closed),
        };
        let scope = SelectScope {
            display: &display,
            expanded,
        };
        let mut selection = req.selection.clone().unwrap_or_default();
        select(read, &scope, &masks, req.iu, &mut selection);
        for (path, sides) in &req.paths {
            let row = row_of(read, path, windows)?;
            let sides = if sides.is_empty() { verb } else { *sides };
            selection.insert(row_on(read, row, sides), sides);
        }
        let hidden = req.include_hidden.unwrap_or_else(|| {
            self.fresh_options()
                .file_operations
                .include_hidden_by_default
        });
        Ok((selection, Include { hidden, display }))
    }

    /// The backups of a plan: `[backups]`, unless the command says. A relative `[backups] folder` is `invalid_input`, an
    /// empty one with `folder` too.
    fn backup_policy(
        &self,
        options: &Options,
        wanted: Option<bool>,
    ) -> Result<Option<BackupPolicy>, ApiError> {
        if !wanted.unwrap_or(options.backups.back_up_before_copy) {
            return Ok(None);
        }
        let style = BackupStyle::from(options.backups.naming);
        if style != BackupStyle::Folder {
            return Ok(Some(BackupPolicy {
                style,
                folder: None,
            }));
        }
        let text = options.backups.folder.as_str();
        if text.is_empty() {
            return Err(ApiError::InvalidArgument(
                "[backups] folder is empty, and naming = \"folder\" needs it".to_owned(),
            ));
        }
        let path = std::path::Path::new(text);
        let absolute = path.is_absolute()
            || (self.env().platform == crate::host::Platform::Windows
                && text.as_bytes().get(1) == Some(&b':'));
        if !absolute {
            return Err(ApiError::InvalidArgument(format!(
                "[backups] folder `{text}` is not an absolute path"
            )));
        }
        let folder = self.parse_spec_as(path.as_os_str(), SpecUse::Folder)?;
        Ok(Some(BackupPolicy {
            style,
            folder: Some(folder),
        }))
    }

    /// The operation with its words resolved.
    fn resolved_op(&self, read: &TreeRead<'_>, op: &OpRequest) -> Result<FolderOp, ApiError> {
        let time = || {
            self.clock_file_time()
                .ok_or_else(|| ApiError::Internal("the clock's time is out of range".into()))
        };
        Ok(match op {
            OpRequest::Op(op) => op.clone(),
            OpRequest::TouchNow { sides } => FolderOp::Touch {
                sides: *sides,
                from: TouchFrom::At(time()?),
            },
            OpRequest::MkdirIn {
                sides,
                parent,
                name,
                folder,
            } => {
                let windows = self.env().platform == crate::host::Platform::Windows;
                let parent = if parent.is_empty() {
                    NodeId::ROOT
                } else {
                    row_of(read, parent, windows)?
                };
                FolderOp::Mkdir {
                    sides: *sides,
                    parent,
                    name: name.clone(),
                    folder: *folder,
                }
            }
            OpRequest::CopyTo {
                sides,
                target,
                structure,
                overwrite,
            } => FolderOp::CopyTo {
                sides: *sides,
                target: self.parse_spec_as(target, SpecUse::Folder)?,
                structure: *structure,
                overwrite: *overwrite,
            },
            OpRequest::MoveTo {
                sides,
                target,
                structure,
                overwrite,
            } => FolderOp::MoveTo {
                sides: *sides,
                target: self.parse_spec_as(target, SpecUse::Folder)?,
                structure: *structure,
                overwrite: *overwrite,
            },
        })
    }

    /// Plans a folder operation from the model: nothing is written (T12). Refuses a read-only side first, so scripts and
    /// commands cannot pass it by (D-184), and a selection with nothing in it: a stray command must not act.
    pub fn folder_op_plan(
        &self,
        model: &Arc<FolderModel>,
        req: &FolderOpRequest,
        job: &JobControl,
    ) -> Result<OpPlan, ApiError> {
        run_guarded(|| {
            let written = written_sides(&req.op);
            if let Some(side) = written.sides().find(|side| req.read_only.get(*side)) {
                return Err(ApiError::ReadOnlySide { side });
            }
            let options = self.fresh_options();
            let read = model.read();
            let (selection, include) = self.select_of(&read, &req.select, verb_sides(&req.op))?;
            if selection.is_empty() && needs_selection(&req.op) {
                return Err(ApiError::InvalidArgument(
                    "nothing selected: pass --select all, or a mask".to_owned(),
                ));
            }
            let op = self.resolved_op(&read, &req.op)?;
            let preserve = req.overrides.preserve.unwrap_or(Preserve {
                created: options.file_operations.preserve_created,
                ..Preserve::default()
            });
            let settings = OpSettings {
                preserve,
                links: req.overrides.links.unwrap_or_default(),
                backup: self.backup_policy(&options, req.overrides.backup)?,
            };
            let scope = OpScope {
                selection,
                include,
                iu: req.select.iu,
                read_only: req.read_only,
            };
            let router = self.router();
            plan_op(&read, &*router, &scope, &op, &settings, &job.ctl())
                .map_err(ApiError::from_folder)
        })
    }

    /// The question machine of `plan`: `mode`, else `Prompt` (`sync_confirmations` in a sync plan, W6c).
    pub fn folder_confirmation(&self, plan: &OpPlan, mode: Option<ConfirmMode>) -> Confirmation {
        let policy = confirm_policy(&self.fresh_options());
        Confirmation::new(plan, &policy, mode.unwrap_or_default())
    }

    /// [`Service::folder_confirmation`] with the operation question asked whatever `confirm_delete` says: a script's safety
    /// never depends on a settings file (D-77).
    pub fn folder_confirmation_strict(
        &self,
        plan: &OpPlan,
        mode: Option<ConfirmMode>,
    ) -> Confirmation {
        let policy = ConfirmPolicy {
            delete: true,
            ..confirm_policy(&self.fresh_options())
        };
        Confirmation::new(plan, &policy, mode.unwrap_or_default())
    }

    /// The plan as a dry run shows it: nothing is written, and its answers are not asked.
    pub fn folder_op_dry_run(&self, plan: OpPlan) -> FolderOpOutcome {
        let OpPlan {
            verb,
            bases,
            mut plan,
            meta,
            exec,
            warnings,
            refresh,
        } = plan;
        let router = self.router();
        ops::note_shadow_drops(&*router, &*router, &mut plan);
        let backups = archive_backups(&plan.items, &exec.archive_backups);
        let mut warnings = warnings;
        warnings.append(&mut plan.warnings);
        FolderOpOutcome {
            report: ops::dry_run(plan),
            meta,
            verb,
            dry_run: true,
            bases: (bases.left, bases.right),
            archive_backups: backups,
            refresh_nodes: refresh,
            warnings,
        }
    }

    /// Executes `plan` on the calling thread, one batch: `src` and `dst` are one router, so an archive on both sides is one
    /// rebuild. The items are `execute_with`'s alone: this adds nothing per file.
    pub fn folder_op_execute(
        &self,
        plan: OpPlan,
        job: &JobControl,
    ) -> Result<FolderOpOutcome, ApiError> {
        run_guarded(|| {
            let OpPlan {
                verb,
                bases,
                plan,
                mut meta,
                exec,
                warnings,
                refresh,
            } = plan;
            let router = self.router();
            let report = ops::execute_with(&*router, &*router, plan, &exec, &job.ctl());
            for &(index, count) in &report.expanded {
                let Some(stood) = meta.get(index).cloned() else {
                    continue;
                };
                meta.splice(index..=index, std::iter::repeat_n(stood, count));
            }
            let backups = archive_backups(&report.plan.items, &exec.archive_backups);
            let mut warnings = warnings;
            warnings.extend(report.warnings.iter().cloned());
            Ok(FolderOpOutcome {
                report,
                meta,
                verb,
                dry_run: false,
                bases: (bases.left, bases.right),
                archive_backups: backups,
                refresh_nodes: refresh,
                warnings,
            })
        })
    }

    /// Refreshes `nodes` in place (§1.6.7). A flat comparison has no folders to refresh: `report` is `None`, and the caller
    /// re-runs.
    pub fn folder_refresh(
        &self,
        model: &Arc<FolderModel>,
        nodes: &[NodeId],
        job: &JobControl,
        observer: &dyn ScanObserver,
    ) -> Result<RefreshDto, ApiError> {
        run_guarded(|| {
            let engines = self.engines_of(model, None)?;
            let now = self.clock().now();
            let router = self.router();
            let env = FolderEnv {
                vfs: &*router,
                content: &engines,
                pool: Some(self.pool()),
                ctl: job.ctl(),
                observer,
                now: &now,
                content_budget: self.options().content_budget_bytes(),
                archive_entries_max: hikasa_folder::SCAN_ARCHIVE_ENTRIES_MAX,
            };
            match refresh_into(model, nodes, &env) {
                Ok(report) => Ok(RefreshDto {
                    report: Some(report),
                    warnings: model.read().warnings().to_vec(),
                }),
                Err(FolderError::RefreshFlat) => Ok(RefreshDto {
                    report: None,
                    warnings: Vec::new(),
                }),
                Err(e) => Err(ApiError::from_folder(e)),
            }
        })
    }

    /// Opens the closed archive of `node` in place: its row becomes a folder (§1.6.7).
    pub fn folder_open_archive(
        &self,
        model: &Arc<FolderModel>,
        node: NodeId,
        job: &JobControl,
        observer: &dyn ScanObserver,
    ) -> Result<RefreshDto, ApiError> {
        let (path, parent) = {
            let read = model.read();
            let view = read
                .node(node)
                .ok_or_else(|| ApiError::NotFound("no such row".to_owned()))?;
            (read.path(node).as_str().to_owned(), view.parent)
        };
        model.open_archive(&path);
        self.folder_refresh(model, &[parent.unwrap_or(NodeId::ROOT)], job, observer)
    }

    /// The engines of a model that exists: its method's, with `text_rules` or the defaults for the rules method.
    fn engines_of(
        &self,
        model: &Arc<FolderModel>,
        text_rules: Option<&TextRules>,
    ) -> Result<hikasa_folder::ContentEngines, ApiError> {
        let mut warnings = Vec::new();
        let overrides: EnabledOverrides =
            self.enabled_overrides(&model.rules().format_overrides, &mut warnings);
        let defaults;
        let rules = match text_rules {
            Some(rules) => Some(rules),
            None => {
                defaults =
                    self.text_rules_from(None, None, &TextRulesPatch::default(), &mut warnings)?;
                Some(&defaults)
            }
        };
        self.content_engines(rules, &overrides)
    }

    /// FC-029(b), FC-040: a content test or CRCs on the rows of a selection (§1.6.8). Results enter the model in place.
    pub fn folder_contents(
        &self,
        model: &Arc<FolderModel>,
        req: &ContentsRequest,
        job: &JobControl,
    ) -> Result<ContentsDto, ApiError> {
        run_guarded(|| {
            let verb = match req.kind {
                ContentsKind::Compare(_) => SideSet::all(),
                ContentsKind::Crc(sides) => sides,
            };
            let (nodes, method_rows) = {
                let read = model.read();
                let (selection, include) = self.select_of(&read, &req.select, verb)?;
                if selection.is_empty() {
                    return Err(ApiError::InvalidArgument(
                        "nothing selected: pass --select all, or a mask".to_owned(),
                    ));
                }
                let rows = resolve(&read, &selection, &include);
                let files: Vec<(NodeId, SideSet)> = rows
                    .iter()
                    .filter(|row| read.node(row.node).is_some_and(|view| !view.is_folder()))
                    .map(|row| (row.node, row.sides))
                    .collect();
                (files.iter().map(|(id, _)| *id).collect::<Vec<_>>(), files)
            };
            let engines = self.engines_of(model, req.text_rules.as_ref())?;
            let now = self.clock().now();
            let router = self.router();
            let env = FolderEnv {
                vfs: &*router,
                content: &engines,
                pool: Some(self.pool()),
                ctl: job.ctl(),
                observer: &hikasa_folder::NullObserver,
                now: &now,
                content_budget: self.options().content_budget_bytes(),
                archive_entries_max: hikasa_folder::SCAN_ARCHIVE_ENTRIES_MAX,
            };
            match req.kind {
                ContentsKind::Compare(method) => {
                    let report = compare_contents(model, &nodes, method, &env)
                        .map_err(ApiError::from_folder)?;
                    let items = report
                        .items
                        .iter()
                        .map(|item| ContentsItemDto {
                            path: item.path.clone(),
                            side: None,
                            result: Some(item.kind),
                            crc32: None,
                            error: None,
                        })
                        .collect();
                    let c = report.counts;
                    Ok(ContentsDto {
                        op: "compare",
                        method: Some(method.as_str()),
                        cancelled: !report.complete,
                        items,
                        counts: ContentsCountsDto {
                            binary_same: c.binary_same,
                            rules_same: c.rules_same,
                            unimportant_only: c.unimportant_only,
                            important_differences: c.important_differences,
                            binary_different: c.binary_different,
                            error: c.error,
                        },
                    })
                }
                ContentsKind::Crc(sides) => {
                    let wanted: Vec<(NodeId, Side)> = method_rows
                        .iter()
                        .flat_map(|(id, on)| (*on & sides).sides().map(move |side| (*id, side)))
                        .collect();
                    let results =
                        compute_crcs(model, &wanted, &env).map_err(ApiError::from_folder)?;
                    let read = model.read();
                    let mut counts = ContentsCountsDto::default();
                    let items = results
                        .into_iter()
                        .map(|(id, side, result)| {
                            let (crc32, error) = match result {
                                Ok(crc) => (Some(format!("{crc:08x}")), None),
                                Err(error) => {
                                    counts.error += 1;
                                    (None, Some(error))
                                }
                            };
                            ContentsItemDto {
                                path: read.path(id).as_str().to_owned(),
                                side: Some(side.as_str()),
                                result: None,
                                crc32,
                                error,
                            }
                        })
                        .collect();
                    Ok(ContentsDto {
                        op: "crc",
                        method: None,
                        cancelled: job.cancel.is_cancelled(),
                        items,
                        counts,
                    })
                }
            }
        })
    }

    /// Sets or clears the Ignored mark on `nodes`: the new minimal list for the session (FC-033).
    pub fn folder_set_ignored(
        &self,
        model: &Arc<FolderModel>,
        nodes: &[NodeId],
        on: bool,
    ) -> Vec<String> {
        model.set_ignored(nodes, on)
    }

    /// The rows of a selection in words, each once (`ignored`).
    pub fn folder_selected_rows(
        &self,
        model: &Arc<FolderModel>,
        select: &SelectRequest,
    ) -> Result<Vec<NodeId>, ApiError> {
        run_guarded(|| {
            let read = model.read();
            let (selection, _) = self.select_of(&read, select, SideSet::all())?;
            Ok(selection.iter().map(|(node, _)| node).collect())
        })
    }

    /// FC-052: the masks Exclude adds for a selection in words: (`exclude_files`, `exclude_folders`).
    pub fn folder_exclude_masks(
        &self,
        model: &Arc<FolderModel>,
        select: &SelectRequest,
        what: ExcludeWhat,
    ) -> Result<(Vec<String>, Vec<String>), ApiError> {
        run_guarded(|| {
            let read = model.read();
            let (selection, _) = self.select_of(&read, select, SideSet::all())?;
            if selection.is_empty() {
                return Err(ApiError::InvalidArgument(
                    "nothing selected: pass --select all, or a mask".to_owned(),
                ));
            }
            Ok(exclude_masks(&read, &selection, what))
        })
    }

    /// The items of `open-with`: each row on `sides`, left first, with its side's base. An unknown row is `not_found`.
    pub fn folder_open_items(
        &self,
        model: &Arc<FolderModel>,
        paths: &[(String, SideSet)],
        sides: SideSet,
    ) -> Result<Vec<(Spec, Spec)>, ApiError> {
        run_guarded(|| {
            let windows = self.env().platform == crate::host::Platform::Windows;
            let read = model.read();
            let mut items = Vec::new();
            for (path, own) in paths {
                let row = row_of(&read, path, windows)?;
                let on = if own.is_empty() { sides } else { *own };
                for side in on.sides() {
                    if let (Some(spec), Some(base)) =
                        (read.spec(row, side), read.spec(NodeId::ROOT, side))
                    {
                        items.push((spec, base));
                    }
                }
            }
            Ok(items)
        })
    }

    /// True when the application of `target` waits for the one before it.
    pub fn open_with_waits(&self, target: &OpenTarget) -> Result<bool, ApiError> {
        Ok(self.open_with_app(target)?.wait_for_previous)
    }

    /// The commands of Open With (§1.6.18): one per item, or per pair with `%…2` variables. A member is extracted, and its
    /// `ExpandItem` is that file with its folder as base.
    pub fn open_with_commands(
        &self,
        req: &OpenWithRequest,
        job: &JobControl,
    ) -> Result<Vec<OpenCommand>, ApiError> {
        run_guarded(|| {
            let app = self.open_with_app(&req.target)?;
            let router = self.router();
            let mut items = Vec::with_capacity(req.items.len());
            for (spec, base) in &req.items {
                let entry = router
                    .stat(spec, FollowLinks::Yes)
                    .map_err(ApiError::from_vfs)?;
                let folder = entry.kind == EntryKind::Dir;
                if (folder && !app.accepts_folders) || (!folder && !app.accepts_files) {
                    let what = if folder { "folders" } else { "files" };
                    return Err(ApiError::InvalidArgument(format!(
                        "`{}` does not take {what}",
                        app.description
                    )));
                }
                items.push(self.expand_item(spec, base, job)?);
            }
            let per_pair = app.instance_mode() == InstanceMode::PerPair;
            let size = if per_pair { 2 } else { 1 };
            items
                .chunks(size)
                .map(|chunk| command_of(&app, chunk, req.line))
                .collect()
        })
    }

    /// The application `target` names.
    fn open_with_app(&self, target: &OpenTarget) -> Result<OpenWithApp, ApiError> {
        let store = SettingsStore::new(self.config());
        let apps = store
            .openwith(LoadMode::Lenient)
            .map_err(ApiError::Session)?
            .value;
        let found = match target {
            OpenTarget::App(index) => apps.app.get(*index),
            OpenTarget::Named(name) => apps.app.iter().find(|app| app.description == *name),
            OpenTarget::Associated => None,
        };
        found
            .cloned()
            .ok_or_else(|| ApiError::NotFound("no such application in openwith.toml".to_owned()))
    }

    /// A real path of `spec`: its own, or a member's copy in the cache.
    fn real_path(&self, spec: &Spec, job: &JobControl) -> Result<PathBuf, ApiError> {
        match spec {
            Spec::Local(path) => Ok(path.as_path().to_path_buf()),
            Spec::ArchiveMember { .. } => {
                let router = self.router();
                let archives = router
                    .archives()
                    .ok_or_else(|| ApiError::Unsupported("archives are off".to_owned()))?;
                archives
                    .extract_to_cache(spec, &job.cancel)
                    .map_err(ApiError::from_vfs)
            }
            other => Err(ApiError::Unsupported(format!(
                "{}: not a file a program can open",
                other.canonical()
            ))),
        }
    }

    /// One item of an expansion.
    fn expand_item(
        &self,
        spec: &Spec,
        base: &Spec,
        job: &JobControl,
    ) -> Result<ExpandItem, ApiError> {
        let path = self.real_path(spec, job)?;
        let base = match (spec, base) {
            (Spec::Local(_), Spec::Local(base)) => base.as_path().to_path_buf(),
            _ => path.parent().map(PathBuf::from).unwrap_or_default(),
        };
        Ok(ExpandItem { path, base })
    }

    /// Runs the commands: one after another with `wait_for_previous`, else all at once, each on a thread of its own. A
    /// program that cannot start is the error; a program's own exit code is not Hikasa's.
    pub fn open_with_run(
        &self,
        commands: &[OpenCommand],
        wait_for_previous: bool,
        job: &JobControl,
    ) -> Result<(), ApiError> {
        run_guarded(|| {
            let launcher = self.launcher();
            if wait_for_previous {
                for command in commands {
                    job.cancel.check().map_err(|_| ApiError::Cancelled)?;
                    launcher.run(command)?;
                }
                return Ok(());
            }
            std::thread::scope(|scope| {
                let runs: Vec<_> = commands
                    .iter()
                    .map(|command| scope.spawn(move || launcher.run(command)))
                    .collect();
                let mut first = Ok(());
                for run in runs {
                    let result = run
                        .join()
                        .unwrap_or_else(|_| Err(ApiError::Internal("a launcher panicked".into())));
                    if let (Ok(()), Err(e)) = (&first, result) {
                        first = Err(e);
                    }
                }
                first
            })
        })
    }

    /// FC-062 *Associated Application* of `spec`.
    pub fn open_associated(&self, spec: &Spec, job: &JobControl) -> Result<(), ApiError> {
        run_guarded(|| {
            let path = self.real_path(spec, job)?;
            self.launcher().open_associated(&path)
        })
    }
}

/// The command of one item, or one pair.
fn command_of(
    app: &OpenWithApp,
    items: &[ExpandItem],
    line: Option<u64>,
) -> Result<OpenCommand, ApiError> {
    let (first, second) = match items {
        [first] => (first, None),
        [first, second] => (first, Some(second)),
        _ => return Err(ApiError::InvalidArgument("nothing to open".to_owned())),
    };
    let working_folder = match &app.working_folder {
        WorkingFolder::Parent => first.path.parent().map(PathBuf::from).unwrap_or_default(),
        WorkingFolder::Base => first.base.clone(),
        WorkingFolder::Path(path) => PathBuf::from(path),
    };
    let context = ExpandContext {
        first: first.clone(),
        second: second.cloned(),
        line,
        working_folder: working_folder.clone(),
        path_delimiter: app.delimiter(),
    };
    let words = expand_command(&app.command, &context)
        .map_err(|e| ApiError::InvalidArgument(format!("`{}`: {e}", app.description)))?;
    let mut words = words.into_iter();
    let program = words.next().ok_or_else(|| {
        ApiError::InvalidArgument(format!("`{}` has no command", app.description))
    })?;
    Ok(OpenCommand {
        program: OsString::from(program),
        args: words.map(OsString::from).collect(),
        cwd: working_folder,
    })
}

/// Each outermost file of `items` with its backup.
fn archive_backups(
    items: &[hikasa_vfs::ops::PlanItem],
    backups: &BTreeMap<String, Spec>,
) -> Vec<(Spec, Spec)> {
    if backups.is_empty() {
        return Vec::new();
    }
    let mut found = BTreeMap::new();
    for item in items {
        let outers = [
            item.target.outermost_file(),
            item.source.as_ref().and_then(Spec::outermost_file),
        ];
        for outer in outers.into_iter().flatten() {
            if let Some(backup) = backups.get(&outer.canonical()) {
                found.insert(outer.canonical(), (outer.clone(), backup.clone()));
            }
        }
    }
    found.into_values().collect()
}

/// The `op` of `open-with` in `hikasa.folder_op.v1`.
pub const fn op_word() -> &'static str {
    "open_with"
}

/// The kinds of open questions with their counts: `read_only 2, system 1`.
pub fn confirm_words(open: &BTreeMap<QuestionKind, u64>) -> String {
    open.iter()
        .map(|(kind, count)| format!("{} {count}", kind.as_str()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The word of a verb in a summary (§1.6.17).
pub fn verb_done(verb: OpVerb) -> &'static str {
    match verb {
        OpVerb::Copy | OpVerb::CopyTo => "copied",
        OpVerb::Move | OpVerb::MoveTo => "moved",
        OpVerb::Exchange => "exchanged",
        OpVerb::Delete => "deleted",
        OpVerb::Rename => "renamed",
        OpVerb::Touch => "touched",
        OpVerb::Mkdir => "created",
        OpVerb::Sync => "synchronized",
    }
}

/// The word of a verb in a dry run: `would copy …`.
pub fn verb_base(verb: OpVerb) -> &'static str {
    match verb {
        OpVerb::Copy | OpVerb::CopyTo => "copy",
        OpVerb::Move | OpVerb::MoveTo => "move",
        OpVerb::Exchange => "exchange",
        OpVerb::Delete => "delete",
        OpVerb::Rename => "rename",
        OpVerb::Touch => "touch",
        OpVerb::Mkdir => "create",
        OpVerb::Sync => "synchronize",
    }
}

/// The summary of an operation: the one text of the human output and of the log (T11).
pub fn summary_text(
    verb: OpVerb,
    dry_run: bool,
    summary: &Summary,
    seconds: Option<f64>,
) -> String {
    if dry_run {
        return format!("would {} {} items", verb_base(verb), summary.planned);
    }
    let mut text = format!(
        "{} {} of {} items ({} skipped, {} failed), {} bytes",
        verb_done(verb),
        summary.done,
        summary.planned,
        summary.skipped,
        summary.failed,
        summary.bytes
    );
    if let Some(seconds) = seconds {
        text.push_str(&format!(", {seconds:.1} s"));
    }
    text
}

/// The word of an item (§1.3.7's, and `exchange`).
pub fn item_word(item: &FolderOpItemDto) -> &'static str {
    match item.action {
        "create_dir" => "mkdir",
        "keep_dir" => "exists",
        "copy" if item.replaces => "replace",
        "remove_dir" => "rmdir",
        "create_archive" => "mkarchive",
        other => other,
    }
}

/// The human line of an item: the side's letter, the words of §1.3.7, ` (backup <spec>)`, a dry run's conflicts.
pub fn item_line(item: &FolderOpItemDto, dry_run: bool) -> String {
    let side = match item.side {
        Some("left") => "L ",
        Some("right") => "R ",
        _ if item.action == "exchange" => "",
        _ => "- ",
    };
    let word = item_word(item);
    let target = escape_controls(&item.target);
    let mut line = match item.status {
        ItemStatus::Failed => {
            let message = item.error.as_ref().map_or("", |e| e.message.as_str());
            format!("{side}failed {word} {target}: {}", escape_controls(message))
        }
        ItemStatus::NotRun => format!("{side}not run {word} {target}"),
        _ => match item.reason {
            Some(reason) => format!("{side}skip ({}) {target}", reason.as_str()),
            None => match (&item.source, item.action) {
                (Some(source), "exchange") => {
                    format!("{side}exchange {} <-> {target}", escape_controls(source))
                }
                (Some(source), "copy" | "link" | "rename" | "move") => {
                    format!("{side}{word} {} -> {target}", escape_controls(source))
                }
                _ => format!("{side}{word} {target}"),
            },
        },
    };
    if let Some(backup) = &item.backup {
        line.push_str(&format!(" (backup {})", escape_controls(backup)));
    }
    if dry_run && !item.conflicts.is_empty() {
        line.push_str(&format!(" [{}]", item.conflicts.join(",")));
    }
    line
}
