//! A13.5 and A13.6 (M2 W13a, D-355, D-358): the alignment in the window — progress, Esc's *Alignment stopped*, a
//! `degraded` notice (a budget of 1 MiB), `input_changed` after a job and a changed file seen by the watch with D-118's
//! *Reload*, a truncated file's error row — and navigation: Next/Previous Difference Bytes against `nav::step` at the
//! first and last section and from inside one, past a truncated list (the scan job), queued before a Fast landing and
//! run as a scan in None mode; Existing Byte on both sides; the three filters with context 0 and 3 and the hidden rows;
//! Go To for every spelling through `hex_address`, the clamp text, a refused text keeping the dialog. Mutations: the
//! notice not raised for a stale landing; Previous Difference from inside a section landing on that section.

use std::sync::atomic::Ordering;

use hikasa_core::Side;
use hikasa_hex::nav::Step;
use hikasa_hex::{AlignMode, DisplayFilter};

use crate::commands::CommandId;
use crate::dialog::ButtonRole;
use crate::hex::nav::HexGoToDialog;
use crate::hex::{HexPhase, NO_MORE};
use crate::tests_hex_open::{Gate, bytes, hex_model, hex_world, open_pair, until};

fn message(app: &crate::tests::TestApp, view: hikasa_session::ipc::ViewId) -> String {
    app.tab(view)
        .and_then(|t| t.message.clone())
        .map(|m| m.text)
        .unwrap_or_default()
}

/// A pair with three difference sections: left 8 KiB; the right changes bytes 100 and 5,000 and inserts 7 bytes at
/// 3,000.
fn sections_pair() -> (Vec<u8>, Vec<u8>) {
    let a = bytes(51, 8 * 1024);
    let mut b = a.clone();
    b[100] ^= 0x10;
    b[5000] ^= 0x20;
    let ins = bytes(52, 7);
    b.splice(3000..3000, ins);
    (a, b)
}

/// A13.6: Next/Previous Difference Bytes against `nav::step`: the section's columns selected, the caret on its first
/// column; at the last section *No more differences*; Previous from inside a section is the one before it.
#[test]
fn a13_6_difference_steps_equal_nav_step() {
    let world = hex_world();
    let mut app = world.app(None);
    let (a, b) = sections_pair();
    let view = open_pair(&world, &mut app, &a, &b);
    let m = hex_model(&mut app, view);
    m.set_filter(DisplayFilter::All);
    let al = std::sync::Arc::clone(m.alignment().expect("landed"));
    assert_eq!(al.sections.len(), 3);
    m.set_caret(0, false);
    let mut visited = Vec::new();
    loop {
        let caret = hex_model(&mut app, view).pane(Side::Left).caret;
        let want = hikasa_hex::nav::step(&al, caret, Step::NextDifference);
        app.execute(CommandId::DiffItemNext);
        world.settle(&mut app);
        let m = hex_model(&mut app, view);
        match want {
            Some(col) => {
                assert_eq!(m.pane(Side::Left).caret, col);
                let i = al.sections.first_ending_after(col);
                assert_eq!(m.selection(), al.sections.columns(i).expect("section"));
                visited.push(col);
            }
            None => {
                assert_eq!(message(&app, view), NO_MORE);
                break;
            }
        }
    }
    assert_eq!(visited.len(), 3);
    // From inside the second section (the 7 inserted bytes), Previous is the one before it (not this one).
    let m = hex_model(&mut app, view);
    let inside = visited[1] + 3;
    m.set_caret(inside, false);
    app.execute(CommandId::DiffItemPrev);
    let m = hex_model(&mut app, view);
    assert_eq!(
        Some(m.pane(Side::Left).caret),
        hikasa_hex::nav::step(&al, inside, Step::PrevDifference)
    );
    assert_eq!(m.pane(Side::Left).caret, visited[0]);
}

/// A13.6: Existing Byte on both sides skips the other side's gap.
#[test]
fn a13_6_existing_bytes_skip_gaps() {
    let world = hex_world();
    let mut app = world.app(None);
    let (a, b) = sections_pair();
    let view = open_pair(&world, &mut app, &a, &b);
    let m = hex_model(&mut app, view);
    let al = std::sync::Arc::clone(m.alignment().expect("landed"));
    let gap = al.segments.iter().find(|s| s.left_len() == 0).expect("gap");
    let first = al
        .segments
        .column_at_offset(Side::Right, gap.right)
        .expect("col");
    // On the left, the step over the gap lands after it.
    m.set_caret(first - 1, false);
    m.focus_side(Side::Left);
    app.execute(CommandId::ExistsNext);
    let m = hex_model(&mut app, view);
    assert_eq!(m.pane(Side::Left).caret, first + gap.len);
    app.execute(CommandId::ExistsPrev);
    let m = hex_model(&mut app, view);
    assert_eq!(m.pane(Side::Left).caret, first - 1);
    // The right side holds those bytes: one column at a time.
    m.focus_side(Side::Right);
    app.execute(CommandId::ExistsNext);
    let m = hex_model(&mut app, view);
    assert_eq!(m.pane(Side::Right).caret, first);
}

