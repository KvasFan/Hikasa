//! Scripts in the window (M2 W8b, §1.8.14, D-229; the minimal part of SCR-004): [`ScriptDialog`] is the status window
//! of a run, [`ContinueDialog`] its stop-on-error question, [`RunScriptDialog`] the Run Script dialog of the catalogue
//! command `RunScript`. Toolkit-free models (§1.4.16 rule 4, architecture check 7); `ui/script_dialog.rs` draws them.
//!
//! A run is [`prepare`]d (located, read and checked: the lookup of §1.8.11 and the check of `script check`, with the
//! GUI's refusal of `output-to:-`), then started on a job thread of the GUI ([`ScriptDialog::start`], the job machinery
//! of D-115) with the GUI's `Service` (another current folder for a `RunScript` of the command line,
//! `Service::with_cwd`). The run's interface ([`WorkerUi`]) posts every [`ScriptEvent`] to the dialog through a
//! channel and wakes the window; W6's questions and the stop-on-error question block the job thread until the dialog
//! stacked on the status window answers (D-112). One script runs at a time: the [`ScriptClaim`] of [`Shared`] is held
//! from the check until the run ended (`RunScript` while it is held: `busy`).
//!
//! *W8b notes:* Stop cancels the run's job as Ctrl-C does (the current call stops as its CLI command does, the rest is
//! `not_run`); the window's close button is Stop while the script runs, and the window stays. The log lines are those
//! of the stream (§1.8.9, without the date), the last `[tweaks.log_panel] display_up_to` kept. A beep asks the window
//! for attention ([`DialogRequest::Beep`]): no sound crate.

use std::any::Any;
use std::collections::{BTreeMap, VecDeque};
use std::ffi::{OsStr, OsString};
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender};
use hikasa_core::{CancelToken, ExitCode, JobKind, Phase, Progress, Unit};
use hikasa_folder::ops::{ConfirmMode, Confirmation, Reply};
use hikasa_session::api::script::{
    HostUi, LineKind, ProgressEvents, Script, ScriptEvent, ScriptRunRequest, ScriptText, ScriptUi,
    elapsed_text,
};
use hikasa_session::api::script_log;
use hikasa_session::clipboard::{ClipboardPayload, ClipboardProvider};
use hikasa_session::ipc::{ScriptSummary, ScriptWaiter};
use hikasa_session::log::LogKind;
use hikasa_session::{ApiError, Service};

use crate::dialog::{ButtonRole, DialogButton, DialogModel, DialogRequest, DialogStep};
use crate::job::{Job, Wake};
use crate::notify::{Notice, NoticeAction, NoticeKind};
use crate::op_dialogs::PlanConfirmDialog;
use crate::shared::Shared;

/// The help topic of the status window and the Run Script dialog.
pub const SCRIPT_TOPIC: &str = "dialog-script";

/// The message of a second script while one runs (IPC `busy`, the Run Script dialog).
pub const BUSY: &str = "a script is running in the window; wait for it or stop it";

/// The width of the status window and the height of its body.
const STATUS_SIZE: (f32, f32) = (640.0, 360.0);

/// The ids of the status windows (a question, the stop-on-error question and the end notice name theirs).
static NEXT_STATUS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// One script at a time (§1.8.13): held from the check of a script until its run ended; dropping it frees the slot.
#[derive(Debug)]
pub struct ScriptClaim(Arc<Shared>);

impl ScriptClaim {
    /// The slot, when no script holds it.
    pub fn take(shared: &Arc<Shared>) -> Option<ScriptClaim> {
        shared
            .claim_script()
            .then(|| ScriptClaim(Arc::clone(shared)))
    }
}

impl Drop for ScriptClaim {
    fn drop(&mut self) {
        self.0.release_script();
    }
}

/// A checked script ready to run: what [`prepare`] made, with where its end goes.
pub struct ScriptStart {
    /// The Service the run uses (its current folder is the run's).
    pub service: Service,
    /// The script as read.
    pub text: ScriptText,
    /// The checked script.
    pub script: Script,
    /// The run's request (arguments, environment, stop on error, confirm mode).
    pub request: ScriptRunRequest,
    /// `--closescript` (`Some(true)`); `None`: `close_when_finished` decides.
    pub close: Option<bool>,
    /// The waiter of a `RunScript { wait: true }`.
    pub waiter: Option<ScriptWaiter>,
    /// The slot of the one script that runs.
    pub claim: Option<ScriptClaim>,
}

