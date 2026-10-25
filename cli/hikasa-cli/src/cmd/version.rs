//! `hikasa version`: `hikasa <version>`, or `hikasa.version.v1` (version, IPC protocol, schemas) with `--json`.
//! Exit 0.

use hikasa_core::ExitCode;
use hikasa_session::HIKASA_VERSION;
use hikasa_session::api::ApiOp;
use hikasa_session::api::dto::{JsonEnvelope, VERSION_SCHEMA, VersionDto};

use crate::output::{Output, WriteError};

/// Runs `version`.
pub fn run(out: &mut Output) -> u8 {
    let code = ExitCode::Success.code();
    let written = if out.json() {
        out.envelope(&JsonEnvelope::success_raw(
            VERSION_SCHEMA,
            code,
            VersionDto::current(),
            Vec::new(),
        ))
    } else {
        out.lines([format!("hikasa {HIKASA_VERSION}")])
    };
    match written {
        Ok(()) | Err(WriteError::BrokenPipe) => code,
        Err(WriteError::Other(e)) => super::fail(out, VERSION_SCHEMA, &e, ApiOp::Other, &[], &[]),
    }
}
