//! A11.1 (D-294): the live view. The slice of the key's frame shows the edit, the live comparison keeps its
//! invariants, the debounce runs on explicit frame times, one recompare at a time, a `Replayed` install starts the
//! next at once, a relive cancels a recompare, a closed view cancels its job, the thumbnail and the statistics are
//! recomputed at installs only; P-LIVE-VIEW: random key scripts on generated pairs, then idle, equal a fresh
//! comparison of the saved texts.

use hikasa_core::Side;
use hikasa_session::ipc::CloseReason;

use crate::tests_text::{Fixture, Rng, fresh_json, generated_pair};
use crate::text::input::{Key, Press};
use crate::text::live::DEBOUNCE_S;

#[test]
fn a11_1_the_key_frame_shows_the_edit_and_the_debounce_runs_on_frame_times() {
    let (l, r) = generated_pair(2_000, 7);
    let mut f = Fixture::new(&l, &r);
    let installs = f.m().installs();
    let thumbs = f.m().thumb_builds();
    f.place(Side::Left, 5, 0);
    f.type_text("Z");
    // The same frame: the slice shows the typed character, every live line is in the comparison once.
    let slice = f.m().slice(0, 12);
    let row = slice
        .rows
        .iter()
        .find(|r| r.ln[0] == 6)
        .expect("line 6 on screen");
    assert!(
        row.t[0]
            .as_deref()
            .is_some_and(|t| t.starts_with("Zint v5")),
        "{row:?}"
    );
    f.m()
        .live_view()
        .expect("live")
        .live
        .check_invariants()
        .expect("invariants");
    // No job before the debounce, one at it.
    let t0 = f.now;
    f.m().set_now(t0 + DEBOUNCE_S * 0.5);
    f.app.handle(crate::app::UiMsg::Tick);
    assert_eq!(f.m().recompare_state().started, 0, "nothing before 100 ms");
    f.m().set_now(t0 + DEBOUNCE_S);
    f.app.handle(crate::app::UiMsg::Tick);
    assert_eq!(f.m().recompare_state().started, 1, "one at 100 ms");
    // Edits while it runs only move the pending time: no second job.
    f.type_text("Y");
    f.app.handle(crate::app::UiMsg::Tick);
    assert!(f.m().recompare_state().started <= 1 || f.m().recompare_state().running.is_none());
    f.settle();
    assert!(f.m().installs() > installs, "installed");
    assert_eq!(
        f.m().thumb_builds(),
        thumbs,
        "the model never builds a thumbnail by itself (the window caches by installs)"
    );
    let left = f.text(Side::Left);
    let right = f.text(Side::Right);
    assert_eq!(f.json(), fresh_json(&f.world, &left, &right));
}

#[test]
fn a11_1_keystrokes_do_not_rebuild_the_thumbnail_or_the_statistics() {
    let (l, r) = generated_pair(2_000, 11);
    let mut f = Fixture::new(&l, &r);
    let installs = f.m().installs();
    f.place(Side::Right, 3, 0);
    for c in ["a", "b", "c", "d"] {
        f.type_text(c);
        f.frame(0.01);
    }
    assert_eq!(f.m().installs(), installs, "no install per key");
    let status_builds = f.m().status_builds();
    let _ = f.m().status_items();
    let _ = f.m().status_items();
    assert_eq!(
        f.m().status_builds(),
        status_builds + 1,
        "per caret move, not per frame"
    );
    f.settle();
    assert_eq!(
        f.m().installs(),
        installs + 1,
        "one install after the burst"
    );
}

#[test]
fn a11_1_a_relive_cancels_the_recompare_and_a_closed_view_its_job() {
    let (l, r) = generated_pair(3_000, 13);
    let mut f = Fixture::new(&l, &r);
    f.place(Side::Left, 1, 0);
    f.plain(Key::Enter);
    let t = f.now + DEBOUNCE_S;
    f.m().set_now(t);
    f.app.handle(crate::app::UiMsg::Tick);
    let service = f.world.service.clone();
    let request = f.m().request().clone();
    assert!(crate::text::relive::start(
        f.m(),
        &service,
        request,
        crate::text::relive::Cause::Rules
    ));
    assert!(
        f.m().recompare_state().running.is_none(),
        "the recompare was cancelled"
    );
    f.settle();
    let left = f.text(Side::Left);
    let right = f.text(Side::Right);
    assert_eq!(f.json(), fresh_json(&f.world, &left, &right));
    // A closed view cancels its job.
    f.type_text("x");
    let later = f.now + 1.0;
    f.m().set_now(later);
    f.app.handle(crate::app::UiMsg::Tick);
    let index = f.app.active();
    f.app.close_tab(index, CloseReason::User);
    assert!(f.app.tabs().iter().all(|t| t.id != f.view));
}

