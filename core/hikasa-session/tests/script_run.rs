//! A8.6 (semantics, D-219), A8.7's Service rows (exit codes, D-222) and A8.14 (cancellation) on temp folders through the
//! Service: one test per row of §1.8.6 that the host carries, the error policy and the stop lines.

mod common;
#[path = "script_common/mod.rs"]
mod script_common;

use std::sync::Arc;

use hikasa_core::{CancelToken, ExitCode, JobKind, Progress, ProgressSink};
use hikasa_session::api::script::{ScriptRunRequest, StopReason};
use script_common::{Ui, World, statuses, yes};

/// `l/a.txt` differs from `r/a.txt` (size), `l/sub/x.txt` differs too, `l/only.txt` is a left orphan, `r/r.txt` a right one.
fn world() -> World {
    let w = World::new();
    w.file("l/a.txt", "left a\n", 2_000_000);
    w.file("r/a.txt", "right a, longer\n", 1_000_000);
    w.file("l/sub/x.txt", "left x\n", 2_000_000);
    w.file("r/sub/x.txt", "right x, longer\n", 1_000_000);
    w.file("l/only.txt", "only\n", 1_000_000);
    w.file("r/r.txt", "r\n", 1_000_000);
    w
}

fn selected(ui: &Ui) -> Vec<String> {
    ui.lines
        .iter()
        .filter(|l| l.ends_with("items selected"))
        .cloned()
        .collect()
}

#[test]
fn a8_6_expand_is_needed_before_select_sees_subfolders_and_masks_are_ored() {
    let w = world();
    let (outcome, ui) = w.run(
        "load l r\nselect left.diff.files\nexpand all\nselect left.diff.files\nselect left.diff.files left.orphan.files\n",
    );
    assert_eq!(outcome.exit_code, ExitCode::Success, "{:?}", ui.lines);
    assert_eq!(
        selected(&ui),
        [
            "info L2 1 items selected",
            "info L4 2 items selected",
            "info L5 3 items selected"
        ]
    );
}

#[test]
fn a8_6_a_copy_refreshes_so_the_next_select_sees_the_tree_as_it_is() {
    let w = world();
    let (outcome, ui) = w.run(
        "load l r\nexpand all\nselect left.diff.files\ncopy left->right\nselect left.diff.files\n",
    );
    assert_eq!(outcome.exit_code, ExitCode::Success, "{:?}", ui.lines);
    assert_eq!(
        selected(&ui),
        ["info L3 2 items selected", "info L5 0 items selected"]
    );
    assert_eq!(
        std::fs::read_to_string(w.path("r/a.txt")).unwrap(),
        "left a\n"
    );
    assert_eq!(
        std::fs::read_to_string(w.path("r/sub/x.txt")).unwrap(),
        "left x\n"
    );
    // `nothing_selected` is a warning of the command, not an error.
    let last = outcome.commands.last().unwrap();
    assert_eq!(last.warnings[0].code.as_str(), "nothing_selected");
}

#[test]
fn a8_6_criteria_after_a_load_reruns_it_keeping_expansion_and_selection_by_path() {
    let w = world();
    // Same size, other bytes: different by `crc`, the same by `size` alone.
    w.file("l/s.txt", "aaaa\n", 1_000_000);
    w.file("r/s.txt", "bbbb\n", 1_000_000);
    let (outcome, ui) = w.run(
        "criteria crc\nload l r\nexpand all\nselect left.diff.files\ncriteria size\nselect left.diff.files\n",
    );
    assert_eq!(outcome.exit_code, ExitCode::Success, "{:?}", ui.lines);
    assert_eq!(
        selected(&ui),
        ["info L4 3 items selected", "info L6 2 items selected"]
    );
}

#[test]
fn a8_6_filter_after_a_load_reruns_it() {
    let w = world();
    let (_, ui) = w.run("load l r\nexpand all\nselect all.orphan.files\nfilter -only.txt\nselect all.orphan.files\n");
    assert_eq!(
        selected(&ui),
        ["info L3 2 items selected", "info L5 1 items selected"],
        "{:?}",
        ui.lines
    );
}

