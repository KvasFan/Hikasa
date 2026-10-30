//! `hikasa session list|show|new|edit|delete|rename|lock|unlock|mkdir|rmdir|run` (M2 W2; contract §1.2.7,
//! D-80…D-84): the `Service::session_*` DTOs as JSON, or the `api::human` lines. `session run` runs the same
//! Service call and output path as `folder compare` / `text compare` (T11). Exit 0, or 100 on error (`run`:
//! the exit codes of the comparison).

use hikasa_core::ExitCode;
use hikasa_session::ApiError;
use hikasa_session::api::dto::{
    FOLDER_COMPARE_SCHEMA, SESSION_LIST_SCHEMA, SESSION_SHOW_SCHEMA, SESSION_WRITE_SCHEMA,
};
use hikasa_session::api::{
    ApiOp, SessionEdit, SessionRunRequest, SessionTarget, SessionWriteDto, human, parse_assignment,
};
use hikasa_session::sessions::{SessionRef, SessionType};

use super::{Ctx, folder, text};
use crate::args::{FolderCompareArgs, SessionCmd, SessionRunArgs, TextCompareArgs};

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

/// `session run`: the session's request through the `folder compare` / `text compare` output path (T11).
fn run_session(ctx: &mut Ctx<'_>, args: &SessionRunArgs) -> u8 {
    let request =
        SessionRef::parse(&args.reference).and_then(|r| ctx.service.session_run_request(&r));
    match request {
        Ok(SessionRunRequest::Text(mut request)) => {
            request.ignore_unimportant = args.ignore_unimportant;
            text::execute(ctx, &text_args(args, &request), &request)
        }
        Ok(SessionRunRequest::Folder(mut request)) => {
            request.ignore_unimportant = args.ignore_unimportant;
            folder::execute(ctx, &folder_args(args, &request), &request)
        }
        // The kind of the session is not known yet: the folder schema stands for both (exit 100 either way).
        Err(e) => ctx.fail(FOLDER_COMPARE_SCHEMA, &e, ApiOp::Other, &[]),
    }
}

/// The `text compare` arguments of `session run` (only the presentation words are read by `text::execute`).
fn text_args(
    args: &SessionRunArgs,
    request: &hikasa_session::api::TextCompareRequest,
) -> TextCompareArgs {
    TextCompareArgs {
        left: request.left.clone(),
        right: request.right.clone(),
        format: request.format.clone(),
        encoding: None,
        left_encoding: request.encodings.left.clone(),
        right_encoding: request.encodings.right.clone(),
        rules_file: None,
        algorithm: None,
        skew: None,
        closeness: false,
        no_closeness: false,
        closeness_threshold: None,
        never_align: false,
        importance: crate::args::ImportanceArgs::default(),
        compare_line_endings: false,
        orphans_unimportant: false,
        ignore_unimportant: args.ignore_unimportant,
        context: args.context,
        display: args.display.clone(),
        report: args.report.clone().unwrap_or_else(|| "unified".into()),
        patch_style: args.patch_style.clone(),
        html: args.html,
        html_mono: args.html_mono,
        line_numbers: !args.no_line_numbers,
        no_line_numbers: args.no_line_numbers,
        width: args.width,
        wrap: args.wrap.clone(),
        title: args.title.clone(),
        output: args.output.clone(),
        stdin_name: None,
        gui: false,
        wait: false,
        no_launch: false,
    }
}

/// The `folder compare` arguments of `session run` (only the presentation words are read by
/// `folder::execute`).
fn folder_args(
    args: &SessionRunArgs,
    request: &hikasa_session::api::FolderCompareRequest,
) -> FolderCompareArgs {
    FolderCompareArgs {
        left: request.left.clone(),
        right: request.right.clone(),
        filters: None,
        rules_file: None,
        criteria: None,
        size: false,
        no_size: false,
        timestamps: false,
        no_timestamps: false,
        tolerance: None,
        ignore_dst: false,
        ignore_timezone: false,
        filename_case: false,
        attributes: None,
        permissions: false,
        owner: false,
        group: false,
        content: None,
        skip_if_quick_same: false,
        no_override_quick: false,
        text_rules_file: None,
        importance: crate::args::ImportanceArgs::default(),
        case_sensitive: false,
        case_insensitive: false,
        align_extensions: false,
        align_normalization: false,
        cutoff: None,
        size_filter: None,
        include_protected: false,
        follow_symlinks: false,
        flatten: false,
        no_scan_orphans: false,
        display: args.display.clone(),
        folders: args.folders.clone(),
        ignore_unimportant: args.ignore_unimportant,
        suppress_filters: args.suppress_filters,
        expand_all: false,
        report: args.report.clone().unwrap_or_else(|| "list".into()),
        columns: args.columns.clone(),
        html: args.html,
        html_mono: args.html_mono,
        title: args.title.clone(),
        output: args.output.clone(),
        gui: false,
        wait: false,
        no_launch: false,
    }
}
