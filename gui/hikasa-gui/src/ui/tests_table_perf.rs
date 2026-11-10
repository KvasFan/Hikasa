//! The UI-thread costs of the grid (phase-3 review F12), headless, `#[ignore]`: run in release
//! (`cargo test --release -p hikasa-gui tests_table_perf -- --ignored --nocapture`). The figures are printed; in a
//! release build they are asserted: a frame of a 16,384-column sheet < 2 ms, a navigation step on 1,000,000 rows under
//! the Same filter < 2 ms.

use std::time::Instant;

use hikasa_core::Side;

use super::HikasaApp;
use super::tests::{frame, new_app, raw};
use crate::commands::CommandId;
use crate::tests::World;
use crate::window::ViewModel;

fn model(
    app: &mut HikasaApp,
    view: hikasa_session::ipc::ViewId,
) -> &mut crate::table_view::TableViewModel {
    match app.core.tab_mut(view).and_then(|t| t.model_mut()) {
        Some(ViewModel::Table(m)) => m,
        _ => panic!("not a table"),
    }
}

/// Milliseconds a frame, over `n` frames.
fn frames(app: &mut HikasaApp, ctx: &egui::Context, t: &mut f64, n: usize) -> f64 {
    let start = Instant::now();
    for _ in 0..n {
        *t += 0.05;
        frame(app, ctx, raw(*t));
    }
    start.elapsed().as_secs_f64() * 1000.0 / n as f64
}

fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

/// A gate asserted in release builds only (a debug build is several times slower).
fn gate(what: &str, value: f64, limit: f64) {
    eprintln!("PERF {what}: {value:.3} ms (gate < {limit} ms)");
    if !cfg!(debug_assertions) {
        assert!(value < limit, "{what}: {value:.3} ms ≥ {limit} ms");
    }
}

#[test]
#[ignore = "UI-thread figures; run in release"]
fn a_wide_sheet_costs_its_screen_only() {
    let world = World::new();
    let ctx = egui::Context::default();
    let mut app = new_app(&world, &ctx);
    let (cols, rows) = (16_384usize, 50usize);
    let (mut l, mut r) = (String::new(), String::new());
    for row in 0..=rows {
        let line = |right: bool| -> String {
            (0..cols)
                .map(|c| match row {
                    0 => format!("h{c}"),
                    _ if right && c % 97 == 0 && row % 3 == 0 => "x".to_owned(),
                    _ => format!("{}", row * 7 + c),
                })
                .collect::<Vec<_>>()
                .join(",")
        };
        l.push_str(&line(false));
        l.push('\n');
        r.push_str(&line(true));
        r.push('\n');
    }
    let a = world.write("work/w/a.csv", &l);
    let b = world.write("work/w/b.csv", &r);
    let view = world.open(&mut app.core, &a, &b);
    world.settle(&mut app.core);
    let mut t = 0.0;
    frames(&mut app, &ctx, &mut t, 3);
    assert_eq!(model(&mut app, view).shown_columns().len(), cols);
    gate(
        "16,384 columns: a frame",
        frames(&mut app, &ctx, &mut t, 20),
        2.0,
    );
    let service = world.service.clone();
    {
        let m = model(&mut app, view);
        m.click(1, 5, Side::Left);
        m.execute(&service, CommandId::ShowDetails);
    }
    gate(
        "16,384 columns: a frame with the details",
        frames(&mut app, &ctx, &mut t, 20),
        2.0,
    );
    let start = Instant::now();
    model(&mut app, view).fit_columns();
    gate("16,384 columns: Resize Columns to Fit", ms(start), 100.0);
    model(&mut app, view).click_row(1, Side::Left);
    let start = Instant::now();
    let text = model(&mut app, view).copy_text().unwrap_or_default();
    eprintln!(
        "PERF 16,384 columns: copy a row {:.1} ms ({} bytes)",
        ms(start),
        text.len()
    );
    let start = Instant::now();
    model(&mut app, view).execute(&service, CommandId::DiffNext);
    gate("16,384 columns: next difference", ms(start), 2.0);
}

#[test]
#[ignore = "UI-thread figures; run in release"]
fn a_million_rows_navigate_under_any_filter() {
    let world = World::new();
    let ctx = egui::Context::default();
    let mut app = new_app(&world, &ctx);
    let n = 1_000_000usize;
    let (mut l, mut r) = (String::from("id,v\n"), String::from("id,v\n"));
    for i in 0..n {
        l.push_str(&format!("{i},{}\n", i % 10));
        r.push_str(&format!("{i},{}\n", if i % 2 == 0 { i % 10 } else { 99 }));
    }
    let a = world.write("work/m/a.csv", &l);
    let b = world.write("work/m/b.csv", &r);
    let view = world.open(&mut app.core, &a, &b);
    world.settle(&mut app.core);
    let mut t = 0.0;
    frames(&mut app, &ctx, &mut t, 3);
    let service = world.service.clone();
    let start = Instant::now();
    model(&mut app, view).execute(&service, CommandId::FavSame);
    eprintln!(
        "PERF 1M rows: the Same filter {:.1} ms → {} rows",
        ms(start),
        model(&mut app, view).len()
    );
    let start = Instant::now();
    model(&mut app, view).click(10, 1, Side::Left);
    gate("1M rows, Same: a click", ms(start), 2.0);
    let start = Instant::now();
    for _ in 0..10 {
        model(&mut app, view).move_cursor(1, 0);
    }
    gate("1M rows, Same: an arrow key", ms(start) / 10.0, 2.0);
    for id in [
        CommandId::DiffNext,
        CommandId::DiffItemNext,
        CommandId::DiffPrev,
    ] {
        let start = Instant::now();
        model(&mut app, view).execute(&service, id);
        gate(&format!("1M rows, Same: {id:?}"), ms(start), 2.0);
    }
    model(&mut app, view).execute(&service, CommandId::FavCompareAll);
    for id in [CommandId::TogSame, CommandId::TogDiff, CommandId::TogSame] {
        model(&mut app, view).execute(&service, id);
    }
    let start = Instant::now();
    model(&mut app, view).click(10, 1, Side::Left);
    gate("1M rows, changed rows hidden: a click", ms(start), 2.0);
    gate("1M rows: a frame", frames(&mut app, &ctx, &mut t, 10), 2.0);
}
