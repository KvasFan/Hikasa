//! `hikasa session list|show|new|edit|delete|rename|lock|unlock|mkdir|rmdir|run` (M2 W2; contract §1.2.7,
//! D-80…D-84): the `Service::session_*` DTOs as JSON, or the `api::human` lines. `session run` runs the same
//! Service call and output path as `folder compare` / `text compare` (T11). Exit 0, or 100 on error (`run`:
//! the exit codes of the comparison).

use hikasa_core::ExitCode;
use hikasa_session::ApiError;
use hikasa_session::api::dto::{
    FOLDER_COMPARE_SCHEMA, FOLDER_SYNC_SCHEMA, HEX_COMPARE_SCHEMA, PICTURE_COMPARE_SCHEMA,
    SESSION_LIST_SCHEMA, SESSION_SHOW_SCHEMA, SESSION_WRITE_SCHEMA, TABLE_COMPARE_SCHEMA,
    TEXT_COMPARE_SCHEMA,
};
use hikasa_session::api::{
    ApiOp, SessionEdit, SessionRunRequest, SessionTarget, SessionWriteDto, human, parse_anchors,
    parse_assignment, parse_ignored,
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
            let words = only_for_sync(args)
                .and_then(|()| only_for_hex(args))
                .and_then(|()| only_for_picture(args))
                .and_then(|()| {
                    Ok((
                        parse_anchors(&args.anchors.align, &args.anchors.isolate)?,
                        parse_ignored(&args.anchors.ignored)?,
                    ))
                });
            match words {
                Ok((anchors, ignored)) => {
                    request.anchors = anchors;
                    // `--ignored` replaces the session's marks of the sides it names (M2 W10b).
                    request = request.with_ignored(ignored);
                }
                Err(e) => return ctx.fail(TEXT_COMPARE_SCHEMA, &e, ApiOp::TextCompare, &[]),
            }
            request.ignore_unimportant = args.ignore_unimportant;
            report::run_text(ctx, &args.report, request, "unified", Shape::Compare)
        }
        Ok(SessionRunRequest::Folder(mut request)) => {
            if let Err(e) = only_for_sync(args)
                .and_then(|()| only_for_text(args))
                .and_then(|()| only_for_hex(args))
                .and_then(|()| only_for_picture(args))
            {
                return ctx.fail(FOLDER_COMPARE_SCHEMA, &e, ApiOp::FolderCompare, &[]);
            }
            request.ignore_unimportant = args.ignore_unimportant;
            report::run_folder(ctx, &args.report, request, "list", Shape::Compare)
        }
        Ok(SessionRunRequest::Table(mut request)) => {
            if let Err(e) = only_for_sync(args)
                .and_then(|()| only_for_text(args))
                .and_then(|()| only_for_hex(args))
                .and_then(|()| only_for_picture(args))
            {
                return ctx.fail(TABLE_COMPARE_SCHEMA, &e, ApiOp::TableCompare, &[]);
            }
            request.ignore_unimportant = args.ignore_unimportant;
            if args.report.display.is_some() {
                request.display.clone_from(&args.report.display);
            }
            report::run_table(ctx, &args.report, request, Shape::Compare, None)
        }
        Ok(SessionRunRequest::Hex(mut request)) => {
            if let Err(e) = only_for_sync(args)
                .and_then(|()| only_for_text(args))
                .and_then(|()| only_for_picture(args))
                .and_then(|()| not_for_hex(args))
            {
                return ctx.fail(HEX_COMPARE_SCHEMA, &e, ApiOp::HexCompare, &[]);
            }
            if args.report.display.is_some() {
                request.layout.display.clone_from(&args.report.display);
            }
            if let Some(n) = args.report.context {
                request.layout.context = Some(n.to_string());
            }
            request.max_sections.clone_from(&args.max_sections);
            request.preview.clone_from(&args.preview);
            let mut report = args.report.clone();
            report.display = None;
            report.context = None;
            // `--context` shapes the rows of `--display all|same` and the reports: elsewhere it would be ignored.
            if args.report.context.is_some()
                && !report::asks_report(&report)
                && let Err(e) = super::hex::rows_shown(request.layout.display.as_deref())
            {
                return ctx.fail(HEX_COMPARE_SCHEMA, &e, ApiOp::HexCompare, &[]);
            }
            report::run_hex(ctx, &report, request, Shape::Compare)
        }
        Ok(SessionRunRequest::Picture(mut request)) => {
            if let Err(e) = only_for_sync(args)
                .and_then(|()| only_for_text(args))
                .and_then(|()| only_for_hex(args))
            {
                return ctx.fail(PICTURE_COMPARE_SCHEMA, &e, ApiOp::PictureCompare, &[]);
            }
            request.rules.ignore_unimportant |= args.ignore_unimportant;
            request.diff_output.clone_from(&args.diff_output);
            request.diff_pane.clone_from(&args.diff_pane);
            request.checkerboard = args.checkerboard;
            request.probes.clone_from(&args.probe);
            report::run_picture(ctx, &args.report, request, Shape::Compare)
        }
        Ok(SessionRunRequest::Sync { request, options }) => {
            if let Err(e) = only_for_text(args)
                .and_then(|()| only_for_hex(args))
                .and_then(|()| only_for_picture(args))
            {
                return ctx.fail(FOLDER_SYNC_SCHEMA, &e, ApiOp::FolderSync, &[]);
            }
            folder_sync::run_session(ctx, args, request, options)
        }
        // The kind of the session is not known yet: the folder schema stands for both (exit 100 either way).
        Err(e) => ctx.fail(FOLDER_COMPARE_SCHEMA, &e, ApiOp::Other, &[]),
    }
}

/// `--ignore-unimportant` means nothing to a Hex Compare session: a hex verdict is the bytes' (D-331), and `hex
/// compare` refuses the flag too (W12c review fixes, F11).
fn not_for_hex(args: &SessionRunArgs) -> Result<(), ApiError> {
    if args.ignore_unimportant {
        return Err(ApiError::InvalidArgument(
            "--ignore-unimportant does not apply to hex-compare sessions (a hex verdict is the bytes')".to_owned(),
        ));
    }
    Ok(())
}

/// `--max-sections` and `--preview` mean something to a Hex Compare session only (M2 W12).
fn only_for_hex(args: &SessionRunArgs) -> Result<(), ApiError> {
    if args.max_sections.is_some() || args.preview.is_some() {
        return Err(ApiError::InvalidArgument(
            "--max-sections and --preview apply to hex-compare sessions".to_owned(),
        ));
    }
    Ok(())
}

/// `--diff-output`, `--diff-pane`, `--checkerboard` and `--probe` mean something to a Picture Compare session only.
fn only_for_picture(args: &SessionRunArgs) -> Result<(), ApiError> {
    if args.diff_output.is_some()
        || args.diff_pane.is_some()
        || args.checkerboard
        || !args.probe.is_empty()
    {
        return Err(ApiError::InvalidArgument(
            "--diff-output, --diff-pane, --checkerboard and --probe apply to picture-compare sessions".to_owned(),
        ));
    }
    Ok(())
}

/// `--align`, `--isolate` and `--ignored` mean something to a Text Compare session only (M2 W10a, W10b).
fn only_for_text(args: &SessionRunArgs) -> Result<(), ApiError> {
    if !args.anchors.is_empty() {
        return Err(ApiError::InvalidArgument(
            "--align, --isolate and --ignored apply to text-compare sessions".to_owned(),
        ));
    }
    Ok(())
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