impl std::fmt::Debug for ScriptStart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScriptStart")
            .field("script", &self.text.display)
            .field("args", &self.request.args)
            .field("close", &self.close)
            .field("waiter", &self.waiter)
            .finish_non_exhaustive()
    }
}

/// Why a script cannot run: it cannot be read (105), it does not check (106, with every error line), or a word of the
/// request is wrong (100).
#[derive(Debug)]
pub struct PrepareError {
    /// The error.
    pub error: ApiError,
    /// The check's lines (`<script>:<line>:<column>: error: <message>`); empty for a script that cannot be read.
    pub lines: Vec<String>,
}

impl PrepareError {
    /// The first line, or the error (the IPC answer, the notice).
    pub fn message(&self) -> String {
        self.lines
            .first()
            .cloned()
            .unwrap_or_else(|| self.error.to_string())
    }
}

/// Locates, reads and checks a script for the window (§1.8.11, §1.8.7): `env` is the environment of a client
/// (`RunScript`), or `None` for this process's (`capture_script_env` of what the script references).
pub fn prepare(
    service: &Service,
    file: &OsStr,
    args: Vec<OsString>,
    env: Option<BTreeMap<String, OsString>>,
    stop_on_error: bool,
    confirm: Option<ConfirmMode>,
    close: Option<bool>,
) -> Result<ScriptStart, Box<PrepareError>> {
    let failed = |error: ApiError| {
        Box::new(PrepareError {
            error,
            lines: Vec::new(),
        })
    };
    let source = service.script_locate(file).map_err(failed)?;
    let text = service.script_load(source, None).map_err(failed)?;
    let env = env.unwrap_or_else(|| {
        hikasa_session::capture_script_env(&service.script_references(&text).env)
    });
    let check = service.script_check(&text, Some(&args), &env);
    let Some(script) = check.script.clone() else {
        let error = check
            .error(&text)
            .unwrap_or_else(|| ApiError::Internal("a failed check without errors".to_owned()));
        return Err(Box::new(PrepareError {
            error,
            lines: check.errors.lines(&text.display),
        }));
    };
    Ok(ScriptStart {
        service: service.clone(),
        text,
        script,
        request: ScriptRunRequest {
            args,
            env,
            stop_on_error,
            confirm,
            ..ScriptRunRequest::default()
        },
        close,
        waiter: None,
        claim: None,
    })
}

/// What the job thread tells the status window.
enum RunMsg {
    /// An event of the run.
    Event(ScriptEvent),
    /// W6's questions of one command: the machine, and where the answered machine goes (dropped: Cancel).
    Ask(Confirmation, Sender<Confirmation>),
    /// The stop-on-error question (dropped: No).
    Continue {
        line: u32,
        message: String,
        reply: Sender<bool>,
    },
    /// `beep`, or `beep_when_finished`.
    Beep,
}

/// The run's interface on the job thread: everything goes to the status window, questions wait for its answer.
struct WorkerUi {
    tx: Sender<RunMsg>,
    wake: Wake,
    clipboard: Arc<dyn ClipboardProvider>,
}

impl WorkerUi {
    fn send(&self, msg: RunMsg) {
        let _ = self.tx.send(msg);
        (self.wake)();
    }
}

impl HostUi for WorkerUi {
    fn event(&mut self, event: ScriptEvent) {
        self.send(RunMsg::Event(event));
    }

    fn can_ask(&self) -> bool {
        true
    }

    fn ask_continue(&mut self, line: u32, message: &str) -> bool {
        let (reply, answer) = crossbeam_channel::bounded(1);
        self.send(RunMsg::Continue {
            line,
            message: message.to_owned(),
            reply,
        });
        answer.recv().unwrap_or(false)
    }

    fn beep(&mut self) {
        self.send(RunMsg::Beep);
    }
}

