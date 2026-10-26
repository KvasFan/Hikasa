//! `hikasa`: the command-line interface of Hikasa. It exposes every core function headless and reaches the
//! running GUI over IPC (`hikasa gui …`, `--gui`).
//!
//! The binary is a thin adapter (contract §1.9, ARCH-L2, T11): it parses arguments with clap, builds one request,
//! calls one `hikasa_session::Service` method and prints the result as JSON (`--json`, §4) or as human output.
//! Vocabulary words go through the core parsers; exit codes come from the responses and from
//! `ApiError::exit_code` (§2.21).
//!
//! Flow (§1.9.1): `capture_env` → clap (a usage error exits 100, D-35; with `--json` before any `--` it prints
//! `hikasa.usage_error.v1`) → a global value flag repeated with different values is a usage error (§1.8.7 item
//! 34) → for `gui open`, `parse_open_args` on the arguments exactly as written after `open` (D-51) and the merge
//! of its output flags with the global ones → logging (`--log`, `--verbose`) → `Service::new` with a lazy
//! standard-input source → the command inside the panic boundary (a panic exits 100, D-35) → exit code. A broken
//! pipe on stdout stops the output and keeps the computed code.

mod args;
mod cmd;
mod launcher;
mod output;
mod progress;

use std::ffi::OsString;
use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use clap::{ArgMatches, CommandFactory, FromArgMatches};
use hikasa_core::ExitCode;
use hikasa_formats::StdProcessRunner;
use hikasa_session::api::dto::JsonEnvelope;
use hikasa_session::api::{OpenArgs, normalize_legacy_args, open_usage, parse_open_args};
use hikasa_session::{
    ApiError, ApiOp, EnvFacts, Platform, Service, ServiceInit, StdinSource, SystemClock,
    capture_env, run_guarded,
};
use hikasa_vfs::{VfsError, VfsOp};

use crate::args::{Cli, Command, FormatCmd, GlobalArgs, GuiCmd, TextCmd};
use crate::output::{Output, colour_enabled, stderr_line};
use crate::progress::ProgressLine;

/// Exit code of usage errors, panics and every other failure (D-35).
const FAILURE: u8 = ExitCode::UnknownError.code();

/// Exit code of `--help` and `--version`.
const SUCCESS: u8 = ExitCode::Success.code();

/// Set once the `--log` file is open: the panic hook then writes the backtrace there.
static LOGGING_TO_FILE: AtomicBool = AtomicBool::new(false);

/// Log target of panic backtraces (the `--log` file only, never stderr).
const PANIC_TARGET: &str = "hikasa::panic";

/// The global flags that take a value; given more than once, the values must agree.
const GLOBAL_VALUE_FLAGS: [&str; 2] = ["--config-dir", "--log"];

fn main() -> std::process::ExitCode {
    install_panic_hook();
    let code = match run_guarded(|| Ok(run())) {
        Ok(code) => code,
        Err(e) => {
            stderr_line(&format!("error: {e}"));
            FAILURE
        }
    };
    std::process::ExitCode::from(code)
}

