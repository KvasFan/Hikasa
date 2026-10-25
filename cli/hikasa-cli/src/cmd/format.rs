//! `hikasa format list|show|detect|tokens` (§3.5, §4.3.4): the `Service::format_*` DTOs as JSON, or the
//! `api::human` lines (`format show` prints the canonical TOML). Exit 0, or 100 on error.

use hikasa_core::{ExitCode, JobKind};
use hikasa_session::ApiError;
use hikasa_session::api::dto::{
    FORMAT_DETECT_SCHEMA, FORMAT_LIST_SCHEMA, FORMAT_SHOW_SCHEMA, FORMAT_TOKENS_SCHEMA,
};
use hikasa_session::api::{ApiOp, FormatTokensRequest, human, parse_line_range};

use super::Ctx;
use crate::args::FormatCmd;

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
    }
}
