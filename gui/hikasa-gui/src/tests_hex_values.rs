//! A13.8 (M2 W13a, D-360): the value panel's lines equal `format_with` at the caret, at a gap and at the file's last
//! bytes, from a job when the bytes are not cached; the thumbnail's pixel rows from the buckets for strip heights 50,
//! 4,096 and 5,000 (a golden) and its clicks; the headers' fields from `info`; every status item; Info's figures and
//! its Copy equal `human_lines`; the context-menu goldens of the hex areas; a drop on a pane reopens that side.
//! Mutation: the thumbnail paints *orphan* over *different*.

use std::sync::Arc;

use hikasa_core::Side;
use hikasa_hex::{DisplayFilter, Radix, ValueFormat};
use hikasa_hex::{ThumbBucket, Thumbnail};
use hikasa_session::api::HexValues;
use hikasa_session::clipboard::FixedClipboard;

use crate::commands::{Area, CommandId};
use crate::hex::thumbnail::{ThumbClass, pixel_rows};
use crate::hex::values::ValuesChoice;
use crate::menus::{Menu, MenuTitle, render_text};
use crate::tests_hex_open::{bytes, hex_model, hex_world, open_pair, until, with_insert};
use crate::window::{CommandArg, Invocation};

/// A13.8: the value panel at the caret, a gap and the last bytes; every column choice and radix through the
/// submenus; a job when the bytes are not cached.
#[test]
fn a13_8_the_value_panel() {
    let world = hex_world();
    let mut app = world.app(None);
    let a = bytes(91, 4096);
    let b = with_insert(&a, 100, 4, 2000);
    let view = open_pair(&world, &mut app, &a, &b);
    let m = hex_model(&mut app, view);
    assert!(
        m.value_lines().is_none(),
        "hidden by default (`[view] values = none`)"
    );
    app.execute(Invocation::with(
        CommandId::LittleEndianList,
        CommandArg::Values(ValuesChoice::Show(true)),
    ));
    app.execute(Invocation::with(
        CommandId::BigEndianList,
        CommandArg::Values(ValuesChoice::Show(false)),
    ));
    let check = |m: &crate::hex::HexViewModel, side: Side, offset: u64, radix: Radix| {
        let doc = if side == Side::Right {
            &m.opened().expect("o").documents.right
        } else {
            &m.opened().expect("o").documents.left
        };
        let v = hikasa_hex::values::at(doc, offset).expect("values");
        let fmt = ValueFormat {
            little: m.display().values.little(),
            big: m.display().values.big(),
            radix,
        };
        assert_eq!(
            m.value_lines().map(|(t, _)| t.clone()),
            Some(hikasa_hex::values::format_with(&v, fmt))
        );
    };
    let m = hex_model(&mut app, view);
    assert_eq!(m.display().values, HexValues::Both);
    m.set_caret(50, false);
    check(m, Side::Left, 50, Radix::Decimal);
    // A gap of the left side: its next byte.
    let src = m.source().expect("s").clone();
    let col = src
        .view()
        .segments()
        .column_at_offset(Side::Right, 101)
        .expect("gap col");
    m.set_caret(col, false);
    let (offset, held) = m.byte_at(Side::Left, col).expect("byte");
    assert!(!held);
    check(m, Side::Left, offset, Radix::Decimal);
    // The last bytes: fewer than 8 remain.
    m.focus_side(Side::Right);
    m.set_caret(m.columns() - 3, false);
    check(m, Side::Right, b.len() as u64 - 3, Radix::Decimal);
    app.execute(Invocation::with(
        CommandId::BigEndianList,
        CommandArg::Values(ValuesChoice::Radix(Radix::Hex)),
    ));
    let m = hex_model(&mut app, view);
    check(m, Side::Right, b.len() as u64 - 3, Radix::Hex);
    app.execute(Invocation::with(
        CommandId::LittleEndianList,
        CommandArg::Values(ValuesChoice::Show(true)),
    ));
    let m = hex_model(&mut app, view);
    assert_eq!(m.display().values, HexValues::Big);
    check(m, Side::Right, b.len() as u64 - 3, Radix::Hex);
    // Paged documents: the values of an uncached place come from a job (the lines stay, greyed, meanwhile).
    let x = bytes(92, 2 << 20);
    let view = open_pair(&world, &mut app, &x, &x);
    let m = hex_model(&mut app, view);
    let mut r = m.request().clone();
    r.rules.unlocked_max_mib = Some("0".into());
    m.relive(r);
    world.settle(&mut app);
    let m = hex_model(&mut app, view);
    m.set_values(HexValues::Little);
    m.set_caret(1_500_000, false);
    until(&world, &mut app, |app| {
        hex_model(app, view)
            .value_lines()
            .is_some_and(|(_, stale)| !stale)
    });
    check(
        hex_model(&mut app, view),
        Side::Left,
        1_500_000,
        Radix::Decimal,
    );
}