impl ScriptUi for WorkerUi {
    fn answer(&mut self, machine: &mut Confirmation, _cancel: &CancelToken) {
        if machine.next().is_none() {
            return;
        }
        let (reply, answer) = crossbeam_channel::bounded(1);
        self.send(RunMsg::Ask(machine.clone(), reply));
        match answer.recv() {
            Ok(answered) => *machine = answered,
            // The window stopped the run or went away: the command is declined.
            Err(_) => {
                while machine.next().is_some() {
                    machine.reply(Reply::Cancel);
                }
            }
        }
    }

    fn clipboard(&mut self, payload: &ClipboardPayload) -> Result<(), ApiError> {
        self.clipboard
            .write_payload(payload)
            .map_err(|e| ApiError::Vfs(e.to_vfs_write()))
    }

    fn progress_events(&self) -> Option<ProgressEvents> {
        let tx = self.tx.clone();
        let wake = Arc::clone(&self.wake);
        Some(Arc::new(move |event| {
            let _ = tx.send(RunMsg::Event(event));
            wake();
        }))
    }
}

/// What the job returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RunEnd {
    exit_code: u8,
    summary: ScriptSummary,
}

/// How a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScriptEnd {
    /// 0, 100, 107.
    pub exit_code: u8,
    /// The line it stopped at.
    pub stopped_at: Option<u32>,
    /// How long it took.
    pub elapsed: Option<Duration>,
}

/// The current call's progress as the status bar's job line shows it: the phase and the counts.
pub fn progress_line(progress: &Progress) -> String {
    let unit = match progress.unit {
        Unit::Items => "items",
        Unit::Bytes => "bytes",
        Unit::Lines => "lines",
        Unit::Folders => "folders",
    };
    let phase = phase_word(progress.phase);
    match progress.total {
        Some(total) => format!("{phase}: {} of {total} {unit}", progress.done),
        None => format!("{phase}: {} {unit}", progress.done),
    }
}

fn phase_word(phase: Phase) -> String {
    phase.as_str().replace('_', " ")
}

/// The status window of a run (§1.8.14): the script's path, `Command <i> of <n>, line <l>: <text>`, the current
/// call's progress, the log lines, the end line; *Stop* while it runs, *Close* after.
pub struct ScriptDialog {
    /// This status window's id: its questions and its notice carry it, so their answers come back here.
    id: u64,
    name: String,
    path: String,
    commands: u32,
    current: Option<(u32, u32, String)>,
    progress: Option<String>,
    lines: VecDeque<(LineKind, String)>,
    cap: usize,
    end: Option<ScriptEnd>,
    summary: Option<ScriptSummary>,
    close: Option<bool>,
    close_when_finished: bool,
    job: Option<Job<RunEnd>>,
    rx: Receiver<RunMsg>,
    /// Alive while the run's job body runs (it holds the claim and the waiter until then): the shutdown waits for it.
    alive: std::sync::Weak<()>,
    confirm_reply: Option<Sender<Confirmation>>,
    continue_reply: Option<Sender<bool>>,
    requests: Vec<DialogRequest>,
    beeps: u32,
    stopped: bool,
    /// `ScriptEvent::Progress` received (tests).
    progress_seen: u32,
}

impl std::fmt::Debug for ScriptDialog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScriptDialog")
            .field("path", &self.path)
            .field("current", &self.current)
            .field("end", &self.end)
            .finish_non_exhaustive()
    }
}

