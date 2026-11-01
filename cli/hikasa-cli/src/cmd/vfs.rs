//! `hikasa vfs ls|stat|cat` (§3.5, §4.3.4; D-32): the `Service::vfs_*` DTOs as JSON, the `api::human` lines, or
//! the raw bytes of `cat` (`cat --json` carries them as base64). Exit 0, or 100 on error.
//!
//! `hikasa vfs cp|mv|rm|mkdir|touch` (M2 W3, D-102): the arguments become one `VfsOpRequest` (the words are parsed
//! by core, T11); `--dry-run` calls `Service::vfs_plan`, otherwise `Service::vfs_run`; `hikasa.vfs_op.v1` or
//! one line per item and the summary. Failed items keep `ok: true` and exit 100.

use hikasa_core::{ExitCode, JobKind};
use hikasa_report::escape_name;
use hikasa_session::ApiError;
use hikasa_session::api::dto::{VFS_CAT_SCHEMA, VFS_LIST_SCHEMA, VFS_OP_SCHEMA, VFS_STAT_SCHEMA};
use hikasa_session::api::{
    ApiOp, TouchTime, VfsOpRequest, human, parse_link_policy, parse_overwrite, parse_time,
};
use hikasa_vfs::FollowLinks;
use hikasa_vfs::ops::{CopyOptions, MoveOptions, Preserve, RemoveOptions};

use super::Ctx;
use crate::args::{VfsCmd, VfsCpArgs, VfsMkdirArgs, VfsMvArgs, VfsRmArgs, VfsTouchArgs};
use crate::output::{WriteError, stderr_line};

/// Exit code of a successful `vfs` command.
const OK: u8 = ExitCode::Success.code();

/// `--follow-symlinks`.
fn follow(follow_symlinks: bool) -> FollowLinks {
    if follow_symlinks {
        FollowLinks::Yes
    } else {
        FollowLinks::No
    }
}

/// Runs a `vfs` subcommand.
pub fn run(ctx: &mut Ctx<'_>, cmd: &VfsCmd) -> u8 {
    let op = ApiOp::Vfs;
    match cmd {
        VfsCmd::Ls(args) => {
            let job = ctx.job(JobKind::VfsList);
            match ctx
                .service
                .vfs_ls(&args.spec, follow(args.follow_symlinks), &job)
            {
                Ok(dto) => {
                    if ctx.out.json() {
                        return ctx.json_success(VFS_LIST_SCHEMA, OK, dto, &[], op);
                    }
                    ctx.progress.clear();
                    let written = ctx.out.lines(dto.entries.iter().map(human::vfs_ls_line));
                    // Entries whose metadata failed are errors: listed on stderr, also with --quiet (with --json
                    // they are the document's `errors`).
                    for failed in &dto.errors {
                        let name = escape_name(&failed.name);
                        stderr_line(&format!("error: {name}: {}", failed.error.message));
                    }
                    ctx.human_done(written, OK, VFS_LIST_SCHEMA, &[], op)
                }
                Err(e) => ctx.fail(VFS_LIST_SCHEMA, &e, op, &[]),
            }
        }
        VfsCmd::Stat(args) => match ctx.service.vfs_stat_in(
            &args.spec,
            follow(args.follow_symlinks),
            &ctx.job(JobKind::VfsList),
        ) {
            Ok(dto) => {
                if ctx.out.json() {
                    ctx.json_success(VFS_STAT_SCHEMA, OK, dto, &[], op)
                } else {
                    let written = ctx.out.lines(human::vfs_stat_lines(&dto));
                    ctx.human_done(written, OK, VFS_STAT_SCHEMA, &[], op)
                }
            }
            Err(e) => ctx.fail(VFS_STAT_SCHEMA, &e, op, &[]),
        },
        VfsCmd::Cat { spec } => {
            let job = ctx.job(JobKind::VfsRead);
            if ctx.out.json() {
                return match ctx.service.vfs_cat_dto(spec, &job) {
                    Ok(dto) => ctx.json_success(VFS_CAT_SCHEMA, OK, dto, &[], op),
                    Err(e) => ctx.fail(VFS_CAT_SCHEMA, &e, op, &[]),
                };
            }
            match ctx.service.vfs_cat(spec, ctx.out.stdout(), &job) {
                Ok(_) => ctx.human_done(Ok(()), OK, VFS_CAT_SCHEMA, &[], op),
                Err(e) => match WriteError::from_api(e) {
                    WriteError::BrokenPipe => {
                        ctx.human_done(Err(WriteError::BrokenPipe), OK, VFS_CAT_SCHEMA, &[], op)
                    }
                    WriteError::Other(e) => ctx.fail(VFS_CAT_SCHEMA, &e, op, &[]),
                },
            }
        }
        VfsCmd::Cp(args) => write(ctx, copy_request(args), args.dry_run),
        VfsCmd::Mv(args) => write(ctx, move_request(args), args.dry_run),
        VfsCmd::Rm(args) => {
            if !args.dry_run && !args.yes {
                let e = ApiError::InvalidArgument(
                    "rm deletes items: pass --yes, or --dry-run to see the plan".into(),
                );
                return ctx.fail(VFS_OP_SCHEMA, &e, op, &[]);
            }
            write(ctx, Ok(remove_request(args)), args.dry_run)
        }
        VfsCmd::Mkdir(args) => write(ctx, Ok(mkdir_request(args)), args.dry_run),
        VfsCmd::Touch(args) => write(ctx, touch_request(args), args.dry_run),
    }
}

