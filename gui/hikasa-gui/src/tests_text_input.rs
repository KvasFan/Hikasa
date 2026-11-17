//! A11.3 (D-296): typed text and the IME. `Text` events (ASCII, AltGr results, a dead-key result, several characters
//! in one event) are one `Type` each; IME preedit leaves the document alone, a commit is one step; the IME output is
//! set while a pane has the keyboard (the window's frame); overwrite at a line end and over a combining mark; the
//! dispatch of the window's keys with a text pane focused (Ctrl+C/X/V/A/Z/Y to the pane's commands, Ctrl+N to the
//! view, a find-panel field keeping Ctrl+C). P-TYPE: random text events equal `plan_command(Type)` applied directly.

use hikasa_core::Side;
use hikasa_session::settings::Shortcut;
use hikasa_text::edit::{EditCommand, GroupHint, plan_command};

use crate::commands::{CommandId, FocusArea};
use crate::tests_text::{Fixture, Rng};
use crate::text::input::{self, Ime, Key};

#[test]
fn a11_3_text_events_are_one_type_each() {
    let mut f = Fixture::new("abc\n", "abc\n");
    f.place(Side::Left, 0, 3);
    f.type_text("@"); // an AltGr result
    f.type_text("é"); // a dead key's result
    f.type_text("xyz"); // several characters in one event
    assert_eq!(f.text(Side::Left), "abc@éxyz\n");
    // Control characters never arrive as text.
    f.type_text("\u{8}");
    assert_eq!(f.text(Side::Left), "abc@éxyz\n");
}

#[test]
fn a11_3_the_ime_composes_without_touching_the_document() {
    let mut f = Fixture::new("abc\n", "abc\n");
    f.place(Side::Left, 0, 1);
    let before = f.doc(Side::Left).generation();
    input::ime(f.m(), Ime::Enabled);
    input::ime(f.m(), Ime::Preedit("に".into()));
    input::ime(f.m(), Ime::Preedit("にほ".into()));
    assert_eq!(
        f.doc(Side::Left).generation(),
        before,
        "the preedit is drawn only"
    );
    assert!(input::composing(f.m()));
    // Keys are the IME's while it composes.
    assert!(f.plain(Key::Left));
    assert_eq!(f.caret(Side::Left).byte, 1);
    input::ime(f.m(), Ime::Commit("日本".into()));
    assert!(!input::composing(f.m()));
    assert_eq!(f.text(Side::Left), "a日本bc\n");
    f.run(CommandId::Undo);
    assert_eq!(f.text(Side::Left), "abc\n", "one step");
    input::ime(f.m(), Ime::Preedit("x".into()));
    input::ime(f.m(), Ime::Preedit(String::new()));
    assert!(!input::composing(f.m()), "an empty preedit dismisses");
}

#[test]
fn a11_3_overwrite_never_eats_a_terminator_or_half_a_cluster() {
    let mut f = Fixture::new("ae\u{301}c\nz\n", "x\n");
    f.place(Side::Left, 0, 1);
    f.plain(Key::Insert);
    assert!(f.m().display().overwrite);
    f.type_text("X");
    assert_eq!(
        f.text(Side::Left),
        "aXc\nz\n",
        "the cluster e+◌́ is replaced whole"
    );
    f.place(Side::Left, 0, 3);
    f.type_text("Y");
    assert_eq!(
        f.text(Side::Left),
        "aXcY\nz\n",
        "at the line's end it inserts"
    );
    f.plain(Key::Insert);
    assert!(!f.m().display().overwrite);
}

#[test]
fn a11_3_window_keys_reach_the_panes_commands() {
    let mut f = Fixture::new("abc\n", "abd\n");
    let key = |s: &str| Shortcut::parse(s).expect("key");
    let at = |f: &Fixture, s: &str, area| f.app.dispatch_key(f.app.win(), &key(s), area);
    f.place(Side::Left, 0, 0);
    assert_eq!(at(&f, "Ctrl+C", FocusArea::Root), Some(CommandId::Copy));
    assert_eq!(at(&f, "Ctrl+X", FocusArea::Root), Some(CommandId::Cut));
    assert_eq!(at(&f, "Ctrl+V", FocusArea::Root), Some(CommandId::Paste));
    assert_eq!(
        at(&f, "Ctrl+A", FocusArea::Root),
        Some(CommandId::SelectAll)
    );
    assert_eq!(
        at(&f, "Ctrl+N", FocusArea::Root),
        Some(CommandId::DiffSectionNext)
    );
    // Nothing to undo yet: Ctrl+Z is not available.
    assert_eq!(at(&f, "Ctrl+Z", FocusArea::Root), None);
    f.type_text("q");
    assert_eq!(at(&f, "Ctrl+Z", FocusArea::Root), Some(CommandId::Undo));
    // A focused text field (the find panel) keeps Ctrl+C (D-252).
    assert_eq!(at(&f, "Ctrl+C", FocusArea::TextField), None);
    // Letters and Enter are no window command: the pane takes them.
    assert_eq!(at(&f, "Enter", FocusArea::Root), None);
}

/// P-TYPE: random text events on random places equal `plan_command(Type)` applied to a copy of the document.
#[test]
fn p_type_random_text_equals_plan_command() {
    let cases: u64 = std::env::var("HIKASA_HEAVY").ok().map_or(100, |_| 2_000);
    let mut f = Fixture::new("one\ttwo\nthree é\n\nfour\r\n", "x\n");
    let mut rng = Rng(0x5EED_7E9E);
    // Tab arrives as a key, never as text (§1.11.5).
    let pieces = ["a", "é", "日本", " ", "👩\u{200d}💻", "xy"];
    for i in 0..cases {
        let doc = f.doc(Side::Left);
        let line = rng.below(u64::from(doc.position_lines())) as u32;
        let len = doc.line_len(hikasa_text::LineIdx(line));
        let byte = doc
            .clamp(hikasa_text::edit::TextPos::new(
                line,
                rng.below(u64::from(len) + 1) as u32,
            ))
            .byte;
        f.place(Side::Left, line, byte);
        let text = pieces[rng.below(pieces.len() as u64) as usize];
        let overwrite = rng.below(4) == 0;
        if overwrite != f.m().display().overwrite {
            f.plain(Key::Insert);
        }
        let mut expected = f.doc(Side::Left);
        let sel = f.m().pane(Side::Left).selection();
        let (req, _) = plan_command(
            &expected,
            sel,
            &EditCommand::Type {
                text: text.to_owned(),
                overwrite,
            },
        )
        .expect("a plan");
        expected.apply(&req, GroupHint::None).expect("apply");
        f.type_text(text);
        assert_eq!(
            f.text(Side::Left),
            expected.text(hikasa_text::edit::TextPos::ZERO..hikasa_text::edit::doc_end(&expected)),
            "step {i}"
        );
    }
}
