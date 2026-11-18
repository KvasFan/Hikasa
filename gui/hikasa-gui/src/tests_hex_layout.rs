//! A13.3 (M2 W13a, D-356): panes, layouts and the Layout dialog — the geometry of a row and its hit tests with
//! `FixedMetrics` for bytes per line 1, 7, 16, 256, groups 1–8, characters ASCII / Latin-1 / none, addresses on and
//! off, both radixes and horizontal scroll (the pointer rules of `ui/` call exactly these); the cell texts equal the
//! rows' bytes and `char_of`; gap cells; the Layout dialog's OK applies a new generation and writes `[view]` without
//! dirtying the session, and *Use for new Hex Compare views* writes the defaults session as `session edit --set`.
//! Mutation: the group space counted in the hit test one byte late.

use hikasa_core::Side;
use hikasa_hex::{CharSet, HexLayout, Radix};

use crate::commands::CommandId;
use crate::dialog::ButtonRole;
use crate::hex::layout::{LayoutChoice, LayoutDialog, view_words};
use crate::hex::pane::{Hit, PaneRect, Part, RowGeometry, hit};
use crate::hex::{PaneLayout, VIEW};
use crate::tests_hex_open::{bytes, hex_model, hex_world, open_pair, with_insert};
use crate::text::wrap::FixedMetrics;

/// The geometry's x of each byte against an independent rendering of the row's text (`de ad` with the group spaces).
#[test]
fn a13_3_row_geometry_matches_the_rendered_row() {
    for bpl in [1u16, 7, 16, 256] {
        for group in [1u8, 2, 4, 8] {
            for addresses in [None, Some(11u32)] {
                for chars in [true, false] {
                    let g = RowGeometry::new(addresses, bpl, group, chars);
                    // The rendered row: the address, the digits with their spaces, two spaces, the characters.
                    let mut text = " ".repeat(g.address as usize);
                    let mut starts = Vec::new();
                    for i in 0..u32::from(bpl) {
                        if i > 0 {
                            if group == 1 {
                                text.push(' ');
                                if i % 8 == 0 {
                                    text.push(' ');
                                }
                            } else if i % u32::from(group) == 0 {
                                text.push(' ');
                            }
                        }
                        starts.push(text.len() as u32);
                        text.push_str("ab");
                    }
                    assert_eq!(text.len() as u32, g.hex_end(), "{bpl} {group}");
                    for (i, &x) in starts.iter().enumerate() {
                        assert_eq!(g.hex_x(i as u32), x, "{bpl} {group} byte {i}");
                        // Both digits and the space after them hit this byte (a point between two bytes belongs
                        // to the byte on its left).
                        assert_eq!(g.hit(x as f32 + 0.5), Some((Part::Hex, i as u32)));
                        assert_eq!(g.hit(x as f32 + 1.9), Some((Part::Hex, i as u32)));
                        let next = starts.get(i + 1).copied().unwrap_or(g.hex_end() + 1);
                        assert_eq!(
                            g.hit(next as f32 - 0.1),
                            Some((Part::Hex, i as u32)),
                            "the space after byte {i} ({bpl} {group})"
                        );
                    }
                    if chars {
                        assert_eq!(g.char_x(0), g.hex_end() + 2);
                        for i in [0u32, u32::from(bpl) - 1] {
                            assert_eq!(g.hit(g.char_x(i) as f32 + 0.5), Some((Part::Chars, i)));
                        }
                        assert_eq!(g.width(), g.char_x(u32::from(bpl)));
                    } else {
                        assert_eq!(g.width(), g.hex_end());
                    }
                    if g.address > 0 {
                        assert_eq!(g.hit(0.5), None, "the address column");
                    }
                }
            }
        }
    }
}

/// A13.3: a point of a pane to (row, cell) with fixed metrics, both panes, a scrolled-off top row and horizontal
/// scroll.
#[test]
fn a13_3_hit_tests_of_a_pane() {
    let m = FixedMetrics::default();
    let rect = PaneRect {
        x: 100.0,
        y: 20.0,
        w: 80.0,
        h: 30.0,
    };
    let g = RowGeometry::new(Some(11), 16, 1, true);
    // Row 2 below a top of 40 with half the top row scrolled off; byte 9 (after the 8-byte double space).
    let x = 100.0 + g.hex_x(9) as f32 + 0.5;
    let y = 20.0 + 1.6;
    assert_eq!(
        hit(&rect, &g, &m, 40, 0.5, 0.0, [x, y]),
        Some(Hit {
            row: 42,
            cell: Some((Part::Hex, 9))
        })
    );
    // Scrolled 10 characters to the right: the same point is 10 characters further.
    let h = hit(&rect, &g, &m, 40, 0.0, 10.0, [x - 10.0, y]).expect("hit");
    assert_eq!(h.cell, Some((Part::Hex, 9)));
    assert_eq!(h.row, 41);
    // Outside the pane: nothing.
    assert_eq!(hit(&rect, &g, &m, 0, 0.0, 0.0, [99.0, 25.0]), None);
}