/// Splits `SRC… DST`.
fn sources_and_target(
    paths: &[std::ffi::OsString],
) -> (Vec<std::ffi::OsString>, std::ffi::OsString) {
    match paths.split_last() {
        Some((target, sources)) => (sources.to_vec(), target.clone()),
        None => (Vec::new(), std::ffi::OsString::new()),
    }
}

fn copy_request(args: &VfsCpArgs) -> Result<VfsOpRequest, ApiError> {
    let (sources, target) = sources_and_target(&args.paths);
    let mut preserve = if args.no_preserve {
        Preserve::NONE
    } else {
        Preserve::default()
    };
    preserve.created = args.preserve_created;
    let options = CopyOptions {
        recursive: args.recursive,
        overwrite: args
            .overwrite
            .as_deref()
            .map(parse_overwrite)
            .transpose()?
            .unwrap_or_default(),
        links: args
            .symlinks
            .as_deref()
            .map(parse_link_policy)
            .transpose()?
            .unwrap_or_default(),
        preserve,
        follow_roots: args.follow_symlinks,
        create_parents: args.parents,
    };
    Ok(VfsOpRequest::Copy {
        sources,
        target,
        options,
    })
}

fn move_request(args: &VfsMvArgs) -> Result<VfsOpRequest, ApiError> {
    let (sources, target) = sources_and_target(&args.paths);
    let options = MoveOptions {
        overwrite: args
            .overwrite
            .as_deref()
            .map(parse_overwrite)
            .transpose()?
            .unwrap_or_default(),
        create_parents: args.parents,
    };
    Ok(VfsOpRequest::Move {
        sources,
        target,
        options,
    })
}

fn remove_request(args: &VfsRmArgs) -> VfsOpRequest {
    VfsOpRequest::Remove {
        specs: args.specs.clone(),
        options: RemoveOptions {
            recursive: args.recursive,
            trash: args.trash,
        },
    }
}

fn mkdir_request(args: &VfsMkdirArgs) -> VfsOpRequest {
    VfsOpRequest::Mkdir {
        specs: args.specs.clone(),
        parents: args.parents,
    }
}

fn touch_request(args: &VfsTouchArgs) -> Result<VfsOpRequest, ApiError> {
    let time = match (&args.time, &args.reference) {
        (Some(time), _) => TouchTime::At(parse_time(time)?),
        (None, Some(reference)) => TouchTime::Reference(reference.clone()),
        (None, None) => TouchTime::Now,
    };
    Ok(VfsOpRequest::Touch {
        specs: args.specs.clone(),
        time,
        created: args.created.as_deref().map(parse_time).transpose()?,
        create: args.create,
    })
}

/// Runs (or plans) a write command and prints `hikasa.vfs_op.v1` or the human lines.
fn write(ctx: &mut Ctx<'_>, request: Result<VfsOpRequest, ApiError>, dry_run: bool) -> u8 {
    let op = ApiOp::Vfs;
    let job = ctx.job(JobKind::VfsWrite);
    let result = request.and_then(|request| {
        if dry_run {
            ctx.service.vfs_plan(&request, &job)
        } else {
            ctx.service.vfs_run(&request, &job)
        }
    });
    match result {
        Ok(dto) => {
            let code = if dto.ok() {
                OK
            } else {
                ExitCode::UnknownError.code()
            };
            if ctx.out.json() {
                ctx.json_success(VFS_OP_SCHEMA, code, &dto, &dto.warnings, op)
            } else {
                ctx.progress.clear();
                let written = ctx.out.lines(human::vfs_op_lines(&dto));
                ctx.human_done(written, code, VFS_OP_SCHEMA, &dto.warnings, op)
            }
        }
        Err(e) => ctx.fail(VFS_OP_SCHEMA, &e, op, &[]),
    }
}