/// One invocation; returns the exit code.
fn run() -> u8 {
    let env = capture_env();
    let argv: Vec<OsString> = std::env::args_os().collect();
    // Only the arguments before the first `--` can be flags: after it, `--json` is a file name.
    let flags = &argv[..options_end(&argv)];
    let json_anywhere = flags.iter().skip(1).any(|a| a == "--json");
    let no_color_anywhere = env.no_color || flags.iter().skip(1).any(|a| a == "--no-color");
    let mut command = Cli::command();
    if no_color_anywhere || !std::io::stderr().is_terminal() {
        command = command.color(clap::ColorChoice::Never);
    }
    let matches = match command.try_get_matches_from(&argv) {
        Ok(matches) => matches,
        Err(e) => return clap_error(&e, json_anywhere),
    };
    let cli = match Cli::from_arg_matches(&matches) {
        Ok(cli) => cli,
        Err(e) => return clap_error(&e, json_anywhere),
    };
    let mut globals = cli.global.clone();
    // The `gui open` arguments as written after `open` (D-51), and where the part clap parsed ends.
    let (open_raw, clap_end) = if let Command::Gui {
        cmd: GuiCmd::Open(raw),
    } = &cli.command
    {
        let (raw, start) = raw_open_args(&argv, &raw.args);
        (Some(raw), start)
    } else {
        (None, argv.len())
    };
    if let Err(message) = check_repeated_globals(&argv[..clap_end], env.cwd.as_deref()) {
        return usage_message(&message, json_anywhere || globals.json);
    }
    let mut stdin_name = stdin_name(&cli.command);
    let mut open: Option<OpenArgs> = None;
    if let Some(raw) = &open_raw {
        let parsed = match parse_open_args(raw, Platform::HOST) {
            Ok(parsed) => parsed,
            Err(e) => {
                let json = json_anywhere || globals.json || asks_for_json(raw);
                return usage_message(&e.to_string(), json);
            }
        };
        if let Err(message) = merge_open_flags(&mut globals, &parsed, env.cwd.as_deref()) {
            return usage_message(&message, json_anywhere || globals.json);
        }
        if parsed.help {
            let mut stdout = std::io::stdout().lock();
            let _ = stdout.write_all(open_usage().as_bytes());
            let _ = stdout.flush();
            return SUCCESS;
        }
        stdin_name.clone_from(&parsed.stdin_name);
        open = Some(parsed);
    }
    let mut out = Output::new(globals.json, globals.quiet, globals.no_color, env.no_color);
    let code = execute(
        &mut out,
        &cli.command,
        &matches,
        open.as_ref(),
        &globals,
        env,
        stdin_name,
    );
    match out.finish() {
        Ok(()) => code,
        Err(e) => {
            out.error(&e);
            FAILURE
        }
    }
}

/// Logging, the service and the command (everything after argument parsing); returns the exit code. A failure
/// before the command runs (the `--log` file, the configuration) is reported with the command's schema.
fn execute(
    out: &mut Output,
    command: &Command,
    matches: &ArgMatches,
    open: Option<&OpenArgs>,
    globals: &GlobalArgs,
    env: EnvFacts,
    stdin_name: Option<String>,
) -> u8 {
    let (schema, op) = cmd::schema_and_op(command, open);
    if let Err(e) = init_logging(globals, &env) {
        return cmd::fail(out, schema, &e, ApiOp::Other, &[], &[]);
    }
    let progress = ProgressLine::new(globals.quiet);
    let code = match cmd::run_without_service(out, command) {
        Some(code) => code,
        None => {
            let init = ServiceInit {
                env,
                config_dir: globals.config_dir.clone(),
                threads: None,
                stdin: Some(StdinSource::new(stdin_name, read_stdin)),
                clock: Arc::new(SystemClock),
                runner: Arc::new(StdProcessRunner),
            };
            match Service::new(init) {
                Ok(service) => {
                    out.warnings(service.startup_warnings());
                    // The panic boundary of the command (§5.6): a panic becomes exit 100 with the command's
                    // failure document (unless stdout already carries output).
                    let dispatched = run_guarded(|| {
                        let mut ctx = cmd::Ctx {
                            service: &service,
                            out: &mut *out,
                            progress: &progress,
                            matches: leaf_matches(matches),
                        };
                        Ok(cmd::dispatch(&mut ctx, command, open))
                    });
                    match dispatched {
                        Ok(code) => code,
                        Err(e) => {
                            progress.clear();
                            cmd::fail(out, schema, &e, op, service.startup_warnings(), &[])
                        }
                    }
                }
                Err(e) => cmd::fail(out, schema, &e, ApiOp::Other, &[], &[]),
            }
        }
    };
    progress.clear();
    code
}

/// The index of the first `--` after the program name (everything after it is an operand), or the length.
fn options_end(argv: &[OsString]) -> usize {
    argv.iter()
        .skip(1)
        .position(|a| a == "--")
        .map_or(argv.len(), |k| k + 1)
}