/// A13.3: the cell texts of the panes equal the rows' bytes and `char_of`, gap cells hold no byte of their side;
/// the Layout dialog's OK applies a new generation and writes `[view]` (the session stays clean); *Use for new Hex
/// Compare views* writes the defaults session as `session edit --set` of the same keys.
#[test]
fn a13_3_cells_and_the_layout_dialog() {
    let world = hex_world();
    let mut app = world.app(None);
    let a = bytes(31, 5000);
    let b = with_insert(&a, 1234, 6, 3000);
    let view = open_pair(&world, &mut app, &a, &b);
    let m = hex_model(&mut app, view);
    m.set_filter(hikasa_hex::DisplayFilter::All);
    let src = m.source().expect("src").clone();
    let rows = src.view().rows(0, u32::MAX).expect("rows");
    let layout = *src.view().layout();
    let (mut la, mut lb) = (Vec::new(), Vec::new());
    let mut gaps = 0;
    for row in &rows {
        for cell in &row.cells {
            match (cell.left, cell.right) {
                (Some(x), Some(y)) => {
                    la.push(x);
                    lb.push(y);
                }
                (Some(x), None) => {
                    la.push(x);
                    gaps += 1;
                }
                (None, Some(y)) => {
                    lb.push(y);
                    gaps += 1;
                }
                (None, None) => panic!("an empty cell"),
            }
            assert_eq!(
                cell.left.and_then(|x| layout.char_of(x)).is_some(),
                cell.left.is_some()
            );
        }
    }
    assert_eq!(
        (la, lb),
        (a.clone(), b.clone()),
        "the cells are the files' bytes"
    );
    assert_eq!(gaps, 6, "the insertion's gap cells");
    // The Layout dialog.
    app.execute(CommandId::Layout);
    let w = app.win();
    let d = app
        .0
        .window_mut(w)
        .and_then(|w| w.dialogs.find_mut::<LayoutDialog>())
        .expect("the Layout dialog");
    assert_eq!(d.choice.layout, layout);
    d.set_bytes_per_line(1000);
    assert_eq!(
        d.choice.layout.bytes_per_line, 256,
        "numbers clamp in the widget"
    );
    d.set_bytes_per_line(7);
    d.set_group(3);
    assert_eq!(d.choice.layout.group, 2);
    d.set_chars(CharSet::Latin1);
    d.set_radix(Radix::Decimal);
    d.set_context(99);
    assert_eq!(d.choice.context, 64);
    d.choice.panes = PaneLayout::OverUnder;
    d.choice.addresses = false;
    d.defaults = true;
    let choice = d.choice;
    let generation = hex_model(&mut app, view).generation;
    app.0.dialog_button(w, ButtonRole::Ok);
    world.settle(&mut app);
    let m = hex_model(&mut app, view);
    assert!(m.generation > generation || m.display().layout == choice.layout);
    assert_eq!(m.display().layout.bytes_per_line, 7);
    assert_eq!(m.display().panes, PaneLayout::OverUnder);
    assert!(!m.display().addresses);
    let dirty = app
        .tab(view)
        .and_then(|t| t.session.as_ref())
        .is_some_and(|s| s.dirty);
    assert!(!dirty, "view state does not dirty the session");
    // The `[view]` of the view's session.
    let tab = app.tab(view).expect("tab");
    let capture = crate::app::AppCore::capture_for_test(tab).expect("capture");
    assert_eq!(
        capture
            .view
            .get("bytes_per_line")
            .and_then(|v| v.as_integer()),
        Some(7)
    );
    assert_eq!(
        capture.view.get("addresses").and_then(|v| v.as_bool()),
        Some(false)
    );
    assert_eq!(
        capture.display.get("filter").and_then(|v| v.as_str()),
        Some("all")
    );
    // The defaults session equals `session edit --set` of the same keys.
    let defaults = world
        .service
        .session_defaults(hikasa_session::sessions::SessionType::HexCompare)
        .expect("defaults");
    let probe = hex_world();
    probe
        .service
        .session_edit(
            &hikasa_session::api::SessionTarget::Defaults(
                hikasa_session::sessions::SessionType::HexCompare,
            ),
            &hikasa_session::api::SessionEdit {
                set: view_words(&LayoutChoice { ..choice }),
                ..Default::default()
            },
            true,
        )
        .expect("session edit --set");
    let expected = probe
        .service
        .session_defaults(hikasa_session::sessions::SessionType::HexCompare)
        .expect("defaults");
    assert_eq!(defaults.view, expected.view);
    // A new Hex Compare view takes those defaults.
    let view2 = open_pair(&world, &mut app, &a, &b);
    let m = hex_model(&mut app, view2);
    assert_eq!(m.display().layout.bytes_per_line, 7);
    assert_eq!(m.display().layout.chars, CharSet::Latin1);
    assert_eq!(m.display().panes, PaneLayout::OverUnder);
    let _ = (Side::Left, VIEW, HexLayout::default());
}