impl ScriptDialog {
    /// Starts the run of `start` on a job thread and returns its status window. `display_up_to` caps the lines kept,
    /// `close_when_finished` is OPT-021's option, `clipboard` the window's clipboard (a report's `output-to:clipboard`).
    pub fn start(
        start: ScriptStart,
        display_up_to: usize,
        close_when_finished: bool,
        wake: &Wake,
        clipboard: Arc<dyn ClipboardProvider>,
    ) -> ScriptDialog {
        let ScriptStart {
            service,
            text,
            script,
            request,
            close,
            waiter,
            claim,
        } = start;
        let path = text.display.clone();
        let name = path
            .rsplit(['/', '\\'])
            .next()
            .filter(|n| !n.is_empty())
            .unwrap_or(&path)
            .to_owned();
        let commands = u32::try_from(script.len()).unwrap_or(u32::MAX);
        let (tx, rx) = crossbeam_channel::unbounded();
        let ui_wake = Arc::clone(wake);
        let running = Arc::new(());
        let alive = Arc::downgrade(&running);
        let job = Job::spawn(
            &service,
            JobKind::Script,
            format!("Script {name}"),
            wake,
            move |svc, job| {
                // The job, not the status window, holds the slot and the waiter (review F6): a status window that
                // goes while its run stops does not free the slot, nor answer a waiting client, before the run
                // really ended (a cancelled copy ends its current item first).
                let _running = running;
                let _claim = claim;
                let mut ui = WorkerUi {
                    tx,
                    wake: ui_wake,
                    clipboard,
                };
                let outcome = svc.script_run(&script, &text, &request, &mut ui, None, job);
                let end = RunEnd {
                    exit_code: outcome.exit_code.code(),
                    summary: outcome.summary(),
                };
                if let Some(waiter) = waiter {
                    waiter.script_finished(outcome.exit_code, end.summary);
                }
                Ok(end)
            },
        );
        ScriptDialog {
            id: NEXT_STATUS.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            name,
            path,
            commands,
            current: None,
            progress: None,
            lines: VecDeque::new(),
            cap: display_up_to.max(1),
            end: None,
            summary: None,
            close,
            close_when_finished,
            job: Some(job),
            rx,
            alive,
            confirm_reply: None,
            continue_reply: None,
            requests: Vec::new(),
            beeps: 0,
            stopped: false,
            progress_seen: 0,
        }
    }

    /// The id its questions, its stop-on-error question and its end notice carry.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// The script's file name (the source of its log entries is `script <name>`).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The script as located.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// `Command <i> of <n>, line <l>: <text>` of the command that runs (or ran last).
    pub fn command_line(&self) -> Option<String> {
        self.current.as_ref().map(|(index, line, text)| {
            format!("Command {index} of {}, line {line}: {text}", self.commands)
        })
    }

    /// The current call's progress line.
    pub fn progress(&self) -> Option<&str> {
        self.progress.as_deref()
    }

    /// The log lines kept, oldest first, with their kinds (errors are drawn in the error colour).
    pub fn lines(&self) -> impl Iterator<Item = (LineKind, &str)> {
        self.lines.iter().map(|(kind, text)| (*kind, text.as_str()))
    }

    /// The end, once the run ended.
    pub fn end(&self) -> Option<ScriptEnd> {
        self.end
    }

