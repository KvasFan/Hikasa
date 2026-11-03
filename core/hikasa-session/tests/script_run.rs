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
            "load l r\nexpand nothere\nload l nothere\nbeep\n",
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

/// A progress sink that cancels the run at the first report of `phase`.
struct CancelIn {
    phase: hikasa_core::Phase,
    token: CancelToken,
}

impl ProgressSink for CancelIn {
    fn report(&self, progress: Progress) {
        if progress.phase == self.phase {
            self.token.cancel();
        }
    }
}

/// The job of a run cancelled at the first report of `phase`.
fn cancelled_in(w: &World, phase: hikasa_core::Phase) -> hikasa_session::jobs::JobControl {
    let token = CancelToken::new();
    hikasa_session::jobs::JobControl {
        cancel: token.clone(),
        progress: Arc::new(CancelIn { phase, token }),
        ..common::job(&w.service, JobKind::Script)
    }
}

/// A8.14: a cancel during a load's comparison stops it as `folder compare` stops: that line fails cancelled, the rest
/// is not run, exit 100, `Script stopped at line <n>`.
#[test]
fn a8_14_a_cancel_during_a_comparison_stops_the_run_there() {
    let w = world();
    for i in 0..30 {
        w.file(&format!("l/many/f{i:02}.txt"), "x", 1_000_000);
    }
    let job = cancelled_in(&w, hikasa_core::Phase::Scanning);
    let mut ui = Ui::default();
    let outcome = w.run_job(
        "beep\nload l r\nselect all\nbeep\n",
        yes(),
        &mut ui,
        &job,
        None,
    );
    assert_eq!(outcome.exit_code, ExitCode::UnknownError, "{:?}", ui.lines);
    assert!(outcome.cancelled);
    assert_eq!(
        outcome.stopped_at.map(|s| (s.line, s.reason)),
        Some((2, StopReason::Cancelled))
    );
    assert_eq!(statuses(&outcome), ["ok", "failed", "not_run", "not_run"]);
    assert_eq!(
        ui.lines.last().unwrap(),
        "info - Script stopped at line 2 (exit 100)"
    );
    // A8.13: the `hikasa.script.v1` result of a cancelled run (paths of the sandbox as `<TMP>`).
    let text = w.text("beep\nload l r\nselect all\nbeep\n");
    let dto = outcome.dto(&text, &[], false, false);
    let json = serde_json::to_string_pretty(&dto)
        .unwrap()
        .replace(
            &w.dir.path().display().to_string().replace('\\', "\\\\"),
            "<TMP>",
        )
        .replace('\\', "/");
    common::assert_golden(
        &common::fixtures("script").join("cancelled.json"),
        format!("{json}\n").as_bytes(),
    );
}

/// A8.14: a cancel while a report is written stops it as `report` stops: no report file and no temporary file are left,
/// the rest is not run.
#[test]
fn a8_14_a_cancel_during_a_report_write_leaves_no_file() {
    let w = world();
    let before = w.dump("");
    // A linked report reports its writing (the files folder and the pair reports) before its first step.
    let job = cancelled_in(&w, hikasa_core::Phase::Writing);
    let mut ui = Ui::default();
    let outcome = w.run_job(
        "load l r\nfolder-report layout:side-by-side output-options:include-file-links,html-color output-to:rep.html\nbeep\n",
        yes(),
        &mut ui,
        &job,
        None,
    );
    assert!(outcome.cancelled, "{:?}", ui.lines);
    assert_eq!(outcome.exit_code, ExitCode::UnknownError);
    assert_eq!(statuses(&outcome), ["ok", "failed", "not_run"]);
    assert!(!w.path("rep.html").exists());
    let after: Vec<String> = w
        .dump("")
        .into_iter()
        .map(|(name, _)| name)
        .filter(|name| name != "s.txt")
        .collect();
    let before: Vec<String> = before.into_iter().map(|(name, _)| name).collect();
    assert_eq!(after, before, "no temporary file is left");
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

/// A6: a sandbox lives under the test's `TMPDIR` and is removed whole when dropped, even after `attrib +r` left its
/// folders and files read-only.
#[test]
fn a_sandbox_with_read_only_rows_is_removed_when_dropped() {
    let w = world();
    assert!(
        w.dir.path().starts_with(std::env::temp_dir()),
        "{}",
        w.dir.path().display()
    );
    let (outcome, ui) = w.run("load l r\nexpand all\nselect all\nattrib +r\n");
    assert_eq!(outcome.exit_code, ExitCode::Success, "{:?}", ui.lines);
    let root = w.dir.path().to_path_buf();
    assert!(
        std::fs::metadata(w.path("l/sub"))
            .unwrap()
            .permissions()
            .readonly(),
        "the folder row is read-only"
    );
    drop(w);
    assert!(!root.exists(), "{} was left behind", root.display());
}

/// Writes the session `name` of the world's configuration folder (`l` and `r` of `work/` as its bases).
fn session(w: &World, name: &str, kind: &str, specs_extra: &str, rest: &str) {
    let dir = w.dir.path().join("config/sessions");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(format!("{name}.toml")),
        format!(
            "schema_version = 1\ntype = \"{kind}\"\n\n[specs]\nleft = '{}'\nright = '{}'\n{specs_extra}\n{rest}",
            w.path("l").display(),
            w.path("r").display()
        ),
    )
    .unwrap();
}