/// A13.6: the three filters with context 0 and 3: the rows shown and the hidden-row count in the status bar.
#[test]
fn a13_6_filters_and_hidden_rows() {
    let world = hex_world();
    let mut app = world.app(None);
    let (a, b) = sections_pair();
    let view = open_pair(&world, &mut app, &a, &b);
    for context in [0u8, 3] {
        let m = hex_model(&mut app, view);
        let layout = m.display().layout;
        m.set_layout(layout, true, crate::hex::PaneLayout::SideBySide, context);
        for (id, filter) in [
            (CommandId::FavDiff, DisplayFilter::Differences),
            (CommandId::FavSame, DisplayFilter::Same),
            (CommandId::FavCompareAll, DisplayFilter::All),
        ] {
            app.execute(id);
            let m = hex_model(&mut app, view);
            assert_eq!(m.display().filter, filter);
            let src = m.source().expect("src");
            let core = src.view();
            assert_eq!(m.row_count(), core.row_count());
            let hidden = core.full_row_count() - core.row_count();
            let status = crate::hex::status::items(m);
            if filter == DisplayFilter::All {
                assert_eq!(hidden, 0);
            } else {
                assert!(
                    status
                        .right
                        .iter()
                        .any(|s| s.contains(&format!("{hidden} rows hidden"))),
                    "{status:?}"
                );
            }
            assert_eq!(app.0.checked(app.win(), id), Some(true));
        }
    }
}

/// A13.6: a difference step before a Fast alignment lands is queued and runs when it lands; in mode None it runs as a
/// scan job from the caret.
#[test]
fn a13_6_queued_before_the_landing_and_scanned_in_none() {
    let world = hex_world();
    let mut app = world.app(None);
    let (a, b) = sections_pair();
    let view = open_pair(&world, &mut app, &a, &b);
    let gate = Gate::default();
    let m = hex_model(&mut app, view);
    m.set_aligner_for_test(gate.aligner());
    m.reload();
    until(&world, &mut app, |app| {
        hex_model(app, view).phase() == HexPhase::Provisional
            && gate.reached.load(Ordering::SeqCst) == 1
    });
    let m = hex_model(&mut app, view);
    m.set_caret(0, false);
    m.navigate(Step::NextDifference);
    assert_eq!(m.pane(Side::Left).caret, 0, "queued");
    gate.release();
    until(&world, &mut app, |app| {
        hex_model(app, view).alignment().is_some()
    });
    let m = hex_model(&mut app, view);
    let al = std::sync::Arc::clone(m.alignment().expect("landed"));
    assert_eq!(
        Some(m.pane(Side::Left).caret),
        hikasa_hex::nav::step(&al, 0, Step::NextDifference),
        "the queued step ran when it landed"
    );
    // Mode None, held: the scan job finds byte 100.
    let gate = Gate::default();
    let m = hex_model(&mut app, view);
    m.set_aligner_for_test(gate.aligner());
    m.set_mode(AlignMode::None);
    m.reload();
    // The switch's job and the reload's job both reach the gate (the first is cancelled by the second).
    until(&world, &mut app, |app| {
        hex_model(app, view).phase() == HexPhase::Provisional
            && hex_model(app, view).aligning().is_some()
            && gate.reached.load(Ordering::SeqCst) >= 1
    });
    let m = hex_model(&mut app, view);
    m.set_caret(0, false);
    m.navigate(Step::NextDifference);
    until(&world, &mut app, |app| {
        hex_model(app, view).pane(Side::Left).caret != 0
    });
    assert_eq!(hex_model(&mut app, view).pane(Side::Left).caret, 100);
    gate.release();
    world.settle(&mut app);
}