/// The `gui open` arguments exactly as written after `open` (D-51), minus the global flags clap took directly
/// after `open`, and the index where they start in `argv` (the end of the part clap parsed).
///
/// `values` are clap's trailing values, which always end the command line; clap consumes a `--` that stands
/// before the first value, so it is restored: `hikasa gui open -- -x a.c` reaches the shared parser as
/// `-- -x a.c`, like `hikasa-gui -- -x a.c`. clap never takes `--` as the value of a flag, so a `--` right
/// before the values is that separator.
fn raw_open_args(argv: &[OsString], values: &[OsString]) -> (Vec<OsString>, usize) {
    let start = argv.len().saturating_sub(values.len());
    if start < 2 || argv[start..] != *values {
        return (values.to_vec(), start);
    }
    if argv[start - 1] == "--" {
        let mut raw = Vec::with_capacity(values.len() + 1);
        raw.push(OsString::from("--"));
        raw.extend_from_slice(values);
        return (raw, start - 1);
    }
    (values.to_vec(), start)
}

/// True when the `gui open` arguments ask for JSON in a spelling the shared parser accepts (`--json`, `-json`,
/// `/json` on Windows) before their `--`: honoured for the usage error of arguments that do not parse.
fn asks_for_json(raw: &[OsString]) -> bool {
    normalize_legacy_args(raw, Platform::HOST)
        .iter()
        .take_while(|a| *a != "--")
        .any(|a| a == "--json")
}

/// A global value flag given more than once must have the same value each time (§1.8.7 item 34): clap refuses a
/// repeat at one level but keeps the innermost value when the flag is given at several levels (`hikasa
/// --config-dir A format list --config-dir B`), so the repeats are found on the command line itself. `parsed` is
/// the part clap parsed (without the `gui open` arguments, which `merge_open_flags` compares).
fn check_repeated_globals(parsed: &[OsString], cwd: Option<&Path>) -> Result<(), String> {
    for flag in GLOBAL_VALUE_FLAGS {
        let values = flag_values(parsed, flag);
        if let Some((first, rest)) = values.split_first()
            && let Some(other) = rest.iter().find(|v| !same_path(first, v, cwd))
        {
            return Err(conflict(flag, first, other));
        }
    }
    Ok(())
}

/// The values of the long flag `flag` in `args` (`flag v` and `flag=v`), up to the first `--`.
fn flag_values(args: &[OsString], flag: &str) -> Vec<PathBuf> {
    let mut values = Vec::new();
    let mut iter = args.iter().skip(1);
    while let Some(arg) = iter.next() {
        if arg == "--" {
            break;
        }
        if arg == flag {
            if let Some(value) = iter.next() {
                values.push(PathBuf::from(value));
            }
        } else if let Some(value) = inline_value(arg, flag) {
            values.push(PathBuf::from(value));
        }
    }
    values
}

/// The value of `--flag=value` (exact, also when it is not valid UTF-8).
fn inline_value(arg: &std::ffi::OsStr, flag: &str) -> Option<OsString> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let rest = arg.as_bytes().strip_prefix(flag.as_bytes())?;
        let value = rest.strip_prefix(b"=")?;
        Some(std::ffi::OsStr::from_bytes(value).to_os_string())
    }
    #[cfg(not(unix))]
    {
        let text = arg.to_str()?;
        text.strip_prefix(flag)?
            .strip_prefix('=')
            .map(OsString::from)
    }
}

/// Two values of a path flag name the same path: equal, equal once made absolute against `cwd` (`cfg` and
/// `<cwd>/cfg`; `Path` equality ignores `.` components and a trailing separator), or the same path once the
/// symbolic links of their longest existing ancestors are resolved (also for a path that does not exist yet:
/// on macOS `/var/…/cfg` and the current folder's `/private/var/…/cfg` are one folder). An empty value is only
/// equal to an empty value (the service refuses it).
fn same_path(a: &Path, b: &Path, cwd: Option<&Path>) -> bool {
    let empty = |p: &Path| p.as_os_str().is_empty();
    if empty(a) || empty(b) {
        return empty(a) && empty(b);
    }
    let absolute = |p: &Path| match cwd {
        Some(cwd) if p.is_relative() => cwd.join(p),
        Some(_) | None => p.to_path_buf(),
    };
    let (a, b) = (absolute(a), absolute(b));
    a == b
        || matches!(
            (resolve_existing(&a), resolve_existing(&b)),
            (Some(x), Some(y)) if x == y
        )
}