/// The `<n> items selected` counts of a run.
fn selected_counts(ui: &Ui) -> Vec<String> {
    ui.lines
        .iter()
        .filter(|l| l.ends_with("items selected"))
        .cloned()
        .collect()
}

/// A8.6, SES-010: a Folder Compare session's rules apply (`flatten`: every file is a row of the base) and its
/// `[display]` is cleared (the equal file is selected, though the session shows differences only).
#[test]
fn a8_6_a_folder_compare_session_applies_its_rules_and_not_its_display() {
    let w = world();
    w.file("l/same.txt", "same\n", 1_000_000);
    w.file("r/same.txt", "same\n", 1_000_000);
    session(
        &w,
        "FC",
        "folder-compare",
        "",
        "[rules.handling]\nflatten = true\n\n[display]\nfilter = \"differences\"\n",
    );
    let (outcome, ui) = w.run("load FC\nselect all.files\n");
    assert_eq!(outcome.exit_code, ExitCode::Success, "{:?}", ui.lines);
    // a.txt, only.txt, r.txt, same.txt and sub/x.txt (flat: no expand needed).
    assert_eq!(
        selected_counts(&ui),
        ["info L2 5 items selected"],
        "{:?}",
        ui.lines
    );
    // The same tree loaded as folders, with the default rules: sub/x.txt is inside a closed folder.
    let (_, ui) = w.run("load l r\nselect all.files\n");
    assert_eq!(selected_counts(&ui), ["info L2 4 items selected"]);
}

/// A8.6: a Folder Sync session's preset is the direction of a bare `sync` (here `mirror_right`: the right orphan goes).
#[test]
fn a8_6_a_folder_sync_session_gives_a_bare_sync_its_preset() {
    let w = world();
    session(
        &w,
        "SY",
        "folder-sync",
        "",
        "[rules.sync]\npreset = \"mirror_right\"\n",
    );
    let (outcome, ui) = w.run("load SY\nsync\n");
    assert_eq!(outcome.exit_code, ExitCode::Success, "{:?}", ui.lines);
    assert!(!w.path("r/r.txt").exists(), "{:?}", ui.lines);
    assert_eq!(
        std::fs::read_to_string(w.path("r/only.txt")).unwrap(),
        "only\n"
    );
}

/// A8.6, §1.8.6: a read-only side of a loaded session refuses a write: the command fails, nothing is written there.
#[test]
fn a8_6_a_read_only_side_of_a_session_refuses_a_write() {
    let w = world();
    session(&w, "RO", "folder-compare", "read_only_right = true\n", "");
    let before = w.dump("r");
    let (outcome, ui) = w.run("load RO\nselect all\ncopy left->right\ndelete right\nbeep\n");
    assert_eq!(outcome.exit_code, ExitCode::UnknownError, "{:?}", ui.lines);
    assert_eq!(statuses(&outcome), ["ok", "ok", "failed", "failed", "ok"]);
    let error = outcome.commands[2].error.as_ref().unwrap();
    assert!(error.message.contains("read-only"), "{error:?}");
    assert_eq!(w.dump("r"), before);
    // The other side is writable.
    let (outcome, ui) = w.run("load RO\nselect right.orphan.files\ncopy right->left\n");
    assert_eq!(outcome.exit_code, ExitCode::Success, "{:?}", ui.lines);
    assert!(w.path("l/r.txt").exists());
}

/// A8.6, §1.8.5: after a mutation in a flat comparison the refresh answers `RefreshFlat` and the comparison runs
/// again, so the next `select` sees the tree as it is.
#[test]
fn a8_6_a_flat_comparison_is_compared_again_after_a_copy() {
    let w = world();
    session(
        &w,
        "FLAT",
        "folder-compare",
        "",
        "[rules.handling]\nflatten = true\n",
    );
    let (outcome, ui) = w.run(
        "load FLAT\nselect left.diff.files\ncopy left->right\nselect left.diff.files\nselect all.exact.files\n",
    );
    assert_eq!(outcome.exit_code, ExitCode::Success, "{:?}", ui.lines);
    assert_eq!(
        selected_counts(&ui),
        [
            "info L2 2 items selected",
            "info L4 0 items selected",
            "info L5 2 items selected"
        ],
        "{:?}",
        ui.lines
    );
}
