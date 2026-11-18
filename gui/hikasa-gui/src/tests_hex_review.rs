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

/// F5 (MED): `TabDuplicate` duplicates the view with its whole view state (§1.13.17): panes, addresses, thumbnail,
/// values and their radix, the font, the headers, the splitter, the rows' layout, filter and context.
#[test]
fn r4_tab_duplicate_keeps_the_whole_view_state() {
    let world = hex_world();
    let mut app = world.app(None);
    let a = bytes(3, 4000);
    let view = open_pair(&world, &mut app, &a, &a);
    let m = hex_model(&mut app, view);
    let layout = HexLayout {
        bytes_per_line: 24,
        group: 4,
        chars: hikasa_hex::CharSet::Latin1,
        address: hikasa_hex::Radix::Decimal,
    };
    m.set_layout(layout, false, crate::hex::PaneLayout::OverUnder, 3);
    m.set_filter(DisplayFilter::Same);
    m.set_values(hikasa_session::api::HexValues::Big);
    m.set_values_radix(hikasa_hex::Radix::Hex);
    m.set_font_px(17);
    m.set_split(0.3);
    app.execute(CommandId::ShowThumbnail);
    app.execute(CommandId::ShowSpecInfo);
    let src = *hex_model(&mut app, view).display();
    assert!(!src.thumbnail && !src.addresses && !src.headers);
    assert_ne!(src, crate::hex::HexDisplay::default());
    app.execute(CommandId::TabDuplicate);
    world.settle(&mut app);
    let dup = app.active_tab().expect("tab").id;
    assert_ne!(dup, view);
    let d = *hex_model(&mut app, dup).display();
    assert_eq!(d, src, "every field of the view state");
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

// ---- W13a review: the decisions' further tests and the mutation survivors' killers ------------------------------

/// Three differing `.bin` pairs and a same one, in a saved folder session `tree` (the hex children's parent).
fn bin_parent(world: &crate::tests::World) {
    for (name, l, r) in [
        ("f1.bin", 1u64, 2u64),
        ("f2.bin", 3, 4),
        ("f3.bin", 5, 6),
        ("same.bin", 7, 7),
    ] {
        let path = |dir: &str| world.path(&format!("{dir}/{name}"));
        std::fs::create_dir_all(world.path("pl")).expect("mkdir");
        std::fs::create_dir_all(world.path("pr")).expect("mkdir");
        // The sizes differ for a differing pair (the parent's quick comparison sees it).
        let n = if l == r { 300 } else { 301 };
        std::fs::write(path("pl"), bytes(l, 300)).expect("w");
        std::fs::write(path("pr"), bytes(r, n)).expect("w");
    }
    let mut s = hikasa_session::sessions::Session::builtin(
        hikasa_session::sessions::SessionType::FolderCompare,
    );
    s.specs.left = world.path("pl").display().to_string();
    s.specs.right = world.path("pr").display().to_string();
    world
        .service
        .session_save(
            &hikasa_session::sessions::SessionRef::parse("tree").expect("ref"),
            &s,
            false,
        )
        .expect("save");
}

/// Opens the parent session and the hex child of `name`: (parent view, child view).
fn bin_child(
    world: &crate::tests::World,
    app: &mut crate::tests::TestApp,
    name: &str,
) -> (hikasa_session::ipc::ViewId, hikasa_session::ipc::ViewId) {
    let target = crate::app::OpenTarget::Window(app.win());
    app.0.open_named(
        hikasa_session::sessions::SessionRef::parse("tree").expect("ref"),
        crate::app::OpenHow::Target(target),
    );
    world.settle(app);
    let parent = app.active_tab().expect("tab").id;
    crate::tests_folder_ops::select(&mut app.0, parent, name);
    app.execute(CommandId::Open);
    world.settle(app);
    let child = app.active_tab().expect("tab").id;
    assert_ne!(child, parent);
    assert_eq!(app.scope(), crate::commands::Scope::Hex);
    (parent, child)
}

/// F6 (MED): Next/Previous Difference Files reopen this view with the next pair and its caption follows; the open
/// parent view's row selection follows; with the parent view closed the parent is compared again in a job (as the
/// text views do) instead of a message.
#[test]
fn f6_next_difference_files_rename_the_caption_and_move_the_parent_row() {
    let world = hex_world();
    bin_parent(&world);
    let mut app = world.app(None);
    world.settle(&mut app);
    let (parent, child) = bin_child(&world, &mut app, "f1.bin");
    assert!(app.tab(child).expect("tab").caption().starts_with("f1.bin"));
    app.execute(CommandId::DiffFileNext);
    world.settle(&mut app);
    let caption = app.tab(child).expect("tab").caption();
    assert!(caption.starts_with("f2.bin"), "caption: {caption}");
    assert!(hex_model(&mut app, child).inputs()[0].ends_with("f2.bin"));
    // The parent's row selection is the pair's.
    let selected = match app.tab(parent).and_then(|t| t.model()) {
        Some(crate::app::ViewModel::Folder(m)) => m.selected().expect("a row"),
        _ => panic!("the parent folder view"),
    };
    let name = match app.tab(parent).and_then(|t| t.model()) {
        Some(crate::app::ViewModel::Folder(m)) => m
            .prepared()
            .expect("prepared")
            .model()
            .read()
            .spec(selected, Side::Left)
            .map(|s| s.to_string())
            .unwrap_or_default(),
        _ => String::new(),
    };
    assert!(name.ends_with("f2.bin"), "the parent's row: {name}");
    // The parent view closed: a job compares the parent again.
    let w = app.win();
    let index = app
        .window(w)
        .and_then(|x| x.index_of(parent))
        .expect("open");
    app.0
        .close_tab(w, index, hikasa_session::ipc::CloseReason::User);
    world.settle(&mut app);
    let index = app.window(w).and_then(|x| x.index_of(child)).expect("open");
    app.activate(index);
    app.execute(CommandId::DiffFileNext);
    world.settle(&mut app);
    let caption = app.tab(child).expect("tab").caption();
    assert!(caption.starts_with("f3.bin"), "caption: {caption}");
    app.execute(CommandId::DiffFilePrev);
    world.settle(&mut app);
    assert!(app.tab(child).expect("tab").caption().starts_with("f2.bin"));
}

/// F7 (MED): a value job that fails shows its error in the panel (never greyed forever).
#[test]
fn f7_a_failed_values_job_shows_its_error() {
    let world = hex_world();
    let mut app = world.app(None);
    let a = bytes(18, 2 << 20);
    let view = paged(&world, &mut app, &a, &a);
    assert!(frames(&world, &mut app, view, 30));
    let m = hex_model(&mut app, view);
    m.set_values(hikasa_session::api::HexValues::Little);
    assert!(m.value_lines().is_some_and(|(_, stale)| !stale));
    // The left file shrinks on disk: the far bytes are no longer there.
    let left = world.path("work/h/a.bin");
    std::fs::write(&left, &a[..1000]).expect("truncate");
    m.set_caret((3 << 19) + 5, false);
    assert!(m.value_lines().is_some_and(|(_, stale)| *stale), "greyed");
    world.settle(&mut app);
    let m = hex_model(&mut app, view);
    let (text, stale) = m.value_lines().cloned().expect("lines");
    assert!(!stale, "not greyed after the job");
    assert!(text.contains("could not be read"), "{text}");
}

/// F11 (LOW): a screen of 400 rows (taller than the ask of three 256-row screens) shows every row once its prefetch
/// landed, and an idle frame starts no prefetch.
#[test]
fn f11_a_screen_of_400_rows_shows_all_rows() {
    let world = hex_world();
    let mut app = world.app(None);
    let a = bytes(20, 1 << 20);
    let view = paged(&world, &mut app, &a, &a);
    let m = hex_model(&mut app, view);
    m.frame(400, 0.0);
    let rows = m.row_count();
    m.screen.set_top(20_000, rows);
    assert!(
        frames(&world, &mut app, view, 400),
        "every row drawn from the cache"
    );
    let m = hex_model(&mut app, view);
    let g = m.generation;
    for k in 0..=400 {
        assert!(
            m.screen.cached(g, 20_000 + k).is_some(),
            "row {k} of the screen"
        );
    }
    for i in 0..5 {
        hex_model(&mut app, view).frame(400, 10.0 + f64::from(i));
        world.settle(&mut app);
    }
    let before = hex_model(&mut app, view).screen.prefetches;
    for i in 0..5 {
        assert!(hex_model(&mut app, view).frame(400, 20.0 + f64::from(i)));
        world.settle(&mut app);
    }
    assert_eq!(hex_model(&mut app, view).screen.prefetches, before, "idle");
}

/// The first page of `doc` (from `from`) whose first byte is not cached.
fn uncached_page(doc: &hikasa_hex::HexDocument, from: u64) -> u64 {
    let page = 64 << 10;
    let mut p = from / page * page + page;
    while doc.is_cached(p..p + 1) {
        p += page;
    }
    p
}

/// F14 (LOW, kills R17): the value panel never reads on the UI thread: when the caret's 8 bytes reach into a page that
/// is not cached (its first byte is), the values come from a job and the panel is greyed until it lands.
#[test]
fn f14_values_over_an_uncached_page_come_from_a_job() {
    let world = hex_world();
    let mut app = world.app(None);
    let a = bytes(21, 4 << 20);
    let view = paged(&world, &mut app, &a, &a);
    assert!(frames(&world, &mut app, view, 30));
    let m = hex_model(&mut app, view);
    m.set_values(hikasa_session::api::HexValues::Little);
    let doc = std::sync::Arc::clone(&m.opened().expect("o").documents.left);
    let edge = uncached_page(&doc, 0);
    assert!(doc.is_cached(edge - 3..edge - 2) && !doc.is_cached(edge..edge + 1));
    let before = m.counters.values_cancelled;
    m.set_caret(edge - 3, false);
    assert!(
        m.value_lines().is_some_and(|(_, stale)| *stale) && m.busy(),
        "a job, not a read on the UI thread"
    );
    // A newer byte cancels the older job (kills R25).
    m.set_caret(edge - 2, false);
    assert_eq!(m.counters.values_cancelled, before + 1);
    world.settle(&mut app);
    let m = hex_model(&mut app, view);
    let want = hikasa_hex::values::format_with(
        &hikasa_hex::values::at(&doc, edge - 2).expect("values"),
        hikasa_hex::ValueFormat {
            little: true,
            big: false,
            radix: hikasa_hex::Radix::Decimal,
        },
    );
    assert_eq!(m.value_lines().cloned(), Some((want, false)));
}

/// F14 (LOW): Copy and the primary selection of bytes that are not cached are read in jobs (no read on the UI thread),
/// and both get the selection's text.
#[test]
fn f14_copy_and_primary_of_uncached_bytes_run_in_jobs() {
    let world = hex_world();
    let mut app = world.app(None);
    let clip = std::sync::Arc::new(hikasa_session::clipboard::FixedClipboard::text("clip"));
    app.0
        .set_script_clipboard(std::sync::Arc::clone(&clip) as _);
    let a = bytes(22, 4 << 20);
    let view = paged(&world, &mut app, &a, &a);
    assert!(frames(&world, &mut app, view, 30));
    let m = hex_model(&mut app, view);
    let doc = std::sync::Arc::clone(&m.opened().expect("o").documents.left);
    let far = uncached_page(&doc, 3 << 20);
    m.set_caret(far + 10, false);
    m.set_caret(far + 13, true);
    assert!(!doc.is_cached(far + 10..far + 14));
    let written = m.primary_written;
    m.release();
    m.now += 1.0;
    m.copy(false);
    assert!(m.busy(), "the copy runs in a job");
    world.settle(&mut app);
    let m = hex_model(&mut app, view);
    m.frame(30, 5.0);
    world.settle(&mut app);
    let want: Vec<String> = a[(far + 10) as usize..(far + 14) as usize]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(
        clip.payloads().last().map(|p| p.text.clone()),
        Some(want.join(" "))
    );
    assert!(
        hex_model(&mut app, view).primary_written > written,
        "the primary selection"
    );
    assert_eq!(clip.primaries().last(), Some(&want.join(" ")));
}

/// F15 (LOW): Clear Session leaves nothing of the old pair: the queued step, the stale flag, the hit, the value lines
/// and the values job.
#[test]
fn f15_clear_session_drops_every_leftover() {
    let world = hex_world();
    let mut app = world.app(None);
    let a = bytes(23, 4 << 20);
    let view = paged(&world, &mut app, &a, &a);
    assert!(frames(&world, &mut app, view, 30));
    let m = hex_model(&mut app, view);
    m.set_values(hikasa_session::api::HexValues::Little);
    let doc = std::sync::Arc::clone(&m.opened().expect("o").documents.left);
    let far = uncached_page(&doc, 3 << 20);
    m.set_caret(far + 1, false);
    assert!(m.busy());
    m.find.pattern = "00".into();
    m.find.hex = true;
    m.validate_find();
    let cancelled = m.counters.values_cancelled;
    app.execute(CommandId::SessionClear);
    let m = hex_model(&mut app, view);
    assert_eq!(m.counters.values_cancelled, cancelled + 1, "the values job");
    assert!(m.value_lines().is_none() && m.find_hit().is_none() && !m.stale());
    world.settle(&mut app);
    let m = hex_model(&mut app, view);
    assert!(
        m.value_lines().is_none(),
        "no late values in the empty view"
    );
    assert!(!m.busy());
    assert_eq!(m.phase(), crate::hex::HexPhase::Empty);
}

/// F8 (MED): Previous Difference past a truncated list scans backwards to the run before the caret's and selects it;
/// in mode None before the landing Next and Previous both scan and select runs.
#[test]
fn f8_previous_difference_scans_back_and_selects_the_run() {
    let world = hex_world();
    let mut app = world.app(None);
    let a = bytes(72, 8000);
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
    m.set_caret(5000, false);
    m.navigate(Step::PrevDifference);
    world.settle(&mut app);
    let m = hex_model(&mut app, view);
    assert_eq!(m.selection(), 3000..3001, "the run before the caret");
    m.navigate(Step::PrevDifference);
    world.settle(&mut app);
    let m = hex_model(&mut app, view);
    assert_eq!(m.selection(), 1000..1032, "the whole run selected");
    assert_eq!(m.pane(Side::Left).caret, 1000);
    // Mode None before the landing: a held alignment, both directions scan.
    let gate = crate::tests_hex_open::Gate::default();
    m.set_aligner_for_test(gate.aligner());
    m.set_mode(AlignMode::None);
    m.reload();
    crate::tests_hex_open::until(&world, &mut app, |app| {
        let m = hex_model(app, view);
        m.phase() == crate::hex::HexPhase::Provisional
            && m.aligning().is_some()
            && gate.reached.load(std::sync::atomic::Ordering::SeqCst) >= 1
    });
    let m = hex_model(&mut app, view);
    m.set_caret(1010, false);
    m.navigate(Step::NextDifference);
    crate::tests_hex_open::until(&world, &mut app, |app| {
        hex_model(app, view).selection() != (1010..1011)
    });
    assert_eq!(hex_model(&mut app, view).selection(), 3000..3001);
    hex_model(&mut app, view).navigate(Step::PrevDifference);
    crate::tests_hex_open::until(&world, &mut app, |app| {
        hex_model(app, view).selection() != (3000..3001)
    });
    let m = hex_model(&mut app, view);
    assert_eq!(m.selection(), 1000..1032);
    assert!(m.alignment().is_none(), "still before the landing");
    gate.release();
    world.settle(&mut app);
}

// ---- mutation killers ----------------------------------------------------------------------------------------------

/// R2: a screen drawn from `try_rows` holds a margin above it (a short scroll up draws from the cache).
#[test]
fn m_r2_the_cached_screen_has_a_margin_above() {
    let world = hex_world();
    let mut app = world.app(None);
    let a = bytes(24, 1 << 20);
    let view = paged(&world, &mut app, &a, &a);
    let m = hex_model(&mut app, view);
    let rows = m.row_count();
    m.frame(30, 0.0);
    m.screen.set_top(10_000, rows);
    assert!(frames(&world, &mut app, view, 30));
    // A new generation over warm pages: `try_rows` fills the cache at once.
    let m = hex_model(&mut app, view);
    m.set_filter(DisplayFilter::All);
    assert!(m.frame(30, 50.0), "warm pages: drawn at once");
    let g = m.generation;
    assert!(
        m.screen.cached(g, 10_000 - 1).is_some(),
        "a row above the screen"
    );
    assert!(m.screen.cached(g, 10_000 - 30).is_some(), "a screen above");
}

/// R3: scrolling up within the cache reads ahead above the screen.
#[test]
fn m_r3_scrolling_up_reads_ahead_above() {
    let world = hex_world();
    let mut app = world.app(None);
    let a = bytes(25, 1 << 20);
    let view = paged(&world, &mut app, &a, &a);
    let m = hex_model(&mut app, view);
    let rows = m.row_count();
    m.frame(30, 0.0);
    m.screen.set_top(20_000, rows);
    assert!(frames(&world, &mut app, view, 30));
    for i in 0..3 {
        hex_model(&mut app, view).frame(30, 100.0 + f64::from(i));
        world.settle(&mut app);
    }
    let m = hex_model(&mut app, view);
    let before = m.screen.prefetches;
    m.screen.scroll_by(-2.0, rows);
    assert!(m.frame(30, 200.0), "drawn from the cache");
    assert_eq!(m.screen.prefetches, before + 1, "a read-ahead above");
}

/// R4: a landed prefetch that no longer covers the screen still fills an empty cache.
#[test]
fn m_r4_a_landed_prefetch_fills_an_empty_cache() {
    let world = hex_world();
    let mut app = world.app(None);
    let a = bytes(26, 4 << 20);
    let view = paged(&world, &mut app, &a, &a);
    let m = hex_model(&mut app, view);
    let rows = m.row_count();
    m.frame(30, 0.0);
    m.screen.set_top(100_000, rows);
    assert!(!m.frame(30, 1.0), "placeholders: a prefetch");
    // The screen moves away before the prefetch lands; no frame in between.
    m.screen.set_top(200_000, rows);
    world.settle(&mut app);
    let m = hex_model(&mut app, view);
    assert!(
        m.screen.cached(m.generation, 100_000).is_some(),
        "the landed rows fill the empty cache"
    );
}

/// R5, R7: the scroll model's edges: at the last top no part of a row is scrolled off; `reveal` of a partly
/// scrolled-off top row scrolls it fully in. R6: a third reveal puts the row at the screen's upper third.
#[test]
fn m_r5_r6_r7_scroll_edges_and_reveal() {
    let mut s = crate::hex::screen::Screen::default();
    s.rows_visible = 10;
    s.top = 50;
    s.frac = 0.5;
    s.clamp(60);
    assert_eq!(
        (s.top, s.frac),
        (50, 0.0),
        "the last top shows the last row whole"
    );
    s.top = 10;
    s.frac = 0.5;
    s.reveal(10, 1000, false);
    assert_eq!(
        (s.top, s.frac),
        (10, 0.0),
        "the partly hidden top row revealed"
    );
    let mut s = crate::hex::screen::Screen::default();
    s.rows_visible = 30;
    s.reveal(500, 10_000, true);
    assert_eq!(s.top, 490, "the upper third");
}

/// R8: a column after the last shown row of a filter is shown on the last row.
#[test]
fn m_r8_a_column_after_the_last_shown_row() {
    let world = hex_world();
    let mut app = world.app(None);
    let a = bytes(27, 16 * 64);
    let mut b = a.clone();
    b[3 * 16] ^= 1;
    let view = open_pair(&world, &mut app, &a, &b);
    let m = hex_model(&mut app, view);
    m.set_filter(DisplayFilter::Differences);
    assert_eq!(m.row_count(), 1);
    assert_eq!(m.row_of_column(60 * 16), Some(0));
}

/// R10: Find Previous whose only hit is at the caret wraps (*Wrapped to the end*).
#[test]
fn m_r10_find_previous_wraps_to_the_hit_at_the_caret() {
    let world = hex_world();
    let mut app = world.app(None);
    let (a, b) = gap_pair();
    let view = open_pair(&world, &mut app, &a, &b);
    let m = hex_model(&mut app, view);
    m.set_filter(DisplayFilter::All);
    m.set_caret(2000, false);
    m.open_find();
    m.find.pattern = "PATTERN!".into();
    m.find.hex = false;
    m.find.wrap = true;
    m.find.side = Side::Left;
    m.validate_find();
    m.find_step(false);
    world.settle(&mut app);
    let m = hex_model(&mut app, view);
    assert_eq!(m.find_hit().map(|(_, r)| r.clone()), Some(2000..2008));
    assert_eq!(message(&app, view), "Wrapped to the end.");
}

/// R12: a double-click selects the group under the pointer, never past the row's end (7 bytes a row, groups of 4).
#[test]
fn m_r12_a_double_click_group_ends_with_its_row() {
    let world = hex_world();
    let mut app = world.app(None);
    let a = bytes(28, 700);
    let view = open_pair(&world, &mut app, &a, &a);
    let m = hex_model(&mut app, view);
    m.set_filter(DisplayFilter::All);
    let layout = HexLayout {
        bytes_per_line: 7,
        group: 4,
        ..m.display().layout
    };
    m.set_layout(layout, true, crate::hex::PaneLayout::SideBySide, 0);
    let hit = crate::hex::pane::Hit {
        row: 2,
        cell: Some((crate::hex::pane::Part::Hex, 5)),
    };
    m.double_click(Side::Left, &hit);
    assert_eq!(m.selection(), 14 + 4..14 + 7);
}

/// R15: Find takes a selection of exactly 64 bytes as its hex text.
#[test]
fn m_r15_find_takes_a_64_byte_selection() {
    let world = hex_world();
    let mut app = world.app(None);
    let a = bytes(29, 700);
    let view = open_pair(&world, &mut app, &a, &a);
    let m = hex_model(&mut app, view);
    m.set_filter(DisplayFilter::All);
    m.set_caret(100, false);
    m.set_caret(163, true);
    m.open_find();
    assert!(m.find.hex);
    assert_eq!(m.find.pattern.split(' ').count(), 64);
}

/// R18, R19: a landing maps the caret and the top by the other side's byte when the focus side has none at their
/// column (a gap), so they stay at the same bytes of the pair.
#[test]
fn m_r18_r19_a_landing_maps_a_gap_column_by_the_other_side() {
    let world = hex_world();
    let mut app = world.app(None);
    let (a, b) = gap_pair();
    let view = open_pair(&world, &mut app, &a, &b);
    let m = hex_model(&mut app, view);
    m.set_filter(DisplayFilter::All);
    m.frame(10, 0.0);
    let gap = left_gap_column(m);
    assert_eq!(m.focus(), Side::Left);
    let caret = gap + 250;
    let (right_byte, held) = m.byte_at(Side::Right, caret).expect("byte");
    assert!(held && !m.byte_at(Side::Left, caret).expect("b").1);
    m.set_caret(caret, false);
    let rows = m.row_count();
    let top = (gap + 160) / 16;
    m.screen.set_top(top, rows);
    let top_right = m.byte_at(Side::Right, top * 16).expect("b").0;
    // Mode None lands: no gaps, each column holds byte n of both sides.
    m.set_mode(AlignMode::None);
    world.settle(&mut app);
    let m = hex_model(&mut app, view);
    assert_eq!(m.alignment().expect("landed").mode, AlignMode::None);
    assert_eq!(
        m.byte_at(Side::Right, m.pane(Side::Left).caret)
            .map(|(o, _)| o),
        Some(right_byte),
        "the caret stays on the right side's byte"
    );
    assert_eq!(
        m.screen.top,
        top_right / 16,
        "the top stays on the right side's byte"
    );
}

/// R20: the status bar's selection length counts the focus side's bytes, not the gap columns.
#[test]
fn m_r20_the_selection_length_counts_bytes() {
    let world = hex_world();
    let mut app = world.app(None);
    let (a, b) = gap_pair();
    let view = open_pair(&world, &mut app, &a, &b);
    let m = hex_model(&mut app, view);
    m.set_filter(DisplayFilter::All);
    let gap = left_gap_column(m);
    // 10 left bytes before the gap, the 500 gap columns, 10 left bytes after.
    m.set_caret(gap - 10, false);
    m.set_caret(gap + 509, true);
    let s = crate::hex::status::items(m);
    assert!(
        s.right.contains(&"20 bytes selected".to_owned()),
        "{:?}",
        s.right
    );
}

/// R21: the thumbnail frames a tiny screen of a huge pair with at least one pixel row.
#[test]
fn m_r21_the_thumbnail_frame_is_never_empty() {
    let (y0, y1) = crate::hex::thumbnail::frame_of(1000..1016, 1 << 40, 600);
    assert!(y1 > y0, "{y0}..{y1}");
}

/// R22 (equivalent, recorded): the Format tab writes a side only when it changed (T-61's identity): OK without an edit
/// leaves `[specs]` as they were; an edit of the right side writes only it.
#[test]
fn m_r22_the_format_tab_writes_only_its_edits() {
    let mut tab = crate::hex_tabs::HexFormatTab::default();
    let mut specs = hikasa_session::sessions::Specs {
        left_format: "firmware".into(),
        ..Default::default()
    };
    tab.load_specs(&specs);
    let before = specs.clone();
    crate::session_dialog::SessionTab::specs(&tab, &mut specs);
    assert_eq!(specs, before);
    tab.right = "firmware".into();
    crate::session_dialog::SessionTab::specs(&tab, &mut specs);
    assert_eq!(specs.left_format, "firmware");
    assert_eq!(specs.right_format, "firmware");
}

/// R24: Go To Next/Previous Pane switch the focus side, the column kept.
#[test]
fn m_r24_go_next_and_previous_pane() {
    let world = hex_world();
    let mut app = world.app(None);
    let a = bytes(30, 700);
    let view = open_pair(&world, &mut app, &a, &a);
    hex_model(&mut app, view).set_caret(77, false);
    app.execute(CommandId::GoNext);
    assert_eq!(hex_model(&mut app, view).focus(), Side::Right);
    assert_eq!(hex_model(&mut app, view).pane(Side::Right).caret, 77);
    app.execute(CommandId::GoPrev);
    assert_eq!(hex_model(&mut app, view).focus(), Side::Left);
}

/// R28: a layout without the characters column puts the caret back in the hex part (Copy then copies hex pairs).
#[test]
fn m_r28_no_characters_column_no_characters_part() {
    let world = hex_world();
    let mut app = world.app(None);
    let a = bytes(31, 700);
    let view = open_pair(&world, &mut app, &a, &a);
    let m = hex_model(&mut app, view);
    m.switch_part();
    assert_eq!(m.pane(Side::Left).part, crate::hex::pane::Part::Chars);
    let layout = HexLayout {
        chars: hikasa_hex::CharSet::None,
        ..m.display().layout
    };
    m.set_layout(layout, true, crate::hex::PaneLayout::SideBySide, 0);
    assert_eq!(m.pane(Side::Left).part, crate::hex::pane::Part::Hex);
    assert_eq!(m.pane(Side::Right).part, crate::hex::pane::Part::Hex);
}