/// `path` with its longest existing ancestor canonicalised and the rest appended (`None` when no ancestor
/// resolves).
fn resolve_existing(path: &Path) -> Option<PathBuf> {
    let mut rest = Vec::new();
    let mut current = path;
    loop {
        if let Ok(resolved) = std::fs::canonicalize(current) {
            return Some(
                rest.iter()
                    .rev()
                    .fold(resolved, |acc: PathBuf, part| acc.join(part)),
            );
        }
        rest.push(current.file_name()?.to_owned());
        current = current.parent()?;
    }
}

/// The usage error of a flag given twice with different values.
fn conflict(flag: &str, a: &Path, b: &Path) -> String {
    format!(
        "`{flag}` is given twice with different values ({} and {})",
        a.display(),
        b.display()
    )
}

/// Reads all of standard input (called by the service at most once, on the first `-` spec).
fn read_stdin() -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    std::io::stdin().lock().read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// The `--stdin-name` of the commands that read files.
fn stdin_name(command: &Command) -> Option<String> {
    match command {
        Command::Quick(args) => args.stdin_name.clone(),
        Command::Text {
            cmd: TextCmd::Compare(args),
        } => args.stdin_name.clone(),
        Command::Format {
            cmd: FormatCmd::Detect(args),
        } => args.stdin_name.clone(),
        Command::Format {
            cmd: FormatCmd::Tokens(args),
        } => args.stdin_name.clone(),
        Command::Folder { .. }
        | Command::Format { .. }
        | Command::Vfs { .. }
        | Command::Gui { .. }
        | Command::Completions { .. }
        | Command::Version => None,
    }
}

/// The matches of the innermost subcommand (where the leaf command's arguments live).
fn leaf_matches(matches: &ArgMatches) -> &ArgMatches {
    let mut leaf = matches;
    while let Some((_, sub)) = leaf.subcommand() {
        leaf = sub;
    }
    leaf
}

/// Merges the output and configuration flags given after the first `gui open` argument with the global ones
/// (§1.9.2 amendment): booleans combine; a value given both ways must name the same path.
fn merge_open_flags(
    globals: &mut GlobalArgs,
    open: &OpenArgs,
    cwd: Option<&Path>,
) -> Result<(), String> {
    globals.json |= open.json;
    globals.quiet |= open.quiet;
    globals.no_color |= open.no_color;
    globals.verbose |= open.verbose;
    merge_path(
        &mut globals.config_dir,
        open.config_dir.as_ref(),
        GLOBAL_VALUE_FLAGS[0],
        cwd,
    )?;
    merge_path(
        &mut globals.log,
        open.log.as_ref(),
        GLOBAL_VALUE_FLAGS[1],
        cwd,
    )
}

/// One value flag of [`merge_open_flags`].
fn merge_path(
    global: &mut Option<PathBuf>,
    open: Option<&PathBuf>,
    flag: &str,
    cwd: Option<&Path>,
) -> Result<(), String> {
    match (global.as_ref(), open) {
        (Some(a), Some(b)) if !same_path(a, b, cwd) => Err(conflict(flag, a, b)),
        (None, Some(b)) => {
            *global = Some(b.clone());
            Ok(())
        }
        (Some(_) | None, _) => Ok(()),
    }
}

