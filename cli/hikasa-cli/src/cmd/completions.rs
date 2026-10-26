//! `hikasa completions <shell>`: the clap_complete script for bash, elvish, fish, powershell or zsh on stdout, or
//! `hikasa.completions.v1` (`{shell, script}`) with `--json`. Exit 0.

use clap::CommandFactory;
use hikasa_core::ExitCode;
use hikasa_session::api::ApiOp;
use hikasa_session::api::dto::{COMPLETIONS_SCHEMA, CompletionsDto, JsonEnvelope};

use crate::args::Cli;
use crate::output::{Output, WriteError};

/// The completion script of `shell`.
pub fn script(shell: clap_complete::Shell) -> Vec<u8> {
    let mut command = Cli::command();
    let mut script = Vec::new();
    clap_complete::generate(shell, &mut command, "hikasa", &mut script);
    script
}

/// Runs `completions`.
pub fn run(out: &mut Output, shell: clap_complete::Shell) -> u8 {
    let code = ExitCode::Success.code();
    let script = script(shell);
    let written = if out.json() {
        let dto = CompletionsDto {
            shell: shell.to_string(),
            script: String::from_utf8_lossy(&script).into_owned(),
        };
        out.envelope(&JsonEnvelope::success_raw(
            COMPLETIONS_SCHEMA,
            code,
            dto,
            Vec::new(),
        ))
    } else {
        out.bytes(&script)
    };
    match written {
        Ok(()) | Err(WriteError::BrokenPipe) => code,
        Err(WriteError::Other(e)) => {
            super::fail(out, COMPLETIONS_SCHEMA, &e, ApiOp::Other, &[], &[])
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::ValueEnum;

    use super::*;

    #[test]
    fn every_shell_has_a_script() {
        for shell in clap_complete::Shell::value_variants() {
            let script = script(*shell);
            assert!(script.len() > 100, "{shell}: {} bytes", script.len());
            assert!(
                String::from_utf8_lossy(&script).contains("hikasa"),
                "{shell}"
            );
        }
    }
}