/// A13.8: the thumbnail's pixel rows for 50, 4,096 and 5,000 pixels from a known set of buckets (a golden), and the
/// precedence different > orphan > same. Mutation: orphan over different.
#[test]
fn a13_8_the_thumbnail_rows() {
    let mut buckets = vec![ThumbBucket::default(); 4096];
    for (i, b) in buckets.iter_mut().enumerate() {
        b.same = 10;
        if i % 97 == 3 {
            b.different = 1;
        }
        if i % 61 == 7 {
            b.left_only = 2;
        }
        if i % 500 == 400 {
            b.right_only = 1;
            b.different = 1;
        }
        if i >= 4000 {
            *b = ThumbBucket::default();
        }
    }
    let t = Thumbnail {
        columns: 1 << 20,
        buckets,
    };
    let mut out = String::new();
    for h in [50u32, 4096, 5000] {
        let rows = pixel_rows(&t, h);
        assert_eq!(rows.len(), h as usize);
        let code = |c: &ThumbClass| match c {
            ThumbClass::Empty => ' ',
            ThumbClass::Same => '.',
            ThumbClass::Orphan => 'o',
            ThumbClass::Different => 'X',
        };
        out.push_str(&format!("== {h}\n"));
        for chunk in rows.chunks(100) {
            out.push_str(&chunk.iter().map(code).collect::<String>());
            out.push('\n');
        }
    }
    crate::tests_shell::golden("hex-thumbnail.txt", &out);
    // At 4,096 pixels each row is one bucket: the precedence is exact.
    let rows = pixel_rows(&t, 4096);
    assert_eq!(rows[400], ThumbClass::Different, "different over orphan");
    assert_eq!(rows[7], ThumbClass::Orphan);
    assert_eq!(rows[0], ThumbClass::Same);
    assert_eq!(rows[4050], ThumbClass::Empty);
}

/// A13.8: a click on the thumbnail scrolls to the column of that place (under a filter the nearest shown row).
#[test]
fn a13_8_thumbnail_clicks_scroll() {
    let world = hex_world();
    let mut app = world.app(None);
    let a = bytes(93, 64 * 1024);
    let view = open_pair(&world, &mut app, &a, &a);
    let m = hex_model(&mut app, view);
    m.set_filter(DisplayFilter::All);
    m.frame(40, 0.0);
    m.thumbnail_to(0.5);
    let rows = m.row_count();
    let want_col = m.columns() / 2;
    let row = m.row_of_column(want_col).expect("row");
    assert_eq!(
        m.screen.top,
        row.saturating_sub(20).min(m.screen.max_top(rows))
    );
    m.thumbnail_to(1.0);
    assert_eq!(m.screen.top, m.screen.max_top(rows));
}

