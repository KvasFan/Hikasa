//! The interpreter loop (§1.8.5–§1.8.9): expansion per command, the script's own state (confirm mode, stop on error,
//! log level), the commands this version refuses (D-221), the error policy and the exit code (D-222). Everything a command
//! means is the host's ([`ScriptHost`]); the interpreter never reads a file, a clock or the environment itself.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::time::Duration;

use hikasa_core::{CancelToken, ExitCode, Progress};
use serde::Serialize;

use crate::ast::{Args, Command, CommandName, OptionArgs, ReportFor, Slot};
use crate::check::script_syntax;
use crate::expand::{ExpandContext, LocalTime, expand, normalize_sides as normalize_word};
use crate::lex::Word;

/// `option confirm:` (§1.8.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConfirmWord {
    /// Ask.
    #[default]
    Prompt,
    /// Yes to every question.
    YesToAll,
    /// The operation yes, every item question no.
    NoToAll,
}

impl ConfirmWord {
    /// The word (ASCII case-insensitive).
    pub fn parse(word: &str) -> Option<ConfirmWord> {
        [
            ("prompt", ConfirmWord::Prompt),
            ("yes-to-all", ConfirmWord::YesToAll),
            ("no-to-all", ConfirmWord::NoToAll),
        ]
        .into_iter()
        .find(|(w, _)| w.eq_ignore_ascii_case(word))
        .map(|(_, mode)| mode)
    }
}

/// `log none|normal|verbose` (§1.8.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    /// Errors only.
    None,
    /// Command lines, summaries, warnings, errors.
    #[default]
    Normal,
    /// Also one line per item.
    Verbose,
}

impl LogLevel {
    /// The word (ASCII case-insensitive).
    pub fn parse(word: &str) -> Option<LogLevel> {
        [
            ("none", LogLevel::None),
            ("normal", LogLevel::Normal),
            ("verbose", LogLevel::Verbose),
        ]
        .into_iter()
        .find(|(w, _)| w.eq_ignore_ascii_case(word))
        .map(|(_, level)| level)
    }

    /// True when a line of `kind` is shown at this level (errors always).
    pub fn shows(self, kind: LineKind) -> bool {
        match kind {
            LineKind::Error => true,
            LineKind::Run | LineKind::Info | LineKind::Warning => self >= LogLevel::Normal,
            LineKind::Detail => self >= LogLevel::Verbose,
        }
    }
}

/// The kind of a log line (§1.8.9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LineKind {
    /// The command as written, after expansion.
    Run,
    /// A summary.
    Info,
    /// One item (verbose).
    Detail,
    /// A warning.
    Warning,
    /// An error.
    Error,
}

impl LineKind {
    /// The word of the line.
    pub const fn as_str(self) -> &'static str {
        match self {
            LineKind::Run => "run",
            LineKind::Info => "info",
            LineKind::Detail => "detail",
            LineKind::Warning => "warning",
            LineKind::Error => "error",
        }
    }
}

/// The interpreter's state a command sees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RunState {
    /// The confirm mode.
    pub confirm: ConfirmWord,
    /// `option stop-on-error`.
    pub stop_on_error: bool,
    /// The log level.
    pub log_level: LogLevel,
    /// No question, prompt, window or notice.
    pub silent: bool,
    /// No plan executed, no file written (T12).
    pub dry_run: bool,
}

/// What the run tells its user interface (§1.8.14).
#[derive(Debug, Clone, PartialEq)]
pub enum ScriptEvent {
    /// The run starts.
    Started {
        /// The script as located.
        script: String,
        /// Its commands.
        commands: u32,
    },
    /// A command starts.
    Command {
        /// From 1.
        index: u32,
        /// Its line.
        line: u32,
        /// Its text, expanded.
        text: String,
    },
    /// The current call's progress.
    Progress {
        /// The report of the call's job.
        status: Progress,
    },
    /// A log line, already filtered by the level.
    Log {
        /// Its kind.
        kind: LineKind,
        /// Its line (`None`: the script itself).
        line: Option<u32>,
        /// Its text.
        text: String,
    },
    /// A question is asked.
    Question {
        /// Its text.
        text: String,
    },
    /// The stop-on-error question is asked.
    Continue {
        /// The line that failed.
        line: u32,
        /// Its message.
        message: String,
    },
    /// `beep`.
    Beep,
    /// The run ended.
    Finished {
        /// The exit code.
        exit_code: u8,
        /// The line it stopped at.
        stopped_at: Option<u32>,
        /// How long it took.
        elapsed: Option<Duration>,
    },
}

