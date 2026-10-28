//! `hikasa help [topic…]` and `hikasa help --list` (M2 W0; docs/12 §1): the topics of `hikasa_session::help`
//! as terminal text, or `hikasa.help.v1` with `--json`. An unknown topic exits 100 with suggestions. Reads no
//! configuration.

use hikasa_core::ExitCode;
use hikasa_session::ApiError;
use hikasa_session::api::ApiOp;
use hikasa_session::api::dto::{HELP_SCHEMA, HelpDto, JsonEnvelope};
use hikasa_session::help;

use crate::args::HelpArgs;
use crate::output::{Output, WriteError};

/// Runs `help`.
pub fn run(out: &mut Output, args: &HelpArgs) -> u8 {
    let code = ExitCode::Success.code();
    let written = if args.list {
        if out.json() {
            out.envelope(&JsonEnvelope::success_raw(
                HELP_SCHEMA,
                code,
                HelpDto::list(),
                Vec::new(),
            ))
        } else {
            out.lines(
                help::topics()
                    .iter()
                    .map(|t| format!("{}\t{}", t.id, t.title())),
            )
        }
    } else {
        match help::resolve(&args.topic) {
            Ok(topic) if out.json() => out.envelope(&JsonEnvelope::success_raw(
                HELP_SCHEMA,
                code,
                HelpDto::topic(topic),
                Vec::new(),
            )),
            Ok(topic) => out.bytes(topic.render_text().as_bytes()),
            Err(unknown) => {
                let err = ApiError::InvalidArgument(unknown.to_string());
                return super::fail(out, HELP_SCHEMA, &err, ApiOp::Other, &[], &[]);
            }
        }
    };
    match written {
        Ok(()) | Err(WriteError::BrokenPipe) => code,
        Err(WriteError::Other(e)) => super::fail(out, HELP_SCHEMA, &e, ApiOp::Other, &[], &[]),
    }
}
