//! The interpreter loop over a fake host: the error policy and exit codes of §1.8.7 (A8.7's pure rows), stop on error,
//! the refusals of D-221, cancellation, the state commands and the log's start and end lines.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::time::Duration;

use hikasa_core::{CancelToken, ExitCode};
use hikasa_script::{
    AnyWord, CommandOutcome, CommandStatus, ConfirmWord, HostUi, LineKind, LocalTime, Resolved,
    RunOptions, RunOutcome, RunState, ScriptEvent, ScriptHost, Slot, StopReason, parse, run,
};

#[derive(Default)]
struct Ui {
    events: Vec<ScriptEvent>,
    can_ask: bool,
    answer: bool,
    asked: u32,
    beeps: u32,
}

impl HostUi for Ui {
    fn event(&mut self, event: ScriptEvent) {
        self.events.push(event);
    }
    fn can_ask(&self) -> bool {
        self.can_ask
    }
    fn ask_continue(&mut self, _line: u32, _message: &str) -> bool {
        self.asked += 1;
        self.answer
    }
    fn beep(&mut self) {
        self.beeps += 1;
    }
}

/// Fails the lines in `fail`, fails loading at `load_fail`, cancels at `cancel_at`; records what ran.
#[derive(Default)]
struct Host {
    ui: Ui,
    fail: Vec<u32>,
    load_fail: Option<u32>,
    cancel_at: Option<(u32, CancelToken)>,
    ran: Vec<(u32, String, RunState)>,
    refuse_expanded: bool,
    /// Each read of the clock is one second later (one instant per command must read it once).
    moving_clock: bool,
    clock: std::cell::Cell<u8>,
    /// The masks of each `select` as the host got them.
    masks: Vec<Vec<OsString>>,
}

impl ScriptHost for Host {
    fn check_resolved(&self, _slot: Slot, word: &OsStr) -> Result<(), String> {
        if self.refuse_expanded && word == "bad" {
            return Err("`bad` refused".to_owned());
        }
        Ok(())
    }
    fn execute(&mut self, cmd: &Resolved<'_>, state: &RunState) -> CommandOutcome {
        let line = cmd.command.pos.line;
        self.ran.push((line, cmd.text.clone(), *state));
        if let hikasa_script::Args::Select { masks } = &cmd.args {
            self.masks.push(masks.clone());
        }
        if let Some((at, token)) = &self.cancel_at
            && *at == line
        {
            token.cancel();
            return CommandOutcome::Cancelled;
        }
        if self.load_fail == Some(line) {
            return CommandOutcome::LoadFailed {
                message: "x: remote folders are not available in this version".to_owned(),
            };
        }
        if self.fail.contains(&line) {
            return CommandOutcome::Failed {
                message: format!("line {line} failed"),
            };
        }
        CommandOutcome::Ok
    }
    fn now(&self) -> LocalTime {
        if !self.moving_clock {
            return LocalTime::default();
        }
        let second = self.clock.get();
        self.clock.set(second + 1);
        LocalTime {
            second,
            ..LocalTime::default()
        }
    }
    fn run_time(&self) -> Option<Duration> {
        Some(Duration::from_millis(2_400))
    }
    fn ui(&mut self) -> &mut dyn HostUi {
        &mut self.ui
    }
}

fn go(source: &str, host: &mut Host, state: RunState, cancel: &CancelToken) -> RunOutcome {
    go_with(source, host, state, cancel, &["A1"])
}

fn go_with(
    source: &str,
    host: &mut Host,
    state: RunState,
    cancel: &CancelToken,
    args: &[&str],
) -> RunOutcome {
    let script = parse(source.as_bytes(), &AnyWord).unwrap();
    let env = BTreeMap::new();
    let args: Vec<OsString> = args.iter().map(OsString::from).collect();
    let opts = RunOptions {
        script: OsStr::new("s.txt"),
        display: "s.txt",
        args: &args,
        env: &env,
        env_case_insensitive: false,
        state,
    };
    run(&script, host, &opts, cancel)
}

