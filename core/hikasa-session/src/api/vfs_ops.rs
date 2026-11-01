//! `hikasa vfs cp|mv|rm|mkdir|touch` (M2 W3, D-102, D-103; contract M2 §1.3.7): the requests, the parsers of
//! their words (T11: the CLI parses nothing itself), [`Service::vfs_plan`] (the dry run) and
//! [`Service::vfs_run`] (plan + execute), and the DTO of `hikasa.vfs_op.v1`.
//!
//! Whole-command errors (a bad spec, the `cp`/`mv` naming rule of [`hikasa_vfs::ops::resolve_targets`]) are an
//! [`ApiError`]; failed items are not: they are in the DTO (`ok: true`, exit 100, as `settings import`).

use std::ffi::{OsStr, OsString};

use hikasa_core::{ErrorReport, FileTime, Spec, Warning};
use hikasa_vfs::ops::{
    self, Action, CopyOptions, ItemStatus, LinkPolicy, MoveOptions, OpReport, OverwritePolicy,
    Plan, RemoveOptions, SkipReason, Summary, TouchOptions,
};
use hikasa_vfs::{EntryKind, FollowLinks, SetTimes, Vfs, VfsError};
use serde::Serialize;

use super::{ApiError, Service};
use crate::host::Platform;
use crate::jobs::{JobControl, run_guarded};

/// The time `touch` sets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TouchTime {
    /// The injected clock's now (T1).
    Now,
    /// A given time (`--time`).
    At(FileTime),
    /// The modification time of another item (`--reference`).
    Reference(OsString),
}

/// One `vfs` write command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VfsOpRequest {
    /// `cp`.
    Copy {
        /// The sources as typed.
        sources: Vec<OsString>,
        /// The target as typed (a trailing separator means "into this folder").
        target: OsString,
        /// The options.
        options: CopyOptions,
    },
    /// `mv`.
    Move {
        /// The sources as typed.
        sources: Vec<OsString>,
        /// The target as typed.
        target: OsString,
        /// The options.
        options: MoveOptions,
    },
    /// `rm`.
    Remove {
        /// The items.
        specs: Vec<OsString>,
        /// The options.
        options: RemoveOptions,
    },
    /// `mkdir`.
    Mkdir {
        /// The folders.
        specs: Vec<OsString>,
        /// `--parents`.
        parents: bool,
    },
    /// `touch`.
    Touch {
        /// The items.
        specs: Vec<OsString>,
        /// The modification time.
        time: TouchTime,
        /// The creation time (`--created`; needs `SET_CREATED` at the item).
        created: Option<FileTime>,
        /// `--create`.
        create: bool,
    },
}

/// One item of `hikasa.vfs_op.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VfsOpItemDto {
    /// The action (`create_dir`, `keep_dir`, `copy`, `link`, `rename`, `move`, `remove`, `remove_dir`, `trash`,
    /// `touch`, `create`, `skip`, `create_archive`).
    pub action: &'static str,
    /// The item's kind.
    pub kind: EntryKind,
    /// The source (copies and moves).
    pub source: Option<String>,
    /// The item written, created, removed or touched.
    pub target: String,
    /// Bytes (files).
    pub size: Option<u64>,
    /// The target exists and is replaced.
    pub replaces: bool,
    /// `planned`, `done`, `skipped`, `failed`, `not_run`.
    pub status: ItemStatus,
    /// The skip reason.
    pub reason: Option<SkipReason>,
    /// The failure.
    pub error: Option<ErrorReport>,
}

/// `hikasa.vfs_op.v1` (`result`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VfsOpDto {
    /// `copy`, `move`, `remove`, `mkdir`, `touch`.
    pub op: &'static str,
    /// A `--dry-run`: nothing was written.
    pub dry_run: bool,
    /// The operation was cancelled.
    pub cancelled: bool,
    /// The items in plan order.
    pub items: Vec<VfsOpItemDto>,
    /// The counts (ARCH-101).
    pub summary: Summary,
    /// The plan's and the execution's warnings (the envelope's `warnings`; not in `result`).
    #[serde(skip)]
    pub warnings: Vec<Warning>,
}

impl VfsOpDto {
    /// The DTO of a report.
    pub fn from_report(report: OpReport, dry_run: bool) -> VfsOpDto {
        let mut warnings = report.plan.warnings.clone();
        warnings.extend(report.warnings);
        let items = report
            .plan
            .items
            .into_iter()
            .zip(report.results)
            .map(|(item, result)| VfsOpItemDto {
                action: item.action.as_str(),
                kind: item.kind,
                source: item.source.as_ref().map(Spec::canonical),
                target: item.target.canonical(),
                size: item.size,
                replaces: item.replaces,
                status: result.status,
                reason: result.skip.or(item.skip),
                error: result.error,
            })
            .collect();
        VfsOpDto {
            op: report.plan.op.as_str(),
            dry_run,
            cancelled: report.cancelled,
            items,
            summary: report.summary,
            warnings,
        }
    }