#[test]
fn a8_6_sync_ignores_the_selection() {
    let w = world();
    let (outcome, ui) = w.run("load l r\nselect left.orphan.files\nsync update:left->right\n");
    assert_eq!(outcome.exit_code, ExitCode::Success, "{:?}", ui.lines);
    assert_eq!(
        std::fs::read_to_string(w.path("r/a.txt")).unwrap(),
        "left a\n"
    );
    assert_eq!(
        std::fs::read_to_string(w.path("r/sub/x.txt")).unwrap(),
        "left x\n"
    );
    assert!(w.path("r/only.txt").exists());
    // Update keeps the right orphan.
    assert!(w.path("r/r.txt").exists());
}

#[test]
fn a8_6_a_sync_without_direction_and_no_session_fails() {
    let w = world();
    let (outcome, ui) = w.run("load l r\nsync\n");
    assert_eq!(outcome.exit_code, ExitCode::UnknownError);
    assert!(
        ui.lines
            .contains(&"error L2 no sync direction and the session holds none".to_owned()),
        "{:?}",
        ui.lines
    );
}

#[test]
fn a8_6_compare_crc_and_touch_and_create() {
    let w = world();
    let (outcome, ui) = w.run(
        "load create:right l r2\nexpand all\nselect all\ncompare crc\ntouch left:2026-01-02T03:04:05Z\n",
    );
    assert_eq!(outcome.exit_code, ExitCode::Success, "{:?}", ui.lines);
    assert!(w.path("r2").is_dir());
    assert!(
        ui.lines.iter().any(|l| l.starts_with("info L1 created ")),
        "{:?}",
        ui.lines
    );
    assert_eq!(
        outcome.commands[3].schema,
        Some("hikasa.folder_contents.v1")
    );
    let modified = std::fs::metadata(w.path("l/a.txt"))
        .unwrap()
        .modified()
        .unwrap();
    let secs = modified
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    assert_eq!(secs, 1_767_323_045);
}

#[test]
fn a8_6_load_default_drops_the_comparison_and_its_settings() {
    let w = world();
    let (outcome, ui) = w.run("criteria size\nload l r\nload <default>\nbeep\n");
    assert_eq!(outcome.exit_code, ExitCode::Success, "{:?}", ui.lines);
}

/// A script, its exit code, where it stopped, the statuses of its commands.
type Row = (
    &'static str,
    ExitCode,
    Option<(u32, StopReason)>,
    &'static [&'static str],
);

/// A8.7: one row per cause of §1.8.7 the Service decides (105 and 106 are `script_locate`/`script_check`'s, in
/// script_check.rs and cli_script.rs).
#[test]
fn a8_7_exit_codes_stop_lines_and_not_run() {
    let rows: &[Row] = &[
        (
            "load l r\nexpand all\n",
            ExitCode::Success,
            None,
            &["ok", "ok"],
        ),
        (
            "load l missing\nbeep\n",
            ExitCode::LoadFailed,
            Some((1, StopReason::LoadFailed)),
            &["failed", "not_run"],
        ),
        (
            "load l sftp://deploy@host/var/www\nbeep\n",
            ExitCode::LoadFailed,
            Some((1, StopReason::LoadFailed)),
            &["failed", "not_run"],
        ),
        (
            "load l backup/project.HKSNAP\nbeep\n",
            ExitCode::LoadFailed,
            Some((1, StopReason::LoadFailed)),
            &["failed", "not_run"],
        ),
        (
            "load l r\ncopyto all -\nload l nothere\nbeep\n",
            ExitCode::LoadFailed,
            Some((3, StopReason::LoadFailed)),
            &["ok", "failed", "failed", "not_run"],
        ),
        (
            "snapshot left\nbeep\n",
            ExitCode::UnknownError,
            None,
            &["refused", "ok"],
        ),
        (
            "option stop-on-error\nsnapshot left\nbeep\n",
            ExitCode::UnknownError,
            Some((2, StopReason::StopOnError)),
            &["ok", "refused", "not_run"],
        ),
    ];
    for (source, code, stop, words) in rows {
        let w = world();
        let (outcome, ui) = w.run(source);
        assert_eq!(outcome.exit_code, *code, "{source}: {:?}", ui.lines);
        assert_eq!(
            outcome.stopped_at.map(|s| (s.line, s.reason)),
            *stop,
            "{source}"
        );
        assert_eq!(statuses(&outcome), *words, "{source}: {:?}", ui.lines);
    }
    let w = world();
    let (_, ui) = w.run("load l sftp://deploy@host/var/www\n");
    assert!(
        ui.lines.contains(&"error L1 sftp://deploy@host/var/www: remote folders are not available in this version".to_owned()),
        "{:?}",
        ui.lines
    );
    let (_, ui) = w.run("load l backup/project.hksnap\n");
    assert!(
        ui.lines.contains(
            &"error L1 backup/project.hksnap: snapshots are not available in this version"
                .to_owned()
        ),
        "{:?}",
        ui.lines
    );
}