/// The user interface of a run: events, the stop-on-error question, beeps (§1.8.7–§1.8.9).
pub trait HostUi {
    /// An event, as it happens.
    fn event(&mut self, event: ScriptEvent);
    /// True when the run can ask (a terminal or a window, and not silent).
    fn can_ask(&self) -> bool;
    /// "The command at line <n> failed: <message>. Continue the script?": true for Yes.
    fn ask_continue(&mut self, line: u32, message: &str) -> bool;
    /// `beep`.
    fn beep(&mut self);
}

/// One command with every word expanded and `lt`/`rt` normalized; words that expanded are checked again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved<'a> {
    /// The command.
    pub command: &'a Command,
    /// Its index, from 0.
    pub index: usize,
    /// The arguments, expanded.
    pub args: Args<OsString>,
    /// The expanded line, as the log shows it.
    pub text: String,
}

/// What one command did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandOutcome {
    /// Done.
    Ok,
    /// Failed: logged, and the script goes on unless stop on error.
    Failed {
        /// The message.
        message: String,
    },
    /// Refused: a command this version does not run (D-221).
    Refused {
        /// The message.
        message: String,
    },
    /// A base could not be loaded: the script ends with 107.
    LoadFailed {
        /// The message.
        message: String,
    },
    /// Cancelled: the script ends.
    Cancelled,
}

/// The host of a run: every vocabulary and every effect.
pub trait ScriptHost {
    /// Judges a word of `slot` that expanded (§1.8.4): `Err` fails the command.
    fn check_resolved(&self, slot: Slot, word: &OsStr) -> Result<(), String>;
    /// Runs one command.
    fn execute(&mut self, cmd: &Resolved<'_>, state: &RunState) -> CommandOutcome;
    /// The local time of the Service's clock in the user's zone.
    fn now(&self) -> LocalTime;
    /// The time since the run started, from the Service's clock; `None` without a clock.
    fn run_time(&self) -> Option<Duration>;
    /// The user interface.
    fn ui(&mut self) -> &mut dyn HostUi;
}

/// What a run starts with.
#[derive(Debug, Clone, Copy)]
pub struct RunOptions<'a> {
    /// `%0`: the script as located.
    pub script: &'a OsStr,
    /// The script as the log names it.
    pub display: &'a str,
    /// `%1`…`%9`.
    pub args: &'a [OsString],
    /// The captured environment.
    pub env: &'a BTreeMap<String, OsString>,
    /// Names are found ASCII case-insensitively (Windows).
    pub env_case_insensitive: bool,
    /// The state before line 1 (`--confirm`, `--stop-on-error`, `--silent`, `--dry-run`).
    pub state: RunState,
}

/// What became of a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandStatus {
    /// Checked, not run (`script check`).
    Checked,
    /// Ran and succeeded.
    Ok,
    /// Ran and failed.
    Failed,
    /// Refused (D-221).
    Refused,
    /// Not run: the script ended before it.
    NotRun,
}

/// One command of a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandRecord {
    /// Its index, from 0.
    pub index: usize,
    /// What became of it.
    pub status: CommandStatus,
    /// The expanded line (run only).
    pub text: Option<String>,
    /// The message of a failure.
    pub message: Option<String>,
}

/// Why a run ended before its last line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// A base could not be loaded (107).
    LoadFailed,
    /// Stop on error.
    StopOnError,
    /// Ctrl-C, Stop.
    Cancelled,
}

/// Where and why a run ended early.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct StopAt {
    /// The line.
    pub line: u32,
    /// Why.
    pub reason: StopReason,
}

/// What a run did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOutcome {
    /// 0, 100 or 107 (D-222).
    pub exit_code: ExitCode,
    /// One per command.
    pub records: Vec<CommandRecord>,
    /// Where it ended early.
    pub stopped_at: Option<StopAt>,
    /// It was cancelled.
    pub cancelled: bool,
    /// How long it took.
    pub elapsed: Option<Duration>,
}

/// `<s.s> s` under a minute, else `<m> min <ss> s` (§1.8.9).
pub fn elapsed_text(elapsed: Duration) -> String {
    let secs = elapsed.as_secs();
    if secs < 60 {
        let tenths = elapsed.as_millis() / 100;
        format!("{}.{} s", tenths / 10, tenths % 10)
    } else {
        format!("{} min {:02} s", secs / 60, secs % 60)
    }
}

/// The bytes of `value` as text: valid UTF-8 as it is, every other byte as `\xNN`.
pub fn display_os(value: &OsStr) -> String {
    let bytes = value.as_encoded_bytes();
    let mut out = String::with_capacity(bytes.len());
    for chunk in bytes.utf8_chunks() {
        out.push_str(chunk.valid());
        for byte in chunk.invalid() {
            out.push_str(&format!("\\x{byte:02X}"));
        }
    }
    out
}

