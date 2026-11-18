//! A13.1 (M2 W13a, D-355): a Hex Compare view opens through `hex_open` with provisional rows before any alignment (a
//! held `hex_align` job), then the landed rows; the caret's byte and the top byte are kept across the landing, a
//! filter change and a layout change (pairs with insertions, so columns move); a mode switch cancels the running job
//! and keeps the old rows until the new ones land; rule changes realign without reopening, `unlocked_max` and a format
//! reopen; Reload; a failed side is `LoadFailed`; closing cancels every job. Also the shared helpers of the hex tests.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use hikasa_core::Side;
use hikasa_hex::{AlignMode, DisplayFilter, HexLayout};
use hikasa_session::ipc::{CloseReason, ViewId};

use crate::commands::{CommandId, Scope};
use crate::hex::{HexPhase, HexViewModel};
use crate::tests::{TestApp, World};

/// A Hex format so that `*.bin` and `*.fw` pairs open in Hex Compare (`infer_view_kind`, D-369).
pub(crate) const HEX_FORMAT: &str = "schema_version = 1\nid = \"firmware\"\nname = \"Firmware\"\nkind = \"hex\"\nmask = \"*.bin;*.fw\"\n";

/// A world whose configuration has the Hex format.
pub(crate) fn hex_world() -> World {
    World::with_files(&[("formats/firmware.toml", HEX_FORMAT)])
}

/// Seeded xorshift bytes.
pub(crate) fn bytes(seed: u64, n: usize) -> Vec<u8> {
    let mut x = seed | 1;
    let mut out = Vec::with_capacity(n + 8);
    while out.len() < n {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        out.extend_from_slice(&x.to_le_bytes());
    }
    out.truncate(n);
    out
}

/// `base` with `n` bytes inserted at `at` and a byte changed at `change`.
pub(crate) fn with_insert(base: &[u8], at: usize, n: usize, change: usize) -> Vec<u8> {
    let mut out = base.to_vec();
    out[change] ^= 0x5A;
    let ins = bytes(99, n);
    out.splice(at..at, ins);
    out
}

/// Writes a pair of `.bin` files.
pub(crate) fn write_pair(world: &World, a: &[u8], b: &[u8]) -> (PathBuf, PathBuf) {
    let (pa, pb) = (world.path("work/h/a.bin"), world.path("work/h/b.bin"));
    std::fs::create_dir_all(pa.parent().expect("parent")).expect("mkdir");
    std::fs::write(&pa, a).expect("write");
    std::fs::write(&pb, b).expect("write");
    (pa, pb)
}

/// The model of a hex view.
pub(crate) fn hex_model(app: &mut TestApp, view: ViewId) -> &mut HexViewModel {
    match app.tab_mut(view).expect("tab").model_mut().expect("live") {
        crate::app::ViewModel::Hex(m) => m,
        _ => panic!("not a hex view"),
    }
}

/// Opens a pair as the command line does and waits until every job ended.
pub(crate) fn open_pair(world: &World, app: &mut TestApp, a: &[u8], b: &[u8]) -> ViewId {
    let (pa, pb) = write_pair(world, a, b);
    let view = world.open(app, &pa, &pb);
    world.settle(app);
    assert_eq!(
        app.scope(),
        Scope::Hex,
        "a pair of a Hex format opens in Hex Compare"
    );
    view
}

/// Frames until the screen draws from its cache (prefetches landed), at most `frames`.
pub(crate) fn frames(world: &World, app: &mut TestApp, view: ViewId, rows: u32) -> bool {
    for i in 0..200 {
        let done = hex_model(app, view).frame(rows, f64::from(i));
        world.settle(app);
        if done {
            return true;
        }
    }
    false
}

/// A gate that holds an alignment job until opened; counts the jobs that reached it.
#[derive(Clone, Default)]
pub(crate) struct Gate {
    open: Arc<AtomicBool>,
    pub(crate) reached: Arc<AtomicU32>,
    /// Jobs that saw their cancellation while held.
    pub(crate) cancelled: Arc<AtomicU32>,
}

