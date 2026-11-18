//! Regression tests of the W13a review findings (Hex Compare view): each test failed before its fix.

use hikasa_core::Side;
use hikasa_hex::nav::Step;
use hikasa_hex::{AlignMode, DisplayFilter, HexLayout};

use crate::commands::CommandId;
use crate::hex::pane::CaretKey;
use crate::tests_hex_open::{bytes, frames, hex_model, hex_world, open_pair};

fn message(app: &crate::tests::TestApp, view: hikasa_session::ipc::ViewId) -> String {
    app.tab(view)
        .and_then(|t| t.message.clone())
        .map(|m| m.text)
        .unwrap_or_default()
}

/// Paged documents (`unlocked_max_mib = 0`): 64 KiB pages, 256 cached (as `tests_hex_screen::paged`).
fn paged(
    world: &crate::tests::World,
    app: &mut crate::tests::TestApp,
    a: &[u8],
    b: &[u8],
) -> hikasa_session::ipc::ViewId {
    let view = open_pair(world, app, a, b);
    let m = hex_model(app, view);
    let mut request = m.request().clone();
    request.rules.unlocked_max_mib = Some("0".into());
    m.relive(request);
    world.settle(app);
    let m = hex_model(app, view);
    assert_eq!(
        m.opened().expect("opened").sides.left.load,
        hikasa_hex::HexLoad::File
    );
    m.set_filter(DisplayFilter::All);
    view
}

/// F1 (MED): the focus side (left, 64 bytes) is much shorter; the screen shows the right side's tail. A layout change
/// must keep the top byte (§1.13.3 "the screen does not jump"); `restore` looks the left side's *offset* up on the
/// right side and jumps to row 2.
#[test]
fn r1_layout_change_keeps_the_top_when_the_focus_side_is_short() {
    let world = hex_world();
    let mut app = world.app(None);
    let a = bytes(5, 64);
    let mut b = a.clone();
    b.extend_from_slice(&bytes(6, 65_536));
    let view = open_pair(&world, &mut app, &a, &b);
    let m = hex_model(&mut app, view);
    m.set_filter(DisplayFilter::All);
    m.frame(20, 0.0);
    assert_eq!(m.focus(), Side::Left);
    let rows = m.row_count();
    m.screen.set_top(3000, rows);
    let top_col = m.first_column_of(m.screen.top).expect("col");
    m.set_layout(
        HexLayout {
            bytes_per_line: 32,
            ..m.display().layout
        },
        true,
        crate::hex::PaneLayout::SideBySide,
        0,
    );
    let new_top_col = m.first_column_of(m.screen.top).expect("col");
    assert!(
        new_top_col.abs_diff(top_col) <= 32 * 20,
        "the screen jumped from column {top_col} to column {new_top_col}"
    );
}

/// F4 (LOW): under *Differences*, Down from a caret whose row is hidden must go to the next shown row; it skips it.
#[test]
fn r2_down_from_a_hidden_row_goes_to_the_next_shown_row() {
    let world = hex_world();
    let mut app = world.app(None);
    let a = bytes(42, 16 * 64);
    let mut b = a.clone();
    b[3 * 16 + 5] ^= 1;
    b[40 * 16 + 9] ^= 1;
    b[50 * 16 + 9] ^= 1;
    let view = open_pair(&world, &mut app, &a, &b);
    let m = hex_model(&mut app, view);
    m.set_filter(DisplayFilter::All);
    m.set_caret(10 * 16 + 2, false);
    m.set_filter(DisplayFilter::Differences);
    assert_eq!(m.row_count(), 3);
    assert_eq!(m.pane(Side::Left).caret, 10 * 16 + 2);
    m.key(CaretKey::Down, false);
    assert_eq!(
        m.pane(Side::Left).caret,
        40 * 16 + 2,
        "the next shown row is row 40"
    );
}

/// F4 (LOW): under *Differences*, a caret on a hidden row after the last shown row cannot move with Up/PgUp at all.
#[test]
fn r2b_up_from_a_hidden_row_after_the_last_shown_row() {
    let world = hex_world();
    let mut app = world.app(None);
    let a = bytes(42, 16 * 64);
    let mut b = a.clone();
    b[3 * 16 + 5] ^= 1;
    b[40 * 16 + 9] ^= 1;
    let view = open_pair(&world, &mut app, &a, &b);
    let m = hex_model(&mut app, view);
    m.set_filter(DisplayFilter::All);
    m.set_caret(60 * 16 + 2, false);
    m.set_filter(DisplayFilter::Differences);
    m.key(CaretKey::Up, false);
    assert_eq!(
        m.pane(Side::Left).caret,
        40 * 16 + 2,
        "Up reaches the last shown row"
    );
}

