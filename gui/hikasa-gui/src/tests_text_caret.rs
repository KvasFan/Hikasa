//! A11.2 (D-295): caret, selection, word wrap with `FixedMetrics`: §1.11.4's keyboard rows on both panes, wrap on and
//! off, gap rows skipped by Up/Down, grapheme steps over combining marks, ZWJ emoji and CRLF, smart Home, the sticky
//! x across short lines, a 50,000-character line edited at its end with wrap off (layout window) and on (256 visual
//! lines, then the mark); the pointer rows through the model (`input::press`, `drag`, gutter, middle click).
//! P-CARET: random key and click sequences keep the caret and the anchor at char boundaries inside the document.

use hikasa_core::Side;
use hikasa_text::edit::TextPos;

use crate::commands::CommandId;
use crate::tests_text::{Fixture, Rng};
use crate::text::input::{self, Clicks, Key, Press};
use crate::text::wrap::{FixedMetrics, LONG_LINE_COLUMNS, MAX_VISUAL_LINES};

fn pos(line: u32, byte: u32) -> TextPos {
    TextPos::new(line, byte)
}

#[test]
fn a11_2_the_keyboard_rows_move_the_caret() {
    let left = "  alpha beta\nx\ngamma delta epsilon\n";
    let mut f = Fixture::new(left, left);
    for side in [Side::Left, Side::Right] {
        f.place(side, 0, 0);
        f.plain(Key::Right);
        assert_eq!(f.caret(side), pos(0, 1));
        f.plain(Key::Left);
        assert_eq!(f.caret(side), pos(0, 0));
        f.key(Press::ctrl(Key::Right));
        assert_eq!(
            f.caret(side),
            pos(0, 2),
            "core's word boundary: the word's start"
        );
        f.key(Press::ctrl(Key::Right));
        assert_eq!(f.caret(side), pos(0, 8), "then the next word's start");
        f.key(Press::ctrl(Key::Left));
        assert_eq!(f.caret(side), pos(0, 2), "the word's start");
        f.plain(Key::Home);
        assert_eq!(
            f.caret(side),
            pos(0, 0),
            "smart Home: column 0 after the first non-blank"
        );
        f.plain(Key::Home);
        assert_eq!(f.caret(side), pos(0, 2), "then the first non-blank");
        f.plain(Key::End);
        assert_eq!(f.caret(side), pos(0, 12));
        // Sticky x across a short line.
        f.plain(Key::Down);
        assert_eq!(f.caret(side), pos(1, 1));
        f.plain(Key::Down);
        assert_eq!(f.caret(side), pos(2, 12), "the sticky x comes back");
        f.key(Press::ctrl(Key::End));
        assert_eq!(f.caret(side), pos(3, 0));
        f.key(Press::ctrl(Key::Home));
        assert_eq!(f.caret(side), pos(0, 0));
        // Shift extends; Esc collapses; Left with a selection goes to its start.
        f.key(Press::shift(Key::End));
        assert!(f.m().pane(side).has_selection());
        f.plain(Key::Left);
        assert_eq!(f.caret(side), pos(0, 0));
        assert!(!f.m().pane(side).has_selection());
        f.key(Press::shift(Key::Down));
        f.plain(Key::Escape);
        assert!(!f.m().pane(side).has_selection());
        f.plain(Key::PageDown);
        assert_eq!(f.caret(side).line.0, 3);
        f.plain(Key::PageUp);
        assert_eq!(f.caret(side).line.0, 0);
    }
    // Ctrl+A selects the focus pane's document.
    f.run(CommandId::SelectAll);
    let sel = f.m().pane(Side::Right).selection().range();
    assert_eq!((sel.start, sel.end), (pos(0, 0), pos(3, 0)));
}

#[test]
fn a11_2_grapheme_steps_and_gap_rows() {
    let left = "e\u{301}x👩\u{200d}💻y\r\nnext\r\n";
    let right = "e\u{301}x👩\u{200d}💻y\r\nonly right\r\nnext\r\n";
    let mut f = Fixture::new(left, right);
    f.place(Side::Left, 0, 0);
    f.plain(Key::Right);
    assert_eq!(f.caret(Side::Left), pos(0, 3), "over the combining mark");
    f.plain(Key::Right);
    f.plain(Key::Right);
    assert_eq!(f.caret(Side::Left), pos(0, 15), "over the ZWJ sequence");
    f.plain(Key::Right);
    f.plain(Key::Right);
    assert_eq!(f.caret(Side::Left), pos(1, 0), "CRLF is one step");
    f.plain(Key::Left);
    assert_eq!(f.caret(Side::Left), pos(0, 16));
    // Up/Down walk the pane's own lines: the right side's orphan row is a gap of the left one.
    f.place(Side::Left, 0, 0);
    f.plain(Key::Down);
    assert_eq!(f.caret(Side::Left), pos(1, 0), "the gap row is skipped");
}

