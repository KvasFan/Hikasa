//! `hikasa format list|show|detect|tokens` (§3.5, §4.3.4) and the write forms `new|edit|delete|import|export|
//! enable|disable|move|restore` (M2 W1, D-75): the `Service::format_*` DTOs as JSON, or the `api::human` lines
//! (`format show` and `format export` print the canonical TOML). Exit 0, or 100 on error.

use hikasa_core::{ExitCode, JobKind};
use hikasa_session::ApiError;
use hikasa_session::api::dto::{
    FORMAT_DETECT_SCHEMA, FORMAT_EXPORT_SCHEMA, FORMAT_LIST_SCHEMA, FORMAT_RESTORE_SCHEMA,
    FORMAT_SHOW_SCHEMA, FORMAT_TOKENS_SCHEMA, FORMAT_WRITE_SCHEMA,
};
use hikasa_session::api::{
    ApiOp, FormatTokensRequest, FormatWriteDto, MoveTo, human, parse_format_kind, parse_line_range,
};

use super::Ctx;
use crate::args::{FormatCmd, FormatMoveArgs, FormatRestoreArgs};

/// Exit code of a successful `format` command.
const OK: u8 = ExitCode::Success.code();

/// Runs a `format` subcommand.
pub fn run(ctx: &mut Ctx<'_>, cmd: &FormatCmd) -> u8 {
    let op = ApiOp::Format;
    match cmd {
        FormatCmd::List => {
            let dto = ctx.service.format_list();
            if ctx.out.json() {
                ctx.json_success(FORMAT_LIST_SCHEMA, OK, dto, &[], op)
            } else {
                let written = ctx
                    .out
                    .lines(dto.formats.iter().map(human::format_list_line));
                ctx.human_done(written, OK, FORMAT_LIST_SCHEMA, &[], op)
            }
        }
        FormatCmd::Show { name } => {
            if ctx.out.json() {
                match ctx.service.format_show(name) {
                    Ok(dto) => ctx.json_success(FORMAT_SHOW_SCHEMA, OK, dto, &[], op),
                    Err(e) => ctx.fail(FORMAT_SHOW_SCHEMA, &e, op, &[]),
                }
            } else {
                match ctx.service.format_toml(name) {
                    Ok(mut text) => {
                        if !text.ends_with('\n') {
                            text.push('\n');
                        }
                        let written = ctx.out.bytes(text.as_bytes());
                        ctx.human_done(written, OK, FORMAT_SHOW_SCHEMA, &[], op)
                    }
                    Err(e) => ctx.fail(FORMAT_SHOW_SCHEMA, &e, op, &[]),
                }
            }
        }
        FormatCmd::Detect(args) => match ctx.service.format_detect(&args.file) {
            Ok(dto) => {
                if ctx.out.json() {
                    ctx.json_success(FORMAT_DETECT_SCHEMA, OK, dto, &[], op)
                } else {
                    let written = ctx.out.lines([human::format_detect_line(&dto)]);
                    ctx.human_done(written, OK, FORMAT_DETECT_SCHEMA, &[], op)
                }
            }
            Err(e) => ctx.fail(FORMAT_DETECT_SCHEMA, &e, op, &[]),
        },
        FormatCmd::Tokens(args) => {
            let lines = match args.lines.as_deref().map(parse_line_range).transpose() {
                Ok(lines) => lines,
                Err(e) => return ctx.fail(FORMAT_TOKENS_SCHEMA, &e, op, &[]),
            };
            let request = FormatTokensRequest {
                file: args.file.clone(),
                format: args.format.clone(),
                encoding: args.encoding.clone(),
                lines,
            };
            let job = ctx.job(JobKind::FormatTokens);
            let result: Result<_, ApiError> = ctx.service.format_tokens(&request, &job);
            match result {
                Ok(dto) => {
                    if ctx.out.json() {
                        ctx.json_success(FORMAT_TOKENS_SCHEMA, OK, dto, &[], op)
                    } else {
                        ctx.progress.clear();
                        let written = ctx.out.lines(human::token_lines(&dto));
                        ctx.human_done(written, OK, FORMAT_TOKENS_SCHEMA, &[], op)
                    }
                }
                Err(e) => ctx.fail(FORMAT_TOKENS_SCHEMA, &e, op, &[]),
            }
        }
        FormatCmd::New(args) => {
            let result = args
                .kind
                .as_deref()
                .map(parse_format_kind)
                .transpose()
                .and_then(|kind| {
                    ctx.service.format_new(
                        &args.id,
                        args.from.as_deref(),
                        args.name.as_deref(),
                        args.mask.as_deref(),
                        kind,
                    )
                });
            write_done(ctx, result)
        }
        FormatCmd::Edit(args) => {
            let mut set = Vec::new();
            for item in &args.set {
                match item.split_once('=') {
                    Some((key, value)) => set.push((key.to_owned(), value.to_owned())),
                    None => {
                        let e = ApiError::InvalidArgument(format!(
                            "--set {item:?}: expected KEY=VALUE"
                        ));
                        return ctx.fail(FORMAT_WRITE_SCHEMA, &e, op, &[]);
                    }
                }
            }
            let result = ctx
                .service
                .format_edit(&args.id, &set, args.from_file.as_deref());
            write_done(ctx, result)
        }
        FormatCmd::Delete { id } => {
            let result = ctx.service.format_delete(id);
            write_done(ctx, result)
        }
        FormatCmd::Import { path, replace } => {
            let result = ctx.service.format_import(path, *replace);
            write_done(ctx, result)
        }
        FormatCmd::Enable { id } => {
            let result = ctx.service.format_enable(id, true);
            write_done(ctx, result)
        }
        FormatCmd::Disable { id } => {
            let result = ctx.service.format_enable(id, false);
            write_done(ctx, result)
        }
        FormatCmd::Move(args) => {
            let result = ctx.service.format_move(&args.id, &move_to(args));
            write_done(ctx, result)
        }
        FormatCmd::Export { name, output } => {
            match ctx.service.format_export(name, output.as_deref()) {
                Ok(dto) => {
                    if ctx.out.json() {
                        ctx.json_success(FORMAT_EXPORT_SCHEMA, OK, &dto, &[], op)
                    } else {
                        let written = match &dto.path {
                            Some(path) => ctx.out.lines([format!("wrote {path}")]),
                            None => ctx.out.bytes(dto.toml.as_bytes()),
                        };
                        ctx.human_done(written, OK, FORMAT_EXPORT_SCHEMA, &[], op)
                    }
                }
                Err(e) => ctx.fail(FORMAT_EXPORT_SCHEMA, &e, op, &[]),
            }
        }
        FormatCmd::Restore(args) => restore(ctx, args),
    }
}

