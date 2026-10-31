//! `hikasa workspace list|show|delete|rename` (M2 W2; contract §1.2.7, D-85): the `Service::workspace_*` DTOs
//! as JSON, or the `api::human` lines. Exit 0, or 100 on error.

use hikasa_core::ExitCode;
use hikasa_session::ApiError;
use hikasa_session::api::dto::{
    WORKSPACE_LIST_SCHEMA, WORKSPACE_SHOW_SCHEMA, WORKSPACE_WRITE_SCHEMA,
};
use hikasa_session::api::{ApiOp, WorkspaceWriteDto, human};
use hikasa_session::sessions::SessionName;

use super::Ctx;
use crate::args::WorkspaceCmd;

/// Exit code of a successful command.
const OK: u8 = ExitCode::Success.code();

/// The operation of the exit codes (every failure is 100).
const OP: ApiOp = ApiOp::Other;

/// Runs a `workspace` subcommand.
pub fn run(ctx: &mut Ctx<'_>, cmd: &WorkspaceCmd) -> u8 {
    match cmd {
        WorkspaceCmd::List => {
            let dto = ctx.service.workspace_list();
            if ctx.out.json() {
                ctx.json_success(WORKSPACE_LIST_SCHEMA, OK, &dto, &dto.warnings, OP)
            } else {
                let written = ctx.out.lines(human::workspace_list_lines(&dto));
                ctx.human_done(written, OK, WORKSPACE_LIST_SCHEMA, &dto.warnings, OP)
            }
        }
        WorkspaceCmd::Show { name } => {
            let schema = WORKSPACE_SHOW_SCHEMA;
            match SessionName::parse(name).and_then(|n| ctx.service.workspace_show(&n)) {
                Ok(dto) => {
                    if ctx.out.json() {
                        ctx.json_success(schema, OK, &dto, &dto.warnings, OP)
                    } else {
                        let written = ctx.out.bytes(dto.toml.as_bytes());
                        ctx.human_done(written, OK, schema, &dto.warnings, OP)
                    }
                }
                Err(e) => ctx.fail(schema, &e, OP, &[]),
            }
        }
        WorkspaceCmd::Delete { name } => {
            let result = SessionName::parse(name).and_then(|n| ctx.service.workspace_delete(&n));
            done(ctx, result)
        }
        WorkspaceCmd::Rename { name, new } => {
            let result = SessionName::parse(name).and_then(|from| {
                let to = SessionName::parse(new)?;
                ctx.service.workspace_rename(&from, &to)
            });
            done(ctx, result)
        }
    }
}

/// The end of a workspace write.
fn done(ctx: &mut Ctx<'_>, result: Result<WorkspaceWriteDto, ApiError>) -> u8 {
    let schema = WORKSPACE_WRITE_SCHEMA;
    match result {
        Ok(dto) => {
            if ctx.out.json() {
                ctx.json_success(schema, OK, &dto, &[], OP)
            } else {
                let written = ctx.out.lines(human::workspace_write_lines(&dto));
                ctx.human_done(written, OK, schema, &[], OP)
            }
        }
        Err(e) => ctx.fail(schema, &e, OP, &[]),
    }
}
