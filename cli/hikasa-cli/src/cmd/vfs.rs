//! `hikasa vfs ls|stat|cat` (§3.5, §4.3.4; D-32): the `Service::vfs_*` DTOs as JSON, the `api::human` lines, or
//! the raw bytes of `cat` (`cat --json` carries them as base64). Exit 0, or 100 on error.

use hikasa_core::{ExitCode, JobKind};
use hikasa_report::escape_name;
use hikasa_session::api::dto::{VFS_CAT_SCHEMA, VFS_LIST_SCHEMA, VFS_STAT_SCHEMA};
use hikasa_session::api::{ApiOp, human};
use hikasa_vfs::FollowLinks;

use super::Ctx;
use crate::args::VfsCmd;
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
        VfsCmd::Stat(args) => match ctx
            .service
            .vfs_stat(&args.spec, follow(args.follow_symlinks))
        {
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
    }
}