/// A13.8: the headers' facts from `HexSide` (size, time, load, Converted, format), every status item, and Info's
/// figures with its Copy equal to the core's summary and verdict.
#[test]
fn a13_8_header_status_and_info() {
    let world = hex_world();
    let mut app = world.app(None);
    let clip = Arc::new(FixedClipboard::text("clip"));
    app.0.set_script_clipboard(Arc::clone(&clip) as _);
    let a = bytes(94, 3000);
    let b = with_insert(&a, 10, 2, 1000);
    let view = open_pair(&world, &mut app, &a, &b);
    let m = hex_model(&mut app, view);
    let facts = m.header_facts(Side::Right).expect("facts");
    assert_eq!(facts.size, "3,002 bytes");
    assert_eq!(facts.load, "in memory");
    assert!(!facts.converted);
    assert_eq!(facts.format.as_deref(), Some("Firmware"));
    assert!(!facts.modified.is_empty());
    assert!(facts.line().starts_with("3,002 bytes · "));
    // The status bar.
    m.set_filter(DisplayFilter::All);
    m.frame(20, 0.0);
    m.set_caret(5, false);
    m.key(crate::hex::pane::CaretKey::Right, true);
    let s = crate::hex::status::items(m);
    assert!(
        s.left.starts_with("Left 0x0000_0006 · right 0x0000_0006"),
        "{}",
        s.left
    );
    assert!(
        s.right
            .contains(&crate::hex::status::state_text(hikasa_hex::ByteState::Same).to_owned()),
        "{:?}",
        s.right
    );
    assert!(
        s.right.contains(&"2 bytes selected".to_owned()),
        "{:?}",
        s.right
    );
    assert!(s.right.contains(&"Fast".to_owned()), "{:?}", s.right);
    assert!(
        s.right.contains(&"Binary differences".to_owned()),
        "{:?}",
        s.right
    );
    // A gap column marks the address.
    let col = m
        .source()
        .expect("s")
        .view()
        .segments()
        .column_at_offset(Side::Right, 10)
        .expect("col");
    m.set_caret(col, false);
    assert!(m.address_of(Side::Left).expect("a").ends_with("(gap)"));
    // Info.
    app.execute(CommandId::Info);
    let w = app.win();
    let d = app
        .0
        .window_mut(w)
        .and_then(|w| w.dialogs.find_mut::<crate::info_dialog::InfoDialog>())
        .expect("Info");
    let text = d.text();
    assert!(text.starts_with("Hex Compare Info"), "{text}");
    assert!(text.contains("Differing"));
    app.0.dialog_button(w, crate::info_dialog::COPY);
    let copied = clip.payloads().last().expect("copied").text.clone();
    let r = hex_model(&mut app, view).response().expect("r").clone();
    let mut summary = r.clone();
    summary.display = None;
    let lines = summary.human_lines().expect("lines");
    assert_eq!(
        copied,
        format!("{}\nBinary differences\n", lines.join("\n"))
    );
}

/// A13.8: the context-menu goldens of the hex areas (A9.21's form).
#[test]
fn a13_8_context_menu_goldens() {
    let world = hex_world();
    let mut app = world.app(None);
    let a = bytes(95, 300);
    let view = open_pair(&world, &mut app, &a, &a);
    let _ = view;
    let w = app.win();
    let mut out = String::new();
    for (name, area) in [
        ("hex pane, left", Area::HexPane(Side::Left)),
        ("hex pane, right", Area::HexPane(Side::Right)),
        ("thumbnail", Area::HexThumbnail),
        ("values", Area::HexValues),
        ("file header, left", Area::FileHeader(Side::Left)),
    ] {
        out.push_str(&format!("## {name}\n"));
        out.push_str(&render_text(&[Menu {
            title: MenuTitle::Edit,
            items: app.0.pane_menu(w, area),
        }]));
    }
    crate::tests_shell::golden("menus/hex-areas.txt", &out);
}

/// A13.8: a file dropped on a pane replaces that side (a reopen); two files replace both.
#[test]
fn a13_8_drops() {
    let world = hex_world();
    let mut app = world.app(None);
    let a = bytes(96, 300);
    let view = open_pair(&world, &mut app, &a, &a);
    let other = world.path("work/h/other.bin");
    std::fs::write(&other, bytes(97, 200)).expect("write");
    let w = app.win();
    app.0.drop_paths(
        w,
        crate::dnd::DropTarget::PanePath(Side::Right),
        vec![other.clone()],
    );
    world.settle(&mut app);
    let m = hex_model(&mut app, view);
    assert_eq!(m.inputs()[1], other.display().to_string());
    assert_eq!(m.opened().expect("o").documents.right.len(), 200);
    let third = world.path("work/h/third.bin");
    std::fs::write(&third, bytes(98, 100)).expect("write");
    app.0.drop_paths(
        w,
        crate::dnd::DropTarget::Window,
        vec![third.clone(), other.clone()],
    );
    world.settle(&mut app);
    let m = hex_model(&mut app, view);
    assert_eq!(
        m.inputs(),
        [third.display().to_string(), other.display().to_string()]
    );
    let dirty = app
        .tab(view)
        .and_then(|t| t.session.as_ref())
        .is_some_and(|s| s.dirty);
    assert!(dirty, "a spec changed: the session is dirty");
}