/// F3 (MED): PgDn/PgUp page the screen and keep the caret at its place on the screen (the window no longer pages a
/// second time: the caret ended one row above the screen after every PgDn); at the end the caret reaches the last
/// rows and stays in view.
#[test]
fn r3_page_down_keeps_the_caret_on_screen() {
    let world = hex_world();
    let mut app = world.app(None);
    let a = bytes(41, 16_000);
    let view = open_pair(&world, &mut app, &a, &a);
    let m = hex_model(&mut app, view);
    m.set_filter(DisplayFilter::All);
    m.frame(20, 0.0);
    m.set_caret(5 * 16 + 3, false);
    let place = |m: &crate::hex::HexViewModel| {
        m.row_of_column(m.pane(Side::Left).caret).expect("row") - m.screen.top
    };
    assert_eq!(place(m), 5);
    for key in [CaretKey::PageDown, CaretKey::PageDown, CaretKey::PageUp] {
        let top = m.screen.top;
        m.key(key, false);
        assert_eq!(m.screen.top.abs_diff(top), 20, "{key:?} pages the screen");
        assert_eq!(
            place(m),
            5,
            "{key:?}: the caret keeps its place on the screen"
        );
        assert_eq!(m.pane(Side::Left).caret % 16, 3, "the position in the row");
    }
    for _ in 0..100 {
        m.key(CaretKey::PageDown, false);
        let row = m.row_of_column(m.pane(Side::Left).caret).expect("row");
        assert!(
            row >= m.screen.top && row < m.screen.top + 20,
            "caret in view"
        );
    }
    assert_eq!(
        m.row_of_column(m.pane(Side::Left).caret),
        Some(m.row_count() - 1),
        "the last row"
    );
}

/// F5 (MED): `TabDuplicate` duplicates the view with its view state (§1.13.17): addresses, thumbnail, values and their
/// radix are lost (the duplicate is built with an empty session `[view]`).
#[test]
fn r4_tab_duplicate_keeps_the_whole_view_state() {
    let world = hex_world();
    let mut app = world.app(None);
    let a = bytes(3, 4000);
    let view = open_pair(&world, &mut app, &a, &a);
    let m = hex_model(&mut app, view);
    let layout = m.display().layout;
    m.set_layout(layout, false, crate::hex::PaneLayout::OverUnder, 3);
    m.set_values(hikasa_session::api::HexValues::Big);
    m.set_values_radix(hikasa_hex::Radix::Hex);
    app.execute(CommandId::ShowThumbnail);
    let src = *hex_model(&mut app, view).display();
    assert!(!src.thumbnail && !src.addresses);
    app.execute(CommandId::TabDuplicate);
    world.settle(&mut app);
    let dup = app.active_tab().expect("tab").id;
    assert_ne!(dup, view);
    let d = *hex_model(&mut app, dup).display();
    assert_eq!(d.panes, src.panes, "panes");
    assert_eq!(d.addresses, src.addresses, "addresses");
    assert_eq!(d.thumbnail, src.thumbnail, "thumbnail");
    assert_eq!(d.values, src.values, "values");
    assert_eq!(d.values_radix, src.values_radix, "values_radix");
}

/// F6 (MED): Swap Sides swaps the specs and documents (A13.1 "Reload and Swap", untested for hex); the tab caption
/// and the job titles keep the old order because `relive` never updates `names`/`titles`.
#[test]
fn r5_swap_sides_swaps_the_caption_too() {
    let world = hex_world();
    let mut app = world.app(None);
    let a = bytes(3, 4000);
    let mut b = a.clone();
    b[7] ^= 1;
    let view = open_pair(&world, &mut app, &a, &b);
    let before = app.tab(view).expect("tab").caption();
    assert!(before.starts_with("a.bin"), "{before}");
    app.execute(CommandId::Swap);
    world.settle(&mut app);
    let m = hex_model(&mut app, view);
    assert!(
        m.request().left.to_string_lossy().ends_with("b.bin"),
        "the specs swapped"
    );
    assert!(
        format!("{:?}", m.opened().expect("o").specs.left).contains("b.bin"),
        "the documents swapped"
    );
    let after = app.tab(view).expect("tab").caption();
    assert!(after.starts_with("b.bin"), "caption after Swap: {after}");
}