    /// The counts of the commands, once the run ended.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn summary(&self) -> Option<ScriptSummary> {
        self.summary
    }

    /// The end line (SCR-004's wording): `Script completed in <time>`, `Script stopped at line <n>` or `Script failed
    /// (exit <code>)`.
    pub fn end_line(&self) -> Option<String> {
        let end = self.end?;
        Some(if end.exit_code == 0 {
            format!(
                "Script completed in {}",
                elapsed_text(end.elapsed.unwrap_or_default())
            )
        } else if let Some(line) = end.stopped_at {
            format!("Script stopped at line {line}")
        } else {
            format!("Script failed (exit {})", end.exit_code)
        })
    }

    /// True while the run goes on.
    pub fn running(&self) -> bool {
        self.job.is_some()
    }

    /// Alive while the run's job body runs, also after this status window went (the shutdown waits for it).
    pub fn alive(&self) -> std::sync::Weak<()> {
        self.alive.clone()
    }

    /// Beeps asked so far.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn beeps(&self) -> u32 {
        self.beeps
    }

    /// `ScriptEvent::Progress` events received so far.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn progress_seen(&self) -> u32 {
        self.progress_seen
    }

    /// *Stop*: cancels the run's job (as Ctrl-C does); a question still open is declined.
    pub fn stop(&mut self) {
        if let Some(job) = &self.job {
            job.cancel();
            self.stopped = true;
        }
        self.confirm_reply = None;
        self.continue_reply = None;
    }

    /// The answers of the confirmation dialog stacked on the window (`None`: closed with a question open, a Cancel).
    pub fn answered(&mut self, confirmation: Option<Confirmation>) {
        if let Some(reply) = self.confirm_reply.take()
            && let Some(confirmation) = confirmation
        {
            let _ = reply.send(confirmation);
        }
    }

    /// The answer to the stop-on-error question.
    pub fn continued(&mut self, yes: bool) {
        if let Some(reply) = self.continue_reply.take() {
            let _ = reply.send(yes);
        }
    }

    fn event(&mut self, event: ScriptEvent) {
        match event {
            ScriptEvent::Started { commands, .. } => self.commands = commands,
            ScriptEvent::Command { index, line, text } => {
                self.current = Some((index, line, text));
                self.progress = None;
            }
            ScriptEvent::Progress { status } => {
                self.progress = Some(progress_line(&status));
                self.progress_seen = self.progress_seen.saturating_add(1);
            }
            ScriptEvent::Log { kind, line, text } => {
                let source = format!("script {}", self.name);
                let at = line.map_or_else(|| "-".to_owned(), |l| format!("L{l}"));
                // The log panel (D-116, §1.8.9): a summary per command and the errors.
                match (kind, line) {
                    (LineKind::Info, Some(_)) => self.requests.push(DialogRequest::Log(
                        LogKind::Summary,
                        source,
                        format!("{at} {text}"),
                    )),
                    (LineKind::Error, _) => self.requests.push(DialogRequest::Log(
                        LogKind::Error,
                        source,
                        format!("{at} {text}"),
                    )),
                    _ => {}
                }
                self.lines
                    .push_back((kind, script_log::line(kind, line, &text)));
                while self.lines.len() > self.cap {
                    self.lines.pop_front();
                }
            }
            ScriptEvent::Finished {
                exit_code,
                stopped_at,
                elapsed,
            } => {
                self.end = Some(ScriptEnd {
                    exit_code,
                    stopped_at,
                    elapsed,
                });
                self.progress = None;
            }
            // The questions come as `RunMsg::Ask`/`Continue`; a beep as `RunMsg::Beep`.
            ScriptEvent::Question { .. } | ScriptEvent::Continue { .. } | ScriptEvent::Beep => {}
        }
    }

    /// One event of the run, as the job thread sends it (tests).
    #[cfg(test)]
    pub(crate) fn feed(&mut self, event: ScriptEvent) {
        self.event(event);
    }

    /// One message of the job thread.
    fn take(&mut self, msg: RunMsg) {
        match msg {
            RunMsg::Event(event) => self.event(event),
            RunMsg::Ask(confirmation, reply) => {
                let subject = self
                    .current
                    .as_ref()
                    .map_or_else(String::new, |(_, line, text)| format!("L{line} {text}"));
                let dialog =
                    PlanConfirmDialog::for_script(self.id, self.title(), subject, confirmation);
                self.confirm_reply = Some(reply);
                self.requests.push(DialogRequest::Push(Box::new(dialog)));
            }
            RunMsg::Continue {
                line,
                message,
                reply,
            } => {
                self.continue_reply = Some(reply);
                let dialog = ContinueDialog::new(self.id, line, &message);
                self.requests.push(DialogRequest::Push(Box::new(dialog)));
            }
            RunMsg::Beep => {
                self.beeps += 1;
                self.requests.push(DialogRequest::Beep);
            }
        }
    }

    /// The run's job ended (it answered the waiter and freed the slot): the notice, the close options (OPT-021,
    /// `--closescript`).
    fn ended(&mut self, result: Result<RunEnd, ApiError>) {
        let end = match result {
            Ok(end) => end,
            Err(e) => {
                self.lines
                    .push_back((LineKind::Error, format!("error - {e}")));
                RunEnd {
                    exit_code: ExitCode::UnknownError.code(),
                    summary: ScriptSummary::default(),
                }
            }
        };
        let finished = self.end.get_or_insert(ScriptEnd {
            exit_code: end.exit_code,
            stopped_at: end.summary.stopped_at,
            elapsed: None,
        });
        finished.exit_code = end.exit_code;
        self.summary = Some(end.summary);
        let notice = if end.exit_code == 0 {
            Notice::new(
                format!("script:{}", self.name),
                NoticeKind::Info,
                format!("Script {} completed", self.name),
            )
        } else {
            Notice::new(
                format!("script:{}", self.name),
                NoticeKind::Error,
                format!("Script {} failed: exit {}", self.name, end.exit_code),
            )
            .with_actions(vec![NoticeAction::ShowScript(self.id)])
        };
        self.requests.push(DialogRequest::Notice(notice));
        self.confirm_reply = None;
        self.continue_reply = None;
        let closes = match self.close {
            Some(close) => close,
            None => self.close_when_finished && end.exit_code == 0,
        };
        if closes {
            self.requests.push(DialogRequest::Close);
        }
    }
}

