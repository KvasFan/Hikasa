//! A13.2 (M2 W13a, D-355): the row cache and scrolling — placeholders and one prefetch job for a screen whose pages
//! are not cached, latest wins; an older generation's prefetch is not drawn; P-HEXSCREEN: random scroll scripts
//! (wheel, pages, thumb jumps, filters, layouts) on generated pairs: the rows drawn equal `HexView::rows` of that range
//! once their pages are cached, and `top` is where the script put it; the scroll model reaches the last row of a
//! 2⁴⁰-byte view exactly.

use hikasa_hex::{DisplayFilter, HexLayout};
use proptest::prelude::*;

use crate::hex::screen::{Screen, ShownRow};
use crate::tests_hex_open::{bytes, frames, hex_model, hex_world, open_pair, with_insert};

/// Paged documents (`unlocked_max_mib = 0`): 64 KiB pages, 256 cached.
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

/// A13.2: a screen whose pages are not cached draws placeholders and starts one prefetch; when it lands the rows
/// are the core's; a jump far away does the same; a second jump before the first lands: the latest wins.
#[test]
fn a13_2_placeholders_then_one_prefetch() {
    let world = hex_world();
    let mut app = world.app(None);
    let a = bytes(7, 4 << 20);
    let b = with_insert(&a, 2 << 20, 11, 3 << 20);
    let view = paged(&world, &mut app, &a, &b);
    let m = hex_model(&mut app, view);
    let rows = m.row_count();
    // Far from anything read so far (the alignment's pass reads without the page cache).
    m.screen.set_top(rows / 2, rows);
    let before = m.screen.prefetches;
    let complete = m.frame(40, 0.0);
    assert!(!complete, "not cached: placeholders");
    assert_eq!(m.screen.row(m.generation, rows / 2), ShownRow::Placeholder);
    assert_eq!(m.screen.prefetches, before + 1, "one prefetch job");
    // The same screen again: no second job.
    assert!(!m.frame(40, 0.1));
    assert_eq!(m.screen.prefetches, before + 1);
    assert!(frames(&world, &mut app, view, 40));
    let m = hex_model(&mut app, view);
    let src = m.source().expect("src").clone();
    let expected = src.view().rows(rows / 2, 41).expect("rows");
    for (i, row) in expected.iter().enumerate() {
        assert_eq!(
            m.screen.cached(m.generation, rows / 2 + i as u64),
            Some(row)
        );
    }
    // Two jumps: the second's prefetch is the one drawn.
    m.screen.set_top(rows / 5, rows);
    m.frame(40, 1.0);
    m.screen.set_top(rows - 100, rows);
    m.frame(40, 1.1);
    assert!(frames(&world, &mut app, view, 40));
    let m = hex_model(&mut app, view);
    assert!(m.screen.cached(m.generation, rows - 100).is_some());
}

/// A13.2: a prefetch of an older generation is not drawn (a filter change while it runs): its answer only warmed the
/// page caches. Mutation: the cache served for a stale generation.
#[test]
fn a13_2_an_older_generation_is_not_drawn() {
    let world = hex_world();
    let mut app = world.app(None);
    let a = bytes(8, 2 << 20);
    let b = with_insert(&a, 1 << 19, 3, 1 << 20);
    let view = paged(&world, &mut app, &a, &b);
    let m = hex_model(&mut app, view);
    let rows = m.row_count();
    m.screen.set_top(rows - 200, rows);
    assert!(!m.frame(30, 0.0));
    let old = m.generation;
    // The layout changes before the prefetch lands: a new generation.
    m.set_layout(
        HexLayout {
            bytes_per_line: 8,
            ..HexLayout::default()
        },
        true,
        crate::hex::PaneLayout::SideBySide,
        0,
    );
    let new = m.generation;
    assert!(new > old);
    world.settle(&mut app);
    let m = hex_model(&mut app, view);
    for k in 0..m.row_count().min(5) {
        assert!(
            m.screen.cached(old, k + rows - 200).is_none() || m.screen.cached(new, k).is_none(),
            "nothing of the old generation is drawn in the new one"
        );
    }
    let top = m.screen.top;
    if let Some(row) = m.screen.cached(new, top) {
        // A row of the new generation is the core's row of the new layout.
        let src = m.source().expect("src").clone();
        assert_eq!(&src.view().rows(top, 1).expect("rows")[0], row);
    }
}

