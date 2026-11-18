//! A13.7 (M2 W13a, D-359): the Find panel — text in each encoding, match case on and off, hex with `??`, both
//! directions, wrap, hits across page and chunk edges equal `HexFinder::next` (and the offsets `hikasa hex find`
//! reports, which is the same finder over the same document); refused patterns greyed with the core's message; a
//! newer search cancels the older; Esc cancels; the hit's selection and caret. Mutation: the search starts at the
//! caret instead of after it (the same hit found again).

use hikasa_core::Side;
use hikasa_hex::Direction;

use crate::commands::CommandId;
use crate::tests_hex_open::{bytes, hex_model, hex_world, open_pair, until};

/// Bytes with planted texts: `Hello` (UTF-8) at 1,000, `HELLO` in UTF-16 LE at 70,000 (across the 64 KiB page edge
/// at 65,536 + 4,464), `de ad ?? ef` at 65,534 (across the first page edge), `hello` again at 120,000.
fn planted() -> Vec<u8> {
    let mut a = bytes(81, 140_000);
    for b in &mut a {
        // No accidental letters: bytes ≥ 0x80 only.
        *b |= 0x80;
    }
    let put =
        |a: &mut Vec<u8>, at: usize, what: &[u8]| a[at..at + what.len()].copy_from_slice(what);
    put(&mut a, 1000, b"Hello");
    let utf16: Vec<u8> = "HELLO".encode_utf16().flat_map(u16::to_le_bytes).collect();
    put(&mut a, 70_000, &utf16);
    put(&mut a, 65_534, &[0xDE, 0xAD, 0x00, 0xEF]);
    put(&mut a, 120_000, b"hello");
    a
}

fn hit_of(m: &crate::hex::HexViewModel) -> Option<std::ops::Range<u64>> {
    m.find_hit().map(|(_, r)| r.clone())
}

/// A13.7: every kind of query equals `HexFinder::next` from the caret's byte + 1 (forward) or the caret's byte
/// (backward), with wrap; the hit's bytes selected, the caret at its start.
#[test]
fn a13_7_find_equals_the_core_finder() {
    let world = hex_world();
    let mut app = world.app(None);
    let a = planted();
    let view = open_pair(&world, &mut app, &a, &a);
    // Paged documents: the hits read across page edges.
    let m = hex_model(&mut app, view);
    let mut r = m.request().clone();
    r.rules.unlocked_max_mib = Some("0".into());
    m.relive(r);
    world.settle(&mut app);
    app.execute(CommandId::Find);
    let m = hex_model(&mut app, view);
    assert!(m.find.open);
    let cases: [(&str, bool, &str, bool); 6] = [
        ("hello", false, "utf-8", false),
        ("Hello", false, "utf-8", true),
        ("hello", false, "utf-16le", false),
        ("HELLO", false, "latin1", true),
        ("de ad ?? ef", true, "utf-8", false),
        ("ad 00", true, "utf-8", false),
    ];
    for (pattern, hex, encoding, case) in cases {
        let m = hex_model(&mut app, view);
        m.find.pattern = pattern.into();
        m.find.hex = hex;
        m.find.encoding = crate::hex::find::ENCODINGS
            .iter()
            .find(|(w, _)| *w == encoding)
            .expect("encoding")
            .0;
        m.find.match_case = case;
        m.find.wrap = true;
        m.validate_find();
        let finder = m.find.finder().expect("valid").clone();
        let doc = std::sync::Arc::clone(&m.opened().expect("o").documents.left);
        m.set_caret(0, false);
        for forward in [true, true, true, false, false] {
            let m = hex_model(&mut app, view);
            let at = m
                .byte_at(Side::Left, m.pane(Side::Left).caret)
                .expect("b")
                .0;
            let from = if forward { at + 1 } else { at };
            let dir = if forward {
                Direction::Forward
            } else {
                Direction::Backward
            };
            let want = finder
                .next(
                    &doc,
                    from,
                    dir,
                    true,
                    &world.service.stream_options(),
                    &hikasa_core::Ctl::detached(),
                )
                .expect("find");
            app.execute(if forward {
                CommandId::FindNext
            } else {
                CommandId::FindPrev
            });
            world.settle(&mut app);
            let m = hex_model(&mut app, view);
            assert_eq!(hit_of(m), want.clone(), "{pattern} {forward}");
            if let Some(range) = want {
                let start = m
                    .source()
                    .expect("s")
                    .view()
                    .segments()
                    .column_at_offset(Side::Left, range.start)
                    .expect("col");
                assert_eq!(
                    m.pane(Side::Left).caret,
                    start,
                    "the caret at the hit's start"
                );
                assert_eq!(
                    m.selection().end - m.selection().start,
                    range.end - range.start
                );
            }
        }
    }
    // The MRU of the panel's patterns.
    assert!(
        app.0
            .state_mru_for_test(hikasa_session::settings::state::MruList::HexFind)
            .contains(&"ad 00".to_owned())
    );
}