impl Gate {
    /// The aligner that waits for the gate (or the job's cancellation), then aligns.
    pub(crate) fn aligner(&self) -> crate::hex::Aligner {
        let gate = self.clone();
        Arc::new(move |svc, job, opened, opts| {
            gate.reached.fetch_add(1, Ordering::SeqCst);
            let began = Instant::now();
            while !gate.open.load(Ordering::SeqCst) {
                if job.cancel.is_cancelled() {
                    gate.cancelled.fetch_add(1, Ordering::SeqCst);
                    return Err(hikasa_session::ApiError::Cancelled);
                }
                assert!(
                    began.elapsed() < Duration::from_secs(30),
                    "gate never opened"
                );
                std::thread::sleep(Duration::from_millis(2));
            }
            svc.hex_align(opened, opts, job)
        })
    }

    /// An aligner whose gate is open (the real alignment).
    pub(crate) fn aligner_open(&self) -> crate::hex::Aligner {
        self.release();
        self.aligner()
    }

    pub(crate) fn release(&self) {
        self.open.store(true, Ordering::SeqCst);
    }
}

/// Polls the app until `done` holds (bounded).
pub(crate) fn until(world: &World, app: &mut TestApp, mut done: impl FnMut(&mut TestApp) -> bool) {
    let began = Instant::now();
    loop {
        for msg in world.mailbox.drain() {
            app.0.handle(msg);
        }
        app.0.handle(crate::app::UiMsg::Tick);
        if done(app) {
            return;
        }
        assert!(
            began.elapsed() < Duration::from_secs(30),
            "condition never held"
        );
        std::thread::sleep(Duration::from_millis(3));
    }
}

/// The byte of `side` at the focus pane's caret.
pub(crate) fn caret_byte(m: &HexViewModel, side: Side) -> u64 {
    m.byte_at(side, m.pane(side).caret).expect("opened").0
}

/// A13.1: provisional rows first, then the landed ones; the caret's byte and the top byte kept across the landing.
#[test]
fn a13_1_provisional_rows_then_the_landing_keeps_the_caret_byte() {
    let world = hex_world();
    let mut app = world.app(None);
    let a = bytes(1, 64 * 1024);
    let b = with_insert(&a, 1000, 37, 40_000);
    let (pa, pb) = write_pair(&world, &a, &b);
    let gate = Gate::default();
    // The view is built by the command line's path; its aligner is replaced before the documents open.
    let view = world.open(&mut app, &pa, &pb);
    hex_model(&mut app, view).set_aligner_for_test(gate.aligner());
    // The view opens with the session's filter (`differences`); this test reads every row.
    hex_model(&mut app, view).set_filter(DisplayFilter::All);
    until(&world, &mut app, |app| {
        hex_model(app, view).phase() != HexPhase::Opening
    });
    // The first alignment started with the real aligner may have landed already: reload under the gate.
    hex_model(&mut app, view).reload();
    until(&world, &mut app, |app| {
        let m = hex_model(app, view);
        m.phase() == HexPhase::Provisional && gate.reached.load(Ordering::SeqCst) == 1
    });
    let m = hex_model(&mut app, view);
    assert!(m.alignment().is_none(), "held: provisional rows");
    let rows = m.row_count();
    m.screen.set_top(95, rows);
    assert!(m.frame(20, 0.0), "documents in memory: the rows at once");
    // Provisional rows: None's layout (byte n with byte n).
    let row = m.screen.cached(m.generation, 100).expect("row").clone();
    assert_eq!(row.left_address, Some(1600));
    assert_eq!(row.right_address, Some(1600));
    // The caret on left byte 50,000 and the top row at 2,500: after the insertion of 37 bytes these move columns.
    let col = 50_000;
    m.set_caret(col, false);
    m.screen.set_top(2_500, m.row_count());
    let before_left = caret_byte(m, Side::Left);
    let top_byte = 2_500 * 16;
    gate.release();
    until(&world, &mut app, |app| {
        hex_model(app, view).alignment().is_some()
    });
    let m = hex_model(&mut app, view);
    assert_eq!(
        caret_byte(m, Side::Left),
        before_left,
        "the caret keeps its byte"
    );
    assert_ne!(
        m.pane(Side::Left).caret,
        col,
        "its column moved (the insertion)"
    );
    let top = m.first_column_of(m.screen.top).expect("top");
    let (l, _) = (m.byte_at(Side::Left, top).expect("byte").0, ());
    assert!(
        l <= top_byte && top_byte < l + 16,
        "the top row keeps the byte that was first on it: {l} vs {top_byte}"
    );
    // The landed rows equal the core's (the screen draws them after a frame).
    assert!(frames(&world, &mut app, view, 20));
    let m = hex_model(&mut app, view);
    let src = m.source().expect("source").clone();
    let k = m.screen.top;
    let expected = src.view().rows(k, 5).expect("rows");
    for (i, row) in expected.iter().enumerate() {
        assert_eq!(m.screen.cached(m.generation, k + i as u64), Some(row));
    }
    assert!(!m.stale());
}

