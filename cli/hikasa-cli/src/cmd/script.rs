//! `hikasa script run|check` (M2 W8a, §1.8.12, D-226): locate and load through `Service::script_locate`/`script_load`,
//! check through `Service::script_check_for`, run through `Service::script_run` with this process's interface (the log
//! stream on stderr, W6's questions on a terminal, BEL, the clipboard provider). Every word is the Service's (T11).

use std::ffi::OsString;
use std::io::{self, IsTerminal, Write};

use hikasa_core::{CancelToken, ExitCode, JobKind};
use hikasa_folder::ops::Confirmation;
use hikasa_session::ApiError;
use hikasa_session::api::dto::{JsonEnvelope, SCRIPT_SCHEMA};
use hikasa_session::api::script::{
    HostUi, LineKind, STDOUT_SHARED, ScriptCheck, ScriptEvent, ScriptRunRequest, ScriptText,
    ScriptUi,
};
use hikasa_session::api::script_log;
use hikasa_session::api::{ApiOp, parse_confirm_mode};
use hikasa_session::clipboard::{ClipboardPayload, ClipboardProvider};

use super::Ctx;
use crate::args::{ScriptCheckArgs, ScriptCmd, ScriptRunArgs};
use crate::confirm;
use crate::output::stderr_line;
use crate::progress::ProgressLine;

/// The operation of the exit codes: 105, 106, 107.
const OP: ApiOp = ApiOp::Script;

/// Runs `script run` or `script check`.
pub fn run(ctx: &mut Ctx<'_>, cmd: &ScriptCmd) -> u8 {
    match cmd {
        ScriptCmd::Run(args) => run_script(ctx, args),
        ScriptCmd::Check(args) => check_script(ctx, args),
    }
}

/// Locates and reads the script (105).
fn load(ctx: &Ctx<'_>, file: &OsString) -> Result<ScriptText, ApiError> {
    let source = ctx.service.script_locate(file)?;
    let stdin = io::stdin();
    let mut lock = stdin.lock();
    ctx.service.script_load(source, Some(&mut lock))
}

/// Prints the errors of a check and fails with 106 (or 105 for a script too large): the lines on stderr, the document
/// with `--json` (`ok: false`, `error` = the first error, `result` with every error).
fn check_failed(
    ctx: &mut Ctx<'_>,
    text: &ScriptText,
    check: &ScriptCheck,
    args: &[OsString],
) -> u8 {
    let error = check
        .error(text)
        .unwrap_or_else(|| ApiError::Internal("a failed check without errors".to_owned()));
    if error.exit_code(OP) == ExitCode::ScriptLoadError {
        return ctx.fail(SCRIPT_SCHEMA, &error, OP, &[]);
    }
    ctx.progress.clear();
    {
        let mut err = io::stderr().lock();
        for line in check.errors.lines(&text.display) {
            let _ = writeln!(err, "{}", hikasa_report::escape_controls(&line));
        }
        let _ = err.flush();
    }
    ctx.out.error(&error);
    if ctx.out.json() {
        let mut report = error.report();
        if let Some(first) = check.errors.first() {
            report.message = first.line(&text.display);
        }
        let envelope = JsonEnvelope {
            schema: SCRIPT_SCHEMA,
            hikasa_version: hikasa_session::HIKASA_VERSION,
            ok: false,
            exit_code: ExitCode::ScriptSyntaxError.code(),
            result: Some(check.dto(text, args)),
            warnings: ctx.warnings(&[]),
            error: Some(report),
        };
        let _ = ctx.out.envelope(&envelope);
    }
    ExitCode::ScriptSyntaxError.code()
}

/// `script check`: exit 0, 105 or 106; `ok: <n> commands` on stdout.
fn check_script(ctx: &mut Ctx<'_>, args: &ScriptCheckArgs) -> u8 {
    let text = match load(ctx, &args.file) {
        Ok(text) => text,
        Err(e) => return ctx.fail(SCRIPT_SCHEMA, &e, OP, &[]),
    };
    let references = ctx.service.script_references(&text);
    let env = hikasa_session::capture_script_env(&references.env);
    let given = (!args.args.is_empty()).then_some(args.args.as_slice());
    let refusal = ctx.out.json().then_some(STDOUT_SHARED);
    let check = ctx.service.script_check_for(&text, given, &env, refusal);
    if !check.errors.is_empty() {
        return check_failed(ctx, &text, &check, &args.args);
    }
    if ctx.out.json() {
        let dto = check.dto(&text, &args.args);
        return ctx.json_success(SCRIPT_SCHEMA, 0, dto, &[], OP);
    }
    // The count only: the document's command list is not built (G14).
    let written = ctx
        .out
        .lines([format!("ok: {} commands", check.commands())]);
    ctx.human_done(written, 0, SCRIPT_SCHEMA, &[], OP)
}

/// The interface of a headless run: the log stream on stderr (errors only with `--quiet`), questions on a terminal, BEL
/// on a terminal's stderr.
struct CliUi<'a> {
    progress: &'a ProgressLine,
    quiet: bool,
    /// A terminal on stdin, and not silent.
    terminal: bool,
    cancel: CancelToken,
    clipboard: &'a dyn ClipboardProvider,
    /// Scripted answers (debug builds, `HIKASA_TEST_ANSWERS`): the test's stand-in for a person at a terminal.
    scripted: Option<std::collections::VecDeque<String>>,
}

