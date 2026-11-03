//! `hikasa session list|show|new|edit|delete|rename|lock|unlock|mkdir|rmdir|run` (M2 W2; contract §1.2.7,
//! D-80…D-84): the `Service::session_*` DTOs as JSON, or the `api::human` lines. `session run` runs the same
//! Service call and output path as `folder compare` / `text compare` (T11). Exit 0, or 100 on error (`run`:
//! the exit codes of the comparison).

use hikasa_core::ExitCode;
use hikasa_session::ApiError;
use hikasa_session::api::dto::{
    FOLDER_COMPARE_SCHEMA, SESSION_LIST_SCHEMA, SESSION_SHOW_SCHEMA, SESSION_WRITE_SCHEMA,
    TEXT_COMPARE_SCHEMA,
};
use hikasa_session::api::{
    ApiOp, SessionEdit, SessionRunRequest, SessionTarget, SessionWriteDto, human, parse_assignment,
};
use hikasa_session::sessions::{SessionRef, SessionType};

use super::report::{self, Shape};
use super::{Ctx, folder_sync};
use crate::args::{SessionCmd, SessionRunArgs};

/// Exit code of a successful command.
const OK: u8 = ExitCode::Success.code();

/// The operation of the exit codes (every failure is 100).
const OP: ApiOp = ApiOp::Other;

/// Runs a `session` subcommand.
pub fn run(ctx: &mut Ctx<'_>, cmd: &SessionCmd) -> u8 {
    match cmd {
        SessionCmd::List {
            folder,
            autosaved,
            defaults,
        } => {
            let schema = SESSION_LIST_SCHEMA;
            match ctx
                .service
                .session_list(folder.as_deref(), *autosaved, *defaults)
            {
                Ok(dto) => {
                    if ctx.out.json() {
                        ctx.json_success(schema, OK, &dto, &dto.warnings, OP)
                    } else {
                        let written = ctx.out.lines(human::session_list_lines(&dto));
                        ctx.human_done(written, OK, schema, &dto.warnings, OP)
                    }
                }
                Err(e) => ctx.fail(schema, &e, OP, &[]),
            }
        }
        SessionCmd::Show {
            reference,
            defaults,
        } => {
            let schema = SESSION_SHOW_SCHEMA;
            let result = target(reference.as_deref(), defaults.as_deref())
                .and_then(|t| ctx.service.session_show(&t));
            match result {
                Ok(dto) => {
                    if ctx.out.json() {
                        ctx.json_success(schema, OK, &dto, &[], OP)
                    } else {
                        let written = ctx.out.bytes(dto.toml.as_bytes());
                        ctx.human_done(written, OK, schema, &[], OP)
                    }
                }
                Err(e) => ctx.fail(schema, &e, OP, &[]),
            }
        }
        SessionCmd::New {
            reference,
            kind,
            left,
            right,
            description,
        } => {
            let result = SessionRef::parse(reference).and_then(|r| {
                let kind = SessionType::parse(kind)?;
                ctx.service.session_new(
                    &r,
                    kind,
                    left.as_deref(),
                    right.as_deref(),
                    description.as_deref(),
                )
            });
            write_done(ctx, result)
        }
        SessionCmd::Edit {
            reference,
            defaults,
            set,
            unset,
            left,
            right,
            description,
            force,
        } => {
            let result = target(reference.as_deref(), defaults.as_deref()).and_then(|t| {
                let edit = SessionEdit {
                    set: set
                        .iter()
                        .map(|s| parse_assignment(s))
                        .collect::<Result<_, _>>()?,
                    unset: unset.clone(),
                    left: left.clone(),
                    right: right.clone(),
                    description: description.clone(),
                };
                ctx.service.session_edit(&t, &edit, *force)
            });
            write_done(ctx, result)
        }
        SessionCmd::Delete { reference, force } => {
            let result =
                SessionRef::parse(reference).and_then(|r| ctx.service.session_delete(&r, *force));
            write_done(ctx, result)
        }
        SessionCmd::Rename {
            reference,
            new,
            force,
        } => {
            let result = SessionRef::parse(reference).and_then(|from| {
                let to = SessionRef::parse(new)?;
                ctx.service.session_rename(&from, &to, *force)
            });
            write_done(ctx, result)
        }
        SessionCmd::Lock { reference } => {
            let result =
                SessionRef::parse(reference).and_then(|r| ctx.service.session_lock(&r, true));
            write_done(ctx, result)
        }
        SessionCmd::Unlock { reference } => {
            let result =
                SessionRef::parse(reference).and_then(|r| ctx.service.session_lock(&r, false));
            write_done(ctx, result)
        }
        SessionCmd::Mkdir { folder } => {
            let result = ctx.service.session_mkdir(folder);
            write_done(ctx, result)
        }
        SessionCmd::Rmdir {
            folder,
            recursive,
            force,
        } => {
            let result = ctx.service.session_rmdir(folder, *recursive, *force);
            write_done(ctx, result)
        }
        SessionCmd::Run(args) => run_session(ctx, args),
    }
}