/// A13.7: refused patterns are the panel's message and grey its buttons.
#[test]
fn a13_7_refused_patterns() {
    let world = hex_world();
    let mut app = world.app(None);
    let a = bytes(82, 1000);
    let view = open_pair(&world, &mut app, &a, &a);
    let m = hex_model(&mut app, view);
    m.open_find();
    for (pattern, hex) in [("de a", true), ("?? ??", true), ("zz", true), ("é", false)] {
        m.find.pattern = pattern.into();
        m.find.hex = hex;
        m.find.encoding = if hex { "utf-8" } else { "latin1" };
        if !hex {
            m.find.pattern = "€".into();
        }
        m.validate_find();
        assert!(m.find.finder().is_none(), "{pattern}");
        assert!(m.find.message.is_some(), "{pattern}");
        let query = m.find.query();
        assert_eq!(
            m.find.message.clone(),
            world
                .service
                .hex_finder(&query)
                .err()
                .map(|e| e.to_string())
        );
    }
    m.find.pattern = String::new();
    m.validate_find();
    assert!(
        m.find.finder().is_none() && m.find.message.is_none(),
        "empty: grey, no message"
    );
}

/// A13.7: a newer search cancels the older; Esc (nothing selected) cancels the running one; *Not found* and
/// *Wrapped to the start*.
#[test]
fn a13_7_cancel_and_messages() {
    let world = hex_world();
    let mut app = world.app(None);
    let mut a = vec![0x41u8; 8 << 20];
    a[100] = 0x42;
    let view = open_pair(&world, &mut app, &a, &a);
    let m = hex_model(&mut app, view);
    m.open_find();
    m.find.pattern = "43 43 43 43 43 43 43 43".into();
    m.find.hex = true;
    m.validate_find();
    m.find_step(true);
    m.find_step(true);
    assert!(m.searching().is_some());
    m.escape();
    assert!(m.searching().is_none(), "Esc cancels within one poll");
    world.settle(&mut app);
    // Not found.
    hex_model(&mut app, view).find_step(true);
    until(&world, &mut app, |app| {
        hex_model(app, view).searching().is_none()
    });
    world.settle(&mut app);
    let msg = app
        .tab(view)
        .and_then(|t| t.message.clone())
        .expect("message");
    assert_eq!(msg.text, "Not found.");
    // Wrapped: from after the only hit, the next is it again, from the start.
    let m = hex_model(&mut app, view);
    m.find.pattern = "42".into();
    m.validate_find();
    m.set_caret(5000, false);
    m.find_step(true);
    until(&world, &mut app, |app| {
        hex_model(app, view).searching().is_none()
    });
    world.settle(&mut app);
    let m = hex_model(&mut app, view);
    assert_eq!(hit_of(m), Some(100..101));
    let msg = app
        .tab(view)
        .and_then(|t| t.message.clone())
        .expect("message");
    assert_eq!(msg.text, "Wrapped to the start.");
    // The next search starts after the caret: with wrap off nothing more is found (the same hit is not found again).
    let m = hex_model(&mut app, view);
    m.find.wrap = false;
    m.find_step(true);
    until(&world, &mut app, |app| {
        hex_model(app, view).searching().is_none()
    });
    world.settle(&mut app);
    let msg = app
        .tab(view)
        .and_then(|t| t.message.clone())
        .expect("message");
    assert_eq!(msg.text, "Not found.");
}