#[test]
fn a11_2_word_wrap_and_long_lines() {
    let long = "x".repeat(50_000);
    let text = format!("{long}\nshort\n");
    let mut f = Fixture::new(&text, &text);
    let metrics = FixedMetrics::default();
    // Wrap off: the layout window follows the caret at the line's end; an edit there lands.
    f.place(Side::Left, 0, 0);
    f.plain(Key::End);
    assert_eq!(f.caret(Side::Left), pos(0, 50_000));
    let g = f.m().line_geom(Side::Left, 0, &metrics);
    assert_eq!(g.bounds.len(), LONG_LINE_COLUMNS + 1);
    assert!(g.cut_before);
    assert_eq!(g.caret(50_000).1, 50_000.0, "x from the line's start");
    f.type_text("!");
    assert_eq!(f.doc(Side::Left).line_len(hikasa_text::LineIdx(0)), 50_001);
    // Wrap on: 256 visual lines of the caret's window, then the mark.
    f.run(CommandId::WordWrap);
    f.m().set_text_width(Side::Left, 80.0);
    f.place(Side::Left, 0, 0);
    let g = f.m().line_geom(Side::Left, 0, &metrics);
    assert_eq!(g.visuals.len(), MAX_VISUAL_LINES);
    assert!(g.continues);
    // Down walks the visual lines at the sticky x.
    f.place(Side::Left, 0, 5);
    // A visual line of width 80 holds 79 columns (the caret fits at its end).
    f.plain(Key::Down);
    assert_eq!(f.caret(Side::Left), pos(0, 84));
    f.plain(Key::End);
    assert_eq!(f.caret(Side::Left), pos(0, 157), "the visual line's end");
    let screen = f.m().screen(0, 10, &metrics);
    assert_eq!(
        screen[0].height,
        MAX_VISUAL_LINES as u32 + 1,
        "the row is as tall as its visual lines"
    );
}

#[test]
fn a11_2_the_pointer_places_selects_and_drags() {
    let text = "alpha beta\nsecond line\nthird\n";
    let mut f = Fixture::new(text, text);
    let metrics = FixedMetrics::default();
    let screen = f.m().screen(0, 10, &metrics);
    let hit = f.m().hit(Side::Right, &screen, 1.5, 3.2).expect("a hit");
    assert_eq!(hit, pos(1, 3));
    input::press(f.m(), Side::Right, hit, false, Clicks::One);
    assert_eq!(f.m().focus(), Side::Right);
    input::drag(f.m(), pos(2, 2));
    let sel = f.m().pane(Side::Right).selection().range();
    assert_eq!((sel.start, sel.end), (pos(1, 3), pos(2, 2)));
    input::release(f.m());
    f.settle();
    assert_eq!(
        f.clip.primaries().last().map(String::as_str),
        Some("ond line\nth")
    );
    // Shift+click extends; double click selects the word; triple click the line with its terminator.
    input::press(f.m(), Side::Right, pos(0, 0), true, Clicks::One);
    assert_eq!(f.m().pane(Side::Right).anchor, pos(1, 3));
    input::press(f.m(), Side::Right, pos(0, 7), false, Clicks::Two);
    let sel = f.m().pane(Side::Right).selection().range();
    assert_eq!((sel.start, sel.end), (pos(0, 6), pos(0, 10)));
    input::press(f.m(), Side::Right, pos(1, 2), false, Clicks::Three);
    let sel = f.m().pane(Side::Right).selection().range();
    assert_eq!((sel.start, sel.end), (pos(1, 0), pos(2, 0)));
    // The gutter selects lines, a drag there more.
    input::gutter_press(f.m(), Side::Left, 0, false);
    input::gutter_drag(f.m(), 1);
    let sel = f.m().pane(Side::Left).selection().range();
    assert_eq!((sel.start, sel.end), (pos(0, 0), pos(2, 0)));
    // Below the last row: the document's end; a gap row: the next line of that side.
    let below = f.m().hit(Side::Left, &screen, 40.0, 0.0).expect("hit");
    assert_eq!(below, pos(3, 0));
}