/// A clap error. `--help` prints clap's help and exits 0 (also with `--json`: help is text). `--version` at any
/// level prints what `hikasa version` prints, `hikasa <version>` or with `--json` the `hikasa.version.v1`
/// document, and exits 0. Everything else is a usage error: exit 100 (never clap's 2), clap's message on stderr
/// and, with `--json`, `hikasa.usage_error.v1` on stdout.
fn clap_error(error: &clap::Error, json: bool) -> u8 {
    use clap::error::ErrorKind as Kind;
    match error.kind() {
        Kind::DisplayHelp => {
            let _ = error.print();
            SUCCESS
        }
        Kind::DisplayVersion => {
            let mut out = Output::new(json, false, true, true);
            let code = cmd::version::run(&mut out);
            match out.finish() {
                Ok(()) => code,
                Err(e) => {
                    out.error(&e);
                    FAILURE
                }
            }
        }
        _ => {
            if json {
                let mut stdout = std::io::stdout().lock();
                let _ = JsonEnvelope::usage_error(&usage_text(error)).write_to(&mut stdout);
                let _ = stdout.flush();
            }
            let _ = error.print();
            FAILURE
        }
    }
}

/// The message of a clap error in one line: the error text up to clap's first blank line (the usage and tips
/// follow it), without the `error: ` prefix, e.g. `the following required arguments were not provided: <RIGHT>`.
fn usage_text(error: &clap::Error) -> String {
    let rendered = error.render().to_string();
    let text = rendered
        .lines()
        .map(str::trim)
        .take_while(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    match text.strip_prefix("error: ") {
        Some(message) => message.to_owned(),
        None if text.is_empty() => "invalid command line".to_owned(),
        None => text,
    }
}

/// A usage error found after clap (a repeated global, the `gui open` arguments): one stderr line, the
/// usage-error document with `--json`, exit 100.
fn usage_message(message: &str, json: bool) -> u8 {
    if json {
        let mut stdout = std::io::stdout().lock();
        let _ = JsonEnvelope::usage_error(message).write_to(&mut stdout);
        let _ = stdout.flush();
    }
    stderr_line(&format!("error: {message}"));
    FAILURE
}

/// Installs the tracing subscriber (before `Service::new`, which logs ignored option keys): stderr at WARN (ERROR
/// with `--quiet`, DEBUG with `--verbose` when there is no `--log`), and the `--log` file at INFO (DEBUG with
/// `--verbose`), appended to.
///
/// A log file that cannot be opened is the VFS error of its path (`ApiError::Vfs`, exit 100).
fn init_logging(globals: &GlobalArgs, env: &EnvFacts) -> Result<(), ApiError> {
    use tracing_subscriber::Layer as _;
    use tracing_subscriber::filter::{LevelFilter, Targets};
    use tracing_subscriber::layer::SubscriberExt as _;
    use tracing_subscriber::util::SubscriberInitExt as _;

    let stderr_level = if globals.quiet {
        LevelFilter::ERROR
    } else if globals.verbose && globals.log.is_none() {
        LevelFilter::DEBUG
    } else {
        LevelFilter::WARN
    };
    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_ansi(colour_enabled(
            std::io::stderr().is_terminal(),
            globals.no_color,
            env.no_color,
        ))
        .without_time()
        .with_filter(
            Targets::new()
                .with_default(stderr_level)
                .with_target(PANIC_TARGET, LevelFilter::OFF),
        );
    let file_layer = match &globals.log {
        Some(path) => {
            let file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .map_err(|e| {
                    // A folder is `is_a_directory` everywhere, as in LocalVfs (Windows reports opening one
                    // for writing as "access denied").
                    if std::fs::metadata(path).is_ok_and(|m| m.is_dir()) {
                        return ApiError::Vfs(VfsError::IsADirectory {
                            spec: path.display().to_string(),
                        });
                    }
                    ApiError::Vfs(VfsError::from_io(
                        path.display().to_string(),
                        VfsOp::Write,
                        e,
                    ))
                })?;
            let level = if globals.verbose {
                LevelFilter::DEBUG
            } else {
                LevelFilter::INFO
            };
            LOGGING_TO_FILE.store(true, Ordering::SeqCst);
            Some(
                tracing_subscriber::fmt::layer()
                    .with_writer(std::sync::Mutex::new(file))
                    .with_ansi(false)
                    .with_filter(Targets::new().with_default(level)),
            )
        }
        None => None,
    };
    tracing_subscriber::registry()
        .with(stderr_layer)
        .with(file_layer)
        .try_init()
        .map_err(|e| ApiError::Internal(format!("cannot install the logger: {e}")))
}

/// The panic hook (§5.6): one line on stderr, the backtrace in the `--log` file. The panic itself is caught by
/// the panic boundary and exits 100 (D-35).
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let payload = info.payload();
        let message = payload
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "(no message)".to_owned());
        let location = info
            .location()
            .map(|l| format!(" at {}:{}", l.file(), l.line()))
            .unwrap_or_default();
        stderr_line(&format!("internal error: panic{location}: {message}"));
        if LOGGING_TO_FILE.load(Ordering::SeqCst) {
            let backtrace = std::backtrace::Backtrace::force_capture();
            tracing::error!(target: PANIC_TARGET, "panic{location}: {message}\n{backtrace}");
        }
    }));
}