fn statuses(outcome: &RunOutcome) -> Vec<CommandStatus> {
    outcome.records.iter().map(|r| r.status).collect()
}

fn log_lines(ui: &Ui) -> Vec<String> {
    ui.events
        .iter()
        .filter_map(|e| match e {
            ScriptEvent::Log { kind, line, text } => Some(format!(
                "{} {} {text}",
                kind.as_str(),
                line.map_or("-".to_owned(), |l| format!("L{l}"))
            )),
            _ => None,
        })
        .collect()
}

const S: &str = "load a b\nselect all\ncopy left->right\nbeep\n";

#[test]
fn all_ok_is_0() {
    let mut host = Host::default();
    let outcome = go(S, &mut host, RunState::default(), &CancelToken::new());
    assert_eq!(outcome.exit_code, ExitCode::Success);
    assert_eq!(host.ran.len(), 3, "beep is the interpreter's");
    assert_eq!(host.ui.beeps, 1);
    assert_eq!(
        log_lines(&host.ui),
        vec![
            "info - script s.txt A1",
            "run L1 load a b",
            "run L2 select all",
            "run L3 copy left->right",
            "run L4 beep",
            "info L4 beep",
            "info - Script completed in 2.4 s (exit 0)",
        ]
    );
}

#[test]
fn a_failure_continues_and_ends_100() {
    let mut host = Host {
        fail: vec![2],
        ..Host::default()
    };
    let outcome = go(S, &mut host, RunState::default(), &CancelToken::new());
    assert_eq!(outcome.exit_code, ExitCode::UnknownError);
    assert_eq!(outcome.stopped_at, None);
    use CommandStatus::*;
    assert_eq!(statuses(&outcome), vec![Ok, Failed, Ok, Ok]);
    assert!(log_lines(&host.ui).contains(&"error L2 line 2 failed".to_owned()));
}

#[test]
fn a_load_failure_ends_107_even_after_a_failure_and_without_stop_on_error() {
    let mut host = Host {
        fail: vec![1],
        load_fail: Some(2),
        ..Host::default()
    };
    let source = "beep\nload a b\nselect all\ncopy left->right\n";
    let outcome = go(source, &mut host, RunState::default(), &CancelToken::new());
    assert_eq!(outcome.exit_code, ExitCode::LoadFailed);
    let stop = outcome.stopped_at.unwrap();
    assert_eq!((stop.line, stop.reason), (2, StopReason::LoadFailed));
    use CommandStatus::*;
    assert_eq!(statuses(&outcome), vec![Ok, Failed, NotRun, NotRun]);
    assert_eq!(
        log_lines(&host.ui).last().unwrap(),
        "info - Script stopped at line 2 (exit 107)"
    );
}

#[test]
fn stop_on_error_asks_where_it_can() {
    let source = "option stop-on-error\nload a b\nselect all\ncopy left->right\n";
    use CommandStatus::*;
    // Yes: continues.
    let mut host = Host {
        fail: vec![3],
        ..Host::default()
    };
    host.ui.can_ask = true;
    host.ui.answer = true;
    let outcome = go(source, &mut host, RunState::default(), &CancelToken::new());
    assert_eq!(
        (outcome.exit_code, host.ui.asked),
        (ExitCode::UnknownError, 1)
    );
    assert_eq!(statuses(&outcome), vec![Ok, Ok, Failed, Ok]);
    // No: ends.
    let mut host = Host {
        fail: vec![3],
        ..Host::default()
    };
    host.ui.can_ask = true;
    let outcome = go(source, &mut host, RunState::default(), &CancelToken::new());
    assert_eq!(statuses(&outcome), vec![Ok, Ok, Failed, NotRun]);
    assert_eq!(outcome.stopped_at.unwrap().reason, StopReason::StopOnError);
    // Silent, or nothing to ask with: ends without asking.
    for (silent, can_ask) in [(true, true), (false, false)] {
        let mut host = Host {
            fail: vec![3],
            ..Host::default()
        };
        host.ui.can_ask = can_ask;
        host.ui.answer = true;
        let state = RunState {
            silent,
            ..RunState::default()
        };
        let outcome = go(source, &mut host, state, &CancelToken::new());
        assert_eq!(host.ui.asked, 0);
        assert_eq!(statuses(&outcome), vec![Ok, Ok, Failed, NotRun]);
    }
    // `--stop-on-error` before line 1 is the same.
    let mut host = Host {
        fail: vec![1],
        ..Host::default()
    };
    let state = RunState {
        stop_on_error: true,
        ..RunState::default()
    };
    let outcome = go(S, &mut host, state, &CancelToken::new());
    assert_eq!(outcome.stopped_at.unwrap().line, 1);
}