/// A13.2: the scroll model of a 2⁴⁰-byte view (2³⁶ rows of 16 bytes): the thumb reaches the last row exactly, pages
/// and the wheel stay within the rows. Mutation: the thumb mapped through `f32`.
#[test]
fn a13_2_the_scroll_model_reaches_the_last_row_of_a_terabyte() {
    let rows: u64 = (1u64 << 40) / 16;
    let mut s = Screen::default();
    s.rows_visible = 50;
    s.thumb_to(1.0, rows);
    assert_eq!(s.top, rows - 50, "the last screen exactly");
    s.thumb_to(0.0, rows);
    assert_eq!(s.top, 0);
    // A jump into the middle lands on an exact row, then the wheel moves rows one by one.
    s.thumb_to(0.5, rows);
    let mid = s.top;
    assert_eq!(mid, ((rows - 50) as f64 * 0.5).round() as u64);
    s.scroll_by(3.0, rows);
    assert_eq!(s.top, mid + 3);
    s.scroll_by(-1.5, rows);
    assert_eq!((s.top, s.frac), (mid + 1, 0.5));
    s.page(1, rows);
    assert_eq!(s.top, mid + 51);
    s.set_top(rows - 51, rows);
    s.scroll_by(1.0, rows);
    assert_eq!(s.top, rows - 50);
    s.scroll_by(10.0, rows);
    assert_eq!((s.top, s.frac), (rows - 50, 0.0), "clamped at the end");
    // Every 2^30-th row of the trough is reachable exactly by the thumb.
    for k in [1u64 << 30, (1 << 35) + 7, rows - 51] {
        s.thumb_to(k as f64 / (rows - 50) as f64, rows);
        assert_eq!(s.top, k);
    }
}

/// One step of a scroll script.
#[derive(Debug, Clone)]
enum Op {
    Wheel(i32),
    Page(i8),
    Thumb(f64),
    Filter(u8),
    Layout(u16),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        (-40i32..40).prop_map(Op::Wheel),
        (-3i8..4).prop_map(Op::Page),
        (0.0f64..=1.0).prop_map(Op::Thumb),
        (0u8..3).prop_map(Op::Filter),
        prop::sample::select(vec![1u16, 7, 16, 33, 256]).prop_map(Op::Layout),
    ]
}

fn cases() -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(6)
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: cases(),
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// P-HEXSCREEN.
    #[test]
    fn p_hexscreen(seed in any::<u64>(), len in 1usize..(1 << 20), script in prop::collection::vec(op(), 1..12)) {
        let world = hex_world();
        let mut app = world.app(None);
        let a = bytes(seed, len);
        let b = with_insert(&a, len / 3, (seed % 50) as usize + 1, len / 2);
        let view = open_pair(&world, &mut app, &a, &b);
        hex_model(&mut app, view).set_filter(DisplayFilter::All);
        let visible = 30u32;
        for op in script {
            let m = hex_model(&mut app, view);
            m.frame(visible, 0.0);
            let rows = m.row_count();
            let (top, frac) = (m.screen.top, m.screen.frac);
            let max = m.screen.max_top(rows);
            match op {
                Op::Wheel(n) => {
                    m.screen.scroll_by(f64::from(n), rows);
                    let want = (top as f64 + f64::from(frac) + f64::from(n)).clamp(0.0, max as f64);
                    prop_assert_eq!(m.screen.top, want.floor() as u64);
                }
                Op::Page(n) => {
                    m.screen.page(i64::from(n), rows);
                    let want = (i128::from(top) + i128::from(visible) * i128::from(n)).clamp(0, i128::from(max));
                    prop_assert_eq!(i128::from(m.screen.top), want);
                }
                Op::Thumb(f) => {
                    m.screen.thumb_to(f, rows);
                    prop_assert_eq!(m.screen.top, ((max as f64) * f).round() as u64);
                }
                Op::Filter(k) => {
                    let f = [DisplayFilter::All, DisplayFilter::Differences, DisplayFilter::Same][k as usize];
                    m.set_filter(f);
                }
                Op::Layout(bpl) => {
                    let layout = HexLayout { bytes_per_line: bpl, ..m.display().layout };
                    m.set_layout(layout, true, crate::hex::PaneLayout::SideBySide, 2);
                }
            }
            prop_assert!(frames(&world, &mut app, view, visible));
            let m = hex_model(&mut app, view);
            let src = m.source().expect("src").clone();
            let top = m.screen.top;
            prop_assert!(top <= m.screen.max_top(m.row_count()));
            let expected = src.view().rows(top, visible + 1).expect("rows");
            for (i, row) in expected.iter().enumerate() {
                prop_assert_eq!(m.screen.cached(m.generation, top + i as u64), Some(row));
            }
        }
    }
}