    /// No item failed and nothing was cancelled (exit 0; else 100).
    pub fn ok(&self) -> bool {
        self.summary.failed == 0 && !self.cancelled
    }
}

/// `--overwrite fail|skip|always|newer`.
pub fn parse_overwrite(word: &str) -> Result<OverwritePolicy, ApiError> {
    OverwritePolicy::parse(word).ok_or_else(|| {
        ApiError::InvalidArgument(format!(
            "unknown overwrite policy `{word}` (expected fail, skip, always or newer)"
        ))
    })
}

/// `--symlinks copy|skip`.
pub fn parse_link_policy(word: &str) -> Result<LinkPolicy, ApiError> {
    LinkPolicy::parse(word).ok_or_else(|| {
        ApiError::InvalidArgument(format!(
            "unknown symlink policy `{word}` (expected copy or skip)"
        ))
    })
}

/// An RFC 3339 time (`--time`, `--created`).
pub fn parse_time(text: &str) -> Result<FileTime, ApiError> {
    FileTime::parse(text).map_err(|e| ApiError::InvalidArgument(e.to_string()))
}

/// True when the raw argument ends with a path separator (`/`, and `\` on Windows): "into this folder".
pub fn ends_with_separator(raw: &OsStr, platform: Platform) -> bool {
    let bytes = raw.as_encoded_bytes();
    match bytes.last() {
        Some(b'/') => true,
        Some(b'\\') => platform == Platform::Windows,
        _ => false,
    }
}

/// True when the raw argument's final component (trailing separators ignored) is `.` or `..`: such an
/// argument names a folder through its child or parent and is never removed or moved (the parsed spec no longer
/// shows the `.`, so the raw text is checked).
pub fn is_dot_name(raw: &OsStr, platform: Platform) -> bool {
    let separator = |b: &u8| *b == b'/' || (*b == b'\\' && platform == Platform::Windows);
    let bytes = raw.as_encoded_bytes();
    let end = bytes.len() - bytes.iter().rev().take_while(|b| separator(b)).count();
    let trimmed = bytes.get(..end).unwrap_or_default();
    let start = trimmed.iter().rposition(separator).map_or(0, |i| i + 1);
    matches!(trimmed.get(start..), Some(b".") | Some(b".."))
}

/// `rm` items and `mv` sources whose final component is `.` or `..` → `invalid_input` "needs a file name"
/// before anything is planned.
fn refuse_dot_names(raw: &[OsString], platform: Platform) -> Result<(), ApiError> {
    match raw.iter().find(|r| is_dot_name(r, platform)) {
        Some(found) => Err(ApiError::from_vfs(VfsError::InvalidTarget {
            spec: found.to_string_lossy().into_owned(),
            reason: "needs a file name",
        })),
        None => Ok(()),
    }
}

impl Service {
    /// The plan of a write command, as `--dry-run` prints it (nothing is written; T12).
    pub fn vfs_plan(&self, req: &VfsOpRequest, job: &JobControl) -> Result<VfsOpDto, ApiError> {
        run_guarded(|| {
            let mut plan = self.build_plan(req, job)?;
            let router = self.router();
            ops::note_shadow_drops(&*router, &*router, &mut plan);
            Ok(VfsOpDto::from_report(ops::dry_run(plan), true))
        })
    }

    /// Plans and executes a write command with per-item results (D-103).
    pub fn vfs_run(&self, req: &VfsOpRequest, job: &JobControl) -> Result<VfsOpDto, ApiError> {
        run_guarded(|| {
            let plan = self.build_plan(req, job)?;
            let router = self.router();
            let report = ops::execute(&*router, &*router, plan, &job.ctl());
            Ok(VfsOpDto::from_report(report, false))
        })
    }

    /// A spec that is written, removed or touched: parsed without reading standard input or the clipboard, then
    /// resolved (§1.5.12): a trailing separator wants a folder.
    pub(crate) fn write_spec(&self, raw: &OsStr) -> Result<Spec, ApiError> {
        let spec = Spec::parse_os(raw, &self.spec_context())?;
        self.resolve(spec, self.use_of(raw))
    }