impl Drop for ScriptDialog {
    /// A status window that goes while its script runs (its window closes, the program quits) stops the run; the job
    /// keeps the slot and the waiter until it ended.
    fn drop(&mut self) {
        self.stop();
    }
}

impl DialogModel for ScriptDialog {
    fn title(&self) -> String {
        format!("Script — {}", self.name)
    }

    fn help_topic(&self) -> &'static str {
        SCRIPT_TOPIC
    }

    fn buttons(&self) -> Vec<DialogButton> {
        if self.running() {
            vec![DialogButton {
                label: "Stop".to_owned(),
                role: ButtonRole::Cancel,
                enabled: !self.stopped,
                default: false,
            }]
        } else {
            vec![DialogButton {
                label: "Close".to_owned(),
                role: ButtonRole::Close,
                enabled: true,
                default: true,
            }]
        }
    }

    fn on_button(&mut self, role: ButtonRole, _svc: &Service) -> DialogStep {
        match role {
            ButtonRole::Cancel if self.running() => {
                self.stop();
                DialogStep::Stay
            }
            ButtonRole::Close if !self.running() => DialogStep::Close,
            _ => DialogStep::Stay,
        }
    }

    /// The window's close button is *Stop* while the script runs: the window stays.
    fn on_close(&mut self) -> bool {
        if self.running() {
            self.stop();
            true
        } else {
            false
        }
    }

    fn poll(&mut self, _svc: &Service) -> bool {
        let mut changed = false;
        while let Ok(msg) = self.rx.try_recv() {
            changed = true;
            self.take(msg);
        }
        if let Some(result) = self.job.as_ref().and_then(Job::try_result) {
            self.job = None;
            // What the job sent before it returned is all in the channel by now (the end's beep included).
            while let Ok(msg) = self.rx.try_recv() {
                self.take(msg);
            }
            self.ended(result);
            changed = true;
        }
        changed
    }

    fn size(&self) -> Option<(f32, f32)> {
        Some(STATUS_SIZE)
    }

    fn take_requests(&mut self) -> Vec<DialogRequest> {
        std::mem::take(&mut self.requests)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// The stop-on-error question (§1.8.7): `The command at line <n> failed: <message>. Continue the script?`, Yes or No;
/// Esc and the window's close button are No.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContinueDialog {
    /// The status window whose run asked.
    script: u64,
    message: String,
    answer: Option<bool>,
}

impl ContinueDialog {
    /// The question of the command at `line` of the run of the status window `script`.
    pub fn new(script: u64, line: u32, message: &str) -> ContinueDialog {
        ContinueDialog {
            script,
            message: format!("The command at line {line} failed: {message}. Continue the script?"),
            answer: None,
        }
    }

    /// The status window whose run asked ([`ScriptDialog::id`]).
    pub fn script(&self) -> u64 {
        self.script
    }

    /// The question.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// The answer (No when the dialog was closed without one).
    pub fn take_answer(&mut self) -> bool {
        self.answer.take().unwrap_or(false)
    }
}

impl DialogModel for ContinueDialog {
    fn title(&self) -> String {
        "Stop on Error".to_owned()
    }