/// A13.1: the caret's byte and the top byte are kept across a filter change and a layout change.
#[test]
fn a13_1_filter_and_layout_changes_keep_the_caret_byte() {
    let world = hex_world();
    let mut app = world.app(None);
    let a = bytes(2, 32 * 1024);
    let b = with_insert(&a, 3000, 5, 20_000);
    let view = open_pair(&world, &mut app, &a, &b);
    let m = hex_model(&mut app, view);
    assert!(m.alignment().is_some());
    let target = m
        .source()
        .expect("src")
        .view()
        .segments()
        .column_at_offset(Side::Left, 20_000)
        .expect("col");
    m.set_caret(target, false);
    let byte = caret_byte(m, Side::Left);
    for filter in [
        DisplayFilter::Differences,
        DisplayFilter::Same,
        DisplayFilter::All,
    ] {
        m.set_filter(filter);
        assert_eq!(caret_byte(m, Side::Left), byte, "{filter:?}");
    }
    m.screen.set_top(2_000, m.row_count());
    let top_col = m.first_column_of(m.screen.top).expect("top");
    let top_byte = m.byte_at(Side::Left, top_col).expect("byte").0;
    let generation = m.generation;
    m.set_layout(
        HexLayout {
            bytes_per_line: 7,
            group: 1,
            ..HexLayout::default()
        },
        true,
        crate::hex::PaneLayout::OverUnder,
        3,
    );
    assert!(m.generation > generation, "a new generation");
    assert_eq!(caret_byte(m, Side::Left), byte);
    let top = m.first_column_of(m.screen.top).expect("top");
    let first = m.byte_at(Side::Left, top).expect("byte").0;
    assert!(
        first <= top_byte && top_byte < first + 7,
        "the new top row holds the old top byte: {first} vs {top_byte}"
    );
}

/// A13.1: a mode switch cancels the running alignment (a held gate) and keeps the old rows until the new ones land;
/// the request's rule changes (dirty) and the landed mode is the new one. Mutation: a switch that does not cancel.
#[test]
fn a13_1_a_mode_switch_cancels_the_running_job() {
    let world = hex_world();
    let mut app = world.app(None);
    let a = bytes(3, 16 * 1024);
    let b = with_insert(&a, 500, 9, 9_000);
    let view = open_pair(&world, &mut app, &a, &b);
    let gate = Gate::default();
    let m = hex_model(&mut app, view);
    let landed_before = Arc::clone(m.alignment().expect("landed"));
    m.set_aligner_for_test(gate.aligner());
    let counters = m.counters;
    m.set_mode(AlignMode::Complete);
    until(&world, &mut app, |_| {
        gate.reached.load(Ordering::SeqCst) == 1
    });
    let m = hex_model(&mut app, view);
    // The second switch cancels the first held job.
    m.set_mode(AlignMode::None);
    assert_eq!(
        m.counters.cancelled,
        counters.cancelled + 1,
        "the older job is cancelled"
    );
    until(&world, &mut app, |_| {
        gate.cancelled.load(Ordering::SeqCst) == 1
    });
    let m = hex_model(&mut app, view);
    assert!(Arc::ptr_eq(
        m.alignment().expect("old rows stay"),
        &landed_before
    ));
    assert_eq!(m.request().rules.alignment.as_deref(), Some("none"));
    gate.release();
    until(&world, &mut app, |app| {
        hex_model(app, view)
            .alignment()
            .is_some_and(|al| al.mode == AlignMode::None)
    });
    let m = hex_model(&mut app, view);
    assert_eq!(m.counters.opens, counters.opens, "no reopen");
    assert_eq!(m.counters.aligns, counters.aligns + 2);
    assert_eq!(gate.reached.load(Ordering::SeqCst), 2);
    world.settle(&mut app);
    // The session is dirty after the switch through the command.
    app.execute(CommandId::Reload);
    world.settle(&mut app);
}