/// P-CARET: random keys and clicks keep the caret and the anchor inside the document at grapheme cluster starts (or a
/// line's end), never inside a cluster (D-295).
#[test]
fn p_caret_random_moves_stay_inside_the_document() {
    let cases: u64 = std::env::var("HIKASA_HEAVY").ok().map_or(200, |_| 10_000);
    let text = "a\u{301}b\t中文\r\nline two 😀\n\n  indented\rcr\n\u{1F1EE}\u{1F1F9}x 👨\u{200D}👩\u{200D}👧 é\nlast";
    let other = "line two 😀\nextra\n  indented\rcr\n";
    let mut f = Fixture::new(text, other);
    let metrics = FixedMetrics::default();
    let mut rng = Rng(0x5EED_CA2E);
    let mut last = String::new();
    let keys = [
        Key::Left,
        Key::Right,
        Key::Up,
        Key::Down,
        Key::Home,
        Key::End,
        Key::PageUp,
        Key::PageDown,
    ];
    for i in 0..cases {
        if i % 50 == 0 {
            let wrap = f.m().display().word_wrap;
            if rng.below(3) == 0 {
                f.run(CommandId::WordWrap);
                assert_ne!(wrap, f.m().display().word_wrap);
                f.m().set_text_width(Side::Left, 4.0 + rng.below(8) as f32);
                f.m().set_text_width(Side::Right, 4.0 + rng.below(8) as f32);
            }
        }
        if rng.below(5) == 0 {
            let side = if rng.below(2) == 0 {
                Side::Left
            } else {
                Side::Right
            };
            let screen = f.m().screen(0, 20, &metrics);
            let y = rng.below(120) as f32 / 10.0;
            let x = rng.below(200) as f32 / 10.0;
            if let Some(p) = f.m().hit(side, &screen, y, x) {
                last = format!("click {side:?} {p:?}");
                input::press(f.m(), side, p, rng.below(2) == 0, Clicks::One);
            }
        } else {
            let press = Press {
                key: keys[rng.below(keys.len() as u64) as usize],
                shift: rng.below(2) == 0,
                ctrl: rng.below(4) == 0,
            };
            let focus = f.m().focus();
            last = format!("{press:?} on {focus:?} from {:?}", f.caret(focus));
            f.key(press);
        }
        for side in [Side::Left, Side::Right] {
            let doc = f.doc(side);
            let pane = f.m().pane(side).clone();
            for p in [pane.caret, pane.anchor] {
                assert_eq!(
                    doc.clamp(p),
                    p,
                    "step {i}: {p:?} not a boundary of {side:?}"
                );
                let starts = hikasa_text::edit::cluster_starts(&doc.line(p.line));
                assert!(
                    starts.contains(&p.byte),
                    "step {i} ({last}): {p:?} inside a cluster of {side:?} ({starts:?})"
                );
            }
        }
        // The caret's row is on screen after the reveal.
        let side = f.m().focus();
        assert!(f.m().caret_pos(side).is_some(), "step {i}");
    }
}

/// F7: Down at a sticky x and a click land at a cluster's start, never between a flag's two regional indicators or
/// inside a ZWJ sequence.
#[test]
fn a11_2_vertical_moves_and_clicks_land_on_clusters() {
    let mut f = Fixture::new("abcd\n\u{1F1EE}\u{1F1F9}x\n", "abcd\n");
    f.place(Side::Left, 0, 1);
    f.plain(Key::Down);
    let c = f.caret(Side::Left);
    assert!(c.byte == 0 || c.byte == 8, "{c:?}");
    let line = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}x";
    let starts = hikasa_text::edit::cluster_starts(line);
    for wrap in [None, Some(80.0)] {
        let g = crate::text::wrap::LineGeom::new(line, 4, &FixedMetrics::default(), wrap, 0);
        for x in 0..8 {
            let b = g.hit(0, x as f32);
            assert!(
                starts.contains(&b),
                "wrap {wrap:?} x {x}: byte {b} ({starts:?})"
            );
        }
    }
}