    fn help_topic(&self) -> &'static str {
        SCRIPT_TOPIC
    }

    fn buttons(&self) -> Vec<DialogButton> {
        [(ButtonRole::Yes, "Yes"), (ButtonRole::No, "No")]
            .into_iter()
            .map(|(role, label)| DialogButton {
                label: label.to_owned(),
                role,
                enabled: true,
                default: role == ButtonRole::No,
            })
            .collect()
    }

    fn on_button(&mut self, role: ButtonRole, _svc: &Service) -> DialogStep {
        match role {
            ButtonRole::Yes => self.answer = Some(true),
            ButtonRole::No => self.answer = Some(false),
            _ => return DialogStep::Stay,
        }
        DialogStep::Close
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// The confirm modes the Run Script dialog offers, in its order.
pub const CONFIRM_MODES: [(ConfirmMode, &str); 3] = [
    (ConfirmMode::Prompt, "Ask"),
    (ConfirmMode::YesToAll, "Yes to all"),
    (ConfirmMode::NoToAll, "No to all"),
];

/// The Run Script dialog (Tools ▸ Run Script…, §1.8.14): a path (the lookup of §1.8.11), the arguments split by the
/// script lexer's word rules (`Service::script_args`), *Stop on error* and the confirm mode. Run checks the script and
/// shows its errors with their positions instead of running. The browse button arrives with W9's browse dialog.
#[derive(Debug)]
pub struct RunScriptDialog {
    /// The script field.
    pub path: String,
    /// The arguments field.
    pub args: String,
    /// *Stop on error*.
    pub stop_on_error: bool,
    /// The confirm mode.
    pub confirm: ConfirmMode,
    errors: Vec<String>,
    shared: Arc<Shared>,
    start: Option<ScriptStart>,
}

impl RunScriptDialog {
    /// An empty dialog; `shared` holds the slot of the one script that runs.
    pub fn new(shared: Arc<Shared>) -> RunScriptDialog {
        RunScriptDialog {
            path: String::new(),
            args: String::new(),
            stop_on_error: false,
            confirm: ConfirmMode::Prompt,
            errors: Vec::new(),
            shared,
            start: None,
        }
    }

    /// The errors of the last Run (positions included), or why it did not start.
    pub fn errors(&self) -> &[String] {
        &self.errors
    }

    /// The checked script, once Run accepted it (the window starts it).
    pub fn take_start(&mut self) -> Option<ScriptStart> {
        self.start.take()
    }

    fn run(&mut self, svc: &Service) -> DialogStep {
        self.errors.clear();
        if self.path.trim().is_empty() {
            self.errors.push("Name a script file.".to_owned());
            return DialogStep::Stay;
        }
        let args = match svc.script_args(&self.args) {
            Ok(words) => words.into_iter().map(OsString::from).collect(),
            Err(e) => {
                self.errors.push(format!(
                    "arguments:{}:{}: error: {}",
                    e.pos.line, e.pos.column, e.message
                ));
                return DialogStep::Stay;
            }
        };
        let Some(claim) = ScriptClaim::take(&self.shared) else {
            self.errors.push(format!("{}.", capitalized(BUSY)));
            return DialogStep::Stay;
        };
        match prepare(
            svc,
            OsStr::new(self.path.trim()),
            args,
            None,
            self.stop_on_error,
            Some(self.confirm),
            None,
        ) {
            Ok(mut start) => {
                start.claim = Some(claim);
                self.start = Some(start);
                DialogStep::Close
            }
            Err(e) => {
                self.errors = if e.lines.is_empty() {
                    vec![e.error.to_string()]
                } else {
                    e.lines
                };
                DialogStep::Stay
            }
        }
    }
}

/// `text` with a capital first letter.
fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

impl DialogModel for RunScriptDialog {
    fn title(&self) -> String {
        "Run Script".to_owned()
    }

    fn help_topic(&self) -> &'static str {
        SCRIPT_TOPIC
    }

    fn buttons(&self) -> Vec<DialogButton> {
        vec![
            DialogButton {
                label: "Run".to_owned(),
                role: ButtonRole::Ok,
                enabled: !self.path.trim().is_empty(),
                default: true,
            },
            DialogButton {
                label: "Cancel".to_owned(),
                role: ButtonRole::Cancel,
                enabled: true,
                default: false,
            },
        ]
    }

    fn on_button(&mut self, role: ButtonRole, svc: &Service) -> DialogStep {
        match role {
            ButtonRole::Ok => self.run(svc),
            ButtonRole::Cancel => DialogStep::Close,
            _ => DialogStep::Stay,
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ---- `hikasa-gui @script` without a window ---------------------------------------------------------------------

/// The interface of `hikasa-gui @script --silent` (§1.8.13): the log stream on stderr, no question, BEL on a terminal.
struct SilentUi {
    clipboard: Arc<dyn ClipboardProvider>,
}

impl HostUi for SilentUi {
    fn event(&mut self, event: ScriptEvent) {
        if let ScriptEvent::Log { kind, line, text } = event {
            eprintln!("{}", script_log::line(kind, line, &text));
        }
    }

    fn can_ask(&self) -> bool {
        false
    }

    fn ask_continue(&mut self, _line: u32, _message: &str) -> bool {
        false
    }

    fn beep(&mut self) {
        use std::io::{IsTerminal as _, Write as _};
        if std::io::stderr().is_terminal() {
            let mut err = std::io::stderr();
            let _ = err.write_all(b"\x07");
            let _ = err.flush();
        }
    }
}

impl ScriptUi for SilentUi {
    fn answer(&mut self, _machine: &mut Confirmation, _cancel: &CancelToken) {}

    fn clipboard(&mut self, payload: &ClipboardPayload) -> Result<(), ApiError> {
        self.clipboard
            .write_payload(payload)
            .map_err(|e| ApiError::Vfs(e.to_vfs_write()))
    }
}

/// Locates, reads and checks the script of the command line here (105, 106; the error lines on stderr).
fn local_check(
    service: &Service,
    script: &hikasa_session::api::ScriptArgs,
    stdout_refusal: Option<&'static str>,
) -> Result<(ScriptText, Script, BTreeMap<String, OsString>), u8> {
    let fail = |e: ApiError| {
        eprintln!("hikasa-gui: error: {e}");
        e.exit_code(hikasa_session::ApiOp::Script).code()
    };
    let source = service.script_locate(&script.file).map_err(fail)?;
    let mut stdin = std::io::stdin().lock();
    let text = service
        .script_load(source, Some(&mut stdin))
        .map_err(fail)?;
    let env = hikasa_session::capture_script_env(&service.script_references(&text).env);
    let check = service.script_check_for(&text, Some(&script.args), &env, stdout_refusal);
    match check.script.clone() {
        Some(checked) => Ok((text, checked, env)),
        None => {
            for line in check.errors.lines(&text.display) {
                eprintln!("{line}");
            }
            Err(check.exit_code().code())
        }
    }
}

/// `hikasa-gui @script --silent` (D-228): never through a GUI; the script runs headless in this process, as `hikasa
/// script run --silent`, and the process exits with its code. Ctrl-C as there (D-159, review F8): the first stops the
/// current call and the rest is `not_run` (exit 100), a second exits 100 at once.
pub fn run_silently(service: &Service, script: &hikasa_session::api::ScriptArgs) -> u8 {
    let (text, checked, env) = match local_check(service, script, None) {
        Ok(checked) => checked,
        Err(code) => return code,
    };
    let request = ScriptRunRequest {
        args: script.args.clone(),
        env,
        silent: true,
        stdout_reports: true,
        ..ScriptRunRequest::default()
    };
    let ctrl_c = hikasa_session::interrupt::CtrlC::new(
        || eprintln!("Cancelling; press Ctrl-C again to exit at once."),
        || std::process::exit(i32::from(ExitCode::UnknownError.code())),
    );
    let job = hikasa_session::JobControl {
        cancel: ctrl_c.token(),
        ..service.job(JobKind::Script, Arc::new(hikasa_core::NullProgress))
    };
    let mut ui = SilentUi {
        clipboard: hikasa_session::clipboard_provider(service.env()),
    };
    let mut stdout = std::io::stdout().lock();
    let outcome = service.script_run(&checked, &text, &request, &mut ui, Some(&mut stdout), &job);
    outcome.exit_code.code()
}

/// `hikasa-gui @script` with a GUI running (D-228, as D-127's packages): checked here, then forwarded as `RunScript`
/// (this process's cwd and the referenced environment) without waiting; exit 0, or the error's code.
pub fn forward(
    service: &Service,
    script: &hikasa_session::api::ScriptArgs,
    closescript: bool,
) -> u8 {
    let (text, _, env) = match local_check(
        service,
        script,
        Some(hikasa_session::api::script::STDOUT_NONE),
    ) {
        Ok(checked) => checked,
        Err(code) => return code,
    };
    let options = hikasa_session::api::GuiScriptOptions {
        closescript,
        ..hikasa_session::api::GuiScriptOptions::default()
    };
    let op = hikasa_session::ApiOp::GuiOpen { wait: false };
    let sent = service
        .gui_script_request(&text, &script.args, &env, &options)
        .and_then(|request| {
            service.gui_run_script(request, None, &hikasa_session::ipc::RetryPolicy::default())
        });
    match sent {
        Ok(_) => 0,
        Err(e) => {
            eprintln!("hikasa-gui: error: {e}");
            e.exit_code(op).code()
        }
    }
}