/// The session named by `REF` or `--defaults <TYPE>`.
fn target(reference: Option<&str>, defaults: Option<&str>) -> Result<SessionTarget, ApiError> {
    match (reference, defaults) {
        (_, Some(kind)) => Ok(SessionTarget::Defaults(SessionType::parse(kind)?)),
        (Some(r), None) => Ok(SessionTarget::Named(SessionRef::parse(r)?)),
        (None, None) => Err(ApiError::InvalidArgument(
            "name a session, or --defaults <TYPE>".into(),
        )),
    }
}

/// The end of a session write.
fn write_done(ctx: &mut Ctx<'_>, result: Result<SessionWriteDto, ApiError>) -> u8 {
    let schema = SESSION_WRITE_SCHEMA;
    match result {
        Ok(dto) => {
            if ctx.out.json() {
                ctx.json_success(schema, OK, &dto, &dto.warnings, OP)
            } else {
                let written = ctx.out.lines(human::session_write_lines(&dto));
                ctx.human_done(written, OK, schema, &dto.warnings, OP)
            }
        }
        Err(e) => ctx.fail(schema, &e, OP, &[]),
    }
}

/// `session run`: the session's request through the `folder compare` / `text compare` report path (T11).
fn run_session(ctx: &mut Ctx<'_>, args: &SessionRunArgs) -> u8 {
    let request =
        SessionRef::parse(&args.reference).and_then(|r| ctx.service.session_run_request(&r));
    match request {
        Ok(SessionRunRequest::Text(mut request)) => {
            if let Err(e) = only_for_sync(args) {
                return ctx.fail(TEXT_COMPARE_SCHEMA, &e, ApiOp::TextCompare, &[]);
            }
            request.ignore_unimportant = args.ignore_unimportant;
            report::run_text(ctx, &args.report, request, "unified", Shape::Compare)
        }
        Ok(SessionRunRequest::Folder(mut request)) => {
            if let Err(e) = only_for_sync(args) {
                return ctx.fail(FOLDER_COMPARE_SCHEMA, &e, ApiOp::FolderCompare, &[]);
            }
            request.ignore_unimportant = args.ignore_unimportant;
            report::run_folder(ctx, &args.report, request, "list", Shape::Compare)
        }
        Ok(SessionRunRequest::Sync { request, options }) => {
            folder_sync::run_session(ctx, args, request, options)
        }
        // The kind of the session is not known yet: the folder schema stands for both (exit 100 either way).
        Err(e) => ctx.fail(FOLDER_COMPARE_SCHEMA, &e, ApiOp::Other, &[]),
    }
}

/// `--yes`, `--confirm`, `--allow-empty-source`, `--all-rows` mean something to a folder-sync session only: a
/// comparison would pass them by in silence. (`--dry-run` is the report's for the others, W7b.)
fn only_for_sync(args: &SessionRunArgs) -> Result<(), ApiError> {
    if args.yes || args.confirm.is_some() || args.allow_empty_source || args.all_rows {
        return Err(ApiError::InvalidArgument(
            "--yes, --confirm, --allow-empty-source and --all-rows apply to folder-sync sessions"
                .to_owned(),
        ));
    }
    Ok(())
}