impl HostUi for CliUi<'_> {
    fn event(&mut self, event: ScriptEvent) {
        let ScriptEvent::Log { kind, line, text } = event else {
            return;
        };
        if self.quiet && kind != LineKind::Error {
            return;
        }
        self.progress.clear();
        let mut err = io::stderr().lock();
        let _ = writeln!(err, "{}", script_log::line(kind, line, &text));
        let _ = err.flush();
    }

    fn can_ask(&self) -> bool {
        self.terminal
    }

    fn ask_continue(&mut self, line: u32, message: &str) -> bool {
        self.progress.clear();
        let question =
            format!("The command at line {line} failed: {message}. Continue the script?");
        let mut err = io::stderr();
        match &mut self.scripted {
            Some(lines) => confirm::ask_yes_no_from(&question, &mut || lines.pop_front(), &mut err),
            None => confirm::ask_yes_no(&question, &self.cancel, &mut err),
        }
    }

    fn beep(&mut self) {
        if !self.quiet && io::stderr().is_terminal() {
            let mut err = io::stderr();
            let _ = err.write_all(b"\x07");
            let _ = err.flush();
        }
    }
}

impl ScriptUi for CliUi<'_> {
    fn answer(&mut self, machine: &mut Confirmation, cancel: &CancelToken) {
        self.progress.clear();
        let mut err = io::stderr();
        match &mut self.scripted {
            Some(lines) => confirm::ask_with(machine, &mut || lines.pop_front(), &mut err),
            None => confirm::ask(machine, cancel, &mut err),
        }
        let _ = err.flush();
    }

    fn clipboard(&mut self, payload: &ClipboardPayload) -> Result<(), ApiError> {
        self.clipboard
            .write_payload(payload)
            .map_err(|e| ApiError::Vfs(e.to_vfs_write()))
    }
}

/// The scripted answers of a test (debug builds only): `HIKASA_TEST_ANSWERS`, lines separated by `;`.
fn scripted_answers() -> Option<std::collections::VecDeque<String>> {
    if !cfg!(debug_assertions) {
        return None;
    }
    let raw = std::env::var("HIKASA_TEST_ANSWERS").ok()?;
    Some(raw.split(';').map(|line| format!("{line}\n")).collect())
}

/// `script run`.
fn run_script(ctx: &mut Ctx<'_>, args: &ScriptRunArgs) -> u8 {
    if args.gui {
        let error = ApiError::Unsupported(
            "script run --gui is not available in this build yet: run the script headless"
                .to_owned(),
        );
        return ctx.fail(SCRIPT_SCHEMA, &error, OP, &[]);
    }
    let confirm = match args.confirm.as_deref().map(parse_confirm_mode).transpose() {
        Ok(mode) => mode,
        Err(e) => return ctx.fail(SCRIPT_SCHEMA, &e, OP, &[]),
    };
    let text = match load(ctx, &args.file) {
        Ok(text) => text,
        Err(e) => return ctx.fail(SCRIPT_SCHEMA, &e, OP, &[]),
    };
    let references = ctx.service.script_references(&text);
    let env = hikasa_session::capture_script_env(&references.env);
    let json = ctx.out.json();
    let refusal = json.then_some(STDOUT_SHARED);
    let check = ctx
        .service
        .script_check_for(&text, Some(&args.args), &env, refusal);
    let Some(script) = check.script.as_ref() else {
        return check_failed(ctx, &text, &check, &args.args);
    };
    let request = ScriptRunRequest {
        args: args.args.clone(),
        env,
        silent: args.silent,
        stop_on_error: args.stop_on_error,
        confirm,
        dry_run: args.dry_run,
        stdout_reports: !json,
        json,
        items_max: test_items_max(),
    };
    let job = ctx.job(JobKind::Script);
    let scripted = scripted_answers();
    let terminal = !args.silent && (scripted.is_some() || io::stdin().is_terminal());
    let mut ui = CliUi {
        progress: ctx.progress,
        quiet: ctx.out.quiet(),
        terminal,
        cancel: job.cancel.clone(),
        clipboard: ctx.clipboard,
        scripted,
    };
    let outcome = if json {
        ctx.service
            .script_run(script, &text, &request, &mut ui, None, &job)
    } else {
        let stdout: &mut dyn Write = ctx.out.stdout();
        ctx.service
            .script_run(script, &text, &request, &mut ui, Some(stdout), &job)
    };
    ctx.progress.clear();
    let code = outcome.exit_code.code();
    if json {
        let dto = outcome.dto(&text, &args.args, args.dry_run, args.silent);
        return ctx.json_success(SCRIPT_SCHEMA, code, dto, &outcome.warnings, OP);
    }
    if code != 0 {
        let failed = outcome
            .commands
            .iter()
            .filter(|c| {
                matches!(
                    c.status,
                    hikasa_session::api::script::CommandStatus::Failed
                        | hikasa_session::api::script::CommandStatus::Refused
                )
            })
            .count();
        let why = match outcome.stopped_at {
            Some(stop) => format!("script {} stopped at line {}", text.display, stop.line),
            None => format!("script {}: {failed} commands failed", text.display),
        };
        stderr_line(&format!("error: {why} (exit {code})"));
    }
    let _ = ctx.out.finish();
    code
}

/// The cap of the JSON item lists: `HIKASA_TEST_SCRIPT_ITEMS_MAX` in debug builds (a test lowers it), else the default.
fn test_items_max() -> usize {
    if !cfg!(debug_assertions) {
        return 0;
    }
    std::env::var("HIKASA_TEST_SCRIPT_ITEMS_MAX")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}