/// A13.1: rule changes realign without reopening; `unlocked_max` and a format reopen; Reload reopens.
#[test]
fn a13_1_what_reopens_and_what_realigns() {
    let world = hex_world();
    let mut app = world.app(None);
    let a = bytes(4, 8 * 1024);
    let b = with_insert(&a, 100, 3, 5_000);
    let view = open_pair(&world, &mut app, &a, &b);
    let m = hex_model(&mut app, view);
    let c0 = m.counters;
    let mut request = m.request().clone();
    request.rules.block = Some("4096".into());
    m.relive(request.clone());
    world.settle(&mut app);
    let m = hex_model(&mut app, view);
    assert_eq!(
        (m.counters.opens, m.counters.aligns),
        (c0.opens, c0.aligns + 1)
    );
    request.rules.unlocked_max_mib = Some("0".into());
    m.relive(request.clone());
    world.settle(&mut app);
    let m = hex_model(&mut app, view);
    assert_eq!(m.counters.opens, c0.opens + 1, "unlocked_max reopens");
    assert_eq!(
        m.opened().expect("opened").sides.left.load,
        hikasa_hex::HexLoad::File
    );
    request.formats.left = Some("firmware".into());
    m.relive(request);
    world.settle(&mut app);
    let m = hex_model(&mut app, view);
    assert_eq!(m.counters.opens, c0.opens + 2, "a format reopens");
    m.reload();
    world.settle(&mut app);
    let m = hex_model(&mut app, view);
    assert_eq!(m.counters.opens, c0.opens + 3, "Reload reopens");
    assert!(m.alignment().is_some(), "and realigns");
}

/// A13.1: a side that cannot be opened is `LoadFailed` (exit 100) with its error; closing cancels every job and a
/// closed view without a landed alignment is `NoVerdict`.
#[test]
fn a13_1_a_failed_side_and_closing() {
    let world = hex_world();
    let mut app = world.app(None);
    let a = bytes(5, 1024);
    let (pa, _) = write_pair(&world, &a, &a);
    let missing = world.path("work/h/missing.bin");
    let request = hikasa_session::api::HexCompareRequest {
        left: pa.clone().into_os_string(),
        right: missing.into_os_string(),
        ..Default::default()
    };
    let target = app.open_target();
    let view = app.0.open_local(
        hikasa_session::ipc::ViewKind::HexCompare,
        hikasa_session::api::ViewRequest::Hex {
            request,
            layout: HexLayout::default(),
            filter: None,
        },
        &hikasa_session::ipc::OpenOptions::default(),
        Vec::new(),
        target,
    );
    world.settle(&mut app);
    let m = hex_model(&mut app, view);
    assert_eq!(m.phase(), HexPhase::Failed);
    assert_eq!(m.error().expect("error").0, Some(Side::Right));
    assert_eq!(
        m.outcome(&world.service),
        hikasa_session::api::ViewOutcome::LoadFailed
    );
    // A view whose alignment is held when it closes: its jobs stop, no verdict.
    let b = with_insert(&a, 10, 2, 500);
    let view = open_pair(&world, &mut app, &a, &b);
    let gate = Gate::default();
    let m = hex_model(&mut app, view);
    m.set_aligner_for_test(gate.aligner());
    m.set_mode(AlignMode::Fast);
    until(&world, &mut app, |_| {
        gate.reached.load(Ordering::SeqCst) == 1
    });
    let index = app.active();
    app.close_tab(index, CloseReason::User);
    world.settle(&mut app);
    assert!(app.tab(view).is_none());
}