#[test]
fn the_refused_commands_log_their_message_and_the_run_goes_on() {
    let source = "snapshot save-crc left\nmedia-report\n\
                  version-report\nregistry-report\nfile-report output-to:x\nbeep\n";
    let mut host = Host::default();
    let outcome = go(source, &mut host, RunState::default(), &CancelToken::new());
    assert_eq!(outcome.exit_code, ExitCode::UnknownError);
    assert!(host.ran.is_empty());
    let errors: Vec<String> = log_lines(&host.ui)
        .into_iter()
        .filter(|l| l.starts_with("error"))
        .collect();
    assert_eq!(
        errors,
        vec![
            "error L1 snapshot is not available in this version (snapshots arrive with a later release)",
            "error L2 media-report is not available in this version",
            "error L3 version-report is not available in this version",
            "error L4 registry-report is not available in this version",
            "error L5 file-report of the selection is not available in this version: name a session or two files",
        ]
    );
    assert_eq!(outcome.records[5].status, CommandStatus::Ok);
    assert!(
        outcome.records[..5]
            .iter()
            .all(|r| r.status == CommandStatus::Refused)
    );
}

#[test]
fn a_cancel_ends_the_run_and_the_rest_is_not_run() {
    let token = CancelToken::new();
    let mut host = Host {
        cancel_at: Some((2, token.clone())),
        ..Host::default()
    };
    let outcome = go(S, &mut host, RunState::default(), &token);
    assert_eq!(outcome.exit_code, ExitCode::UnknownError);
    assert!(outcome.cancelled);
    use CommandStatus::*;
    assert_eq!(statuses(&outcome), vec![Ok, Failed, NotRun, NotRun]);
    assert_eq!(outcome.stopped_at.unwrap().reason, StopReason::Cancelled);
    // A token cancelled before the run runs nothing.
    let token = CancelToken::new();
    token.cancel();
    let mut host = Host::default();
    let outcome = go(S, &mut host, RunState::default(), &token);
    assert!(host.ran.is_empty());
    assert_eq!(statuses(&outcome), vec![NotRun; 4]);
}

#[test]
fn option_confirm_changes_the_mode_from_its_line_on() {
    let source = "load a b\noption confirm:yes-to-all\ncopy left->right\noption confirm:NO-TO-ALL\ncopy left->right\n";
    let mut host = Host::default();
    let state = RunState {
        confirm: ConfirmWord::Prompt,
        ..RunState::default()
    };
    go(source, &mut host, state, &CancelToken::new());
    let modes: Vec<ConfirmWord> = host.ran.iter().map(|(_, _, s)| s.confirm).collect();
    assert_eq!(
        modes,
        vec![
            ConfirmWord::Prompt,
            ConfirmWord::YesToAll,
            ConfirmWord::NoToAll
        ]
    );
}