/// The word as a script line shows it after expansion.
fn shown(value: &OsStr, quoted: bool) -> String {
    Word::new(display_os(value), quoted, crate::Pos::START)
        .to_source()
        .replace("%%", "%")
}

/// True for the slots whose words keep their bytes (specs and report targets, §1.8.4).
fn keeps_bytes(slot: Slot) -> bool {
    matches!(slot, Slot::Spec | Slot::ReportTarget)
}

/// The message of a refused command (D-221), or `None`.
fn refusal(command: &Command) -> Option<String> {
    match (command.name, &command.args) {
        (CommandName::Snapshot, _) => Some(
            "snapshot is not available in this version (snapshots arrive with a later release)".to_owned(),
        ),
        (CommandName::FileReport, Args::Report(args)) if args.specs.is_empty() => Some(
            "file-report of the selection is not available in this version: name a session or two files".to_owned(),
        ),
        (name, _) if name.report() == Some(ReportFor::Other) => Some(format!(
            "{} is not available in this version",
            command
                .words
                .first()
                .map_or(name.as_str(), |w| w.text.as_str())
                .to_ascii_lowercase()
        )),
        _ => None,
    }
}

/// Expands the words of `command` (one instant), normalizes their sides and checks again the words that expanded.
fn resolve<'a>(
    command: &'a Command,
    index: usize,
    host: &dyn ScriptHost,
    opts: &RunOptions<'_>,
) -> Result<Resolved<'a>, (String, String)> {
    let ctx = ExpandContext {
        script: opts.script,
        args: opts.args,
        env: opts.env,
        env_case_insensitive: opts.env_case_insensitive,
        now: host.now(),
    };
    // The shown line: every word expanded (the name and the keywords included), as one line.
    let mut text_words = Vec::with_capacity(command.words.len());
    for word in &command.words {
        match expand(word, &ctx) {
            Ok(value) => text_words.push(shown(&value, word.quoted)),
            Err(e) => return Err((command.source(), e.message)),
        }
    }
    let text = text_words.join(" ");
    let args = command
        .args
        .try_map_for(command.name, &mut |slot, word: &Word| {
            let value = expand(word, &ctx).map_err(|e| e.message)?;
            if !word.expands {
                // Checked before the run: only the sides are normalized here.
                let normal = normalize_word(slot, &word.text, word.quoted);
                return Ok(OsString::from(normal.as_ref()));
            }
            let Some(utf8) = value.to_str() else {
                if keeps_bytes(slot) {
                    return Ok(value);
                }
                return Err(format!("`{}` is not valid UTF-8", display_os(&value)));
            };
            let normal = normalize_word(slot, utf8, word.quoted);
            script_syntax(slot, &normal)?;
            host.check_resolved(slot, OsStr::new(normal.as_ref()))?;
            Ok(OsString::from(normal.as_ref()))
        });
    match args {
        Ok(args) => Ok(Resolved {
            command,
            index,
            args,
            text,
        }),
        Err(message) => Err((text, message)),
    }
}

/// Logs a line through the host's interface.
fn log(host: &mut dyn ScriptHost, kind: LineKind, line: Option<u32>, text: String) {
    host.ui().event(ScriptEvent::Log { kind, line, text });
}

