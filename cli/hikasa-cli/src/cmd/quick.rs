//! `hikasa quick <left> <right>` (§3.1): `Service::quick`; no stdout in human mode, `hikasa.quick.v1` with
//! `--json`; the exit code is the response's (1/2/11/12/13) or 100.
//!
//! A content comparison that could not read a file (`--crc`, `--binary`: the response's result is `error`, exit
//! 100) is a failed command like any other: its error is one stderr line (also with `--quiet`), and the JSON
//! document has `ok: false` and the `error` next to the result that carries it (§4.2, §4.3.1).

use hikasa_core::{ContentResult, JobKind};
use hikasa_session::ApiError;
use hikasa_session::api::dto::{JsonEnvelope, QUICK_SCHEMA, QuickDto};
use hikasa_session::api::{ApiOp, QuickMethod, QuickRequest};

use super::{Ctx, importance_patch};
use crate::args::QuickArgs;
use crate::output::stderr_line;

/// Runs `quick`.
pub fn run(ctx: &mut Ctx<'_>, args: &QuickArgs) -> u8 {
    let op = ApiOp::Quick;
    let request = match request(ctx, args) {
        Ok(request) => request,
        Err(e) => return ctx.fail(QUICK_SCHEMA, &e, op, &[]),
    };
    let job = ctx.job(JobKind::QuickCompare);
    match ctx.service.quick(&request, &job) {
        Ok(resp) => {
            let code = resp.exit_code.code();
            let warnings = resp.outcome.warnings.clone();
            let failed = match &resp.outcome.result {
                ContentResult::Error { error } => Some(error.clone()),
                ContentResult::BinarySame
                | ContentResult::RulesSame
                | ContentResult::UnimportantOnly
                | ContentResult::ImportantDifferences
                | ContentResult::BinaryDifferent => None,
            };
            if ctx.out.json() {
                let mut envelope = JsonEnvelope::success_raw(
                    QUICK_SCHEMA,
                    code,
                    QuickDto::new(&resp),
                    ctx.warnings(&warnings),
                );
                if let Some(error) = failed {
                    ctx.progress.clear();
                    stderr_line(&format!("error: {}", error.message));
                    envelope.ok = false;
                    envelope.error = Some(error);
                }
                ctx.json_envelope(&envelope, &warnings, op)
            } else {
                if let Some(error) = &failed {
                    ctx.progress.clear();
                    stderr_line(&format!("error: {}", error.message));
                }
                ctx.human_done(Ok(()), code, QUICK_SCHEMA, &warnings, op)
            }
        }
        Err(e) => ctx.fail(QUICK_SCHEMA, &e, op, &[]),
    }
}

/// The request of the flags (§1.9.2 mapping).
fn request(ctx: &Ctx<'_>, args: &QuickArgs) -> Result<QuickRequest, ApiError> {
    let method = if args.size {
        QuickMethod::Size
    } else if args.crc {
        QuickMethod::Crc
    } else if args.binary {
        QuickMethod::Binary
    } else {
        QuickMethod::Rules
    };
    let mut rules_patch = importance_patch(&args.importance, ctx.matches)?;
    rules_patch.compare_line_endings = args.compare_line_endings.then_some(true);
    Ok(QuickRequest {
        left: args.left.clone(),
        right: args.right.clone(),
        method,
        ignore_unimportant: args.ignore_unimportant,
        format: args.format.clone(),
        rules_file: args.rules_file.clone(),
        rules_patch,
    })
}