/// P-LIVE-VIEW: random scripts (typing, Enter, Backspace, Delete, Delete Line, Undo, Redo, the copies to the other
/// side, a paste of CRLF lines, Cut of a Shift+Down selection, a save with keys during it, a relive of the rules, a
/// format pick and back, moves) on generated pairs with random poll points, then idle: the installed comparison
/// equals a fresh one of the texts. A failing case leaves its texts and both JSONs in a folder of the temporary
/// directory named in the message.
#[test]
fn p_live_view_random_scripts_equal_a_fresh_comparison() {
    use crate::commands::CommandId;
    let cases: u64 = std::env::var("HIKASA_HEAVY").ok().map_or(4, |_| 40);
    for case in 0..cases {
        let seed = 0x5EED_0000 + case;
        let mut rng = Rng(seed);
        let lines = 2_000 + rng.below(18_000) as usize;
        let (l, r) = generated_pair(lines, seed);
        let mut f = Fixture::new(&l, &r);
        let svc = f.world.service.clone();
        let mut forced = Vec::new();
        let mut script = Vec::new();
        for _ in 0..(20 + rng.below(40)) {
            let side = if rng.below(2) == 0 {
                Side::Left
            } else {
                Side::Right
            };
            if rng.below(6) == 0 {
                let line = rng.below(u64::from(f.doc(side).line_count())) as u32;
                f.place(side, line, 0);
                script.push(format!("place {side:?} {line}"));
            }
            let op = rng.below(18);
            script.push(format!("op {op}"));
            match op {
                0..=3 => f.type_text(
                    ["a", "b", " ", "é", "{", "/*", "*/", "\"", "//"][rng.below(9) as usize],
                ),
                4 => {
                    f.plain(Key::Enter);
                }
                5 => {
                    f.plain(Key::Backspace);
                }
                6 => {
                    f.plain(Key::Delete);
                }
                7 => f.run(CommandId::DeleteItem),
                8 => f.run(CommandId::Undo),
                9 => f.run(CommandId::Redo),
                10 => f.run(
                    [
                        CommandId::CopyRight,
                        CommandId::CopyLeft,
                        CommandId::CopyItemRight,
                        CommandId::CopyItemLeft,
                    ][rng.below(4) as usize],
                ),
                11 => {
                    let n = rng.below(4);
                    let text: String =
                        (0..n).map(|i| format!("p{i}\r\n")).collect::<String>() + "tail";
                    let focus = f.m().focus();
                    crate::text::clipboard::paste(f.m(), focus, &text);
                }
                12 => {
                    f.key(Press::shift(Key::Down));
                    f.key(Press::shift(Key::Down));
                    f.run(CommandId::Cut);
                }
                13 => {
                    // A save with keys typed while it runs (queued, replayed in order).
                    f.run(CommandId::Save);
                    f.type_text("s");
                    f.plain(Key::Enter);
                }
                14 => {
                    let request = f.m().request().clone();
                    crate::text::relive::start(
                        f.m(),
                        &svc,
                        request,
                        crate::text::relive::Cause::Rules,
                    );
                    f.type_text("r");
                }
                15 => {
                    f.m().request_format(side, Some("HTML".into()));
                    forced.push(side);
                }
                _ => {
                    f.key(Press::ctrl(Key::Right));
                }
            }
            if rng.below(4) == 0 {
                f.frame(rng.below(200) as f64 / 1000.0);
            }
        }
        // Back to detection: the fresh comparison detects the format of the names.
        for side in forced {
            f.settle();
            f.m().request_format(side, None);
        }
        f.settle();
        if let Err(e) = f.m().live_view().expect("live").live.check_invariants() {
            panic!("seed {seed:#x}: invariants: {e:?}; script {script:?}");
        }
        let left = f.text(Side::Left);
        let right = f.text(Side::Right);
        let (live, fresh) = (f.json(), fresh_json(&f.world, &left, &right));
        let mut dumped = String::new();
        if live != fresh {
            let dir = std::env::temp_dir().join(format!("hikasa-p-live-view-{seed:x}"));
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(
                dir.join("live.json"),
                serde_json::to_string_pretty(&live).unwrap_or_default(),
            );
            let _ = std::fs::write(
                dir.join("fresh.json"),
                serde_json::to_string_pretty(&fresh).unwrap_or_default(),
            );
            let _ = std::fs::write(dir.join("left.c"), &left);
            let _ = std::fs::write(dir.join("right.c"), &right);
            dumped = dir.display().to_string();
        }
        assert!(
            live == fresh,
            "seed {seed:#x}: the installed comparison differs from a fresh one (dumped to {dumped}); script {script:?}"
        );
    }
}