#[test]
fn a8_7_a_declined_question_and_no_terminal_fail_the_command_and_write_nothing() {
    let w = world();
    let before = w.dump("r");
    let mut ui = Ui::default();
    let req = ScriptRunRequest {
        json: true,
        ..ScriptRunRequest::default()
    };
    let outcome = w.run_with(
        "load l r\nselect left.diff.files\ncopy left->right\nbeep\n",
        req.clone(),
        &mut ui,
    );
    assert_eq!(outcome.exit_code, ExitCode::UnknownError);
    assert_eq!(statuses(&outcome), ["ok", "ok", "failed", "ok"]);
    assert!(
        ui.lines
            .iter()
            .any(|l| l.starts_with("error L3 ") && l.contains("there is no terminal")),
        "{:?}",
        ui.lines
    );
    assert_eq!(w.dump("r"), before);
    // A terminal that answers Cancel declines that command only.
    let mut ui = Ui {
        can_ask: true,
        ..Ui::default()
    };
    let outcome = w.run_with(
        "load l r\nselect left.diff.files\ncopy left->right\nbeep\n",
        req,
        &mut ui,
    );
    assert_eq!(statuses(&outcome), ["ok", "ok", "failed", "ok"]);
    assert!(!ui.questions.is_empty());
    assert_eq!(w.dump("r"), before);
}

/// A progress sink that cancels the run once `n` items of an operation were reported done.
struct CancelAfter {
    n: u64,
    token: CancelToken,
}

impl ProgressSink for CancelAfter {
    fn report(&self, progress: Progress) {
        if progress.phase == hikasa_core::Phase::Writing && progress.done >= self.n {
            self.token.cancel();
        }
    }
}

#[test]
fn a8_14_a_cancel_during_a_copy_stops_it_and_the_rest_is_not_run() {
    let w = World::new();
    for i in 0..40 {
        w.file(&format!("l/f{i:02}.txt"), &"x".repeat(100), 1_000_000);
    }
    w.dir("r");
    let token = CancelToken::new();
    let job = hikasa_session::jobs::JobControl {
        cancel: token.clone(),
        progress: Arc::new(CancelAfter {
            n: 1,
            token: token.clone(),
        }),
        ..common::job(&w.service, JobKind::Script)
    };
    let mut ui = Ui::default();
    let outcome = w.run_job(
        "load l r\nselect all\ncopy left->right\nbeep\n",
        yes(),
        &mut ui,
        &job,
        None,
    );
    assert_eq!(outcome.exit_code, ExitCode::UnknownError, "{:?}", ui.lines);
    assert!(outcome.cancelled);
    assert_eq!(
        outcome.stopped_at.map(|s| s.reason),
        Some(StopReason::Cancelled)
    );
    assert_eq!(statuses(&outcome), ["ok", "ok", "failed", "not_run"]);
    let copied = w.dump("r").len();
    assert!(copied < 40, "the copy stopped ({copied} copied)");
    assert!(
        ui.lines
            .last()
            .unwrap()
            .starts_with("info - Script stopped at line 3"),
        "{:?}",
        ui.lines
    );
    // No temporary file is left.
    assert!(
        w.dump("r")
            .iter()
            .all(|(name, _)| !name.contains(".hikasa")),
        "{:?}",
        w.dump("r")
    );
}

#[test]
fn a8_14_a_cancel_while_a_question_waits_ends_the_run() {
    let w = world();
    let token = CancelToken::new();
    let job = hikasa_session::jobs::JobControl {
        cancel: token.clone(),
        ..common::job(&w.service, JobKind::Script)
    };
    let mut ui = Ui {
        can_ask: true,
        cancel_at_question: Some(token),
        ..Ui::default()
    };
    let req = ScriptRunRequest::default();
    let before = w.dump("r");
    let outcome = w.run_job(
        "load l r\nselect all\ndelete right\nbeep\n",
        req,
        &mut ui,
        &job,
        None,
    );
    assert!(outcome.cancelled, "{:?}", ui.lines);
    assert_eq!(statuses(&outcome), ["ok", "ok", "failed", "not_run"]);
    assert_eq!(w.dump("r"), before);
}