#[cfg(test)]
mod tests {
    use hikasa_session::ApiError;

    use super::*;

    #[test]
    fn open_flags_merge_with_the_globals() {
        let mut globals = GlobalArgs {
            config_dir: Some(PathBuf::from("/c")),
            ..GlobalArgs::default()
        };
        let open = OpenArgs {
            json: true,
            verbose: true,
            log: Some(PathBuf::from("/l.log")),
            config_dir: Some(PathBuf::from("/c")),
            ..OpenArgs::default()
        };
        merge_open_flags(&mut globals, &open, None).unwrap();
        assert!(globals.json && globals.verbose && !globals.quiet);
        assert_eq!(globals.log, Some(PathBuf::from("/l.log")));
        let other = OpenArgs {
            config_dir: Some(PathBuf::from("/other")),
            ..OpenArgs::default()
        };
        let err = merge_open_flags(&mut globals, &other, None).unwrap_err();
        assert!(err.contains("--config-dir"), "{err}");
        // The same folder spelled relative and absolute is the same value.
        let relative = OpenArgs {
            config_dir: Some(PathBuf::from("c")),
            ..OpenArgs::default()
        };
        merge_open_flags(&mut globals, &relative, Some(Path::new("/"))).unwrap();
    }

    fn os(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    #[test]
    fn the_open_arguments_keep_the_separator_clap_consumed() {
        // `gui open -- -x a.c`: clap's values are `-x a.c`, the `--` before them is restored (D-51).
        let argv = os(&["hikasa", "gui", "open", "--", "-x", "a.c"]);
        let (raw, start) = raw_open_args(&argv, &os(&["-x", "a.c"]));
        assert_eq!(raw, os(&["--", "-x", "a.c"]));
        assert_eq!(start, 3);
        // A global flag before the separator stays clap's.
        let argv = os(&["hikasa", "gui", "open", "--json", "--", "-x", "a.c"]);
        assert_eq!(
            raw_open_args(&argv, &os(&["-x", "a.c"])),
            (os(&["--", "-x", "a.c"]), 4)
        );
        // No separator, or one inside the values.
        let argv = os(&["hikasa", "gui", "open", "--json", "a.c", "--", "-x"]);
        let values = os(&["a.c", "--", "-x"]);
        assert_eq!(raw_open_args(&argv, &values), (values.clone(), 4));
        // Only the separator.
        let argv = os(&["hikasa", "gui", "open", "--"]);
        assert_eq!(raw_open_args(&argv, &[]), (os(&["--"]), 3));
        let argv = os(&["hikasa", "gui", "open"]);
        assert_eq!(raw_open_args(&argv, &[]), (Vec::new(), 3));
    }

    #[test]
    fn flags_end_at_the_separator() {
        assert_eq!(options_end(&os(&["hikasa", "quick", "--", "--json"])), 2);
        assert_eq!(options_end(&os(&["hikasa", "quick", "a", "b"])), 4);
        assert_eq!(options_end(&os(&["--", "x"])), 2, "argv[0] is the program");
    }

    #[test]
    fn legacy_json_spellings_are_seen_in_the_open_arguments() {
        assert!(asks_for_json(&os(&["a.c", "b.c", "-json", "--bogus"])));
        assert!(asks_for_json(&os(&["--json", "a.c"])));
        assert!(!asks_for_json(&os(&["a.c", "--", "--json"])));
        assert!(!asks_for_json(&os(&["a.c", "b.c"])));
    }

    #[test]
    fn repeated_global_values_must_agree() {
        let cwd = Some(Path::new("/w"));
        let check = |args: &[&str]| check_repeated_globals(&os(args), cwd);
        assert!(check(&["hikasa", "--config-dir", "c", "format", "list"]).is_ok());
        let err = check(&[
            "hikasa",
            "--config-dir",
            "a",
            "format",
            "list",
            "--config-dir",
            "b",
        ])
        .unwrap_err();
        assert!(
            err.contains("`--config-dir`") && err.contains("a and b"),
            "{err}"
        );
        assert!(
            check(&[
                "hikasa",
                "--config-dir",
                "c",
                "format",
                "--config-dir=/w/c/"
            ])
            .is_ok()
        );
        assert!(check(&["hikasa", "--log=x.log", "version", "--log", "y.log"]).is_err());
        // After `--` nothing is a flag.
        assert!(
            check(&[
                "hikasa",
                "--config-dir",
                "a",
                "quick",
                "--",
                "--config-dir",
                "b"
            ])
            .is_ok()
        );
        // An empty value differs from every other value.
        assert!(check(&["hikasa", "--config-dir", "", "version", "--config-dir", "."]).is_err());
    }

    #[test]
    fn paths_are_compared_absolute() {
        let cwd = Some(Path::new("/w"));
        assert!(same_path(Path::new("c"), Path::new("/w/c"), cwd));
        assert!(same_path(Path::new("./c/"), Path::new("c"), cwd));
        assert!(!same_path(Path::new("c"), Path::new("/w/d"), cwd));
        assert!(!same_path(Path::new("c"), Path::new("/w/c"), None));
        assert!(same_path(Path::new(""), Path::new(""), cwd));
    }

    /// Regression (macOS CI): a path that does not exist yet behind a symbolic link (`/var` → `/private/var`)
    /// is the same path as its resolved spelling.
    #[cfg(unix)]
    #[test]
    fn missing_paths_behind_a_symbolic_link_are_the_same() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        std::fs::create_dir(&real).unwrap();
        std::os::unix::fs::symlink(&real, dir.path().join("link")).unwrap();
        let via_link = dir.path().join("link/cfg");
        assert!(same_path(&via_link, &real.join("cfg"), None));
        assert!(same_path(Path::new("cfg"), &via_link, Some(&real)));
        assert!(!same_path(&via_link, &real.join("other"), None));
    }