#[test]
fn an_expanded_word_is_judged_when_it_runs() {
    let mut host = Host {
        refuse_expanded: true,
        ..Host::default()
    };
    let script = parse(b"load a b\nselect %1\nbeep\n", &AnyWord).unwrap();
    let env = BTreeMap::new();
    let args: Vec<OsString> = vec!["bad".into()];
    let opts = RunOptions {
        script: OsStr::new("s.txt"),
        display: "s.txt",
        args: &args,
        env: &env,
        env_case_insensitive: false,
        state: RunState::default(),
    };
    let outcome = run(&script, &mut host, &opts, &CancelToken::new());
    assert_eq!(outcome.records[1].status, CommandStatus::Failed);
    assert_eq!(outcome.records[1].text.as_deref(), Some("select bad"));
    assert_eq!(outcome.records[2].status, CommandStatus::Ok);
    assert_eq!(host.ran.len(), 1);
}

#[test]
fn the_elapsed_forms() {
    assert_eq!(hikasa_script::elapsed_text(Duration::ZERO), "0.0 s");
    assert_eq!(
        hikasa_script::elapsed_text(Duration::from_millis(59_990)),
        "59.9 s"
    );
    assert_eq!(
        hikasa_script::elapsed_text(Duration::from_secs(61)),
        "1 min 01 s"
    );
    assert_eq!(
        hikasa_script::elapsed_text(Duration::from_secs(3_725)),
        "62 min 05 s"
    );
    assert!(LineKind::Detail != LineKind::Info);
}

#[test]
fn a_load_whose_word_is_refused_when_it_runs_ends_107() {
    // A1: the second load's word is refused after expansion; nothing may run on the first comparison.
    let mut host = Host {
        refuse_expanded: true,
        ..Host::default()
    };
    let source = "load l r\nload %1 other\nselect all\ndelete right\n";
    let outcome = go_with(
        source,
        &mut host,
        RunState::default(),
        &CancelToken::new(),
        &["bad"],
    );
    assert_eq!(outcome.exit_code, ExitCode::LoadFailed);
    let stop = outcome.stopped_at.unwrap();
    assert_eq!((stop.line, stop.reason), (2, StopReason::LoadFailed));
    use CommandStatus::*;
    assert_eq!(statuses(&outcome), vec![Ok, Failed, NotRun, NotRun]);
    assert_eq!(host.ran.len(), 1, "only the first load ran");
}

#[test]
fn a_percent_pair_in_an_argument_is_shown_as_it_is() {
    // A9: `%%` written in the script is one `%`; a `%%` inside an argument's value is the value, shown as it is.
    let mut host = Host::default();
    let outcome = go_with(
        "load a b\nselect %1 50%%\n",
        &mut host,
        RunState::default(),
        &CancelToken::new(),
        &["50%%"],
    );
    assert_eq!(outcome.exit_code, ExitCode::Success);
    assert_eq!(outcome.records[1].text.as_deref(), Some("select 50%% 50%"));
    assert_eq!(
        host.masks[0],
        vec![OsString::from("50%%"), OsString::from("50%")]
    );
    assert!(log_lines(&host.ui).contains(&"run L2 select 50%% 50%".to_owned()));
}

#[test]
fn one_instant_per_command_even_when_the_clock_moves() {
    // Every dynamic name of one line shows the same instant: the clock is read once per command.
    let mut host = Host {
        moving_clock: true,
        ..Host::default()
    };
    let outcome = go(
        "load a b\nselect %time% x%fn_time% %TIME%\nselect %time%\n",
        &mut host,
        RunState::default(),
        &CancelToken::new(),
    );
    assert_eq!(outcome.exit_code, ExitCode::Success);
    let first = &host.masks[0];
    assert_eq!(first.len(), 3);
    let time = first[0].to_str().unwrap().to_owned();
    assert_eq!(first[2].to_str().unwrap(), time);
    assert_eq!(
        first[1].to_str().unwrap(),
        format!("x{}", time.replace(':', "-"))
    );
    assert_eq!(
        outcome.records[1].text.as_deref(),
        Some(format!("select {time} x{} {time}", time.replace(':', "-")).as_str())
    );
    // The next command reads the clock again: another instant.
    assert_ne!(host.masks[1][0].to_str().unwrap(), time);
}