    fn build_plan(&self, req: &VfsOpRequest, job: &JobControl) -> Result<Plan, ApiError> {
        let cancelled = |e: VfsError| ApiError::from_vfs(e);
        let platform = self.env().platform;
        match req {
            VfsOpRequest::Copy {
                sources,
                target,
                options,
            } => {
                let pairs = self.pairs(sources, target, options.create_parents, platform, job)?;
                let router = self.router();
                ops::plan_copy(&*router, &*router, &pairs, options, &job.ctl()).map_err(cancelled)
            }
            VfsOpRequest::Move {
                sources,
                target,
                options,
            } => {
                refuse_dot_names(sources, platform)?;
                let pairs = self.pairs(sources, target, options.create_parents, platform, job)?;
                let router = self.router();
                ops::plan_move(&*router, &*router, &pairs, options, &job.ctl()).map_err(cancelled)
            }
            VfsOpRequest::Remove { specs, options } => {
                refuse_dot_names(specs, platform)?;
                let specs = self.write_specs(specs)?;
                let router = self.router();
                ops::plan_remove(&*router, &specs, options, &job.ctl()).map_err(cancelled)
            }
            VfsOpRequest::Mkdir { specs, parents } => {
                let raw = specs;
                let specs = self.write_specs(raw)?;
                let mut plan = ops::plan_mkdir(&*self.router(), &specs, *parents);
                // A trailing separator wants a folder (§1.5.12), whatever the name looks like.
                for (item, raw) in plan.items.iter_mut().zip(raw) {
                    if item.action == Action::CreateArchive && ends_with_separator(raw, platform) {
                        item.action = Action::CreateDir;
                        item.kind = EntryKind::Dir;
                    }
                }
                Ok(plan)
            }
            VfsOpRequest::Touch {
                specs,
                time,
                created,
                create,
            } => {
                let specs = self.write_specs(specs)?;
                let modified = match time {
                    TouchTime::Now => self.clock_file_time().ok_or_else(|| {
                        ApiError::Internal("the clock's time is out of range".into())
                    })?,
                    TouchTime::At(at) => *at,
                    TouchTime::Reference(raw) => {
                        let spec = self.parse_spec(raw)?;
                        let entry = self
                            .router()
                            .stat(&spec, FollowLinks::Yes)
                            .map_err(ApiError::from_vfs)?;
                        entry.mtime.ok_or_else(|| {
                            ApiError::InvalidArgument(format!(
                                "{}: has no modification time",
                                spec.canonical()
                            ))
                        })?
                    }
                };
                let options = TouchOptions {
                    times: SetTimes {
                        modified: Some(modified),
                        created: *created,
                    },
                    create: *create,
                };
                Ok(ops::plan_touch(&*self.router(), &specs, &options))
            }
        }
    }

    fn write_specs(&self, raw: &[OsString]) -> Result<Vec<Spec>, ApiError> {
        raw.iter().map(|r| self.write_spec(r)).collect()
    }

    /// The `(source, target)` pairs of `cp`/`mv` (D-102, `resolve_targets`).
    fn pairs(
        &self,
        sources: &[OsString],
        target: &OsStr,
        create_parents: bool,
        platform: Platform,
        job: &JobControl,
    ) -> Result<Vec<(Spec, Spec)>, ApiError> {
        let sources = sources
            .iter()
            .map(|s| self.parse_spec_in(s, self.use_of(s), &job.cancel))
            .collect::<Result<Vec<_>, _>>()?;
        let target_spec = self.write_spec(target)?;
        let into = ends_with_separator(target, platform);
        let router = self.router();
        // Indexing runs here, where it can be cancelled; `stat` cannot. Any other failure is its item's error.
        for source in &sources {
            if let Err(error) = router.prepare(source, &job.ctl())
                && error.is_cancelled()
            {
                return Err(ApiError::Cancelled);
            }
        }
        ops::resolve_targets(
            &*router as &dyn Vfs,
            &sources,
            &target_spec,
            into,
            create_parents,
        )
        .map_err(ApiError::from_vfs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separators() {
        assert!(ends_with_separator(OsStr::new("a/"), Platform::Linux));
        assert!(!ends_with_separator(OsStr::new("a\\"), Platform::Linux));
        assert!(ends_with_separator(OsStr::new("a\\"), Platform::Windows));
        assert!(!ends_with_separator(OsStr::new("a"), Platform::Windows));
    }

    #[test]
    fn dot_names() {
        for raw in [".", "..", "./", "../", "x/.", "x/..", "/var/..", "a/b/..//"] {
            assert!(is_dot_name(OsStr::new(raw), Platform::Linux), "{raw}");
        }
        for raw in ["x", ".x", "..x", "x.", "/", "", "-", "x\\.."] {
            assert!(!is_dot_name(OsStr::new(raw), Platform::Linux), "{raw}");
        }
        assert!(is_dot_name(OsStr::new("x\\.."), Platform::Windows));
        assert!(is_dot_name(OsStr::new("C:\\x\\.\\"), Platform::Windows));
    }

    #[test]
    fn words() {
        assert_eq!(parse_overwrite("newer").unwrap(), OverwritePolicy::Newer);
        assert!(parse_overwrite("Newer").is_err());
        assert_eq!(parse_link_policy("skip").unwrap(), LinkPolicy::Skip);
        assert!(parse_time("2026-01-02T03:04:05Z").is_ok());
        assert!(parse_time("yesterday").is_err());
    }
}
