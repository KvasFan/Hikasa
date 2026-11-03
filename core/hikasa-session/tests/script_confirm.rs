//! A8.8 (D-223): the script as the fourth driver of W6's question machine: each mode, silent mode's message, no terminal,
//! `--confirm` before line 1 and `option confirm:` after it, `Cancel` declining one command, D-77's delete question.

mod common;
#[path = "script_common/mod.rs"]
mod script_common;

use hikasa_core::ExitCode;
use hikasa_folder::ops::{ConfirmMode, Reply};
use hikasa_session::api::script::ScriptRunRequest;
use script_common::{Ui, World, statuses};

fn world() -> World {
    let w = World::new();
    w.file("l/a.txt", "new left\n", 2_000_000);
    w.file("r/a.txt", "older right\n", 1_000_000);
    w.file("l/b.txt", "b\n", 1_000_000);
    w
}

fn run(
    w: &World,
    source: &str,
    req: ScriptRunRequest,
    ui: &mut Ui,
) -> hikasa_session::api::script::ScriptOutcome {
    w.run_with(source, req, ui)
}

const COPY: &str = "load l r\nselect left.all.files\ncopy left->right\n";

#[test]
fn a8_8_yes_to_all_asks_nothing_and_copies() {
    let w = world();
    let mut ui = Ui::default();
    let req = ScriptRunRequest {
        confirm: Some(ConfirmMode::YesToAll),
        ..ScriptRunRequest::default()
    };
    let outcome = run(&w, COPY, req, &mut ui);
    assert_eq!(outcome.exit_code, ExitCode::Success, "{:?}", ui.lines);
    assert!(ui.questions.is_empty());
    assert_eq!(
        std::fs::read_to_string(w.path("r/a.txt")).unwrap(),
        "new left\n"
    );
}

#[test]
fn a8_8_prompt_asks_on_a_terminal_and_the_answers_decide() {
    let w = world();
    let mut ui = Ui {
        can_ask: true,
        replies: [Reply::Yes, Reply::YesToAll, Reply::YesToAll].into(),
        ..Ui::default()
    };
    let outcome = run(&w, COPY, ScriptRunRequest::default(), &mut ui);
    assert_eq!(outcome.exit_code, ExitCode::Success, "{:?}", ui.lines);
    assert!(!ui.questions.is_empty());
    assert!(w.path("r/b.txt").exists());
}

#[test]
fn a8_8_silent_declines_with_the_message_of_scr_003() {
    let w = world();
    let before = w.dump("r");
    let mut ui = Ui {
        can_ask: true,
        ..Ui::default()
    };
    let req = ScriptRunRequest {
        silent: true,
        ..ScriptRunRequest::default()
    };
    let outcome = run(&w, COPY, req, &mut ui);
    assert_eq!(outcome.exit_code, ExitCode::UnknownError);
    assert!(ui.questions.is_empty());
    let error = ui
        .lines
        .iter()
        .find(|l| l.starts_with("error L3 "))
        .unwrap();
    assert!(
        error.starts_with("error L3 Attempted to open a dialog when running silently: "),
        "{error}"
    );
    assert_eq!(w.dump("r"), before);
}

#[test]
fn a8_8_no_terminal_declines_with_its_own_message() {
    let w = world();
    let mut ui = Ui::default();
    let outcome = run(&w, COPY, ScriptRunRequest::default(), &mut ui);
    assert_eq!(outcome.exit_code, ExitCode::UnknownError);
    let error = ui
        .lines
        .iter()
        .find(|l| l.starts_with("error L3 "))
        .unwrap();
    assert!(
        error.ends_with(
            "questions need an answer and there is no terminal: use option confirm:yes-to-all or confirm:no-to-all, or --confirm"
        ),
        "{error}"
    );
}

#[test]
fn a8_8_option_confirm_changes_the_mode_from_its_line_on() {
    let w = world();
    let mut ui = Ui::default();
    let req = ScriptRunRequest {
        confirm: Some(ConfirmMode::Prompt),
        ..ScriptRunRequest::default()
    };
    let outcome = run(
        &w,
        "load l r\nselect left.all.files\ncopy left->right\noption confirm:yes-to-all\ncopy left->right\n",
        req,
        &mut ui,
    );
    assert_eq!(
        statuses(&outcome),
        ["ok", "ok", "failed", "ok", "ok"],
        "{:?}",
        ui.lines
    );
    // `no-to-all`: the operation yes, every item question no: the newer target stays.
    let w = world();
    std::fs::write(w.path("r/a.txt"), "much newer right\n").unwrap();
    script_common::set_mtime(&w.path("r/a.txt"), 3_000_000);
    let mut ui = Ui::default();
    let req = ScriptRunRequest {
        confirm: Some(ConfirmMode::NoToAll),
        ..ScriptRunRequest::default()
    };
    let outcome = run(&w, COPY, req, &mut ui);
    assert_eq!(outcome.exit_code, ExitCode::Success, "{:?}", ui.lines);
    assert_eq!(
        std::fs::read_to_string(w.path("r/a.txt")).unwrap(),
        "much newer right\n"
    );
    assert!(w.path("r/b.txt").exists());
}

#[test]
fn a8_8_cancel_declines_that_command_only_and_delete_asks_whatever_confirm_delete_says() {
    let mut w = world();
    std::fs::create_dir_all(w.dir.path().join("config")).unwrap();
    std::fs::write(
        w.dir.path().join("config/options.toml"),
        "[file_operations]\nconfirm_delete = false\n",
    )
    .unwrap();
    w.service = common::service(w.dir.path());
    let mut ui = Ui {
        can_ask: true,
        replies: [Reply::Cancel].into(),
        ..Ui::default()
    };
    let outcome = run(
        &w,
        "load l r\nselect right.all.files\ndelete right\nbeep\n",
        ScriptRunRequest::default(),
        &mut ui,
    );
    assert_eq!(
        statuses(&outcome),
        ["ok", "ok", "failed", "ok"],
        "{:?}",
        ui.lines
    );
    assert_eq!(ui.questions.len(), 1, "D-77: the delete is asked");
    assert!(w.path("r/a.txt").exists());
}