/// Where `format move` puts the format (clap allows exactly one of the flags).
fn move_to(args: &FormatMoveArgs) -> MoveTo {
    if let Some(id) = &args.before {
        MoveTo::Before(id.clone())
    } else if let Some(id) = &args.after {
        MoveTo::After(id.clone())
    } else if args.up {
        MoveTo::Up
    } else if args.down {
        MoveTo::Down
    } else if args.top {
        MoveTo::Top
    } else {
        MoveTo::Bottom
    }
}

/// The end of a format write: `hikasa.format_write.v1`, or `wrote`/`removed` lines.
fn write_done(ctx: &mut Ctx<'_>, result: Result<FormatWriteDto, ApiError>) -> u8 {
    let op = ApiOp::Format;
    match result {
        Ok(dto) => {
            if ctx.out.json() {
                ctx.json_success(FORMAT_WRITE_SCHEMA, OK, &dto, &[], op)
            } else {
                let written = ctx.out.lines(human::format_write_lines(&dto));
                ctx.human_done(written, OK, FORMAT_WRITE_SCHEMA, &[], op)
            }
        }
        Err(e) => ctx.fail(FORMAT_WRITE_SCHEMA, &e, op, &[]),
    }
}

/// `format restore`: without `--dry-run` it needs `--yes` (D-77).
fn restore(ctx: &mut Ctx<'_>, args: &FormatRestoreArgs) -> u8 {
    let op = ApiOp::Format;
    if !args.dry_run && !args.yes {
        let e = ApiError::InvalidArgument(
            "format restore deletes format files: add --yes, or --dry-run to see the plan".into(),
        );
        return ctx.fail(FORMAT_RESTORE_SCHEMA, &e, op, &[]);
    }
    match ctx
        .service
        .format_restore(&args.ids, args.all, args.dry_run)
    {
        Ok(dto) => {
            if ctx.out.json() {
                ctx.json_success(FORMAT_RESTORE_SCHEMA, OK, &dto, &[], op)
            } else {
                let written = ctx.out.lines(human::format_restore_lines(&dto));
                ctx.human_done(written, OK, FORMAT_RESTORE_SCHEMA, &[], op)
            }
        }
        Err(e) => ctx.fail(FORMAT_RESTORE_SCHEMA, &e, op, &[]),
    }
}