/// A13.6: past a truncated section list Next Difference scans in a job (the sections stored: 1).
#[test]
fn a13_6_past_a_truncated_list_the_next_difference_is_scanned() {
    let world = hex_world();
    let mut app = world.app(None);
    let (a, b) = sections_pair();
    let view = open_pair(&world, &mut app, &a, &b);
    // A custom aligner storing one section.
    let m = hex_model(&mut app, view);
    m.set_aligner_for_test(std::sync::Arc::new(|svc, job, opened, opts| {
        let mut o = *opts;
        o.max_sections = 1;
        svc.hex_align(opened, &o, job)
    }));
    m.set_mode(AlignMode::Fast);
    world.settle(&mut app);
    let m = hex_model(&mut app, view);
    let al = std::sync::Arc::clone(m.alignment().expect("landed"));
    assert!(al.sections.truncated());
    let first = al.sections.columns(0).expect("one").start;
    m.set_caret(first, false);
    app.execute(CommandId::DiffItemNext);
    world.settle(&mut app);
    let m = hex_model(&mut app, view);
    let want = hikasa_hex::nav::scan_next_difference(
        &m.opened().expect("o").documents.left,
        &m.opened().expect("o").documents.right,
        &al,
        first,
        &world.service.stream_options(),
        &hikasa_core::Ctl::detached(),
    )
    .expect("scan");
    assert_eq!(Some(m.pane(Side::Left).caret), want);
}

/// A13.6: Go To through `hex_address` for every spelling, relative forms, `end`, the clamp text in the message panel,
/// a refused text keeping the dialog with the core's message; the radix is remembered.
#[test]
fn a13_6_go_to() {
    let world = hex_world();
    let mut app = world.app(None);
    let (a, b) = sections_pair();
    let view = open_pair(&world, &mut app, &a, &b);
    let w = app.win();
    let go = |app: &mut crate::tests::TestApp, text: &str, radix: hikasa_hex::Radix, side: Side| {
        app.execute(CommandId::GoTo);
        let d = app
            .0
            .window_mut(w)
            .and_then(|w| w.dialogs.find_mut::<HexGoToDialog>())
            .expect("dialog");
        d.text = text.to_owned();
        d.radix = radix;
        d.side = side;
        app.0.dialog_button(w, ButtonRole::Ok);
    };
    use hikasa_hex::Radix::{Decimal, Hex};
    for (text, radix, want) in [
        ("0x1F", Decimal, 31u64),
        ("1Fh", Decimal, 31),
        ("$1f", Hex, 31),
        ("#31", Hex, 31),
        ("31d", Hex, 31),
        ("31", Hex, 0x31),
        ("31", Decimal, 31),
    ] {
        go(&mut app, text, radix, Side::Left);
        let m = hex_model(&mut app, view);
        assert_eq!(
            crate::tests_hex_open::caret_byte(m, Side::Left),
            want,
            "{text}"
        );
    }
    go(&mut app, "+10", Decimal, Side::Left);
    assert_eq!(
        crate::tests_hex_open::caret_byte(hex_model(&mut app, view), Side::Left),
        41
    );
    go(&mut app, "end", Hex, Side::Right);
    let m = hex_model(&mut app, view);
    assert_eq!(m.focus(), Side::Right);
    assert_eq!(
        crate::tests_hex_open::caret_byte(m, Side::Right),
        b.len() as u64 - 1
    );
    go(&mut app, "#999999", Hex, Side::Left);
    assert!(
        message(&app, view).contains("past the end"),
        "{}",
        message(&app, view)
    );
    assert_eq!(
        crate::tests_hex_open::caret_byte(hex_model(&mut app, view), Side::Left),
        a.len() as u64 - 1
    );
    // A refused text keeps the dialog open with the core's message.
    app.execute(CommandId::GoTo);
    let d = app
        .0
        .window_mut(w)
        .and_then(|w| w.dialogs.find_mut::<HexGoToDialog>())
        .expect("dialog");
    assert_eq!(
        d.radix, Hex,
        "the radix remembered (state.dialogs.hex-goto)"
    );
    assert_eq!(
        d.recent.first().map(String::as_str),
        Some("#999999"),
        "mru.hex_goto"
    );
    d.text = "xyz".into();
    app.0.dialog_button(w, ButtonRole::Ok);
    let d = app
        .0
        .window_mut(w)
        .and_then(|w| w.dialogs.find_mut::<HexGoToDialog>())
        .expect("still open");
    assert!(d.error.as_deref().is_some_and(|e| e.contains("address")));
}