/// F7 (MED): the value panel's job of an earlier caret lands after the caret moved to a cached byte (whose lines were
/// computed at once) and overwrites them with the old byte's values: `refresh_values` does not cancel the running
/// job on its cached path, and `poll_values` does not check the job's offset.
#[test]
fn r6_a_stale_values_job_never_overwrites_the_current_lines() {
    let world = hex_world();
    let mut app = world.app(None);
    let a = bytes(17, 4 << 20);
    let view = paged(&world, &mut app, &a, &a);
    assert!(frames(&world, &mut app, view, 30));
    let m = hex_model(&mut app, view);
    m.set_caret(16, false);
    m.set_values(hikasa_session::api::HexValues::Little);
    // Far away: not cached, a job.
    m.set_caret(3 << 20, false);
    assert!(m.value_lines().is_none_or(|(_, stale)| *stale) || m.busy());
    // Back on a cached byte before the job is polled: the lines at once.
    m.set_caret(16, false);
    world.settle(&mut app);
    let m = hex_model(&mut app, view);
    let doc = std::sync::Arc::clone(&m.opened().expect("o").documents.left);
    let want = hikasa_hex::values::format_with(
        &hikasa_hex::values::at(&doc, 16).expect("values"),
        hikasa_hex::ValueFormat {
            little: true,
            big: false,
            radix: hikasa_hex::Radix::Decimal,
        },
    );
    assert_eq!(
        m.value_lines().map(|(t, _)| t.clone()),
        Some(want),
        "the lines of the caret's byte"
    );
}

/// F11 (LOW): idle is idle (§1.13.2 rule 6). With a tall screen (≥ 192 rows, e.g. font 6 on a 4K screen) the read-ahead
/// can never cover `end + 2 screens` within `MAX_ASK` rows, so every frame starts another prefetch job (and its wake
/// repaints): an idle view loops forever.
#[test]
fn r7_an_idle_tall_screen_starts_no_prefetch() {
    let world = hex_world();
    let mut app = world.app(None);
    let a = bytes(19, 1 << 20);
    let view = open_pair(&world, &mut app, &a, &a);
    let m = hex_model(&mut app, view);
    m.set_filter(DisplayFilter::All);
    m.frame(200, 0.0);
    let rows = m.row_count();
    m.screen.set_top(10_000, rows);
    assert!(frames(&world, &mut app, view, 200));
    for i in 0..5 {
        hex_model(&mut app, view).frame(200, f64::from(i));
        world.settle(&mut app);
    }
    let before = hex_model(&mut app, view).screen.prefetches;
    for i in 0..10 {
        hex_model(&mut app, view).frame(200, 10.0 + f64::from(i));
        world.settle(&mut app);
    }
    let after = hex_model(&mut app, view).screen.prefetches;
    assert_eq!(after, before, "prefetch jobs while nothing scrolled");
}

/// A pair whose left side has a gap (the right inserts 500 bytes at 2,000) followed on the left by a unique pattern.
fn gap_pair() -> (Vec<u8>, Vec<u8>) {
    let mut a: Vec<u8> = bytes(61, 8000).into_iter().map(|b| b | 0x80).collect();
    a[2000..2008].copy_from_slice(b"PATTERN!");
    let mut b = a.clone();
    let ins: Vec<u8> = bytes(62, 500).into_iter().map(|b| b | 0x80).collect();
    b.splice(2000..2000, ins);
    (a, b)
}

fn left_gap_column(m: &crate::hex::HexViewModel) -> u64 {
    (0..m.columns())
        .find(|&c| m.byte_at(Side::Left, c).is_some_and(|(_, held)| !held))
        .expect("a left gap column")
}

/// F2 (LOW): the caret is an aligned column and survives a filter change (§1.13.5); on a gap column `kept`/`restore`
/// round-trips it through the side's *next byte* and it lands after the gap.
#[test]
fn r8_a_caret_on_a_gap_column_survives_a_filter_change() {
    let world = hex_world();
    let mut app = world.app(None);
    let (a, b) = gap_pair();
    let view = open_pair(&world, &mut app, &a, &b);
    let m = hex_model(&mut app, view);
    m.set_filter(DisplayFilter::All);
    let gap = left_gap_column(m) + 100;
    assert!(!m.byte_at(Side::Left, gap).expect("byte").1, "still a gap");
    m.set_caret(gap, false);
    m.set_filter(DisplayFilter::Differences);
    assert_eq!(m.pane(Side::Left).caret, gap, "the column survives");
}