/// Runs a checked script (§1.8.5–§1.8.9): each command in order, expanded just before it runs; a failure is logged and
/// the script goes on unless stop on error; a load failure ends it with 107; a cancel ends it with 100.
pub fn run(
    script: &crate::Script,
    host: &mut dyn ScriptHost,
    opts: &RunOptions<'_>,
    cancel: &CancelToken,
) -> RunOutcome {
    let mut state = opts.state;
    let total = script.commands.len();
    host.ui().event(ScriptEvent::Started {
        script: opts.display.to_owned(),
        commands: u32::try_from(total).unwrap_or(u32::MAX),
    });
    let mut start = format!("script {}", opts.display);
    for arg in opts.args {
        start.push(' ');
        start.push_str(&shown(arg, false));
    }
    log(host, LineKind::Info, None, start);
    let mut records: Vec<CommandRecord> = Vec::with_capacity(total);
    let mut stopped_at = None;
    let mut failed = false;
    let mut cancelled = false;
    for (index, command) in script.commands.iter().enumerate() {
        let line = command.pos.line;
        if cancel.is_cancelled() {
            cancelled = true;
            stopped_at = Some(StopAt {
                line,
                reason: StopReason::Cancelled,
            });
            break;
        }
        let (outcome, text) = match resolve(command, index, &*host, opts) {
            Err((text, message)) => {
                host.ui().event(ScriptEvent::Command {
                    index: u32::try_from(index + 1).unwrap_or(u32::MAX),
                    line,
                    text: text.clone(),
                });
                log(host, LineKind::Run, Some(line), text.clone());
                (CommandOutcome::Failed { message }, text)
            }
            Ok(resolved) => {
                host.ui().event(ScriptEvent::Command {
                    index: u32::try_from(index + 1).unwrap_or(u32::MAX),
                    line,
                    text: resolved.text.clone(),
                });
                log(host, LineKind::Run, Some(line), resolved.text.clone());
                let outcome = match (&resolved.args, refusal(command)) {
                    (_, Some(message)) => CommandOutcome::Refused { message },
                    (Args::Option(OptionArgs::StopOnError), _) => {
                        state.stop_on_error = true;
                        CommandOutcome::Ok
                    }
                    (Args::Option(OptionArgs::Confirm(word)), _) => {
                        state.confirm = word
                            .to_str()
                            .and_then(ConfirmWord::parse)
                            .unwrap_or(state.confirm);
                        CommandOutcome::Ok
                    }
                    (Args::Beep, _) => {
                        host.ui().beep();
                        host.ui().event(ScriptEvent::Beep);
                        log(host, LineKind::Info, Some(line), "beep".to_owned());
                        CommandOutcome::Ok
                    }
                    (args, None) => {
                        if let Args::Log(log_args) = args
                            && let Some(level) = log_args
                                .level
                                .as_ref()
                                .and_then(|w| w.to_str())
                                .and_then(LogLevel::parse)
                        {
                            state.log_level = level;
                        }
                        host.execute(&resolved, &state)
                    }
                };
                (outcome, resolved.text)
            }
        };
        let mut record = CommandRecord {
            index,
            status: CommandStatus::Ok,
            text: Some(text),
            message: None,
        };
        let mut message = None;
        match outcome {
            CommandOutcome::Ok => {}
            CommandOutcome::Failed { message: m } => {
                record.status = CommandStatus::Failed;
                message = Some(m);
            }
            CommandOutcome::Refused { message: m } => {
                record.status = CommandStatus::Refused;
                message = Some(m);
            }
            CommandOutcome::LoadFailed { message: m } => {
                record.status = CommandStatus::Failed;
                log(host, LineKind::Error, Some(line), m.clone());
                record.message = Some(m);
                records.push(record);
                stopped_at = Some(StopAt {
                    line,
                    reason: StopReason::LoadFailed,
                });
                break;
            }
            CommandOutcome::Cancelled => {
                record.status = CommandStatus::Failed;
                record.message = Some("cancelled".to_owned());
                log(host, LineKind::Error, Some(line), "cancelled".to_owned());
                records.push(record);
                cancelled = true;
                stopped_at = Some(StopAt {
                    line,
                    reason: StopReason::Cancelled,
                });
                break;
            }
        }
        if let Some(m) = message {
            failed = true;
            log(host, LineKind::Error, Some(line), m.clone());
            record.message = Some(m.clone());
            records.push(record);
            if state.stop_on_error {
                let go_on = if host.ui().can_ask() && !state.silent {
                    host.ui().event(ScriptEvent::Continue {
                        line,
                        message: m.clone(),
                    });
                    host.ui().ask_continue(line, &m)
                } else {
                    false
                };
                if !go_on {
                    stopped_at = Some(StopAt {
                        line,
                        reason: StopReason::StopOnError,
                    });
                    break;
                }
            }
            continue;
        }
        records.push(record);
    }
    for index in records.len()..total {
        records.push(CommandRecord {
            index,
            status: CommandStatus::NotRun,
            text: None,
            message: None,
        });
    }
    let exit_code = match stopped_at {
        Some(StopAt {
            reason: StopReason::LoadFailed,
            ..
        }) => ExitCode::LoadFailed,
        _ if failed || cancelled || stopped_at.is_some() => ExitCode::UnknownError,
        _ => ExitCode::Success,
    };
    let elapsed = host.run_time();
    let end = match (stopped_at, elapsed) {
        (Some(stop), _) => format!(
            "Script stopped at line {} (exit {})",
            stop.line,
            exit_code.code()
        ),
        (None, Some(elapsed)) => format!(
            "Script completed in {} (exit {})",
            elapsed_text(elapsed),
            exit_code.code()
        ),
        (None, None) => format!("Script completed (exit {})", exit_code.code()),
    };
    log(host, LineKind::Info, None, end);
    host.ui().event(ScriptEvent::Finished {
        exit_code: exit_code.code(),
        stopped_at: stopped_at.map(|s| s.line),
        elapsed,
    });
    RunOutcome {
        exit_code,
        records,
        stopped_at,
        cancelled,
        elapsed,
    }
}