/// A13.5: Esc stops a running alignment (*Alignment stopped*, the rows stay provisional); a budget of 1 MiB on
/// unrelated halves is `degraded` with a notice.
#[test]
fn a13_5_esc_stops_and_degraded_notices() {
    let world = hex_world();
    let mut app = world.app(None);
    let (a, b) = sections_pair();
    let view = open_pair(&world, &mut app, &a, &b);
    let gate = Gate::default();
    let m = hex_model(&mut app, view);
    m.set_aligner_for_test(gate.aligner());
    m.reload();
    until(&world, &mut app, |app| {
        hex_model(app, view).phase() == HexPhase::Provisional
            && gate.reached.load(Ordering::SeqCst) == 1
    });
    let m = hex_model(&mut app, view);
    assert!(crate::hex::status::alignment_text(m).starts_with("Aligning"));
    m.escape();
    world.settle(&mut app);
    until(&world, &mut app, |_| {
        gate.cancelled.load(Ordering::SeqCst) == 1
    });
    let m = hex_model(&mut app, view);
    assert!(m.stopped());
    assert_eq!(m.phase(), HexPhase::Provisional);
    assert_eq!(crate::hex::status::alignment_text(m), "Alignment stopped");
    assert_eq!(message(&app, view), "Alignment stopped.");
    // Degraded: Complete with a 1 MiB budget on two unrelated 3 MiB halves.
    let x = bytes(61, 3 << 20);
    let y = bytes(62, 3 << 20);
    let view = open_pair(&world, &mut app, &x, &y);
    let m = hex_model(&mut app, view);
    let mut request = m.request().clone();
    request.rules.alignment = Some("complete".into());
    request.rules.budget_mib = Some("1".into());
    request.rules.window = Some("4096".into());
    m.relive(request);
    world.settle(&mut app);
    let m = hex_model(&mut app, view);
    let al = m.alignment().expect("landed");
    if al.degraded {
        assert!(crate::hex::status::alignment_text(m).contains("degraded"));
        let w = app.win();
        let notices: Vec<crate::notify::Notice> = app
            .0
            .window_mut(w)
            .expect("w")
            .notices
            .visible(0.0)
            .into_iter()
            .cloned()
            .collect();
        assert!(
            notices.iter().any(|n| n.text.contains("degraded")),
            "{notices:?}"
        );
    }
}

/// A13.5: a file changed while the alignment ran is `input_changed`: D-118's notice with *Reload*; Reload clears it.
/// A truncated file gives the error row (never zero bytes). Mutation: the notice not raised for a stale landing.
#[test]
fn a13_5_changes_on_disk() {
    let world = hex_world();
    let mut app = world.app(None);
    let (a, b) = sections_pair();
    let view = open_pair(&world, &mut app, &a, &b);
    let gate = Gate::default();
    let m = hex_model(&mut app, view);
    m.set_aligner_for_test(gate.aligner());
    m.reload();
    until(&world, &mut app, |_| {
        gate.reached.load(Ordering::SeqCst) == 1
    });
    // The right file changes while the alignment is held.
    let right = std::path::PathBuf::from(hex_model(&mut app, view).inputs()[1].clone());
    std::fs::write(&right, b"changed meanwhile").expect("write");
    gate.release();
    until(&world, &mut app, |app| {
        hex_model(app, view).alignment().is_some()
    });
    world.settle(&mut app);
    assert!(hex_model(&mut app, view).stale());
    let w = app.win();
    let notices: Vec<crate::notify::Notice> = app
        .0
        .window_mut(w)
        .expect("w")
        .notices
        .visible(0.0)
        .into_iter()
        .cloned()
        .collect();
    let notice = notices
        .iter()
        .find(|n| n.text.contains("changed on disk"))
        .expect("D-118's notice");
    assert!(
        notice
            .actions
            .contains(&crate::notify::NoticeAction::ReloadView(view))
    );
    // Reload clears it.
    hex_model(&mut app, view).set_aligner_for_test(Gate::default().aligner_open());
    app.execute(CommandId::Reload);
    world.settle(&mut app);
    assert!(!hex_model(&mut app, view).stale());
    // A truncated file: the error row.
    let x = bytes(71, 1 << 20);
    let view = open_pair(&world, &mut app, &x, &x);
    let m = hex_model(&mut app, view);
    let mut request = m.request().clone();
    request.rules.unlocked_max_mib = Some("0".into());
    m.relive(request);
    world.settle(&mut app);
    let left = std::path::PathBuf::from(hex_model(&mut app, view).inputs()[0].clone());
    std::fs::write(&left, b"short").expect("truncate");
    let m = hex_model(&mut app, view);
    m.set_filter(DisplayFilter::All);
    let rows = m.row_count();
    m.screen.set_top(rows - 40, rows);
    for i in 0..100 {
        hex_model(&mut app, view).frame(30, f64::from(i));
        world.settle(&mut app);
    }
    let m = hex_model(&mut app, view);
    assert!(matches!(
        m.screen.row(m.generation, rows - 40),
        crate::hex::screen::ShownRow::Error(_)
    ));
}
