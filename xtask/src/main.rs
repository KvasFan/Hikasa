//! `cargo xtask <task>`: repository tasks (M2 W0; docs/17 §4.2, §4.3).
//!
//! | Task | Effect |
//! |---|---|
//! | `commands-doc` | writes `docs/reference/commands.md` from `core/hikasa-session/catalogue/commands.toml` |
//! | `commands-doc --check` | fails (exit 1) when `docs/reference/commands.md` is not what `commands-doc` writes |
//! | `help-check` | fails when the catalogue or a help topic has a problem (unknown topic, broken link, …) |
//!
//! Exit codes: 0 success, 1 a check failed, 2 a usage error.
#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use hikasa_session::commands;
use hikasa_session::help;

/// Where `commands-doc` writes, relative to the workspace root.
const COMMANDS_DOC: &str = "docs/reference/commands.md";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let root = workspace_root();
    let result = match args.as_slice() {
        ["commands-doc"] => commands_doc(&root, false),
        ["commands-doc", "--check"] => commands_doc(&root, true),
        ["help-check"] => help_check(),
        _ => {
            eprintln!("usage: cargo xtask (commands-doc [--check] | help-check)");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Err(problems) => {
            for problem in problems {
                eprintln!("xtask: {problem}");
            }
            ExitCode::FAILURE
        }
    }
}

/// The workspace root (the parent of this crate's folder).
fn workspace_root() -> PathBuf {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    here.parent()
        .map_or_else(|| here.to_path_buf(), Path::to_path_buf)
}

/// Generates (or with `check` compares) the commands reference.
fn commands_doc(root: &Path, check: bool) -> Result<String, Vec<String>> {
    let catalogue = commands::try_catalogue().map_err(|e| vec![e.to_string()])?;
    let expected = catalogue.render_markdown(true);
    let path = root.join(COMMANDS_DOC);
    if check {
        let actual = std::fs::read_to_string(&path).unwrap_or_default();
        if actual == expected {
            Ok(format!("{COMMANDS_DOC} is up to date"))
        } else {
            Err(vec![format!(
                "{COMMANDS_DOC} is stale: run `cargo xtask commands-doc`"
            )])
        }
    } else {
        std::fs::write(&path, expected).map_err(|e| vec![format!("{COMMANDS_DOC}: {e}")])?;
        Ok(format!(
            "wrote {COMMANDS_DOC} ({} commands)",
            catalogue.entries().count()
        ))
    }
}

/// Checks the catalogue and the help topics.
fn help_check() -> Result<String, Vec<String>> {
    let mut problems = help::problems();
    match commands::try_catalogue() {
        Ok(catalogue) => problems.extend(catalogue.problems()),
        Err(e) => problems.push(e.to_string()),
    }
    if problems.is_empty() {
        Ok(format!("{} help topics, no problems", help::topics().len()))
    } else {
        Err(problems)
    }
}