/// F10 (LOW): Find Next from a caret on a gap column of the searched side skips the byte right after the gap (the
/// caret's "byte" is the gap's next byte, and the search starts at it + 1): a hit there is not found without wrap.
#[test]
fn r9_find_next_from_a_gap_finds_the_hit_right_after_it() {
    let world = hex_world();
    let mut app = world.app(None);
    let (a, b) = gap_pair();
    let view = open_pair(&world, &mut app, &a, &b);
    let m = hex_model(&mut app, view);
    m.set_filter(DisplayFilter::All);
    let gap = left_gap_column(m) + 10;
    m.set_caret(gap, false);
    m.open_find();
    m.find.pattern = "PATTERN!".into();
    m.find.hex = false;
    m.find.wrap = false;
    m.find.side = Side::Left;
    m.validate_find();
    m.find_step(true);
    world.settle(&mut app);
    let m = hex_model(&mut app, view);
    assert_eq!(
        m.find_hit().map(|(_, r)| r.clone()),
        Some(2000..2008),
        "message: {}",
        message(&app, view)
    );
}

/// F8 (MED): past a truncated section list (and in mode None before the landing) Next Difference scans for the next
/// differing *byte* from the caret: inside a 32-byte differing run each Ctrl+N moves one byte instead of reaching the
/// next difference (a 1 MiB run takes a million presses).
#[test]
fn r10_next_difference_past_a_truncated_list_leaves_the_run() {
    let world = hex_world();
    let mut app = world.app(None);
    let a = bytes(71, 8000);
    let mut b = a.clone();
    b[100] ^= 1;
    for x in &mut b[1000..1032] {
        *x ^= 0xFF;
    }
    b[3000] ^= 1;
    let view = open_pair(&world, &mut app, &a, &b);
    let m = hex_model(&mut app, view);
    m.set_filter(DisplayFilter::All);
    m.set_aligner_for_test(std::sync::Arc::new(|svc, job, opened, opts| {
        let mut o = *opts;
        o.max_sections = 1;
        svc.hex_align(opened, &o, job)
    }));
    m.set_mode(AlignMode::Fast);
    world.settle(&mut app);
    let m = hex_model(&mut app, view);
    assert!(m.alignment().expect("landed").sections.truncated());
    m.set_caret(100, false);
    m.navigate(Step::NextDifference);
    world.settle(&mut app);
    assert_eq!(hex_model(&mut app, view).pane(Side::Left).caret, 1000);
    hex_model(&mut app, view).navigate(Step::NextDifference);
    world.settle(&mut app);
    let caret = hex_model(&mut app, view).pane(Side::Left).caret;
    assert!(
        caret >= 1032,
        "the second Next Difference stays in the run: {caret}"
    );
}

/// F9 (LOW): Current Byte Address copies `nav::address_text` of the caret's byte (§1.13.5), the text Go To reads back;
/// on a gap column the clipboard gets `… (gap)` (the status bar's marker), which Go To refuses.
#[test]
fn r11_current_byte_address_on_a_gap_copies_the_bare_address() {
    let world = hex_world();
    let mut app = world.app(None);
    let clip = std::sync::Arc::new(hikasa_session::clipboard::FixedClipboard::text("clip"));
    app.0
        .set_script_clipboard(std::sync::Arc::clone(&clip) as _);
    let (a, b) = gap_pair();
    let view = open_pair(&world, &mut app, &a, &b);
    let m = hex_model(&mut app, view);
    m.set_filter(DisplayFilter::All);
    let gap = left_gap_column(m) + 10;
    m.set_caret(gap, false);
    assert_eq!(m.focus(), Side::Left);
    app.execute(CommandId::CurrentAddress);
    world.settle(&mut app);
    let text = clip
        .payloads()
        .last()
        .map(|p| p.text.clone())
        .expect("copied");
    assert!(
        world
            .service
            .hex_address(&text, hikasa_hex::Radix::Hex)
            .is_ok(),
        "Go To refuses the copied address {text:?}"
    );
}

/// F1 (MED): the same jump inside a gap of the focus side: the top row lies in the left side's 500-column gap; a
/// filter change moves the screen to the row after the gap (the top "byte" is the gap's next byte), although the
/// alignment and its columns did not change.
#[test]
fn r1b_a_filter_change_keeps_the_top_inside_a_gap() {
    let world = hex_world();
    let mut app = world.app(None);
    let (a, b) = gap_pair();
    let view = open_pair(&world, &mut app, &a, &b);
    let m = hex_model(&mut app, view);
    m.set_filter(DisplayFilter::All);
    m.frame(10, 0.0);
    let gap = left_gap_column(m);
    let rows = m.row_count();
    let top = (gap + 160) / 16;
    m.screen.set_top(top, rows);
    m.set_filter(DisplayFilter::All);
    assert_eq!(m.screen.top, top, "the top row survives a filter change");
}