    #[test]
    fn usage_messages_keep_clap_s_whole_first_paragraph() {
        let err = Cli::command()
            .try_get_matches_from(["hikasa", "quick", "a.c"])
            .unwrap_err();
        let message = usage_text(&err);
        assert!(
            message.starts_with("the following required arguments were not provided:")
                && message.contains("<RIGHT>"),
            "{message}"
        );
        assert!(
            !message.contains('\n') && !message.contains("Usage"),
            "{message}"
        );
    }

    #[test]
    fn a_panic_in_a_command_exits_100() {
        let err = run_guarded(|| -> Result<u8, ApiError> { panic!("boom") }).unwrap_err();
        assert!(err.to_string().contains("boom"), "{err}");
        for op in [
            ApiOp::Quick,
            ApiOp::FolderCompare,
            ApiOp::GuiOpen { wait: true },
        ] {
            assert_eq!(err.exit_code(op).code(), FAILURE);
        }
    }

    #[test]
    fn the_leaf_matches_hold_the_command_arguments() {
        let argv = [
            "hikasa",
            "text",
            "compare",
            "a",
            "b",
            "--unimportant-text",
            "x",
        ];
        let matches = Cli::command().try_get_matches_from(argv).unwrap();
        let leaf = leaf_matches(&matches);
        assert_eq!(
            leaf.indices_of("unimportant_text").map(Iterator::count),
            Some(1)
        );
        let cli = Cli::from_arg_matches(&matches).unwrap();
        assert_eq!(stdin_name(&cli.command), None);
    }
}
